//! The state machine: funding, unlock, flips, settlement, auto-return and
//! purging. `tick` is called every few seconds by [`super::spawn_worker`];
//! tests call it directly with a chosen clock.
//!
//! Ownership of rows: the worker owns `funding`, `flipping`, `settling` and
//! kept rows and rewrites them freely. Handlers may move `sealed` and
//! `unlocked` rows (flip, settle, keep), so for those the worker only uses
//! guarded updates.

use zandglass_core::drand;
use zandglass_core::heir::HeirCode;

use super::real::code_of;
use super::store::{Session, State as S};
use super::{Playground, AUTO_RETURN_SECS, KEEP_RETURN_AFTER_SECS, PURGE_AFTER_SECS};

/// Minimum seconds between balance polls of one session.
const POLL_SECS: u64 = 10;
/// A funding transaction that has not confirmed after this long fails the session.
const FUNDING_TIMEOUT_SECS: u64 = 30 * 60;
const MAX_FUND_ATTEMPTS: u32 = 3;
const MAX_FLIP_ATTEMPTS: u32 = 3;
const MAX_SETTLE_ATTEMPTS: u32 = 5;

impl Playground {
    /// One pass over every live session.
    pub async fn tick(&self, now: u64) {
        let live = match self.sessions.live() {
            Ok(l) => l,
            Err(e) => {
                eprintln!("playground: reading sessions failed: {e:#}");
                return;
            }
        };
        for s in live {
            self.step(s, now).await;
        }
        match self
            .sessions
            .purge_settled_before(now.saturating_sub(PURGE_AFTER_SECS))
        {
            Ok(gone) => {
                for (id, mailbox) in gone {
                    self.chain.forget(&id).await;
                    let _ = self.mailboxes.delete(&mailbox);
                }
            }
            Err(e) => eprintln!("playground: purge failed: {e:#}"),
        }
        let mut polled = self.polled.lock().unwrap_or_else(|p| p.into_inner());
        polled.retain(|_, t| now.saturating_sub(*t) < 3600);
    }

    async fn step(&self, s: Session, now: u64) {
        match s.state {
            S::Funding => self.step_funding(s, now).await,
            S::Sealed | S::Unlocked => self.step_idle(s, now).await,
            S::Flipping => self.step_flip(s, now).await,
            S::Settling => self.step_settle(s, now).await,
            S::Kept => self.step_kept(s, now).await,
            S::Returned | S::Sent | S::Failed => {}
        }
    }

    /// True (and records the poll) when the session may be polled now.
    fn poll_due(&self, id: &str, now: u64) -> bool {
        let mut polled = self.polled.lock().unwrap_or_else(|p| p.into_inner());
        match polled.get(id) {
            Some(t) if now.saturating_sub(*t) < POLL_SECS => false,
            _ => {
                polled.insert(id.to_string(), now);
                true
            }
        }
    }

    fn save(&self, s: &Session) {
        if let Err(e) = self.sessions.save(s) {
            eprintln!("playground: saving a session failed: {e:#}");
        }
    }

    fn fail(&self, mut s: Session, message: &str, now: u64) {
        s.state = S::Failed;
        s.error = Some(message.to_string());
        s.message = message.to_string();
        s.settled_at = Some(now);
        s.op = None;
        s.op_to = None;
        self.save(&s);
    }

    fn unlocked_by_time(s: &Session, now: u64) -> bool {
        drand::round_at(now) >= s.unlock_round
    }

    async fn step_funding(&self, mut s: Session, now: u64) {
        if !self.sessions.has_tx(&s.id, "fund").unwrap_or(false) {
            match self.chain.fund(&s.vault_address, s.funded_zat).await {
                Ok(txid) => {
                    if let Err(e) = self.sessions.add_tx(&s.id, "fund", &txid, now) {
                        eprintln!("playground: recording a tx failed: {e:#}");
                    }
                    s.message = "Funding transaction sent. Waiting for it to confirm.".into();
                    s.attempts = 0;
                    self.save(&s);
                }
                Err(e) => {
                    eprintln!("playground: funding {} failed: {e:#}", s.id);
                    s.attempts += 1;
                    if s.attempts >= MAX_FUND_ATTEMPTS {
                        self.fail(
                            s,
                            "The demo wallet could not fund this vault. Nothing was spent. Start a new session.",
                            now,
                        );
                    } else {
                        self.save(&s);
                    }
                }
            }
            return;
        }
        if now.saturating_sub(s.created) > FUNDING_TIMEOUT_SECS {
            self.fail(
                s,
                "The funding transaction did not confirm in time. Start a new session.",
                now,
            );
            return;
        }
        if !self.poll_due(&s.id, now) {
            return;
        }
        match self.chain.balance(&s.id, s.epoch).await {
            Ok(b) => {
                s.balance_zat = b.total;
                if b.total > 0 && b.spendable >= b.total {
                    s.confirmed = true;
                    if Self::unlocked_by_time(&s, now) {
                        s.state = S::Unlocked;
                        s.message = "Unlocked. Open the kit with your heir key.".into();
                    } else {
                        s.state = S::Sealed;
                        s.message =
                            "Sealed. The funds are confirmed and the timer is running.".into();
                    }
                }
                self.save(&s);
            }
            Err(e) => eprintln!("playground: reading a vault failed: {e:#}"),
        }
    }

    async fn step_idle(&self, s: Session, now: u64) {
        let mut state = s.state;
        if state == S::Sealed
            && Self::unlocked_by_time(&s, now)
            && self
                .sessions
                .transition(
                    &s.id,
                    S::Sealed,
                    S::Unlocked,
                    None,
                    None,
                    "Unlocked. Open the kit with your heir key.",
                )
                .unwrap_or(false)
        {
            state = S::Unlocked;
        }
        if !s.confirmed && self.poll_due(&s.id, now) {
            match self.chain.balance(&s.id, s.epoch).await {
                Ok(b) => {
                    let confirmed = b.total > 0 && b.spendable >= b.total;
                    if let Err(e) = self.sessions.set_balance(&s.id, b.total, confirmed) {
                        eprintln!("playground: saving a balance failed: {e:#}");
                    }
                }
                Err(e) => eprintln!("playground: reading a vault failed: {e:#}"),
            }
        }
        if state == S::Unlocked && now >= s.unlock_time + AUTO_RETURN_SECS {
            let _ = self.sessions.transition(
                &s.id,
                S::Unlocked,
                S::Settling,
                Some("return"),
                None,
                "Time is up: returning the funds to the demo float.",
            );
        }
    }

    async fn step_flip(&self, mut s: Session, now: u64) {
        let Ok(heir) = HeirCode::parse(&s.heir_code) else {
            self.fail(s, "The heir code is no longer valid.", now);
            return;
        };
        // Each attempt gets an epoch of its own: an earlier attempt's sweep
        // may have reached the network without the reply arriving.
        let new_epoch = match self.chain.next_free_epoch(&s.id, s.epoch + 1).await {
            Ok(e) => e,
            Err(e) => {
                eprintln!("playground: choosing a flip epoch failed: {e:#}");
                self.flip_failed(s);
                return;
            }
        };
        let attempt = self
            .build_kit(&s.id, new_epoch, &heir, s.minutes, now)
            .await;
        let (kit_json, address, round) = match attempt {
            Ok(v) => v,
            Err(e) => {
                eprintln!("playground: preparing a flip failed: {e:#}");
                self.flip_failed(s);
                return;
            }
        };
        match self.chain.sweep(&s.id, s.epoch, &address).await {
            Ok(txid) => {
                if let Err(e) = self.sessions.add_tx(&s.id, "flip", &txid, now) {
                    eprintln!("playground: recording a tx failed: {e:#}");
                }
                s.epoch = new_epoch;
                s.flips += 1;
                s.vault_address = address;
                s.unlock_round = round;
                s.unlock_time = drand::round_time(round);
                s.confirmed = false;
                s.state = S::Sealed;
                s.op = None;
                s.attempts = 0;
                s.message =
                    "Flipped. The funds moved to a new vault and the timer restarted.".into();
                self.save(&s);
                if let Err(e) = self.mailboxes.put_kit(&s.mailbox_id, &kit_json, now) {
                    eprintln!("playground: replacing the kit failed: {e:#}");
                }
            }
            Err(e) if code_of(&e) == Some("vault_empty") => {
                self.fail(s, "The vault was already emptied.", now);
            }
            Err(e) => {
                eprintln!("playground: flip sweep failed: {e:#}");
                self.flip_failed(s);
            }
        }
    }

    /// A flip attempt failed before any funds moved.
    fn flip_failed(&self, mut s: Session) {
        s.attempts += 1;
        if s.attempts >= MAX_FLIP_ATTEMPTS {
            s.state = S::Sealed;
            s.op = None;
            s.attempts = 0;
            s.message = "The flip did not go through. Your vault is unchanged; try again.".into();
        }
        self.save(&s);
    }

    async fn step_settle(&self, mut s: Session, now: u64) {
        if !s.confirmed && !self.poll_due(&s.id, now) {
            return;
        }
        let send = s.op.as_deref() == Some("send");
        let to = if send {
            match s.op_to.clone() {
                Some(t) => t,
                None => {
                    self.fail(s, "The destination address was lost.", now);
                    return;
                }
            }
        } else {
            match self.chain.float_address().await {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("playground: float address unavailable: {e:#}");
                    return;
                }
            }
        };
        match self.chain.sweep(&s.id, s.epoch, &to).await {
            Ok(txid) => {
                let kind = if send { "send" } else { "return" };
                if let Err(e) = self.sessions.add_tx(&s.id, kind, &txid, now) {
                    eprintln!("playground: recording a tx failed: {e:#}");
                }
                s.state = if send { S::Sent } else { S::Returned };
                s.message = if send {
                    "Sent to your address.".into()
                } else {
                    "Returned to the demo float.".into()
                };
                s.balance_zat = 0;
                s.confirmed = true;
                s.settled_at = Some(now);
                s.op = None;
                s.op_to = None;
                s.attempts = 0;
                self.save(&s);
            }
            Err(e) if code_of(&e) == Some("insufficient_funds") => {
                s.message = "Waiting for the vault's funds to confirm.".into();
                self.save(&s);
            }
            Err(e) if code_of(&e) == Some("vault_empty") => {
                s.state = S::Kept;
                s.message =
                    "The vault was already emptied with its seed, so nothing is left to move."
                        .into();
                s.balance_zat = 0;
                s.settled_at = Some(now);
                s.kept_swept = true;
                s.op = None;
                s.op_to = None;
                self.save(&s);
            }
            Err(e) => {
                eprintln!("playground: settling {} failed: {e:#}", s.id);
                s.attempts += 1;
                if s.attempts >= MAX_SETTLE_ATTEMPTS {
                    s.op_to = None;
                    self.fail(
                        s,
                        "The funds could not be moved right now. The vault is kept safe on the server.",
                        now,
                    );
                } else {
                    self.save(&s);
                }
            }
        }
    }

    /// A kept vault that nobody emptied within 24 hours returns to the float.
    async fn step_kept(&self, mut s: Session, now: u64) {
        let Some(at) = s.settled_at else { return };
        if now < at + KEEP_RETURN_AFTER_SECS || !self.poll_due(&s.id, now) {
            return;
        }
        let Ok(float) = self.chain.float_address().await else {
            return;
        };
        match self.chain.sweep(&s.id, s.epoch, &float).await {
            Ok(txid) => {
                if let Err(e) = self.sessions.add_tx(&s.id, "return", &txid, now) {
                    eprintln!("playground: recording a tx failed: {e:#}");
                }
                s.message =
                    "Nobody claimed the vault in 24 hours, so it returned to the float.".into();
                s.kept_swept = true;
                self.save(&s);
            }
            Err(e) if code_of(&e) == Some("vault_empty") => {
                let _ = self.sessions.set_kept_swept(&s.id);
            }
            Err(e) => {
                eprintln!("playground: returning a kept vault failed: {e:#}");
                s.attempts += 1;
                if s.attempts >= MAX_SETTLE_ATTEMPTS {
                    s.kept_swept = true;
                }
                self.save(&s);
            }
        }
    }
}

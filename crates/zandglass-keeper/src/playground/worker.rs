//! The state machine: funding, unlock, flips, settlement, auto-return and
//! purging. `tick` is called every few seconds by [`super::spawn_worker`];
//! tests call it directly with a chosen clock.
//!
//! Ownership of rows: the worker owns `funding`, `flipping`, `settling` and
//! kept rows and rewrites them freely. Handlers may move `sealed` and
//! `unlocked` rows (flip, settle, keep), so for those the worker only uses
//! guarded updates.
//!
//! Money safety (these paths move real funds):
//! - Funding fails closed. The intent (`op = 'fund'`) is written with a
//!   guarded update before the float is asked to pay, and any database error
//!   skips the tick. A session with the intent is never funded again unless
//!   the attempt certainly sent nothing ([`definitely_unsent`]), and a vault
//!   that already holds funds is never funded.
//! - A flip whose sweep may have reached the network keeps the new epoch as
//!   pending until the balances show where the funds are.
//! - Seeds are deleted only after every epoch of a session was seen empty.
//!   Otherwise the funds go back to the float, or the seeds are archived.

use serde::{Deserialize, Serialize};
use zandglass_core::drand;
use zandglass_core::heir::HeirCode;

use super::chain::definitely_unsent;
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
/// A flip whose outcome is still unknown after this long fails the session.
/// Its vaults stay on the server and are checked before any purge.
const FLIP_PENDING_TIMEOUT_SECS: u64 = 6 * 3600;
/// Seconds between purge checks of one session.
const PURGE_CHECK_SECS: u64 = 600;
/// Purge checks per tick (each one syncs every epoch of a session).
const PURGE_BATCH: usize = 2;
/// Purge checks that could not prove a session empty before its seeds are
/// archived (moved aside, never deleted) and its row is purged.
const PURGE_ARCHIVE_AFTER: u32 = 6;

const FUNDING_MESSAGE: &str = "Funding the vault from the demo float.";
const FLIP_UNCHANGED: &str = "The flip did not go through. Your vault is unchanged; try again.";
const KEPT_ON_SERVER: &str = "The funds could not be moved right now. The vault is kept safe on the server, and its funds return to the demo float later.";

/// A flip's new epoch, stored as JSON in the session row while the sweep's
/// outcome is unknown.
#[derive(Serialize, Deserialize)]
struct PendingFlip {
    epoch: u32,
    address: String,
    round: u64,
    kit: String,
    since: u64,
}

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
        self.purge(now).await;
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

    /// True (and records the time) when `key` was not seen in the last `every` seconds.
    fn due(&self, key: &str, now: u64, every: u64) -> bool {
        let mut polled = self.polled.lock().unwrap_or_else(|p| p.into_inner());
        match polled.get(key) {
            Some(t) if now.saturating_sub(*t) < every => false,
            _ => {
                polled.insert(key.to_string(), now);
                true
            }
        }
    }

    /// True (and records the poll) when the session may be polled now.
    fn poll_due(&self, id: &str, now: u64) -> bool {
        self.due(id, now, POLL_SECS)
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

    async fn step_funding(&self, s: Session, now: u64) {
        // Fail closed: without a sure answer from the database, never fund.
        let recorded = match self.sessions.has_tx(&s.id, "fund") {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "playground: reading the transactions of {} failed; not funding: {e:#}",
                    s.id
                );
                return;
            }
        };
        if !recorded && s.op.is_none() {
            self.fund_once(s, now).await;
        } else {
            self.wait_for_funding(s, now).await;
        }
    }

    /// Sends the funding transaction at most once per intent.
    async fn fund_once(&self, mut s: Session, now: u64) {
        // Second guard: a vault that already holds funds is never funded again.
        match self.chain.balance(&s.id, s.epoch).await {
            Ok(b) if b.total == 0 => {}
            Ok(_) => {
                eprintln!(
                    "playground: the vault of {} already holds funds; not funding it",
                    s.id
                );
                if let Err(e) = self.sessions.claim_fund(&s.id) {
                    eprintln!("playground: recording the funding intent failed: {e:#}");
                }
                return;
            }
            Err(e) => {
                eprintln!("playground: reading a vault before funding failed: {e:#}");
                return;
            }
        }
        match self.sessions.claim_fund(&s.id) {
            Ok(true) => s.op = Some("fund".into()),
            Ok(false) => return,
            Err(e) => {
                eprintln!("playground: recording the funding intent failed: {e:#}");
                return;
            }
        }
        match self.chain.fund(&s.vault_address, s.funded_zat).await {
            Ok(txid) => {
                if let Err(e) = self.sessions.add_tx(&s.id, "fund", &txid, now) {
                    // The intent still blocks a second funding.
                    eprintln!("playground: recording a tx failed: {e:#}");
                }
                s.message = "Funding transaction sent. Waiting for it to confirm.".into();
                s.attempts = 0;
                self.save(&s);
            }
            Err(e) if definitely_unsent(&e) => {
                eprintln!(
                    "playground: funding {} failed before anything was sent: {e:#}",
                    s.id
                );
                s.attempts += 1;
                if s.attempts >= MAX_FUND_ATTEMPTS {
                    self.fail(
                        s,
                        "The demo wallet could not fund this vault. Nothing was spent. Start a new session.",
                        now,
                    );
                } else if let Err(e) =
                    self.sessions
                        .release_fund(&s.id, s.attempts, FUNDING_MESSAGE)
                {
                    // The intent stays, so the session waits and times out
                    // instead of being funded twice.
                    eprintln!("playground: clearing the funding intent failed: {e:#}");
                }
            }
            Err(e) => {
                // The broadcast may have reached the network: never send
                // again. The vault balance decides (see `wait_for_funding`).
                eprintln!(
                    "playground: funding {} may or may not have been broadcast; not retrying: {e:#}",
                    s.id
                );
                s.message =
                    "The funding transaction may not have reached the network. Waiting to see whether it arrives."
                        .into();
                self.save(&s);
            }
        }
    }

    /// Waits for the funds of a funded (or possibly funded) vault to confirm.
    async fn wait_for_funding(&self, mut s: Session, now: u64) {
        if now.saturating_sub(s.created) > FUNDING_TIMEOUT_SECS {
            // Funds that arrive later stay in the vault; the purge checks
            // every epoch and returns them to the float.
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
                    s.op = None;
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
        if let Some(pending) = s.pending.clone() {
            self.resolve_flip(s, &pending, now).await;
            return;
        }
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
        let (kit, address, round) = match attempt {
            Ok(v) => v,
            Err(e) => {
                eprintln!("playground: preparing a flip failed: {e:#}");
                self.flip_failed(s);
                return;
            }
        };
        let p = PendingFlip {
            epoch: new_epoch,
            address,
            round,
            kit,
            since: now,
        };
        match self.chain.sweep(&s.id, s.epoch, &p.address).await {
            Ok(txid) => self.adopt_flip(s, p, Some(&txid), now),
            Err(e) if code_of(&e) == Some("vault_empty") => {
                self.fail(s, "The vault was already emptied.", now);
            }
            Err(e) if definitely_unsent(&e) || code_of(&e) == Some("insufficient_funds") => {
                eprintln!("playground: flip sweep failed before anything was sent: {e:#}");
                self.flip_failed(s);
            }
            Err(e) => {
                // The sweep may have reached the network: track the new
                // epoch until the balances tell where the funds are.
                eprintln!(
                    "playground: flip sweep of {} may or may not have been broadcast; tracking epoch {}: {e:#}",
                    s.id, p.epoch
                );
                s.pending = serde_json::to_string(&p).ok();
                s.message = "Flipping: checking whether the move reached the network.".into();
                self.save(&s);
            }
        }
    }

    /// Makes a flip's new epoch the current one.
    fn adopt_flip(&self, mut s: Session, p: PendingFlip, txid: Option<&str>, now: u64) {
        if let Some(txid) = txid {
            if let Err(e) = self.sessions.add_tx(&s.id, "flip", txid, now) {
                eprintln!("playground: recording a tx failed: {e:#}");
            }
        }
        s.epoch = p.epoch;
        s.flips += 1;
        s.vault_address = p.address;
        s.unlock_round = p.round;
        s.unlock_time = drand::round_time(p.round);
        s.confirmed = false;
        s.state = S::Sealed;
        s.op = None;
        s.attempts = 0;
        s.pending = None;
        s.message = "Flipped. The funds moved to a new vault and the timer restarted.".into();
        self.save(&s);
        if let Err(e) = self.mailboxes.put_kit(&s.mailbox_id, &p.kit, now) {
            eprintln!("playground: replacing the kit failed: {e:#}");
        }
    }

    /// Settles a flip whose sweep outcome was unknown: funds in the new
    /// epoch adopt it; spendable funds back in the old epoch (the sweep never
    /// happened or expired) leave the vault unchanged. The new epoch's vault
    /// is never deleted here; the purge checks every epoch.
    async fn resolve_flip(&self, mut s: Session, pending: &str, now: u64) {
        let Ok(p) = serde_json::from_str::<PendingFlip>(pending) else {
            self.fail(s, KEPT_ON_SERVER, now);
            return;
        };
        if !self.poll_due(&s.id, now) {
            return;
        }
        match self.chain.balance(&s.id, p.epoch).await {
            Ok(b) if b.total > 0 => {
                self.adopt_flip(s, p, None, now);
                return;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("playground: reading a pending flip vault failed: {e:#}");
                return;
            }
        }
        match self.chain.balance(&s.id, s.epoch).await {
            Ok(b) if b.total > 0 && b.spendable >= b.total => {
                s.pending = None;
                s.state = S::Sealed;
                s.op = None;
                s.attempts = 0;
                s.message = FLIP_UNCHANGED.into();
                self.save(&s);
                return;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("playground: reading a vault failed: {e:#}");
                return;
            }
        }
        if now.saturating_sub(p.since) > FLIP_PENDING_TIMEOUT_SECS {
            self.fail(s, KEPT_ON_SERVER, now);
        }
    }

    /// A flip attempt failed before any funds moved.
    fn flip_failed(&self, mut s: Session) {
        s.attempts += 1;
        if s.attempts >= MAX_FLIP_ATTEMPTS {
            s.state = S::Sealed;
            s.op = None;
            s.attempts = 0;
            s.message = FLIP_UNCHANGED.into();
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
                    // True because the purge never deletes a vault that
                    // holds funds: it returns them to the float first.
                    s.op_to = None;
                    self.fail(s, KEPT_ON_SERVER, now);
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
                    // Stops the retries; the purge checks the vault again.
                    s.kept_swept = true;
                }
                self.save(&s);
            }
        }
    }

    /// Purges sessions settled more than a week ago, a few per tick, each
    /// at most every [`PURGE_CHECK_SECS`].
    async fn purge(&self, now: u64) {
        let rows = match self
            .sessions
            .purge_candidates(now.saturating_sub(PURGE_AFTER_SECS))
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("playground: listing sessions to purge failed: {e:#}");
                return;
            }
        };
        let due: Vec<(String, String)> = rows
            .into_iter()
            .filter(|(id, _)| self.due(&format!("purge:{id}"), now, PURGE_CHECK_SECS))
            .take(PURGE_BATCH)
            .collect();
        for (id, mailbox) in due {
            self.purge_one(&id, &mailbox, now).await;
        }
    }

    /// Deletes a session's seeds only when every epoch was just seen empty.
    /// Spendable funds are returned to the float first; a session that
    /// cannot be proven empty after [`PURGE_ARCHIVE_AFTER`] checks has its
    /// seeds archived (moved aside, never deleted) before the row goes.
    async fn purge_one(&self, id: &str, mailbox: &str, now: u64) {
        let mut known = true;
        let mut funded = Vec::new();
        match self.chain.epochs(id).await {
            Ok(epochs) => {
                for epoch in epochs {
                    match self.chain.balance(id, epoch).await {
                        Ok(b) if b.total == 0 => {}
                        Ok(b) if b.spendable >= b.total => funded.push(epoch),
                        Ok(_) => known = false,
                        Err(e) => {
                            eprintln!("playground: checking a vault before purging failed: {e:#}");
                            known = false;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("playground: listing the vaults of {id} failed: {e:#}");
                known = false;
            }
        }
        if known && funded.is_empty() {
            self.chain.forget(id).await;
            self.drop_session(id, mailbox);
            return;
        }
        if !funded.is_empty() {
            match self.chain.float_address().await {
                Ok(float) => {
                    for epoch in funded {
                        match self.chain.sweep(id, epoch, &float).await {
                            Ok(txid) => {
                                eprintln!(
                                    "playground: session {id} epoch {epoch} still held funds; returned them to the float"
                                );
                                if let Err(e) = self.sessions.add_tx(id, "return", &txid, now) {
                                    eprintln!("playground: recording a tx failed: {e:#}");
                                }
                            }
                            Err(e) => eprintln!(
                                "playground: returning session {id} epoch {epoch} to the float failed: {e:#}"
                            ),
                        }
                    }
                }
                Err(e) => eprintln!("playground: float address unavailable: {e:#}"),
            }
        }
        let tries = {
            let mut t = self.purge_tries.lock().unwrap_or_else(|p| p.into_inner());
            let n = t.entry(id.to_string()).or_insert(0);
            *n += 1;
            *n
        };
        if tries < PURGE_ARCHIVE_AFTER {
            return;
        }
        match self.chain.archive(id).await {
            Ok(()) => {
                eprintln!(
                    "playground: session {id} could not be proven empty; its vault seeds were archived, not deleted"
                );
                self.drop_session(id, mailbox);
            }
            Err(e) => eprintln!("playground: archiving session {id} failed; keeping it: {e:#}"),
        }
    }

    fn drop_session(&self, id: &str, mailbox: &str) {
        if let Err(e) = self.sessions.delete(id) {
            eprintln!("playground: purging a session failed: {e:#}");
            return;
        }
        let _ = self.mailboxes.delete(mailbox);
        let mut t = self.purge_tries.lock().unwrap_or_else(|p| p.into_inner());
        t.remove(id);
    }
}

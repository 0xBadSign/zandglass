//! State machine and caps, offline with the mock chain.

use std::sync::Arc;

use sandglass_core::heir::{generate_phrase, identity_from_phrase};

use super::chain::MockChain;
use super::store::State as S;
use super::*;

const JUDGE: &str = "judge-code-for-tests";

pub(crate) fn heir_code() -> String {
    identity_from_phrase(&generate_phrase())
        .unwrap()
        .code()
        .to_string()
}

pub(crate) fn test_config() -> PlaygroundConfig {
    PlaygroundConfig {
        network: "test",
        amount_zat: 20_000,
        max_active: 10,
        max_per_day: 60,
        max_keeps_per_day: 5,
        max_flips: 3,
        judge_hash: sha256(JUDGE),
        public_origin: "https://sandglass.example".into(),
        explorer_tx_url: Some("https://explorer.example/tx/{txid}".into()),
        allowed_minutes: vec![3, 5, 10],
    }
}

struct Rig {
    pg: Arc<Playground>,
    chain: Arc<MockChain>,
    _dir: tempfile::TempDir,
}

fn rig(tweak: impl FnOnce(&mut PlaygroundConfig)) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let chain = Arc::new(MockChain::new());
    let mut cfg = test_config();
    tweak(&mut cfg);
    let mailboxes = Arc::new(Db::open(&dir.path().join("k.sqlite")).unwrap());
    let pg = Playground::new(cfg, chain.clone(), &dir.path().join("p.sqlite"), mailboxes).unwrap();
    Rig {
        pg,
        chain,
        _dir: dir,
    }
}

async fn start(r: &Rig, now: u64) -> String {
    r.pg.create_session(JUDGE, &heir_code(), 3, now)
        .await
        .map_err(|e| e.code)
        .unwrap()
        .id
}

fn state(r: &Rig, id: &str) -> S {
    r.pg.sessions().get(id).unwrap().unwrap().state
}

fn err_code<T>(r: Result<T, ApiError>) -> &'static str {
    match r {
        Ok(_) => "ok",
        Err(e) => e.code,
    }
}

/// Drives a session to `sealed` with confirmed funds.
async fn to_sealed(r: &Rig, now: u64) -> String {
    let id = start(r, now).await;
    r.pg.tick(now).await;
    r.pg.tick(now + 11).await;
    assert_eq!(state(r, &id), S::Sealed);
    id
}

fn now() -> u64 {
    unix_now()
}

#[tokio::test]
async fn full_lifecycle_with_return() {
    let r = rig(|_| {});
    let t = now();
    let id = start(&r, t).await;
    let s = r.pg.sessions().get(&id).unwrap().unwrap();
    assert_eq!(s.state, S::Funding);
    assert_eq!(s.funded_zat, 20_000 + 10_000 * 4);
    assert!(s
        .claim_link
        .starts_with("https://sandglass.example/claim.html#m="));

    r.pg.tick(t).await; // funds
    assert!(r.chain.calls().iter().any(|c| c == "fund:60000"));
    assert_eq!(state(&r, &id), S::Funding);
    r.pg.tick(t + 11).await; // confirms
    assert_eq!(state(&r, &id), S::Sealed);
    let s = r.pg.sessions().get(&id).unwrap().unwrap();
    assert!(s.confirmed);
    assert_eq!(s.balance_zat, 60_000);

    // Not unlocked yet.
    r.pg.tick(t + 60).await;
    assert_eq!(state(&r, &id), S::Sealed);
    // Past the drand round.
    r.pg.tick(t + 200).await;
    assert_eq!(state(&r, &id), S::Unlocked);

    let s =
        r.pg.begin_settle(&id, "return", None, t + 201)
            .ok()
            .unwrap();
    assert_eq!(s.state, S::Settling);
    r.pg.tick(t + 205).await;
    assert_eq!(state(&r, &id), S::Returned);
    let kinds: Vec<_> =
        r.pg.sessions()
            .txs(&id)
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect();
    assert_eq!(kinds, ["fund", "return"]);
    assert_eq!(r.chain.vault(&id, 1), Some((0, 0)));
}

#[tokio::test]
async fn funds_must_confirm_before_sealing_and_flipping() {
    let r = rig(|_| {});
    r.chain.set_manual_confirm(true);
    let t = now();
    let id = start(&r, t).await;
    r.pg.tick(t).await;
    r.pg.tick(t + 11).await;
    assert_eq!(state(&r, &id), S::Funding);
    assert_eq!(err_code(r.pg.begin_flip(&id)), "invalid_state");
    r.chain.confirm_all();
    r.pg.tick(t + 22).await;
    assert_eq!(state(&r, &id), S::Sealed);
}

#[tokio::test]
async fn flip_moves_funds_and_replaces_the_kit() {
    let r = rig(|_| {});
    let t = now();
    let id = to_sealed(&r, t).await;
    let mailbox = r.pg.sessions().get(&id).unwrap().unwrap().mailbox_id;
    let before = r.pg.mailboxes.get_kit(&mailbox).unwrap().unwrap();

    let s = r.pg.begin_flip(&id).ok().unwrap();
    assert_eq!(s.state, S::Flipping);
    // Another flip request while flipping is refused.
    assert_eq!(err_code(r.pg.begin_flip(&id)), "invalid_state");
    r.pg.tick(t + 20).await;

    let s = r.pg.sessions().get(&id).unwrap().unwrap();
    assert_eq!((s.state, s.epoch, s.flips), (S::Sealed, 2, 1));
    assert!(!s.confirmed);
    assert_eq!(r.chain.vault(&id, 1), Some((0, 0)));
    assert_eq!(r.chain.vault(&id, 2), Some((50_000, 50_000)));
    let after = r.pg.mailboxes.get_kit(&mailbox).unwrap().unwrap();
    assert_ne!(before, after);
    assert!(after.contains("\"epoch\": 2") || after.contains("\"epoch\":2"));
    // The unlock moved later: a fresh timer of the same length.
    assert!(s.unlock_time > t + 150);

    // The flipped funds confirm on the next poll, then flipping works again.
    assert_eq!(err_code(r.pg.begin_flip(&id)), "invalid_state");
    r.pg.tick(t + 40).await;
    assert!(r.pg.sessions().get(&id).unwrap().unwrap().confirmed);
    assert!(r.pg.begin_flip(&id).is_ok());
}

#[tokio::test]
async fn flips_are_capped_per_session() {
    let r = rig(|c| c.max_flips = 1);
    let t = now();
    let id = to_sealed(&r, t).await;
    assert!(r.pg.begin_flip(&id).is_ok());
    r.pg.tick(t + 20).await;
    r.pg.tick(t + 40).await;
    assert_eq!(state(&r, &id), S::Sealed);
    assert_eq!(err_code(r.pg.begin_flip(&id)), "invalid_state");
}

#[tokio::test]
async fn failed_flip_leaves_the_vault_untouched() {
    let r = rig(|_| {});
    let t = now();
    let id = to_sealed(&r, t).await;
    r.pg.begin_flip(&id).ok().unwrap();
    for _ in 0..3 {
        r.chain.fail_next_sweep("broadcast rejected: network down");
    }
    for i in 0..3 {
        r.pg.tick(t + 20 + i).await;
    }
    let s = r.pg.sessions().get(&id).unwrap().unwrap();
    assert_eq!((s.state, s.epoch, s.flips), (S::Sealed, 1, 0));
    assert_eq!(r.chain.vault(&id, 1), Some((60_000, 60_000)));
    assert!(s.message.contains("did not go through"));
}

#[tokio::test]
async fn retried_flip_never_reuses_an_attempted_epoch() {
    let r = rig(|_| {});
    let t = now();
    let id = to_sealed(&r, t).await;
    r.pg.begin_flip(&id).ok().unwrap();
    // The node accepts the sweep to epoch 2 but the reply is lost.
    r.chain.lose_reply_of_next_sweep("deadline has elapsed");
    r.pg.tick(t + 20).await;
    let s = r.pg.sessions().get(&id).unwrap().unwrap();
    assert_eq!((s.state, s.epoch), (S::Flipping, 1));
    assert_eq!(r.chain.vault(&id, 2), Some((50_000, 50_000)));

    // The retry must not touch epoch 2: it creates epoch 3 instead.
    r.pg.tick(t + 21).await;
    assert_eq!(r.chain.vault(&id, 2), Some((50_000, 50_000)));
    let calls = r.chain.calls();
    assert!(calls.contains(&format!("create:{id}:2")));
    assert!(calls.contains(&format!("create:{id}:3")));
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.as_str() == format!("create:{id}:2"))
            .count(),
        1
    );
}

#[tokio::test]
async fn create_vault_refuses_an_existing_epoch() {
    let chain = MockChain::new();
    chain.create_vault("s", 1).await.unwrap();
    chain.fund("mock-vault-s-1", 10).await.unwrap();
    assert!(chain.create_vault("s", 1).await.is_err());
    assert_eq!(chain.vault("s", 1), Some((10, 10)));
    assert_eq!(chain.next_free_epoch("s", 1).await.unwrap(), 2);
}

#[tokio::test]
async fn auto_return_thirty_minutes_after_unlock() {
    let r = rig(|_| {});
    let t = now();
    let id = to_sealed(&r, t).await;
    r.pg.tick(t + 200).await;
    assert_eq!(state(&r, &id), S::Unlocked);
    r.pg.tick(t + 200 + 600).await;
    assert_eq!(state(&r, &id), S::Unlocked);
    // 30 minutes after the unlock time (t + 180ish).
    r.pg.tick(t + 190 + AUTO_RETURN_SECS).await;
    assert_eq!(state(&r, &id), S::Settling);
    r.pg.tick(t + 200 + AUTO_RETURN_SECS).await;
    assert_eq!(state(&r, &id), S::Returned);
}

#[tokio::test]
async fn settle_is_only_allowed_after_the_unlock() {
    let r = rig(|_| {});
    let t = now();
    let id = to_sealed(&r, t).await;
    assert_eq!(
        err_code(r.pg.begin_settle(&id, "return", None, t)),
        "invalid_state"
    );
    r.pg.tick(t + 200).await;
    assert_eq!(
        err_code(r.pg.begin_settle(&id, "bogus", None, t)),
        "invalid_request"
    );
    assert_eq!(
        err_code(r.pg.begin_settle(&id, "send", None, t)),
        "invalid_request"
    );
    assert_eq!(
        err_code(r.pg.begin_settle(&id, "send", Some("not-an-address"), t)),
        "network_mismatch"
    );
    assert!(r
        .pg
        .begin_settle(&id, "send", Some("mock-visitor"), t)
        .is_ok());
    r.pg.tick(t + 205).await;
    let s = r.pg.sessions().get(&id).unwrap().unwrap();
    assert_eq!(s.state, S::Sent);
    // The visitor's address is not kept after the sweep.
    assert_eq!(s.op_to, None);
    assert!(r.chain.calls().iter().any(|c| c.ends_with(":mock-visitor")));
}

#[tokio::test]
async fn keep_is_capped_per_day() {
    let r = rig(|c| c.max_keeps_per_day = 1);
    let t = now();
    let a = to_sealed(&r, t).await;
    let b = to_sealed(&r, t).await;
    r.pg.tick(t + 200).await;
    assert_eq!(state(&r, &a), S::Unlocked);
    assert_eq!(state(&r, &b), S::Unlocked);
    let kept = r.pg.begin_settle(&a, "keep", None, t + 201).ok().unwrap();
    assert_eq!(kept.state, S::Kept);
    assert_eq!(
        err_code(r.pg.begin_settle(&b, "keep", None, t + 202)),
        "daily_cap_reached"
    );
    // The visitor can still return or send.
    assert!(r.pg.begin_settle(&b, "return", None, t + 203).is_ok());
}

#[tokio::test]
async fn kept_vault_returns_after_a_day_unless_emptied() {
    let r = rig(|_| {});
    let t = now();
    let a = to_sealed(&r, t).await;
    let b = to_sealed(&r, t).await;
    r.pg.tick(t + 200).await;
    r.pg.begin_settle(&a, "keep", None, t + 201).ok().unwrap();
    r.pg.begin_settle(&b, "keep", None, t + 201).ok().unwrap();
    // b's visitor restores the seed and moves the funds by hand.
    r.chain.drain(&b, 1);
    r.pg.tick(t + 3600).await;
    assert_eq!(r.chain.vault(&a, 1), Some((60_000, 60_000)));
    r.pg.tick(t + 201 + KEEP_RETURN_AFTER_SECS + 1).await;
    assert_eq!(r.chain.vault(&a, 1), Some((0, 0)));
    let kinds: Vec<_> =
        r.pg.sessions()
            .txs(&a)
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect();
    assert_eq!(kinds, ["fund", "return"]);
    assert!(r.pg.sessions().live().unwrap().is_empty());
}

#[tokio::test]
async fn emptied_vault_at_settle_counts_as_kept() {
    let r = rig(|_| {});
    let t = now();
    let id = to_sealed(&r, t).await;
    r.pg.tick(t + 200).await;
    r.chain.drain(&id, 1);
    r.pg.begin_settle(&id, "return", None, t + 201)
        .ok()
        .unwrap();
    r.pg.tick(t + 205).await;
    assert_eq!(state(&r, &id), S::Kept);
}

#[tokio::test]
async fn funding_failure_fails_the_session_without_spending() {
    let r = rig(|_| {});
    let t = now();
    let id = start(&r, t).await;
    for _ in 0..3 {
        r.chain.fail_next_fund("insufficient funds");
    }
    for i in 0..3 {
        r.pg.tick(t + i).await;
    }
    let s = r.pg.sessions().get(&id).unwrap().unwrap();
    assert_eq!(s.state, S::Failed);
    assert!(s.error.is_some());
    assert_eq!(r.chain.vault(&id, 1), Some((0, 0)));
    // A failed session does not count against the active cap.
    assert_eq!(r.pg.sessions().count_active().unwrap(), 0);
}

#[tokio::test]
async fn caps_and_validation() {
    let r = rig(|c| {
        c.max_active = 2;
        c.max_per_day = 3;
    });
    let t = now();
    let h = heir_code();
    assert_eq!(
        err_code(r.pg.create_session("wrong", &h, 3, t).await),
        "judge_code_invalid"
    );
    assert_eq!(
        err_code(r.pg.create_session(JUDGE, "age1nope", 3, t).await),
        "heir_code_invalid"
    );
    assert_eq!(
        err_code(r.pg.create_session(JUDGE, &h, 4, t).await),
        "invalid_request"
    );
    let a = start(&r, t).await;
    let _b = start(&r, t).await;
    assert_eq!(
        err_code(r.pg.create_session(JUDGE, &h, 3, t).await),
        "playground_busy"
    );
    // Finish both sessions; the active cap frees up, the daily cap does not.
    r.pg.tick(t).await;
    r.pg.tick(t + 11).await;
    r.pg.tick(t + 200).await;
    r.pg.begin_settle(&a, "return", None, t + 201).ok().unwrap();
    r.pg.begin_settle(&_b, "return", None, t + 201)
        .ok()
        .unwrap();
    r.pg.tick(t + 205).await;
    let _c = start(&r, t + 206).await;
    assert_eq!(
        err_code(r.pg.create_session(JUDGE, &h, 3, t + 207).await),
        "daily_cap_reached"
    );
}

#[tokio::test]
async fn float_must_be_ready_and_funded() {
    let r = rig(|_| {});
    let h = heir_code();
    r.chain.set_float_ready(false);
    assert_eq!(
        err_code(r.pg.create_session(JUDGE, &h, 3, now()).await),
        "playground_busy"
    );
    r.chain.set_float_ready(true);
    r.chain.set_float_spendable(Some(1000));
    assert_eq!(
        err_code(r.pg.create_session(JUDGE, &h, 3, now()).await),
        "playground_busy"
    );
    r.chain.set_float_spendable(Some(1_000_000));
    assert!(r.pg.create_session(JUDGE, &h, 3, now()).await.is_ok());
}

#[tokio::test]
async fn settled_sessions_are_purged_after_a_week() {
    let r = rig(|_| {});
    let t = now();
    let id = to_sealed(&r, t).await;
    let mailbox = r.pg.sessions().get(&id).unwrap().unwrap().mailbox_id;
    r.pg.tick(t + 200).await;
    r.pg.begin_settle(&id, "return", None, t + 201)
        .ok()
        .unwrap();
    r.pg.tick(t + 205).await;
    assert_eq!(state(&r, &id), S::Returned);
    r.pg.tick(t + 205 + PURGE_AFTER_SECS - 10).await;
    assert!(r.pg.sessions().get(&id).unwrap().is_some());
    r.pg.tick(t + 205 + PURGE_AFTER_SECS + 10).await;
    assert!(r.pg.sessions().get(&id).unwrap().is_none());
    assert!(r.pg.mailboxes.hashes(&mailbox).unwrap().is_none());
    assert!(r.chain.forgotten().contains(&id));
}

#[tokio::test]
async fn funding_that_never_confirms_fails_after_thirty_minutes() {
    let r = rig(|_| {});
    r.chain.set_manual_confirm(true);
    let t = now();
    let id = start(&r, t).await;
    r.pg.tick(t).await;
    r.pg.tick(t + 1900).await;
    assert_eq!(state(&r, &id), S::Failed);
}

#[test]
fn min_confirmations_setting_is_validated() {
    assert_eq!(DEFAULT_MIN_CONFIRMATIONS, 1);
    assert_eq!(check_min_confirmations(1).unwrap(), 1);
    assert_eq!(check_min_confirmations(6).unwrap(), 6);
    assert!(check_min_confirmations(0).is_err());
}

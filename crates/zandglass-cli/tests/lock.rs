//! Offline CLI tests for the savings lock: the confirmation flag, argument
//! checks, and the refusals of flip and cancel. No network, no funds.

use std::path::Path;
use std::process::{Command, Output};

use zandglass_core::heir::{generate_phrase, identity_from_phrase};
use zandglass_wallet::{EpochRecord, LockInfo, Plan, PlanStatus, Store};

const PLAN: &str = "00112233445566778899aabbccddeeff";
const PASS: &str = "test-passphrase";

fn sg(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zandglass"))
        .env("ZANDGLASS_PASSPHRASE", PASS)
        .args(["--network", "test", "--data-dir"])
        .arg(home)
        .args(args)
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// A fresh heir phrase written to `dir/heir.phrase`: its path and heir code.
fn heir_words(dir: &Path) -> (String, String) {
    let phrase = generate_phrase();
    let path = dir.join("heir.phrase");
    std::fs::write(&path, phrase.as_str()).unwrap();
    let code = identity_from_phrase(&phrase).unwrap().code().to_string();
    (path.to_str().unwrap().to_string(), code)
}

fn heir_code() -> String {
    identity_from_phrase(&generate_phrase())
        .unwrap()
        .code()
        .to_string()
}

#[test]
fn a_lock_is_refused_without_the_confirmation_flag() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (words, _) = heir_words(dir.path());
    // No store exists: the refusal comes before anything else happens.
    let out = sg(
        &home,
        &["plan", "lock", "--phrase-file", &words, "--until", "90d"],
    );
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(err.contains("--i-have-my-heir-words"), "{err}");
    assert!(err.contains("lost for good"), "{err}");
    assert!(!home.exists());
}

#[test]
fn a_lock_needs_the_words_not_just_a_pasted_code() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (words, code) = heir_words(dir.path());
    let lock = |extra: &[&str]| {
        let mut a = vec!["plan", "lock", "--i-have-my-heir-words", "--until", "90d"];
        a.extend_from_slice(extra);
        let out = sg(&home, &a);
        assert!(!out.status.success());
        text(&out.stderr)
    };
    // A code alone is refused, before any network use or store access.
    let err = lock(&["--heir", &code]);
    assert!(err.contains("--phrase-file"), "{err}");
    assert!(err.contains("proof"), "{err}");
    // A code that does not belong to the words is refused; the right one is named.
    let other = heir_code();
    let err = lock(&["--phrase-file", &words, "--heir", &other]);
    assert!(err.contains("None of the --heir codes"), "{err}");
    assert!(err.contains(&code), "{err}");
    // A bad phrase file is refused without echoing its content.
    let bad = dir.path().join("bad.phrase");
    std::fs::write(&bad, "abandon abandon abandon").unwrap();
    let err = lock(&["--phrase-file", bad.to_str().unwrap()]);
    assert!(err.contains("expected 24 words"), "{err}");
    assert!(!home.exists());
}

#[test]
fn lock_arguments_are_checked_before_any_network_use() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (words, code) = heir_words(dir.path());
    let run = |extra: &[&str]| {
        let mut a = vec![
            "plan",
            "lock",
            "--i-have-my-heir-words",
            "--phrase-file",
            &words,
        ];
        a.extend_from_slice(extra);
        text(&sg(&home, &a).stderr)
    };
    assert!(run(&["--until", "soon"]).contains("not a duration"));
    assert!(run(&["--until", "2027-06-01"]).contains("not an unlock time"));
    assert!(run(&["--until", "2020-01-01T00:00:00Z"]).contains("at least"));
    assert!(run(&["--until", "10s"]).contains("at least 30 seconds"));
    assert!(run(&["--until", "999999d"]).contains("30 years"));
    assert!(run(&["--until", "200d", "--mailbox", "http://localhost:1"]).contains("170 days"));
    // Extra heir codes may accompany the words; the derived one must be among them.
    assert!(run(&["--heir", &code, "--heir", "nope", "--until", "90d"]).contains("heir code #2"));
    assert!(run(&["--heir", &code, "--heir", &code, "--until", "90d"]).contains("duplicate"));
}

#[test]
fn flip_cancel_and_status_know_a_savings_lock() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let out = sg(&home, &["init"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let store = Store::open(&home.join("test"), PASS).unwrap();
    let unlock = zandglass_wallet::ops::now() + 90 * 86_400;
    store
        .put_plan(&Plan {
            id: PLAN.into(),
            kind: "lock".into(),
            network: "test".into(),
            heirs: vec![heir_code()],
            period_secs: 90 * 86_400,
            message: None,
            epoch: 1,
            status: PlanStatus::Active,
            created: 1,
            mailbox_url: None,
            mailbox_id: None,
            mailbox_write_token: None,
            epochs: vec![EpochRecord {
                epoch: 1,
                dir: Store::vault_rel_dir(PLAN, 1),
                birthday_height: 10,
                unlock_round: 1,
                unlock_time: unlock,
                created: 1,
            }],
            installments: None,
            pending: None,
            archived: Vec::new(),
            lock: Some(LockInfo {
                address: "utest1lockaddress".into(),
            }),
        })
        .unwrap();

    // Flip and cancel refuse with the same clear message, by id or bare.
    for args in [
        vec!["flip", "0011"],
        vec!["flip"],
        vec!["cancel", "0011", "--to", "utest1elsewhere"],
    ] {
        let out = sg(&home, &args);
        assert!(!out.status.success(), "{args:?}");
        let err = text(&out.stderr);
        assert!(
            err.contains("This is a savings lock: nobody can move the funds before"),
            "{args:?}: {err}"
        );
        assert!(err.contains("not even you"), "{err}");
        assert!(err.contains("zandglass.cash/claim"), "{err}");
        assert!(err.contains("zandglass claim"), "{err}");
    }

    // Status shows the unlock date, the vault address and that the seed is gone.
    let out = sg(&home, &["status"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let s = text(&out.stdout);
    assert!(s.contains("(lock, test) active"), "{s}");
    assert!(s.contains("locked until:"), "{s}");
    assert!(s.contains("utest1lockaddress"), "{s}");
    assert!(s.contains("seed:         deleted"), "{s}");
}

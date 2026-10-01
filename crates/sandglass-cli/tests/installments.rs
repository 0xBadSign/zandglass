//! Offline CLI tests for multiple heirs (T2.4) and installments (T2.3).

use std::path::Path;
use std::process::{Command, Output};

use sandglass_core::heir::{generate_phrase, identity_from_phrase};
use sandglass_core::kit::{BuildParams, Kind, Network, Payload};
use sandglass_core::{drand, Kit};

const PLAN: &str = "00112233445566778899aabbccddeeff";
const SEED: &str = "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth title";

fn sg(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sandglass"))
        .env("SANDGLASS_PASSPHRASE", "test-passphrase")
        .args(["--network", "test", "--data-dir"])
        .arg(home)
        .args(args)
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn heir_code() -> String {
    identity_from_phrase(&generate_phrase())
        .unwrap()
        .code()
        .to_string()
}

fn write_kit(
    dir: &Path,
    name: &str,
    seconds_from_now: u64,
    index: u32,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let phrase = generate_phrase();
    let heir = identity_from_phrase(&phrase).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let payload = Payload {
        vault_seed: SEED.into(),
        birthday_height: 1234,
        network: Network::Test,
        plan: PLAN.into(),
        epoch: 1,
        message: None,
        restore_hints: vec![],
    };
    let kit = Kit::build(
        &payload,
        &BuildParams {
            kind: Kind::Installment,
            sequence: Some(sandglass_core::kit::Sequence { index, count: 2 }),
            now,
            unlock_round: drand::unlock_round(now + seconds_from_now),
            heirs: &[heir.code()],
        },
    )
    .unwrap();
    let kit_path = dir.join(name);
    std::fs::write(&kit_path, kit.to_json().unwrap()).unwrap();
    let phrase_path = dir.join("payee.phrase");
    std::fs::write(&phrase_path, phrase.as_str()).unwrap();
    (kit_path, phrase_path)
}

#[test]
fn plan_legacy_rejects_zero_and_too_many_heirs() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let out = sg(&home, &["plan", "legacy", "--every", "10m"]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("at least one heir"),
        "{}",
        text(&out.stderr)
    );

    let codes: Vec<String> = (0..4).map(|_| heir_code()).collect();
    let mut args = vec!["plan", "legacy", "--every", "10m"];
    for c in &codes {
        args.push("--heir");
        args.push(c);
    }
    let out = sg(&home, &args);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("at most 3 heirs"),
        "{}",
        text(&out.stderr)
    );

    let out = sg(
        &home,
        &[
            "plan", "legacy", "--every", "10m", "--heir", &codes[0], "--heir", &codes[0],
        ],
    );
    assert!(
        text(&out.stderr).contains("duplicate"),
        "{}",
        text(&out.stderr)
    );

    let out = sg(
        &home,
        &["plan", "legacy", "--every", "10m", "--heir", "nope"],
    );
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("heir code #1"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn plan_installments_validates_its_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let payee = heir_code();
    let run = |extra: &[&str]| {
        let mut a: Vec<&str> = vec!["plan", "installments", "--every", "2m"];
        a.extend_from_slice(extra);
        sg(&home, &a)
    };
    let out = run(&["--amount", "0.01", "--count", "3"]);
    assert!(
        text(&out.stderr).contains("at least one heir"),
        "{}",
        text(&out.stderr)
    );
    let out = run(&["--payee", &payee, "--amount", "0.01", "--count", "0"]);
    assert!(
        text(&out.stderr).contains("--count"),
        "{}",
        text(&out.stderr)
    );
    let out = run(&["--payee", &payee, "--amount", "0.00000001", "--count", "2"]);
    assert!(
        text(&out.stderr).contains("--amount must be at least"),
        "{}",
        text(&out.stderr)
    );
    let out = run(&[
        "--payee", &payee, "--amount", "0.01", "--count", "2", "--start", "soon",
    ]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("not a duration"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn claim_takes_a_links_file_and_orders_kits_by_unlock() {
    let dir = tempfile::tempdir().unwrap();
    let (late, phrase) = write_kit(dir.path(), "late.sandglass", 7200, 2);
    let early_dir = dir.path().join("e");
    std::fs::create_dir(&early_dir).unwrap();
    let (early, _) = write_kit(&early_dir, "early.sandglass", 3600, 1);
    let list = dir.path().join("links.txt");
    std::fs::write(
        &list,
        format!(
            "Some prose here.\n{}\nmore prose {}\n",
            late.display(),
            early.display()
        ),
    )
    .unwrap();
    let p = phrase.to_str().unwrap();
    let l = list.to_str().unwrap();
    let out = sg(
        dir.path(),
        &["claim", "--links-file", l, "--phrase-file", p],
    );
    assert!(!out.status.success());
    let s = text(&out.stdout);
    assert!(s.contains("Kit 1 of 2: installment 1 of 2"), "{s}");
    assert!(text(&out.stderr).contains("error[not_yet_unlocked]"));

    let out = sg(
        dir.path(),
        &["claim", "--watch", "--links-file", l, "--phrase-file", p],
    );
    assert!(text(&out.stderr).contains("--to"));
    let out = sg(dir.path(), &["claim", "--phrase-file", p]);
    assert!(text(&out.stderr).contains("at least one kit"));
}

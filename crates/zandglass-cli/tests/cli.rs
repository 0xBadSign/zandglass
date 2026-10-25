//! Offline CLI tests: no network, no funds.

use std::path::Path;
use std::process::{Command, Output};

use zandglass_core::heir::{generate_phrase, identity_from_phrase};
use zandglass_core::kit::{BuildParams, Kind, Network, Payload};
use zandglass_core::{drand, Kit};

const PLAN: &str = "00112233445566778899aabbccddeeff";
const SEED: &str = "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth title";

fn sg(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zandglass"))
        .env("ZANDGLASS_PASSPHRASE", "test-passphrase")
        .arg("--network")
        .arg("test")
        .arg("--data-dir")
        .arg(home)
        .args(args)
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn write_kit(dir: &Path, seconds_from_now: u64) -> (std::path::PathBuf, std::path::PathBuf) {
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
            kind: Kind::Legacy,
            sequence: None,
            now,
            unlock_round: drand::unlock_round(now + seconds_from_now),
            heirs: &[heir.code()],
        },
    )
    .unwrap();
    let kit_path = dir.join("x-e1.zandglass");
    std::fs::write(&kit_path, kit.to_json().unwrap()).unwrap();
    let phrase_path = dir.join("heir.phrase");
    std::fs::write(&phrase_path, phrase.as_str()).unwrap();
    (kit_path, phrase_path)
}

#[test]
fn heir_new_prints_phrase_and_code() {
    let dir = tempfile::tempdir().unwrap();
    let out = sg(dir.path(), &["heir", "new"]);
    assert!(out.status.success());
    let s = text(&out.stdout);
    assert!(s.contains("age1"));
    let phrase_line = s
        .lines()
        .find(|l| l.split_whitespace().count() == 24)
        .expect("a 24-word line");
    assert!(identity_from_phrase(phrase_line.trim()).is_ok());
}

#[test]
fn init_and_empty_status() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let out = sg(&home, &["status"]);
    assert!(!out.status.success(), "status before init must fail");
    assert!(text(&out.stderr).contains("zandglass init"));

    assert!(sg(&home, &["init"]).status.success());
    assert!(!sg(&home, &["init"]).status.success(), "second init fails");
    let out = sg(&home, &["status"]);
    assert!(out.status.success());
    assert!(text(&out.stdout).contains("No plans yet"));

    let bad = Command::new(env!("CARGO_BIN_EXE_zandglass"))
        .env("ZANDGLASS_PASSPHRASE", "wrong")
        .args(["--network", "test", "--data-dir"])
        .arg(&home)
        .arg("status")
        .output()
        .unwrap();
    assert!(text(&bad.stderr).contains("error[unauthorized]"));
}

#[test]
fn claim_before_unlock_is_refused_and_inspect_works() {
    let dir = tempfile::tempdir().unwrap();
    let (kit, phrase) = write_kit(dir.path(), 3600);
    let k = kit.to_str().unwrap();
    let p = phrase.to_str().unwrap();

    let out = sg(dir.path(), &["kit", "inspect", k]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let s = text(&out.stdout);
    assert!(s.contains(PLAN) && s.contains("network:  test"));
    assert!(!s.contains("legal winner"));

    let out = sg(dir.path(), &["claim", k, "--phrase-file", p]);
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("error[not_yet_unlocked]"));
}

#[test]
fn claim_rejects_network_mismatch_and_bad_kits() {
    let dir = tempfile::tempdir().unwrap();
    let (kit, phrase) = write_kit(dir.path(), 3600);
    let out = Command::new(env!("CARGO_BIN_EXE_zandglass"))
        .args(["--network", "main", "claim"])
        .arg(&kit)
        .arg("--phrase-file")
        .arg(&phrase)
        .output()
        .unwrap();
    assert!(text(&out.stderr).contains("error[network_mismatch]"));

    let junk = dir.path().join("junk.zandglass");
    std::fs::write(&junk, "{\"format\":\"zandglass-kit\",\"version\":9}").unwrap();
    let out = sg(dir.path(), &["kit", "inspect", junk.to_str().unwrap()]);
    assert!(text(&out.stderr).contains("error[kit_version_unsupported]"));
    std::fs::write(&junk, "nope").unwrap();
    let out = sg(dir.path(), &["kit", "inspect", junk.to_str().unwrap()]);
    assert!(text(&out.stderr).contains("error[kit_invalid]"));
}

#[test]
fn claim_link_fetches_the_kit_from_the_keeper() {
    use zandglass_core::mailbox::{ClaimLink, MailboxClient};
    use zandglass_wallet::ops::{delete_mailbox, upload_kit};
    use zandglass_wallet::{EpochRecord, Plan, PlanStatus};

    let dir = tempfile::tempdir().unwrap();
    let (addr, _state) =
        zandglass_keeper::spawn_background(&dir.path().join("k.sqlite"), Default::default())
            .unwrap();
    let base = format!("http://{addr}");
    let (kit_path, phrase) = write_kit(dir.path(), 3600);
    let p = phrase.to_str().unwrap();
    let kit = Kit::from_json(&std::fs::read_to_string(&kit_path).unwrap()).unwrap();

    // The plan side: what `plan legacy --mailbox` and `flip` do.
    let created = MailboxClient::new(&base).create().unwrap();
    let plan = Plan {
        id: PLAN.into(),
        kind: "legacy".into(),
        network: "test".into(),
        heirs: vec![],
        period_secs: 3600,
        message: None,
        epoch: 1,
        status: PlanStatus::Active,
        created: 1,
        mailbox_url: Some(base.clone()),
        mailbox_id: Some(created.id.clone()),
        mailbox_write_token: Some(created.write_token.clone()),
        epochs: vec![EpochRecord {
            epoch: 1,
            dir: "x".into(),
            birthday_height: 1,
            unlock_round: 1,
            unlock_time: 1,
            created: 1,
        }],
        installments: None,
        pending: None,
        archived: Vec::new(),
        lock: None,
    };
    upload_kit(&plan, &kit).unwrap();
    let link = ClaimLink::new(&base, &created).to_string();

    // The heir side: the link stands in for the kit file.
    let out = sg(dir.path(), &["claim", &link, "--phrase-file", p]);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("error[not_yet_unlocked]"),
        "the kit must have been fetched: {}",
        text(&out.stderr)
    );

    // A wrong read token is refused by the keeper.
    let bad = format!("{base}/claim#m={}&r={}", created.id, "0".repeat(64));
    let out = sg(dir.path(), &["claim", &bad, "--phrase-file", p]);
    assert!(text(&out.stderr).contains("error[unauthorized]"));

    // Cancel deletes the mailbox.
    delete_mailbox(&plan).unwrap();
    let out = sg(dir.path(), &["claim", &link, "--phrase-file", p]);
    assert!(text(&out.stderr).contains("error[mailbox_not_found]"));
}

#[test]
fn claim_link_with_an_owner_hint_insists_on_that_signer() {
    use zandglass_core::mailbox::{ClaimLink, MailboxClient};
    use zandglass_core::owner;

    let dir = tempfile::tempdir().unwrap();
    let (addr, _state) =
        zandglass_keeper::spawn_background(&dir.path().join("k.sqlite"), Default::default())
            .unwrap();
    let base = format!("http://{addr}");
    let mb = MailboxClient::new(&base);
    let (kit_path, phrase) = write_kit(dir.path(), 3600);
    let p = phrase.to_str().unwrap();
    let unsigned = Kit::from_json(&std::fs::read_to_string(&kit_path).unwrap()).unwrap();
    let secret = [9u8; 32];
    let mut signed = unsigned.clone();
    signed.sign_owner(&secret);
    let hint = owner::link_hint(&owner::public_key(&secret));
    let other_hint = owner::link_hint(&owner::public_key(&[10u8; 32]));

    let publish = |kit: &Kit| {
        let c = mb.create().unwrap();
        mb.put(&c.id, &c.write_token, &kit.to_json().unwrap())
            .unwrap();
        c
    };
    let claim = |link: &str| sg(dir.path(), &["claim", link, "--phrase-file", p]);
    let with_hint = |c: &zandglass_core::mailbox::Created, h: &str| {
        let mut l = ClaimLink::new(&base, c);
        l.owner_hint = Some(h.to_string());
        l.to_string()
    };

    // Signed kit, matching hint: fetched (it is not unlocked yet, which proves it got that far).
    let c = publish(&signed);
    let out = claim(&with_hint(&c, &hint));
    assert!(
        text(&out.stderr).contains("error[not_yet_unlocked]"),
        "{}",
        text(&out.stderr)
    );
    // Signed kit, hint of another key: refused.
    let out = claim(&with_hint(&c, &other_hint));
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("error[kit_invalid]")
            && text(&out.stderr).contains("not sealed by the owner"),
        "{}",
        text(&out.stderr)
    );
    // Unsigned kit behind a link that names a key: refused.
    let c = publish(&unsigned);
    let out = claim(&with_hint(&c, &hint));
    assert!(
        text(&out.stderr).contains("error[kit_invalid]")
            && text(&out.stderr).contains("not sealed by the owner"),
        "{}",
        text(&out.stderr)
    );
    // An old link without a hint keeps working for the unsigned kit.
    let out = claim(&ClaimLink::new(&base, &c).to_string());
    assert!(
        text(&out.stderr).contains("error[not_yet_unlocked]"),
        "{}",
        text(&out.stderr)
    );
    // A malformed hint is not a claim link at all.
    let bad = format!("{base}/claim#m={}&r={}&o=zz", c.id, c.read_token);
    let out = claim(&bad);
    assert!(!out.status.success());
    assert!(!text(&out.stderr).contains("not_yet_unlocked"));
}

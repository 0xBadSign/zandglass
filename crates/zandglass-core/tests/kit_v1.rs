//! Kit v1 build, validation, JSON and opening, offline against the recorded beacon.

use zandglass_core::heir::{generate_phrase, identity_from_phrase, HeirCode, HeirIdentity};
use zandglass_core::kit::{BuildParams, Kind, Network, Sequence};
use zandglass_core::{Beacon, Kit, KitError, Payload};

const VECTORS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors");
const PLAN: &str = "00112233445566778899aabbccddeeff";
const SEED: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

fn beacon() -> Beacon {
    let json = std::fs::read_to_string(format!("{VECTORS}/beacon.json")).unwrap();
    Beacon::from_json(&json).unwrap()
}

fn heir() -> HeirIdentity {
    identity_from_phrase(&generate_phrase()).unwrap()
}

fn payload() -> Payload {
    Payload {
        vault_seed: SEED.into(),
        birthday_height: 4_423_551,
        network: Network::Test,
        plan: PLAN.into(),
        epoch: 2,
        message: Some("With love.".into()),
        restore_hints: Payload::default_hints(4_423_551),
    }
}

fn build(heirs: &[HeirCode], round: u64) -> Kit {
    Kit::build(
        &payload(),
        &BuildParams {
            kind: Kind::Legacy,
            sequence: None,
            now: 1_790_000_000,
            unlock_round: round,
            heirs,
        },
    )
    .unwrap()
}

#[test]
fn build_open_round_trip_through_json() {
    let b = beacon();
    let h = heir();
    let kit = build(&[h.code()], b.round());
    let text = kit.to_json().unwrap();
    assert!(text.contains("\"format\": \"zandglass-kit\""));
    assert!(!text.contains(SEED));
    assert!(!text.contains("sequence"));
    let parsed = Kit::from_json(&text).unwrap();
    assert_eq!(parsed, kit);
    assert_eq!(parsed.unlock.round, b.round());
    assert_eq!(parsed.network, Network::Test);
    let opened = parsed.open(&h, &b).unwrap();
    assert_eq!(opened, payload());
}

#[test]
fn three_heirs_and_sequence() {
    let b = beacon();
    let heirs = [heir(), heir(), heir()];
    let codes: Vec<HeirCode> = heirs.iter().map(HeirIdentity::code).collect();
    let kit = Kit::build(
        &payload(),
        &BuildParams {
            kind: Kind::Installment,
            sequence: Some(Sequence {
                index: 1,
                count: 12,
            }),
            now: 1_790_000_000,
            unlock_round: b.round(),
            heirs: &codes,
        },
    )
    .unwrap();
    let kit = Kit::from_json(&kit.to_json().unwrap()).unwrap();
    assert_eq!(
        kit.sequence,
        Some(Sequence {
            index: 1,
            count: 12
        })
    );
    for h in &heirs {
        assert_eq!(kit.open(h, &b).unwrap().vault_seed, SEED);
    }
    let too_many: Vec<HeirCode> = (0..4).map(|_| heir().code()).collect();
    let err = Kit::build(
        &payload(),
        &BuildParams {
            kind: Kind::Legacy,
            sequence: None,
            now: 0,
            unlock_round: 5,
            heirs: &too_many,
        },
    )
    .unwrap_err();
    assert_eq!(err.code(), "kit_invalid");
}

#[test]
fn unknown_version_is_rejected_and_unknown_fields_ignored() {
    let b = beacon();
    let kit = build(&[heir().code()], b.round());
    let mut v: serde_json::Value = serde_json::from_str(&kit.to_json().unwrap()).unwrap();

    v["extra_field"] = serde_json::json!({"anything": 1});
    v["unlock"]["note"] = serde_json::json!("ignored");
    assert!(Kit::from_json(&v.to_string()).is_ok());

    v["version"] = serde_json::json!(2);
    let err = Kit::from_json(&v.to_string()).unwrap_err();
    assert!(matches!(err, KitError::VersionUnsupported(2)));
    assert_eq!(err.code(), "kit_version_unsupported");
}

#[test]
fn validation_catches_inconsistencies() {
    let b = beacon();
    let kit = build(&[heir().code()], b.round());
    let code = |k: &Kit| Kit::from_json(&k.to_json().unwrap()).unwrap_err().code();

    let mut k = kit.clone();
    k.format = "other".into();
    assert_eq!(code(&k), "kit_invalid");

    let mut k = kit.clone();
    k.unlock.round += 1;
    k.unlock.time =
        zandglass_core::kit::format_time(zandglass_core::drand::round_time(k.unlock.round));
    assert_eq!(
        code(&k),
        "kit_invalid",
        "round differs from the sealed header"
    );

    let mut k = kit.clone();
    k.unlock.time = "2030-01-01T00:00:00Z".into();
    assert_eq!(code(&k), "kit_invalid");

    let mut k = kit.clone();
    k.unlock.drand_chain = "00".repeat(32);
    assert_eq!(code(&k), "kit_invalid");

    let mut k = kit.clone();
    k.plan = "NOT-HEX".into();
    assert_eq!(code(&k), "kit_invalid");

    let mut k = kit.clone();
    k.sealed = "garbage".into();
    assert_eq!(code(&k), "kit_invalid");

    assert_eq!(
        Kit::from_json("not json").unwrap_err().code(),
        "kit_invalid"
    );
    assert_eq!(Kit::from_json("{}").unwrap_err().code(), "kit_invalid");
}

#[test]
fn wrong_key_early_and_mismatch_errors() {
    let b = beacon();
    let h = heir();
    let kit = build(&[h.code()], b.round());
    assert_eq!(kit.open(&heir(), &b).unwrap_err().code(), "wrong_heir_key");

    let future = build(&[h.code()], 500_000_000);
    assert_eq!(future.open(&h, &b).unwrap_err().code(), "not_yet_unlocked");

    // A payload that disagrees with the public part is refused.
    let mut lying = kit.clone();
    lying.epoch = 9;
    assert_eq!(lying.open(&h, &b).unwrap_err().code(), "decrypt_failed");
}

#[test]
fn payload_ignores_unknown_fields_and_defaults_optionals() {
    let json = format!(
        r#"{{"vault_seed":"{SEED}","birthday_height":5,"network":"main","plan":"{PLAN}","epoch":1,"future":true}}"#
    );
    let p: Payload = serde_json::from_str(&json).unwrap();
    assert!(p.message.is_none());
    assert!(p.restore_hints.is_empty());
}

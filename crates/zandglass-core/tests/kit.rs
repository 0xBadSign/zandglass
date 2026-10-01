//! Kit sealing and opening, offline against a recorded quicknet beacon.
//!
//! The recorded beacon (round in `vectors/round.txt`) is a real, verified
//! quicknet beacon. Kits sealed to exactly that round open offline with it;
//! kits sealed to a later round must stay locked.

use zandglass_core::heir::{generate_phrase, identity_from_phrase, HeirCode, HeirIdentity};
use zandglass_core::seal::kit_round;
use zandglass_core::{open, seal, Beacon, SealError};

const VECTORS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors");

fn read(name: &str) -> String {
    std::fs::read_to_string(format!("{VECTORS}/{name}")).expect(name)
}

fn recorded_beacon() -> Beacon {
    Beacon::from_json(&read("beacon.json")).expect("recorded beacon verifies")
}

fn new_heir() -> HeirIdentity {
    identity_from_phrase(&generate_phrase()).expect("identity")
}

#[test]
fn round_trip_one_heir() {
    let beacon = recorded_beacon();
    let heir = new_heir();
    let kit = seal(b"vault seed words", &[heir.code()], beacon.round()).unwrap();
    assert!(kit.starts_with("-----BEGIN AGE ENCRYPTED FILE-----"));
    assert!(kit.is_ascii());
    assert_eq!(kit_round(&kit).unwrap(), beacon.round());
    assert_eq!(open(&kit, &heir, &beacon).unwrap(), b"vault seed words");
}

#[test]
fn round_trip_three_heirs_each_can_open() {
    let beacon = recorded_beacon();
    let heirs = [new_heir(), new_heir(), new_heir()];
    let codes: Vec<HeirCode> = heirs.iter().map(HeirIdentity::code).collect();
    let kit = seal(b"shared", &codes, beacon.round()).unwrap();
    for heir in &heirs {
        assert_eq!(open(&kit, heir, &beacon).unwrap(), b"shared");
    }
}

#[test]
fn wrong_heir_key_is_refused() {
    let beacon = recorded_beacon();
    let kit = seal(b"secret", &[new_heir().code()], beacon.round()).unwrap();
    let err = open(&kit, &new_heir(), &beacon).unwrap_err();
    assert!(matches!(err, SealError::WrongHeirKey), "{err:?}");
}

#[test]
fn tampered_kit_is_damaged() {
    let beacon = recorded_beacon();
    let heir = new_heir();
    let kit = seal(b"secret", &[heir.code()], beacon.round()).unwrap();

    // Flip one character in the middle of the armored body.
    let mut bytes = kit.clone().into_bytes();
    let mid = kit.len() / 2;
    bytes[mid] = if bytes[mid] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(bytes).unwrap();
    let err = open(&tampered, &heir, &beacon).unwrap_err();
    assert!(matches!(err, SealError::Damaged(_)), "{err:?}");

    // Truncation.
    let truncated = &kit[..kit.len() - 80];
    let err = open(truncated, &heir, &beacon).unwrap_err();
    assert!(matches!(err, SealError::Damaged(_)), "{err:?}");

    // Not a kit at all.
    let err = open("hello", &heir, &beacon).unwrap_err();
    assert!(matches!(err, SealError::Damaged(_)), "{err:?}");
}

#[test]
fn far_future_round_stays_locked() {
    let beacon = recorded_beacon();
    let heir = new_heir();
    let future = 500_000_000;
    let kit = seal(b"later", &[heir.code()], future).unwrap();
    match open(&kit, &heir, &beacon).unwrap_err() {
        SealError::NotYetUnlocked { needed, have } => {
            assert_eq!(needed, future);
            assert_eq!(have, beacon.round());
        }
        other => panic!("unexpected: {other:?}"),
    }
    // A forged signature for the future round must not open it either.
    let forged = Beacon::from_parts(future, vec![7u8; 48]);
    let err = open(&kit, &heir, &forged).unwrap_err();
    assert!(matches!(err, SealError::Damaged(_)), "{err:?}");
}

#[test]
fn beacon_of_a_later_round_is_not_the_key() {
    let beacon = recorded_beacon();
    let heir = new_heir();
    let kit = seal(b"earlier", &[heir.code()], beacon.round() - 1).unwrap();
    let err = open(&kit, &heir, &beacon).unwrap_err();
    assert!(matches!(err, SealError::WrongBeaconRound { .. }), "{err:?}");
}

#[test]
fn heir_count_limits() {
    let heirs: Vec<HeirCode> = (0..4).map(|_| new_heir().code()).collect();
    assert!(matches!(seal(b"x", &[], 1), Err(SealError::NoHeirs)));
    assert!(matches!(
        seal(b"x", &heirs, 1),
        Err(SealError::TooManyHeirs)
    ));
}

#[test]
fn committed_vector_opens_offline() {
    let identity = HeirIdentity::from_secret_string(&read("TEST_ONLY_identity.txt")).unwrap();
    let phrase_identity = identity_from_phrase(read("TEST_ONLY_phrase.txt").trim()).unwrap();
    assert_eq!(identity.code(), phrase_identity.code());
    assert_eq!(
        identity.code(),
        HeirCode::parse(&read("heir_code.txt")).unwrap()
    );

    let kit = read("kit.txt");
    let round: u64 = read("round.txt").trim().parse().unwrap();
    assert_eq!(kit_round(&kit).unwrap(), round);
    let payload = open(&kit, &identity, &recorded_beacon()).unwrap();
    assert_eq!(payload, read("payload.txt").as_bytes());
}

#[test]
#[ignore = "network"]
fn committed_vector_opens_with_live_beacon() {
    let identity = HeirIdentity::from_secret_string(&read("TEST_ONLY_identity.txt")).unwrap();
    let kit = read("kit.txt");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let payload = zandglass_core::seal::open_online(&kit, &identity, now).unwrap();
    assert_eq!(payload, read("payload.txt").as_bytes());
}

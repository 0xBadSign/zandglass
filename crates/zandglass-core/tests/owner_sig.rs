//! Owner-signed kits: sign and verify, tampering, unsigned kits, and the
//! cross-language vector the TypeScript tests verify as well.

use sha2::{Digest, Sha256};
use zandglass_core::heir::{generate_phrase, identity_from_phrase};
use zandglass_core::kit::{BuildParams, Kind, Network, Payload, Sequence};
use zandglass_core::owner;
use zandglass_core::{Kit, KitError};

const VECTORS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors");
const PLAN: &str = "00112233445566778899aabbccddeeff";
const SEED: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
const KEY_LABEL: &[u8] = b"zandglass TEST_ONLY owner signing key v1";

fn test_secret() -> [u8; 32] {
    Sha256::digest(KEY_LABEL).into()
}

fn unsigned_kit() -> Kit {
    let heir = identity_from_phrase(&generate_phrase()).unwrap();
    Kit::build(
        &Payload {
            vault_seed: SEED.into(),
            birthday_height: 4_423_551,
            network: Network::Test,
            plan: PLAN.into(),
            epoch: 2,
            message: None,
            restore_hints: Payload::default_hints(4_423_551),
        },
        &BuildParams {
            kind: Kind::Installment,
            sequence: Some(Sequence { index: 2, count: 5 }),
            now: 1_790_000_000,
            unlock_round: 32_665_869,
            heirs: &[heir.code()],
        },
    )
    .unwrap()
}

fn reparse(kit: &Kit) -> Result<Kit, KitError> {
    Kit::from_json(&kit.to_json().unwrap())
}

#[test]
fn sign_and_verify_round_trip() {
    let mut kit = unsigned_kit();
    assert_eq!(kit.verify_owner_sig().unwrap(), None);
    kit.sign_owner(&test_secret());
    let key = owner::public_key(&test_secret());
    assert_eq!(kit.owner_key.as_deref(), Some(hex::encode(key).as_str()));
    assert_eq!(kit.verify_owner_sig().unwrap(), Some(key));
    // Survives JSON, and fixed-length fields keep the kit size constant.
    let parsed = reparse(&kit).unwrap();
    assert_eq!(parsed, kit);
    assert_eq!(parsed.verify_owner_sig().unwrap(), Some(key));
    let mut other = unsigned_kit();
    other.sign_owner(&[7u8; 32]);
    assert_eq!(
        other.to_json().unwrap().len() - other.sealed.len(),
        kit.to_json().unwrap().len() - kit.sealed.len(),
        "signed kits have a fixed overhead"
    );
}

#[test]
fn claim_link_carries_the_owner_hint_of_a_signed_kit() {
    use zandglass_core::mailbox::{ClaimLink, Created};
    let created = Created {
        id: "ab12".into(),
        write_token: "w".into(),
        read_token: "cd34".into(),
    };
    let mut kit = unsigned_kit();
    let plain = ClaimLink::for_kit("https://k.example", &created, &kit);
    assert_eq!(plain.owner_hint, None);
    assert_eq!(plain.to_string(), "https://k.example/claim#m=ab12&r=cd34");
    kit.sign_owner(&test_secret());
    let key = owner::public_key(&test_secret());
    let link = ClaimLink::for_kit("https://k.example", &created, &kit);
    let want = hex::encode(&Sha256::digest(key)[..16]);
    assert_eq!(want.len(), 32);
    assert_eq!(link.owner_hint.as_deref(), Some(want.as_str()));
    assert_eq!(owner::link_hint(&key), want);
    assert_eq!(
        link.to_string(),
        format!("https://k.example/claim#m=ab12&r=cd34&o={want}")
    );
    assert_eq!(ClaimLink::parse(&link.to_string()), Some(link));
}

#[test]
fn unsigned_kits_still_parse() {
    let kit = unsigned_kit();
    let text = kit.to_json().unwrap();
    assert!(!text.contains("owner_"));
    let parsed = Kit::from_json(&text).unwrap();
    assert_eq!(parsed.verify_owner_sig().unwrap(), None);
}

#[test]
fn any_change_to_the_public_part_or_sealed_breaks_the_signature() {
    let mut signed = unsigned_kit();
    signed.sign_owner(&test_secret());

    type Mutation = Box<dyn Fn(&mut Kit)>;
    let mutations: Vec<(&str, Mutation)> = vec![
        ("sealed", Box::new(|k| k.sealed.push('\n'))),
        ("epoch", Box::new(|k| k.epoch += 1)),
        ("plan", Box::new(|k| k.plan = "f".repeat(32))),
        (
            "created",
            Box::new(|k| k.created = "2026-01-01T00:00:00Z".into()),
        ),
        ("network", Box::new(|k| k.network = Network::Main)),
        ("kind", Box::new(|k| k.kind = Kind::Legacy)),
        ("sequence", Box::new(|k| k.sequence = None)),
        (
            "sequence index",
            Box::new(|k| k.sequence = Some(Sequence { index: 3, count: 5 })),
        ),
    ];
    for (name, mutate) in mutations {
        let mut k = signed.clone();
        mutate(&mut k);
        let err = k.verify_owner_sig().unwrap_err();
        assert_eq!(err.code(), "kit_invalid", "{name}");
        assert!(
            err.to_string().contains("owner signature") || name == "sealed",
            "{name}: {err}"
        );
    }

    // A different owner key with the old signature fails too.
    let mut k = signed.clone();
    k.owner_key = Some(hex::encode(owner::public_key(&[9u8; 32])));
    assert!(k.verify_owner_sig().is_err());

    // The signature of another kit does not transfer.
    let mut a = unsigned_kit();
    a.sign_owner(&test_secret());
    let mut b = unsigned_kit();
    b.epoch = 3;
    b.owner_key = a.owner_key.clone();
    b.owner_sig = a.owner_sig.clone();
    assert!(b.verify_owner_sig().is_err());
}

#[test]
fn malformed_owner_fields_are_refused_at_parse() {
    let mut signed = unsigned_kit();
    signed.sign_owner(&test_secret());
    let json = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut v: serde_json::Value = serde_json::from_str(&signed.to_json().unwrap()).unwrap();
        edit(&mut v);
        Kit::from_json(&v.to_string())
    };
    // Only one of the two fields.
    assert!(json(&|v| {
        v.as_object_mut().unwrap().remove("owner_sig");
    })
    .is_err());
    assert!(json(&|v| {
        v.as_object_mut().unwrap().remove("owner_key");
    })
    .is_err());
    // Wrong length, upper case, non-hex.
    assert!(json(&|v| v["owner_key"] = "ab".into()).is_err());
    let upper = signed.owner_sig.clone().unwrap().to_uppercase();
    assert!(json(&|v| v["owner_sig"] = upper.clone().into()).is_err());
    assert!(json(&|v| v["owner_sig"] = "zz".repeat(64).into()).is_err());
    // Both removed: simply unsigned.
    assert!(json(&|v| {
        let o = v.as_object_mut().unwrap();
        o.remove("owner_key");
        o.remove("owner_sig");
    })
    .is_ok());
    // A bad signature is refused by `from_json`, so `zandglass claim` stops.
    let mut bad = signed.clone();
    bad.epoch = 9;
    assert_eq!(reparse(&bad).unwrap_err().code(), "kit_invalid");
}

#[test]
fn fingerprint_format() {
    let fp = owner::fingerprint(&owner::public_key(&test_secret()));
    assert_eq!(fp.len(), 19);
    let groups: Vec<&str> = fp.split('-').collect();
    assert_eq!(groups.len(), 4);
    assert!(groups
        .iter()
        .all(|g| g.len() == 4 && g.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))));
}

// ---- cross-language vector --------------------------------------------

/// The signed kit used by `web/src/lib/owner-sig.test.ts` too. The sealed
/// part is the recorded `kit.txt` vector, so it is a real armoured kit.
fn vector_kit() -> Kit {
    let sealed = std::fs::read_to_string(format!("{VECTORS}/kit.txt")).unwrap();
    let round: u64 = std::fs::read_to_string(format!("{VECTORS}/round.txt"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let json = serde_json::json!({
        "format": "zandglass-kit",
        "version": 1,
        "kind": "installment",
        "network": "test",
        "created": "2026-09-30T20:30:00Z",
        "plan": PLAN,
        "epoch": 2,
        "sequence": {"index": 2, "count": 5},
        "unlock": {
            "drand_chain": zandglass_core::drand::CHAIN_HASH_HEX,
            "round": round,
            "time": zandglass_core::kit::format_time(zandglass_core::drand::round_time(round)),
        },
        "sealed": sealed,
    });
    let mut kit = Kit::from_json(&json.to_string()).unwrap();
    kit.sign_owner(&test_secret());
    kit
}

fn key_file_text(kit: &Kit) -> String {
    let key = kit.verify_owner_sig().unwrap().unwrap();
    format!(
        "TEST_ONLY owner signing key vector. The secret is sha256 of the label \"{}\".\npublic={}\nfingerprint={}\n",
        String::from_utf8_lossy(KEY_LABEL),
        hex::encode(key),
        owner::fingerprint(&key)
    )
}

#[test]
fn cross_language_vector_is_current() {
    let kit = vector_kit();
    let files = [
        ("TEST_ONLY_owner_kit.zandglass", kit.to_json().unwrap()),
        (
            "TEST_ONLY_owner_message.txt",
            String::from_utf8(owner::signing_message(
                &kit,
                kit.owner_key.as_deref().unwrap(),
            ))
            .unwrap(),
        ),
        ("TEST_ONLY_owner_key.txt", key_file_text(&kit)),
    ];
    for (name, text) in files {
        let path = format!("{VECTORS}/{name}");
        if std::env::var_os("ZANDGLASS_WRITE_VECTORS").is_some() {
            std::fs::write(&path, &text).unwrap();
        }
        // Ed25519 signing is deterministic, so the committed file must match.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{name}");
    }
    // And the committed kit verifies as parsed from disk.
    let on_disk =
        std::fs::read_to_string(format!("{VECTORS}/TEST_ONLY_owner_kit.zandglass")).unwrap();
    let parsed = Kit::from_json(&on_disk).unwrap();
    assert_eq!(
        parsed.verify_owner_sig().unwrap(),
        Some(owner::public_key(&test_secret()))
    );
}

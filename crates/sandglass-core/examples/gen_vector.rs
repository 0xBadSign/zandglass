//! Regenerates the committed test vectors in `tests/vectors/`.
//!
//! Usage: `cargo run -p sandglass-core --example gen_vector -- <beacon.json> <out-dir>`
//!
//! The beacon JSON is a recorded quicknet beacon of a round in the past
//! (for example `curl https://api.drand.sh/<chain>/public/<round>`). The kit
//! is sealed to exactly that round, so anyone can open it offline with the
//! recorded beacon, or online with any drand client. The heir phrase is a
//! fixed TEST ONLY phrase and protects nothing.

use std::fs;
use std::path::PathBuf;

use age::secrecy::ExposeSecret;
use bip0039::Mnemonic;
use sandglass_core::heir::identity_from_phrase;
use sandglass_core::{open, seal, Beacon};

const PAYLOAD: &str = "TEST ONLY sandglass vector payload: not a real seed.\n";

fn main() {
    let mut args = std::env::args().skip(1);
    let beacon_path = PathBuf::from(args.next().expect("beacon.json path"));
    let out = PathBuf::from(args.next().expect("output directory"));

    let beacon_json = fs::read_to_string(&beacon_path).expect("read beacon");
    let beacon = Beacon::from_json(&beacon_json).expect("beacon verifies");

    let phrase = Mnemonic::<bip0039::English>::from_entropy([0x42u8; 32])
        .expect("entropy")
        .into_phrase();
    let identity = identity_from_phrase(&phrase).expect("identity");
    let code = identity.code();

    let kit = seal(
        PAYLOAD.as_bytes(),
        std::slice::from_ref(&code),
        beacon.round(),
    )
    .expect("seal");
    let opened = open(&kit, &identity, &beacon).expect("self-check open");
    assert_eq!(opened, PAYLOAD.as_bytes());

    let w = |name: &str, content: &str| fs::write(out.join(name), content).expect(name);
    w("TEST_ONLY_phrase.txt", &format!("{phrase}\n"));
    w(
        "TEST_ONLY_identity.txt",
        &format!("{}\n", identity.to_secret_string().expose_secret()),
    );
    w("heir_code.txt", &format!("{code}\n"));
    w("payload.txt", PAYLOAD);
    w("kit.txt", &kit);
    w("round.txt", &format!("{}\n", beacon.round()));
    w("beacon.json", &format!("{}\n", beacon_json.trim()));
    println!("wrote vectors for round {}", beacon.round());
}

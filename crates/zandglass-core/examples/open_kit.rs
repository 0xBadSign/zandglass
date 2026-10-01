//! Opens a kit file with the Rust library (used by `scripts/interop-kit.sh`).
//!
//! Usage: `open_kit <kit-file> <identity-file> [beacon.json]`
//!
//! Without a beacon file the beacon is fetched from the drand relays.
//! The payload is written to stdout.

use std::io::Write;

use zandglass_core::seal::open_online;
use zandglass_core::{open, Beacon, HeirIdentity};

fn main() {
    let mut args = std::env::args().skip(1);
    let kit = std::fs::read_to_string(args.next().expect("kit file")).expect("read kit");
    let identity =
        std::fs::read_to_string(args.next().expect("identity file")).expect("read identity");
    let identity = HeirIdentity::from_secret_string(&identity).expect("identity");

    let payload = match args.next() {
        Some(path) => {
            let json = std::fs::read_to_string(path).expect("read beacon");
            let beacon = Beacon::from_json(&json).expect("beacon");
            open(&kit, &identity, &beacon)
        }
        None => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_secs();
            open_online(&kit, &identity, now)
        }
    };
    match payload {
        Ok(p) => std::io::stdout().write_all(&p).expect("write"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

//! `sandglass-keeper`: hosts blind mailboxes (it can never open a kit) and the
//! mainnet playground.

fn main() {
    // allow-print: startup banner until the server lands (T2.1).
    println!("sandglass-keeper {}", sandglass_core::VERSION);
}

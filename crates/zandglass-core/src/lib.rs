//! Zandglass core: drand round math, sealed kits and heir keys.
//!
//! A kit is a vault seed encrypted to one or more heir codes (age X25519)
//! and then to a future drand quicknet round (tlock). See `docs/PLAN.md`.

pub mod drand;
pub mod heir;
pub mod kit;
pub mod mailbox;
pub mod seal;

pub use drand::Beacon;
pub use heir::{HeirCode, HeirError, HeirIdentity};
pub use kit::{Kit, KitError, Payload};
pub use seal::{open, seal, SealError};

/// Crate version, for kit metadata and diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}

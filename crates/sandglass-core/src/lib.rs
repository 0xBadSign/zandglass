//! Sandglass core: drand round math, sealed kits and heir keys.
//!
//! A kit is a vault seed encrypted to one or more heir codes (age X25519)
//! and then to a future drand quicknet round (tlock). See `docs/PLAN.md`.

/// Crate version, for kit metadata and diagnostics.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}

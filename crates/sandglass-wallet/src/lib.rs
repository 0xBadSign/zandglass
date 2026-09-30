//! Sandglass vault manager: one Zcash seed per vault epoch, light-client
//! sync, and ordinary shielded sweeps (fund, flip, split, claim).
//!
//! A vault is a dedicated BIP 39 24-word seed that uses account 0 only. All
//! value movement uses the standard `zcash_client_backend` proposal flow.

pub mod blockcache;
pub mod lightwalletd;
pub mod vault;

pub use lightwalletd::{connect, default_url, Client};
pub use vault::{Balances, PoolBalance, SweepReport, Vault};
pub use zcash_protocol::consensus::Network;

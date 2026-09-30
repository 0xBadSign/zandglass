//! Sandglass vault manager: one Zcash seed per vault epoch, light-client
//! sync, and ordinary shielded sweeps (fund, flip, split, claim).
//!
//! A vault is a dedicated BIP 39 24-word seed that uses account 0 only. All
//! value movement uses the standard `zcash_client_backend` proposal flow.

pub mod blockcache;
pub mod error;
pub mod lightwalletd;
pub mod ops;
pub mod store;
pub mod vault;

pub use error::{coded, CodedError};
pub use lightwalletd::{connect, default_url, Client};
pub use store::{EpochRecord, Plan, PlanStatus, Store, StoreError};
pub use vault::{Balances, PoolBalance, SweepReport, Vault};
pub use zcash_protocol::consensus::Network;

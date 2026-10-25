//! Kit v1: the `*.zandglass` JSON file (PLAN section 5.4).
//!
//! The public part carries only metadata (plan id, epoch, unlock round). The
//! `sealed` field is the nested age/tlock armor from [`crate::seal`], holding
//! the [`Payload`] with the vault seed.

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::drand::{self, Beacon, CHAIN_HASH_HEX};
use crate::heir::{HeirCode, HeirIdentity};
use crate::seal::{self, SealError};

/// The `format` field of every kit.
pub const FORMAT: &str = "zandglass-kit";
/// The only kit version this library reads and writes.
pub const VERSION: u32 = 1;

/// Errors from building, parsing, validating and opening kits. Each maps to
/// a shared error code (PLAN section 5.10) through [`KitError::code`].
#[derive(Debug, thiserror::Error)]
pub enum KitError {
    /// The file is not a valid kit (bad JSON, wrong format, inconsistent fields).
    #[error("this is not a valid Zandglass kit: {0}")]
    Invalid(String),
    /// The kit has a version this build does not understand.
    #[error("this kit has version {0}, which this version of Zandglass cannot read")]
    VersionUnsupported(u64),
    /// The kit's unlock round has not been published yet.
    #[error("the kit is still locked: it unlocks at drand round {needed} and the newest round is {have}")]
    NotYetUnlocked {
        /// Round the kit is locked to.
        needed: u64,
        /// Newest round known.
        have: u64,
    },
    /// The given heir key is not one of the kit's heirs.
    #[error("this heir key cannot open the kit")]
    WrongHeirKey,
    /// The sealed part is damaged or was modified.
    #[error("the kit could not be decrypted, it may be damaged: {0}")]
    DecryptFailed(String),
    /// The drand beacon could not be fetched.
    #[error("could not fetch the drand beacon: {0}")]
    BeaconUnavailable(String),
    /// The payload disagrees with the public part, or the network differs.
    #[error("the kit is for network {kit}, but {wanted} was requested")]
    NetworkMismatch {
        /// Network named in the kit.
        kit: Network,
        /// Network that was requested.
        wanted: Network,
    },
}

impl KitError {
    /// The shared machine-readable error code (PLAN section 5.10).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "kit_invalid",
            Self::VersionUnsupported(_) => "kit_version_unsupported",
            Self::NotYetUnlocked { .. } => "not_yet_unlocked",
            Self::WrongHeirKey => "wrong_heir_key",
            Self::DecryptFailed(_) => "decrypt_failed",
            Self::BeaconUnavailable(_) => "beacon_unavailable",
            Self::NetworkMismatch { .. } => "network_mismatch",
        }
    }
}

impl From<SealError> for KitError {
    fn from(e: SealError) -> Self {
        match e {
            SealError::NotYetUnlocked { needed, have } => Self::NotYetUnlocked { needed, have },
            SealError::WrongHeirKey => Self::WrongHeirKey,
            SealError::Drand(d) => Self::BeaconUnavailable(d.to_string()),
            SealError::WrongChain => Self::Invalid("locked to a different drand chain".into()),
            SealError::NoHeirs | SealError::TooManyHeirs | SealError::Seal(_) => {
                Self::Invalid(e.to_string())
            }
            SealError::WrongBeaconRound { .. } | SealError::Damaged(_) => {
                Self::DecryptFailed(e.to_string())
            }
        }
    }
}

/// Zcash network a kit belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    /// Mainnet.
    Main,
    /// Testnet.
    Test,
}

impl std::fmt::Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Main => "main",
            Self::Test => "test",
        })
    }
}

/// What the plan is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A legacy (inheritance) plan.
    Legacy,
    /// An installment schedule.
    Installment,
}

/// Position inside an installment schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sequence {
    /// 1-based index of this installment.
    pub index: u32,
    /// Number of installments.
    pub count: u32,
}

/// When and how the kit unlocks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unlock {
    /// Hex hash of the drand chain (quicknet).
    pub drand_chain: String,
    /// The drand round whose beacon opens the outer layer.
    pub round: u64,
    /// That round's publication time, RFC 3339 UTC.
    pub time: String,
}

/// A kit: public metadata plus the sealed vault.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kit {
    /// Always `zandglass-kit`.
    pub format: String,
    /// Kit version (1).
    pub version: u32,
    /// Plan kind.
    pub kind: Kind,
    /// Zcash network.
    pub network: Network,
    /// Creation time, RFC 3339 UTC.
    pub created: String,
    /// Random 128-bit plan id, hex.
    pub plan: String,
    /// Vault epoch (1 for the first vault, plus one per flip).
    pub epoch: u32,
    /// Installment position (installment plans only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<Sequence>,
    /// Unlock round and time.
    pub unlock: Unlock,
    /// Armored nested ciphertext (tlock over age over the payload).
    pub sealed: String,
    /// Owner public key (Ed25519, 64 lowercase hex), present on signed kits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_key: Option<String>,
    /// Owner signature over the public part (128 lowercase hex), see [`crate::owner`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_sig: Option<String>,
}

/// The secret contents of a kit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    /// The vault's 24-word seed.
    pub vault_seed: String,
    /// Block height to start scanning from when restoring.
    pub birthday_height: u32,
    /// Network of the vault.
    pub network: Network,
    /// Plan id (same as the public part).
    pub plan: String,
    /// Epoch (same as the public part).
    pub epoch: u32,
    /// Optional words from the owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Human steps for restoring in common wallets.
    #[serde(default)]
    pub restore_hints: Vec<String>,
}

impl Drop for Payload {
    fn drop(&mut self) {
        self.vault_seed.zeroize();
    }
}

impl Payload {
    /// The standard restore hints for a vault.
    pub fn default_hints(birthday_height: u32) -> Vec<String> {
        vec![
            format!(
                "Zodl, Zkool, Zingo, Vizor: choose Restore, enter the 24 words and set the birthday height to {birthday_height}."
            ),
            format!(
                "zandglass claim <kit-file> --phrase-file <file> --to <your address> sweeps the vault for you (birthday {birthday_height})."
            ),
        ]
    }
}

/// Unix seconds to RFC 3339 UTC (`2026-10-05T12:00:00Z`).
pub fn format_time(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let rem = unix % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn is_hex_id(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Inputs for [`Kit::build`].
pub struct BuildParams<'a> {
    /// Plan kind.
    pub kind: Kind,
    /// Installment position, if any.
    pub sequence: Option<Sequence>,
    /// Current unix time, for the `created` field.
    pub now: u64,
    /// The drand round to lock to.
    pub unlock_round: u64,
    /// 1 to 3 heir codes.
    pub heirs: &'a [HeirCode],
}

impl Kit {
    /// Seal `payload` into a new kit. The public fields are taken from the
    /// payload so the two cannot disagree.
    pub fn build(payload: &Payload, params: &BuildParams<'_>) -> Result<Self, KitError> {
        if !is_hex_id(&payload.plan) {
            return Err(KitError::Invalid(
                "plan id must be 32 lowercase hex characters".into(),
            ));
        }
        let mut json = serde_json::to_vec(payload).map_err(|e| KitError::Invalid(e.to_string()))?;
        let sealed = seal::seal(&json, params.heirs, params.unlock_round);
        json.zeroize();
        let kit = Self {
            format: FORMAT.into(),
            version: VERSION,
            kind: params.kind,
            network: payload.network,
            created: format_time(params.now),
            plan: payload.plan.clone(),
            epoch: payload.epoch,
            sequence: params.sequence,
            unlock: Unlock {
                drand_chain: CHAIN_HASH_HEX.into(),
                round: params.unlock_round,
                time: format_time(drand::round_time(params.unlock_round)),
            },
            sealed: sealed?,
            owner_key: None,
            owner_sig: None,
        };
        kit.validate()?;
        Ok(kit)
    }

    /// Parse a kit from JSON. Unknown versions are rejected; unknown fields
    /// are ignored. The result is validated.
    pub fn from_json(text: &str) -> Result<Self, KitError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| KitError::Invalid(e.to_string()))?;
        if value.get("format").and_then(|f| f.as_str()) != Some(FORMAT) {
            return Err(KitError::Invalid("missing format \"zandglass-kit\"".into()));
        }
        match value.get("version").and_then(|v| v.as_u64()) {
            Some(v) if v == u64::from(VERSION) => {}
            Some(v) => return Err(KitError::VersionUnsupported(v)),
            None => return Err(KitError::Invalid("missing version".into())),
        }
        let kit: Self =
            serde_json::from_value(value).map_err(|e| KitError::Invalid(e.to_string()))?;
        kit.validate()?;
        Ok(kit)
    }

    /// Pretty JSON for writing to a `*.zandglass` file.
    pub fn to_json(&self) -> Result<String, KitError> {
        let mut s =
            serde_json::to_string_pretty(self).map_err(|e| KitError::Invalid(e.to_string()))?;
        s.push('\n');
        Ok(s)
    }

    /// Check format, version, chain and round consistency (offline).
    pub fn validate(&self) -> Result<(), KitError> {
        if self.format != FORMAT {
            return Err(KitError::Invalid("wrong format".into()));
        }
        if self.version != VERSION {
            return Err(KitError::VersionUnsupported(u64::from(self.version)));
        }
        if !is_hex_id(&self.plan) {
            return Err(KitError::Invalid("malformed plan id".into()));
        }
        if self.epoch == 0 {
            return Err(KitError::Invalid("epoch must be at least 1".into()));
        }
        if let Some(s) = self.sequence {
            if s.index == 0 || s.index > s.count {
                return Err(KitError::Invalid("bad installment sequence".into()));
            }
        }
        if self.unlock.drand_chain != CHAIN_HASH_HEX {
            return Err(KitError::Invalid("locked to an unknown drand chain".into()));
        }
        if self.unlock.time != format_time(drand::round_time(self.unlock.round)) {
            return Err(KitError::Invalid(
                "unlock time does not match the unlock round".into(),
            ));
        }
        let header_round = seal::kit_round(&self.sealed).map_err(|e| match e {
            SealError::WrongChain => KitError::Invalid("sealed part uses another chain".into()),
            other => KitError::Invalid(format!("sealed part is damaged: {other}")),
        })?;
        if header_round != self.unlock.round {
            return Err(KitError::Invalid(
                "sealed part is locked to a different round than unlock.round".into(),
            ));
        }
        self.verify_owner_sig()?;
        Ok(())
    }

    /// Signs the kit with the plan's owner key (a 32-byte Ed25519 secret),
    /// setting `owner_key` and `owner_sig`. Call it on a freshly built kit.
    pub fn sign_owner(&mut self, secret: &[u8; 32]) {
        crate::owner::sign(self, secret);
    }

    /// Checks the owner signature: `Ok(None)` for an unsigned kit, the owner
    /// public key when the signature is valid, an error when owner fields are
    /// present but wrong. Offline. `validate` calls it, so every parsed or
    /// opened kit has passed it.
    pub fn verify_owner_sig(&self) -> Result<Option<[u8; 32]>, KitError> {
        crate::owner::verify(self).map_err(|p| match p {
            crate::owner::SigProblem::Malformed => {
                KitError::Invalid("malformed owner key or signature".into())
            }
            crate::owner::SigProblem::Invalid => KitError::Invalid(
                "the owner signature is invalid: this kit was changed after it was sealed, or was not sealed by the owner it names".into(),
            ),
        })
    }

    /// Unix time at which the kit unlocks.
    pub fn unlock_time(&self) -> u64 {
        drand::round_time(self.unlock.round)
    }

    /// Open with a beacon that has already been obtained.
    pub fn open(&self, identity: &HeirIdentity, beacon: &Beacon) -> Result<Payload, KitError> {
        self.validate()?;
        let bytes = seal::open(&self.sealed, identity, beacon)?;
        self.payload_from(bytes)
    }

    /// Open by fetching the beacon from the drand relays. Refuses without any
    /// network call if `now` (unix seconds) is before the unlock round.
    pub fn open_online(&self, identity: &HeirIdentity, now: u64) -> Result<Payload, KitError> {
        self.validate()?;
        let bytes = seal::open_online(&self.sealed, identity, now)?;
        self.payload_from(bytes)
    }

    fn payload_from(&self, mut bytes: Vec<u8>) -> Result<Payload, KitError> {
        let parsed = serde_json::from_slice::<Payload>(&bytes);
        bytes.zeroize();
        let payload = parsed.map_err(|e| KitError::DecryptFailed(format!("payload: {e}")))?;
        if payload.network != self.network
            || payload.plan != self.plan
            || payload.epoch != self.epoch
        {
            return Err(KitError::DecryptFailed(
                "the sealed payload does not match the kit's public part".into(),
            ));
        }
        Ok(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_formatting() {
        assert_eq!(format_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_time(1_692_803_367), "2023-08-23T15:09:27Z");
        assert_eq!(format_time(951_782_400), "2000-02-29T00:00:00Z");
    }
}

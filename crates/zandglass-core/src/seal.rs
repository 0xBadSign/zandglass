//! Sealing and opening kits.
//!
//! `sealed = tlock_R( age_to(heir_codes, payload) )`, the outer layer being
//! drand quicknet timelock encryption written as ASCII armor. Both layers are
//! standard, so a kit opens with the reference tools: `tle --decrypt` followed
//! by `age --decrypt -i <identity>`.

use std::io::{Read, Write};
use std::iter;

use age::{DecryptError, Decryptor, Encryptor, Identity, Recipient};

use crate::drand::{Beacon, CHAIN_HASH_HEX, PUBLIC_KEY_HEX};
use crate::heir::{HeirCode, HeirIdentity};

/// Kits are sealed to at least one and at most this many heirs.
pub const MAX_HEIRS: usize = 3;

/// Errors from sealing and opening kits.
#[derive(Debug, thiserror::Error)]
pub enum SealError {
    /// No heir code was given.
    #[error("at least one heir code is required")]
    NoHeirs,
    /// More than [`MAX_HEIRS`] heir codes were given.
    #[error("at most {MAX_HEIRS} heir codes are supported")]
    TooManyHeirs,
    /// The beacon for the kit's round is not available yet.
    #[error("the kit unlocks at round {needed}; the newest beacon is round {have}")]
    NotYetUnlocked {
        /// Round the kit is locked to.
        needed: u64,
        /// Round of the beacon that was supplied.
        have: u64,
    },
    /// A beacon for a later round was supplied; the exact round is required.
    #[error("the kit needs the beacon of round {needed}, not round {have}")]
    WrongBeaconRound {
        /// Round the kit is locked to.
        needed: u64,
        /// Round of the beacon that was supplied.
        have: u64,
    },
    /// The kit is timelocked to a different drand chain.
    #[error("the kit is locked to a different drand chain")]
    WrongChain,
    /// The kit was unlocked but the given heir key is not one of its heirs.
    #[error("this heir key cannot open the kit")]
    WrongHeirKey,
    /// The kit is corrupted or was modified.
    #[error("the kit is damaged: {0}")]
    Damaged(String),
    /// Beacon retrieval failed (online opening only).
    #[error(transparent)]
    Drand(#[from] crate::drand::DrandError),
    /// Encryption failed.
    #[error("sealing failed: {0}")]
    Seal(String),
}

/// Seal `payload` to `heirs` (1 to 3 codes), unlocking at drand quicknet
/// `round`. Returns the armored kit text.
pub fn seal(payload: &[u8], heirs: &[HeirCode], round: u64) -> Result<String, SealError> {
    if heirs.is_empty() {
        return Err(SealError::NoHeirs);
    }
    if heirs.len() > MAX_HEIRS {
        return Err(SealError::TooManyHeirs);
    }

    // Inner layer: age to the heirs.
    let recipients = heirs.iter().map(|h| &h.0 as &dyn Recipient);
    let encryptor =
        Encryptor::with_recipients(recipients).map_err(|e| SealError::Seal(e.to_string()))?;
    let mut inner = Vec::new();
    let mut w = encryptor
        .wrap_output(&mut inner)
        .map_err(|e| SealError::Seal(e.to_string()))?;
    w.write_all(payload)
        .and_then(|()| w.finish().map(|_| ()))
        .map_err(|e| SealError::Seal(e.to_string()))?;

    // Outer layer: tlock to the round, ASCII armored.
    let chain = decode_hex(CHAIN_HASH_HEX);
    let pk = decode_hex(PUBLIC_KEY_HEX);
    let mut armored = tlock_age::armor::ArmoredWriter::wrap_output(Vec::new()).map_err(seal_err)?;
    tlock_age::encrypt(&mut armored, inner.as_slice(), &chain, &pk, round).map_err(seal_err)?;
    let bytes = armored
        .finish()
        .map_err(|e| SealError::Seal(e.to_string()))?;
    String::from_utf8(bytes).map_err(|e| SealError::Seal(e.to_string()))
}

/// The drand round a kit is locked to, read from its public header.
pub fn kit_round(armored: &str) -> Result<u64, SealError> {
    let header = tlock_age::decrypt_header(armored.as_bytes())
        .map_err(|e| SealError::Damaged(e.to_string()))?;
    if header.hash() != decode_hex(CHAIN_HASH_HEX) {
        return Err(SealError::WrongChain);
    }
    Ok(header.round())
}

/// Open a kit with the heir's identity and the beacon of the kit's round.
pub fn open(armored: &str, identity: &HeirIdentity, beacon: &Beacon) -> Result<Vec<u8>, SealError> {
    let needed = kit_round(armored)?;
    if beacon.round() < needed {
        return Err(SealError::NotYetUnlocked {
            needed,
            have: beacon.round(),
        });
    }
    if beacon.round() > needed {
        return Err(SealError::WrongBeaconRound {
            needed,
            have: beacon.round(),
        });
    }

    // Outer layer.
    let mut inner = Vec::new();
    tlock_age::decrypt(
        &mut inner,
        armored.as_bytes(),
        &decode_hex(CHAIN_HASH_HEX),
        beacon.signature(),
    )
    .map_err(|e| SealError::Damaged(e.to_string()))?;

    // Inner layer.
    let decryptor = Decryptor::new(inner.as_slice()).map_err(map_inner)?;
    let mut reader = decryptor
        .decrypt(iter::once(&identity.0 as &dyn Identity))
        .map_err(map_inner)?;
    let mut payload = Vec::new();
    reader
        .read_to_end(&mut payload)
        .map_err(|e| SealError::Damaged(e.to_string()))?;
    Ok(payload)
}

/// Like [`open`], but fetches the beacon of the kit's round from the drand
/// relays. Refuses early, without any network call, if the round has not
/// been published yet according to `now` (unix seconds).
pub fn open_online(armored: &str, identity: &HeirIdentity, now: u64) -> Result<Vec<u8>, SealError> {
    let needed = kit_round(armored)?;
    let current = crate::drand::round_at(now);
    if current < needed {
        return Err(SealError::NotYetUnlocked {
            needed,
            have: current,
        });
    }
    let beacon = Beacon::fetch(needed)?;
    open(armored, identity, &beacon)
}

fn map_inner(e: DecryptError) -> SealError {
    match e {
        DecryptError::NoMatchingKeys => SealError::WrongHeirKey,
        other => SealError::Damaged(other.to_string()),
    }
}

fn seal_err(e: impl std::fmt::Display) -> SealError {
    SealError::Seal(e.to_string())
}

fn decode_hex(s: &str) -> Vec<u8> {
    // The pinned constants are valid hex; a failure would be a build-time bug.
    hex::decode(s).unwrap_or_default()
}

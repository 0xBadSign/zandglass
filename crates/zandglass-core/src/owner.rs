//! Owner authentication of kits (optional, within kit version 1).
//!
//! Every plan has one random Ed25519 signing key. Each kit of the plan carries
//! the public key (`owner_key`) and a signature over the kit's public part
//! and the hash of its `sealed` field (`owner_sig`). An heir page that has seen
//! one kit of a plan can therefore tell when a later kit was not sealed by
//! the same owner (a decoy planted by a mailbox operator). The key is random
//! and per plan: it says nothing about the owner's wallet or identity. See
//! `docs/KIT_FORMAT.md` section 8.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::kit::{Kind, Kit};

/// First line of the signed message; it names the scheme and its version.
pub const SIG_DOMAIN: &str = "zandglass-kit-sig-v1";

/// The public key for a 32-byte signing secret.
pub fn public_key(secret: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(secret).verifying_key().to_bytes()
}

/// A short, human-comparable fingerprint of an owner public key: the first
/// eight bytes of its SHA-256 as `xxxx-xxxx-xxxx-xxxx` (lowercase hex).
pub fn fingerprint(key: &[u8; 32]) -> String {
    let h = Sha256::digest(key);
    let hex = hex::encode(&h[..8]);
    format!(
        "{}-{}-{}-{}",
        &hex[0..4],
        &hex[4..8],
        &hex[8..12],
        &hex[12..16]
    )
}

/// The exact bytes that are signed, for a kit whose `owner_key` is `owner_key`
/// (lowercase hex). Fixed field order, one `name=value` line each, every
/// line ends in `\n`. `owner_sig` is not part of it; `sealed` is covered by
/// its SHA-256 (hex).
pub fn signing_message(kit: &Kit, owner_key: &str) -> Vec<u8> {
    let kind = match kit.kind {
        Kind::Legacy => "legacy",
        Kind::Installment => "installment",
    };
    let sequence = match kit.sequence {
        Some(s) => format!("{}/{}", s.index, s.count),
        None => "none".to_string(),
    };
    let sealed_hash = hex::encode(Sha256::digest(kit.sealed.as_bytes()));
    format!(
        "{SIG_DOMAIN}\n\
         format={}\n\
         version={}\n\
         kind={kind}\n\
         network={}\n\
         created={}\n\
         plan={}\n\
         epoch={}\n\
         sequence={sequence}\n\
         unlock.drand_chain={}\n\
         unlock.round={}\n\
         unlock.time={}\n\
         owner_key={owner_key}\n\
         sealed_sha256={sealed_hash}\n",
        kit.format,
        kit.version,
        kit.network,
        kit.created,
        kit.plan,
        kit.epoch,
        kit.unlock.drand_chain,
        kit.unlock.round,
        kit.unlock.time,
    )
    .into_bytes()
}

/// Signs `kit` in place: sets `owner_key` and `owner_sig`.
pub fn sign(kit: &mut Kit, secret: &[u8; 32]) {
    let sk = SigningKey::from_bytes(secret);
    let key_hex = hex::encode(sk.verifying_key().to_bytes());
    let sig = sk.sign(&signing_message(kit, &key_hex));
    kit.owner_key = Some(key_hex);
    kit.owner_sig = Some(hex::encode(sig.to_bytes()));
}

/// Why an owner signature is unusable.
#[derive(Debug, PartialEq, Eq)]
pub enum SigProblem {
    /// A field is not well-formed hex of the right length, or only one of the
    /// two fields is present.
    Malformed,
    /// The signature does not verify.
    Invalid,
}

fn decode_fixed<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 || s.bytes().any(|b| !matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    hex::decode(s).ok()?.try_into().ok()
}

/// Checks the owner fields of `kit`: `Ok(None)` when it has none, the public
/// key when the signature verifies.
pub fn verify(kit: &Kit) -> Result<Option<[u8; 32]>, SigProblem> {
    let (key_hex, sig_hex) = match (&kit.owner_key, &kit.owner_sig) {
        (None, None) => return Ok(None),
        (Some(k), Some(s)) => (k, s),
        _ => return Err(SigProblem::Malformed),
    };
    let key: [u8; 32] = decode_fixed(key_hex).ok_or(SigProblem::Malformed)?;
    let sig: [u8; 64] = decode_fixed(sig_hex).ok_or(SigProblem::Malformed)?;
    let vk = VerifyingKey::from_bytes(&key).map_err(|_| SigProblem::Malformed)?;
    // Free-text fields must not be able to forge the line structure.
    if [
        &kit.created,
        &kit.plan,
        &kit.unlock.time,
        &kit.unlock.drand_chain,
    ]
    .iter()
    .any(|f| f.contains('\n'))
    {
        return Err(SigProblem::Invalid);
    }
    vk.verify_strict(&signing_message(kit, key_hex), &Signature::from_bytes(&sig))
        .map_err(|_| SigProblem::Invalid)?;
    Ok(Some(key))
}

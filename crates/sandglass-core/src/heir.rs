//! Heir keys: a 24-word BIP 39 phrase becomes an age X25519 identity.
//!
//! `phrase -> BIP 39 seed (64 bytes, empty passphrase) -> HKDF-SHA256
//! (salt empty, info "sandglass-heir-v1") -> 32-byte X25519 secret`, encoded
//! as a standard `AGE-SECRET-KEY-1...` identity. The heir code is the matching
//! `age1...` recipient string.

use std::fmt;
use std::str::FromStr;

use age::secrecy::SecretString;
use age::x25519;
use bech32::{Bech32, Hrp};
use bip0039::{Count, Mnemonic};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

/// HKDF domain separation string.
pub const HKDF_INFO: &[u8] = b"sandglass-heir-v1";

/// Errors from heir key handling.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HeirError {
    /// The phrase is not a valid 24-word BIP 39 English mnemonic.
    #[error("invalid recovery phrase: {0}")]
    InvalidPhrase(String),
    /// The heir code is not a valid `age1...` X25519 recipient.
    #[error("invalid heir code: {0}")]
    InvalidCode(String),
    /// Internal key derivation failure.
    #[error("key derivation failed")]
    Derivation,
}

/// A heir's public code (`age1...`). Safe to share.
#[derive(Clone, PartialEq, Eq)]
pub struct HeirCode(pub(crate) x25519::Recipient);

impl HeirCode {
    /// Parse an `age1...` recipient string.
    pub fn parse(s: &str) -> Result<Self, HeirError> {
        x25519::Recipient::from_str(s.trim())
            .map(Self)
            .map_err(|e| HeirError::InvalidCode(e.to_string()))
    }
}

impl fmt::Display for HeirCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for HeirCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HeirCode({})", self.0)
    }
}

impl FromStr for HeirCode {
    type Err = HeirError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// A heir's secret identity. Never logged; the secret string is only
/// available through [`HeirIdentity::to_secret_string`].
pub struct HeirIdentity(pub(crate) x25519::Identity);

impl HeirIdentity {
    /// The heir code (recipient) matching this identity.
    pub fn code(&self) -> HeirCode {
        HeirCode(self.0.to_public())
    }

    /// The `AGE-SECRET-KEY-1...` string, as used by the `age` CLI
    /// (`age --decrypt -i <file>`).
    pub fn to_secret_string(&self) -> SecretString {
        self.0.to_string()
    }

    /// Parse an `AGE-SECRET-KEY-1...` string.
    pub fn from_secret_string(s: &str) -> Result<Self, HeirError> {
        x25519::Identity::from_str(s.trim())
            .map(Self)
            .map_err(|e| HeirError::InvalidPhrase(e.to_string()))
    }
}

impl fmt::Debug for HeirIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HeirIdentity(<secret>)")
    }
}

/// Generate a fresh random 24-word phrase.
pub fn generate_phrase() -> Zeroizing<String> {
    Zeroizing::new(Mnemonic::<bip0039::English>::generate(Count::Words24).into_phrase())
}

/// Derive the heir identity from a 24-word phrase.
pub fn identity_from_phrase(phrase: &str) -> Result<HeirIdentity, HeirError> {
    let words = phrase.split_whitespace().count();
    if words != 24 {
        return Err(HeirError::InvalidPhrase(format!(
            "expected 24 words, got {words}"
        )));
    }
    let mnemonic = Mnemonic::<bip0039::English>::from_phrase(phrase.trim())
        .map_err(|e| HeirError::InvalidPhrase(e.to_string()))?;
    let mut seed = mnemonic.to_seed("");
    let mut secret = [0u8; 32];
    let expanded = Hkdf::<Sha256>::new(None, &seed).expand(HKDF_INFO, &mut secret);
    seed.zeroize();
    expanded.map_err(|_| HeirError::Derivation)?;

    let encoded = bech32::encode::<Bech32>(
        Hrp::parse("age-secret-key-").map_err(|_| HeirError::Derivation)?,
        &secret,
    );
    secret.zeroize();
    let encoded = Zeroizing::new(encoded.map_err(|_| HeirError::Derivation)?.to_uppercase());
    let identity =
        x25519::Identity::from_str(encoded.as_str()).map_err(|_| HeirError::Derivation)?;
    Ok(HeirIdentity(identity))
}

#[cfg(test)]
mod tests {
    use super::*;

    use age::secrecy::ExposeSecret;

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon abandon abandon abandon \
        abandon abandon abandon abandon abandon abandon abandon art";

    #[test]
    fn derivation_is_deterministic() {
        let a = identity_from_phrase(PHRASE).unwrap();
        let b = identity_from_phrase(PHRASE).unwrap();
        assert_eq!(a.code(), b.code());
        assert!(a.code().to_string().starts_with("age1"));
        assert!(a
            .to_secret_string()
            .expose_secret()
            .starts_with("AGE-SECRET-KEY-1"));
    }

    #[test]
    fn different_phrases_give_different_codes() {
        let other = generate_phrase();
        assert_ne!(
            identity_from_phrase(PHRASE).unwrap().code(),
            identity_from_phrase(&other).unwrap().code()
        );
    }

    #[test]
    fn generated_phrase_has_24_words() {
        assert_eq!(generate_phrase().split_whitespace().count(), 24);
    }

    #[test]
    fn rejects_bad_phrases() {
        assert!(identity_from_phrase("abandon abandon").is_err());
        let bad_checksum = PHRASE.replace("art", "zoo");
        assert!(identity_from_phrase(&bad_checksum).is_err());
    }

    #[test]
    fn code_round_trips() {
        let code = identity_from_phrase(PHRASE).unwrap().code();
        assert_eq!(HeirCode::parse(&code.to_string()).unwrap(), code);
        assert!(HeirCode::parse("age1notacode").is_err());
    }

    #[test]
    fn secret_string_round_trips() {
        let id = identity_from_phrase(PHRASE).unwrap();
        let s = id.to_secret_string();
        let back = HeirIdentity::from_secret_string(s.expose_secret()).unwrap();
        assert_eq!(back.code(), id.code());
    }
}

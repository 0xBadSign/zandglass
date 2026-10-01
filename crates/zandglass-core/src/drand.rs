//! drand quicknet: pinned parameters, round math and beacon retrieval.
//!
//! Beacons fetched over HTTPS are verified against the pinned chain hash and
//! public key before they are returned. The pinned values were checked
//! against the network's `/info` endpoint.

use drand_core::beacon::ApiBeacon;
use drand_core::chain::{ChainInfo, ChainVerification};
use drand_core::{ChainOptions, HttpClient};

/// Quicknet chain hash (hex).
pub const CHAIN_HASH_HEX: &str = "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971";
/// Quicknet group public key (hex, compressed G2).
pub const PUBLIC_KEY_HEX: &str = "83cf0f2896adee7eb8b5f01fcad3912212c437e0073e911fb90022d3e760183c8c4b450b6a0a6c3ac6a5776a2d1064510d1fec758c921cc22b0e17e63aaf4bcb5ed66304de9cf809bd274ca73bab4af5a6e9c76a4bc09e76eae8991ef5ece45a";
/// Quicknet scheme identifier.
pub const SCHEME_ID: &str = "bls-unchained-g1-rfc9380";
/// Unix time of round 1 (seconds).
pub const GENESIS_TIME: u64 = 1_692_803_367;
/// Seconds between rounds.
pub const PERIOD: u64 = 3;
/// Relays tried in order; the first verified answer wins.
pub const RELAYS: [&str; 2] = ["https://api.drand.sh", "https://drand.cloudflare.com"];

/// Pinned chain info as served at `/info`; used for offline verification.
const CHAIN_INFO_JSON: &str = r#"{
  "public_key": "83cf0f2896adee7eb8b5f01fcad3912212c437e0073e911fb90022d3e760183c8c4b450b6a0a6c3ac6a5776a2d1064510d1fec758c921cc22b0e17e63aaf4bcb5ed66304de9cf809bd274ca73bab4af5a6e9c76a4bc09e76eae8991ef5ece45a",
  "period": 3,
  "genesis_time": 1692803367,
  "hash": "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971",
  "groupHash": "f477d5c89f21a17c863a7f937c6a6d15859414d2be09cd448d4279af331c5d3e",
  "schemeID": "bls-unchained-g1-rfc9380",
  "metadata": { "beaconID": "quicknet" }
}"#;

/// Errors from beacon retrieval and verification.
#[derive(Debug, thiserror::Error)]
pub enum DrandError {
    /// Every relay failed; the messages are listed in order.
    #[error("no relay returned a valid beacon: {0}")]
    Network(String),
    /// The beacon JSON was malformed or its signature did not verify.
    #[error("invalid beacon: {0}")]
    Invalid(String),
}

/// The round published at unix time `t` (0 before genesis).
pub fn round_at(t: u64) -> u64 {
    match t.checked_sub(GENESIS_TIME) {
        Some(d) => d / PERIOD + 1,
        None => 0,
    }
}

/// The first round published at or after unix time `t`.
pub fn unlock_round(t: u64) -> u64 {
    t.saturating_sub(GENESIS_TIME).div_ceil(PERIOD) + 1
}

/// Unix time at which `round` is published.
pub fn round_time(round: u64) -> u64 {
    GENESIS_TIME + round.saturating_sub(1) * PERIOD
}

/// A verified drand beacon: the signature for `round` is the decryption key
/// for anything timelocked to that round.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Beacon {
    round: u64,
    signature: Vec<u8>,
}

impl Beacon {
    /// Parse a beacon from the relay JSON (`{"round","randomness","signature"}`)
    /// and verify it against the pinned quicknet public key.
    pub fn from_json(json: &str) -> Result<Self, DrandError> {
        let api: ApiBeacon =
            serde_json::from_str(json).map_err(|e| DrandError::Invalid(e.to_string()))?;
        let info: ChainInfo = serde_json::from_str(CHAIN_INFO_JSON)
            .map_err(|e| DrandError::Invalid(e.to_string()))?;
        match api.verify(info) {
            Ok(true) => Ok(Self {
                round: api.round(),
                signature: api.signature(),
            }),
            Ok(false) => Err(DrandError::Invalid("signature does not verify".into())),
            Err(e) => Err(DrandError::Invalid(e.to_string())),
        }
    }

    /// Build a beacon from raw parts without verification. The tlock layer
    /// still fails to decrypt if the signature is wrong.
    pub fn from_parts(round: u64, signature: Vec<u8>) -> Self {
        Self { round, signature }
    }

    /// The beacon's round.
    pub fn round(&self) -> u64 {
        self.round
    }

    /// The BLS signature bytes (48 bytes on quicknet).
    pub fn signature(&self) -> &[u8] {
        &self.signature
    }

    /// Fetch and verify the beacon for `round`, trying each relay in turn.
    /// A round that is not yet published yields an error from every relay.
    pub fn fetch(round: u64) -> Result<Self, DrandError> {
        fetch_with(|c| c.get(round))
    }

    /// Fetch and verify the latest beacon, trying each relay in turn.
    pub fn fetch_latest() -> Result<Self, DrandError> {
        fetch_with(|c| c.latest())
    }
}

fn client(relay: &str) -> Result<HttpClient, String> {
    let hash = hex::decode(CHAIN_HASH_HEX).map_err(|e| e.to_string())?;
    let pk = hex::decode(PUBLIC_KEY_HEX).map_err(|e| e.to_string())?;
    let verification = ChainVerification::new(Some(hash), Some(pk));
    let options = ChainOptions::new(true, true, Some(verification));
    HttpClient::new(&format!("{relay}/{CHAIN_HASH_HEX}"), Some(options)).map_err(|e| e.to_string())
}

fn fetch_with(
    get: impl Fn(&HttpClient) -> Result<drand_core::beacon::RandomnessBeacon, drand_core::DrandError>,
) -> Result<Beacon, DrandError> {
    let mut errors = Vec::new();
    for relay in RELAYS {
        let attempt = client(relay).and_then(|c| get(&c).map_err(|e| e.to_string()));
        match attempt {
            Ok(b) => {
                return Ok(Beacon {
                    round: b.round(),
                    signature: b.signature(),
                })
            }
            Err(e) => errors.push(format!("{relay}: {e}")),
        }
    }
    Err(DrandError::Network(errors.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_math_matches_spec() {
        assert_eq!(round_at(GENESIS_TIME), 1);
        assert_eq!(round_at(GENESIS_TIME - 1), 0);
        assert_eq!(round_at(GENESIS_TIME + 2), 1);
        assert_eq!(round_at(GENESIS_TIME + 3), 2);
        assert_eq!(round_time(1), GENESIS_TIME);
        assert_eq!(round_time(2), GENESIS_TIME + 3);
        // Known live value: round 32665821 was current at 1790800828.
        assert_eq!(round_at(1_790_800_828), 32_665_821);
    }

    #[test]
    fn unlock_round_is_never_early() {
        for t in [GENESIS_TIME, GENESIS_TIME + 1, 1_790_000_000, 1_790_000_002] {
            let r = unlock_round(t);
            assert!(round_time(r) >= t);
            assert!(round_time(r) - t < PERIOD);
        }
    }

    #[test]
    fn rejects_forged_beacon() {
        let forged = r#"{"round":32665821,"randomness":"e0a6d601c59f53baa63e56e9734bacb5b8cc8eca250f216443c781d75daaa691","signature":"b6bfc8542f58aacb3dae0fbfb2ed24c47790aaf9cf657a7a79f697a5f5e7b55e8f83f2db7c83ece59271c24dadf5cea6"}"#;
        assert!(Beacon::from_json(forged).is_err());
    }

    #[test]
    fn verifies_real_beacon_offline() {
        let json = r#"{"round":32665821,"randomness":"e0a6d601c59f53baa63e56e9734bacb5b8cc8eca250f216443c781d75daaa690","signature":"b6bfc8542f58aacb3dae0fbfb2ed24c47790aaf9cf657a7a79f697a5f5e7b55e8f83f2db7c83ece59271c24dadf5cea6"}"#;
        let b = Beacon::from_json(json).expect("real quicknet beacon verifies");
        assert_eq!(b.round(), 32_665_821);
        assert_eq!(b.signature().len(), 48);
    }

    #[test]
    #[ignore = "network"]
    fn fetches_latest_from_relays() {
        let b = Beacon::fetch_latest().expect("latest beacon");
        assert!(b.round() > 32_000_000);
    }
}

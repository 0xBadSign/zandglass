//! The hosted playground (PLAN 5.8, contract in `docs/PLAYGROUND_API.md`).
//!
//! The server plays the owner with a small float wallet; the visitor plays
//! the heir. Enabled only when `ZANDGLASS_PLAYGROUND_NETWORK` is set.
//!
//! Privacy: session rows hold the public heir code, the claim link and
//! timestamps, nothing else about the visitor. No IPs, no request logs.

pub mod chain;
pub mod real;
pub mod store;
mod worker;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use subtle::ConstantTimeEq;
use zandglass_core::drand;
use zandglass_core::heir::HeirCode;
use zandglass_core::kit::{format_time, BuildParams, Kind, Network as KitNetwork, Payload};
use zandglass_core::Kit;
use zeroize::Zeroizing;

use crate::{random_hex, sha256, unix_now, ApiError, AppState, Db};
use chain::{Chain, SWEEP_FEE_ZAT};
use store::{day_of, Session, SessionDb, State as S};

/// The session lives this long after the unlock before it returns by itself.
pub const AUTO_RETURN_SECS: u64 = 30 * 60;
/// Settled sessions are purged after this long.
pub const PURGE_AFTER_SECS: u64 = 7 * 86_400;
/// An unclaimed kept vault may be returned after this long.
pub const KEEP_RETURN_AFTER_SECS: u64 = 24 * 3600;

/// Playground settings (everything but the wallet).
#[derive(Clone, Debug)]
pub struct PlaygroundConfig {
    /// `main` or `test`.
    pub network: &'static str,
    /// Nominal value per session, in zatoshi.
    pub amount_zat: u64,
    pub max_active: u32,
    pub max_per_day: u32,
    pub max_keeps_per_day: u32,
    /// Flips allowed per session; each costs one sweep fee, which the
    /// funding transaction pre-pays.
    pub max_flips: u32,
    /// SHA-256 of the judge code.
    pub judge_hash: Vec<u8>,
    /// Public origin of the site, for claim links (no trailing slash).
    pub public_origin: String,
    /// Block explorer URL with `{txid}`.
    pub explorer_tx_url: Option<String>,
    /// Allowed timer lengths in minutes.
    pub allowed_minutes: Vec<u32>,
}

impl PlaygroundConfig {
    /// Zatoshi sent to a new vault: the nominal amount plus a sweep fee for
    /// every flip and one more for the final sweep.
    pub fn funding_zat(&self) -> u64 {
        self.amount_zat + SWEEP_FEE_ZAT * (u64::from(self.max_flips) + 1)
    }
}

/// Everything read from the environment.
pub struct PlaygroundEnv {
    pub config: PlaygroundConfig,
    pub real: real::RealConfig,
}

fn env_num<T: std::str::FromStr>(name: &str, default: T) -> Result<T> {
    match std::env::var(name) {
        Ok(v) => v
            .trim()
            .parse()
            .map_err(|_| anyhow!("{name} must be a number")),
        Err(_) => Ok(default),
    }
}

/// Confirmations the playground waits for by default (the wallet library's
/// own default would be 3 trusted / 10 untrusted, far too slow for a demo).
pub const DEFAULT_MIN_CONFIRMATIONS: u32 = 1;

/// `ZANDGLASS_PLAYGROUND_MIN_CONFIRMATIONS` must be 1 or more.
pub fn check_min_confirmations(n: u32) -> Result<u32> {
    if n == 0 {
        return Err(anyhow!(
            "ZANDGLASS_PLAYGROUND_MIN_CONFIRMATIONS must be at least 1"
        ));
    }
    Ok(n)
}

fn env_req(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| anyhow!("{name} is required when the playground is enabled"))
}

impl PlaygroundEnv {
    /// `Ok(None)` when `ZANDGLASS_PLAYGROUND_NETWORK` is unset.
    pub fn from_env() -> Result<Option<Self>> {
        let Ok(net) = std::env::var("ZANDGLASS_PLAYGROUND_NETWORK") else {
            return Ok(None);
        };
        let (network, name) = match net.trim() {
            "main" => (zandglass_wallet::Network::MainNetwork, "main"),
            "test" => (zandglass_wallet::Network::TestNetwork, "test"),
            _ => return Err(anyhow!("ZANDGLASS_PLAYGROUND_NETWORK must be main or test")),
        };
        let judge_hash = hex::decode(env_req("ZANDGLASS_PLAYGROUND_JUDGE_CODE_SHA256")?.trim())
            .ok()
            .filter(|h| h.len() == 32)
            .ok_or_else(|| {
                anyhow!("ZANDGLASS_PLAYGROUND_JUDGE_CODE_SHA256 must be 64 hex digits")
            })?;
        let secret = env_req("ZANDGLASS_PLAYGROUND_SECRET")?;
        if secret.len() < 12 {
            return Err(anyhow!(
                "ZANDGLASS_PLAYGROUND_SECRET must be at least 12 characters"
            ));
        }
        let public_origin = env_req("ZANDGLASS_PUBLIC_ORIGIN")?
            .trim()
            .trim_end_matches('/')
            .to_string();
        if !(public_origin.starts_with("http://") || public_origin.starts_with("https://")) {
            return Err(anyhow!(
                "ZANDGLASS_PUBLIC_ORIGIN must start with http(s)://"
            ));
        }
        let explorer_tx_url = std::env::var("ZANDGLASS_EXPLORER_TX_URL")
            .ok()
            .filter(|u| !u.trim().is_empty());
        if explorer_tx_url
            .as_ref()
            .is_some_and(|u| !u.contains("{txid}"))
        {
            return Err(anyhow!("ZANDGLASS_EXPLORER_TX_URL must contain {{txid}}"));
        }
        let data_dir = PathBuf::from(env_req("ZANDGLASS_PLAYGROUND_DATA")?);
        let config = PlaygroundConfig {
            network: name,
            amount_zat: env_num("ZANDGLASS_PLAYGROUND_AMOUNT_ZAT", 20_000)?,
            max_active: env_num("ZANDGLASS_PLAYGROUND_MAX_ACTIVE", 10)?,
            max_per_day: env_num("ZANDGLASS_PLAYGROUND_MAX_PER_DAY", 60)?,
            max_keeps_per_day: env_num("ZANDGLASS_PLAYGROUND_MAX_KEEPS_PER_DAY", 5)?,
            max_flips: env_num("ZANDGLASS_PLAYGROUND_MAX_FLIPS", 3)?,
            judge_hash,
            public_origin,
            explorer_tx_url,
            allowed_minutes: vec![3, 5, 10],
        };
        let real = real::RealConfig {
            network,
            // A comma-separated list; servers are tried in order.
            lwd_urls: match std::env::var("ZANDGLASS_LWD_URL") {
                Ok(list) if !zandglass_wallet::parse_urls(&list).is_empty() => {
                    zandglass_wallet::parse_urls(&list)
                }
                _ => zandglass_wallet::default_urls(network)
                    .iter()
                    .map(|u| (*u).to_string())
                    .collect(),
            },
            data_dir,
            secret: Zeroizing::new(secret),
            float_seed_file: PathBuf::from(env_req("ZANDGLASS_PLAYGROUND_FLOAT_SEED_FILE")?),
            float_birthday: env_num("ZANDGLASS_PLAYGROUND_FLOAT_BIRTHDAY", 0)?,
            min_confirmations: check_min_confirmations(env_num(
                "ZANDGLASS_PLAYGROUND_MIN_CONFIRMATIONS",
                DEFAULT_MIN_CONFIRMATIONS,
            )?)?,
        };
        if real.float_birthday == 0 {
            return Err(anyhow!(
                "ZANDGLASS_PLAYGROUND_FLOAT_BIRTHDAY must be the float wallet's birthday height"
            ));
        }
        Ok(Some(Self { config, real }))
    }
}

/// The playground: configuration, chain, sessions and mailboxes.
pub struct Playground {
    cfg: PlaygroundConfig,
    chain: Arc<dyn Chain>,
    sessions: SessionDb,
    mailboxes: Arc<Db>,
    /// Serialises the cap checks and the creation of sessions.
    gate: tokio::sync::Mutex<()>,
    /// When each session's balance was last polled (memory only).
    polled: std::sync::Mutex<std::collections::HashMap<String, u64>>,
}

/// What a created session returns.
pub struct Created {
    pub id: String,
    pub token: String,
    pub claim_link: String,
    pub minutes: u32,
}

#[derive(Deserialize)]
struct CreateBody {
    judge_code: String,
    heir_code: String,
    minutes: u32,
}

#[derive(Deserialize)]
struct SettleBody {
    action: String,
    to: Option<String>,
}

fn bad_request(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_request", message)
}

fn state_error(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "invalid_state", message)
}

fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    if body.len() > 4096 {
        return Err(bad_request("The request body is too large."));
    }
    serde_json::from_slice(body).map_err(|_| bad_request("The request body is not valid JSON."))
}

impl Playground {
    /// Builds the playground. `sessions_path` is its own SQLite file.
    pub fn new(
        cfg: PlaygroundConfig,
        chain: Arc<dyn Chain>,
        sessions_path: &std::path::Path,
        mailboxes: Arc<Db>,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            cfg,
            chain,
            sessions: SessionDb::open(sessions_path).context("opening the session database")?,
            mailboxes,
            gate: tokio::sync::Mutex::new(()),
            polled: std::sync::Mutex::new(std::collections::HashMap::new()),
        }))
    }

    /// The settings.
    pub fn config(&self) -> &PlaygroundConfig {
        &self.cfg
    }

    /// The session database (tests).
    pub fn sessions(&self) -> &SessionDb {
        &self.sessions
    }

    fn internal<E>(_: E) -> ApiError {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "The playground could not complete the request. Try again later.",
        )
    }

    fn status_json(&self, now: u64) -> Value {
        let active = self.sessions.count_active().unwrap_or(0);
        let today = self
            .sessions
            .count_created_since(day_of(now) * 86_400)
            .unwrap_or(0);
        json!({
            "enabled": true,
            "network": self.cfg.network,
            "amount_zat": self.cfg.amount_zat,
            "active": active,
            "max_active": self.cfg.max_active,
            "today": today,
            "max_per_day": self.cfg.max_per_day,
            "float_ready": self.chain.float_ready(),
        })
    }

    fn explorer(&self, txid: &str) -> Value {
        match &self.cfg.explorer_tx_url {
            Some(t) => Value::String(t.replace("{txid}", txid)),
            None => Value::Null,
        }
    }

    fn session_json(&self, s: &Session) -> Value {
        let txs: Vec<Value> = self
            .sessions
            .txs(&s.id)
            .unwrap_or_default()
            .iter()
            .map(|t| json!({ "kind": t.kind, "txid": t.txid, "explorer": self.explorer(&t.txid) }))
            .collect();
        json!({
            "id": s.id,
            "state": s.state.as_str(),
            "epoch": s.epoch,
            "unlock_time": format_time(s.unlock_time),
            "unlock_round": s.unlock_round,
            "balance_zat": s.balance_zat,
            "confirmed": s.confirmed,
            "flips": s.flips,
            "max_flips": self.cfg.max_flips,
            "txs": txs,
            "claim_link": s.claim_link,
            "message": s.message,
            "error": s.error,
        })
    }

    /// Creates a session (judge code, caps, vault, kit, mailbox). The funding
    /// transaction is sent by the worker.
    pub async fn create_session(
        &self,
        judge_code: &str,
        heir_code: &str,
        minutes: u32,
        now: u64,
    ) -> Result<Created, ApiError> {
        let given = sha256(judge_code);
        if !bool::from(given.ct_eq(&self.cfg.judge_hash)) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "judge_code_invalid",
                "The judge code is not valid.",
            ));
        }
        let heir = HeirCode::parse(heir_code).map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "heir_code_invalid",
                "The heir code is not a valid age1... code.",
            )
        })?;
        if !self.cfg.allowed_minutes.contains(&minutes) {
            return Err(bad_request("The timer must be 3, 5 or 10 minutes."));
        }

        let _gate = self.gate.lock().await;
        if !self.chain.float_ready() {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "playground_busy",
                "The demo wallet is not ready yet. Try again in a minute.",
            ));
        }
        if self.sessions.count_active().map_err(Self::internal)? >= self.cfg.max_active {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "playground_busy",
                "The playground is busy. Try again in a few minutes.",
            ));
        }
        if self
            .sessions
            .count_created_since(day_of(now) * 86_400)
            .map_err(Self::internal)?
            >= self.cfg.max_per_day
        {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "daily_cap_reached",
                "The playground has reached its daily limit. Try again tomorrow.",
            ));
        }
        if self
            .chain
            .float_spendable()
            .is_some_and(|b| b < self.cfg.funding_zat())
        {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "playground_busy",
                "The demo wallet is out of funds for now.",
            ));
        }

        let id = random_hex(16);
        let token = random_hex(32);
        let built = self.build_kit(&id, 1, &heir, minutes, now).await;
        let (kit_json, vault_address, unlock_round) = match built {
            Ok(v) => v,
            Err(e) => {
                eprintln!("playground: creating a vault failed: {e:#}");
                self.chain.forget(&id).await;
                return Err(Self::internal(()));
            }
        };

        let mailbox_id = random_hex(16);
        let read_token = random_hex(32);
        let write_token = random_hex(32);
        self.mailboxes
            .create(
                &mailbox_id,
                &sha256(&write_token),
                &sha256(&read_token),
                now,
            )
            .map_err(Self::internal)?;
        self.mailboxes
            .put_kit(&mailbox_id, &kit_json, now)
            .map_err(Self::internal)?;
        let claim_link = format!(
            "{}/claim#m={mailbox_id}&r={read_token}",
            self.cfg.public_origin
        );
        let session = Session {
            id: id.clone(),
            token_hash: sha256(&token),
            heir_code: heir_code.trim().to_string(),
            minutes,
            state: S::Funding,
            epoch: 1,
            flips: 0,
            created: now,
            unlock_round,
            unlock_time: drand::round_time(unlock_round),
            funded_zat: self.cfg.funding_zat(),
            vault_address,
            balance_zat: 0,
            confirmed: false,
            mailbox_id: mailbox_id.clone(),
            claim_link: claim_link.clone(),
            message: "Funding the vault from the demo float.".into(),
            error: None,
            settled_at: None,
            op: None,
            op_to: None,
            attempts: 0,
            kept_swept: false,
        };
        if let Err(e) = self.sessions.insert(&session) {
            eprintln!("playground: saving a session failed: {e:#}");
            let _ = self.mailboxes.delete(&mailbox_id);
            self.chain.forget(&id).await;
            return Err(Self::internal(()));
        }
        Ok(Created {
            id,
            token,
            claim_link,
            minutes,
        })
    }

    /// Creates the vault of `epoch`, seals its seed to the heir code and
    /// returns `(kit json, vault address, unlock round)`.
    async fn build_kit(
        &self,
        id: &str,
        epoch: u32,
        heir: &HeirCode,
        minutes: u32,
        now: u64,
    ) -> Result<(String, String, u64)> {
        let nv = self.chain.create_vault(id, epoch).await?;
        let unlock_round = drand::unlock_round(now + u64::from(minutes) * 60);
        let payload = Payload {
            vault_seed: nv.seed.to_string(),
            birthday_height: nv.birthday,
            network: if self.cfg.network == "main" {
                KitNetwork::Main
            } else {
                KitNetwork::Test
            },
            plan: id.to_string(),
            epoch,
            message: None,
            restore_hints: Payload::default_hints(nv.birthday),
        };
        let kit = Kit::build(
            &payload,
            &BuildParams {
                kind: Kind::Legacy,
                sequence: None,
                now,
                unlock_round,
                heirs: std::slice::from_ref(heir),
            },
        )?;
        Ok((kit.to_json()?, nv.address, unlock_round))
    }

    /// Looks up a session and checks its bearer token (constant time).
    fn authorize(&self, id: &str, headers: &HeaderMap) -> Result<Session, ApiError> {
        let not_found = || {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "session_not_found",
                "There is no session with that id.",
            )
        };
        if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(not_found());
        }
        let s = self
            .sessions
            .get(id)
            .map_err(Self::internal)?
            .ok_or_else(not_found)?;
        let token = headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::trim)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    "The session token is missing or wrong.",
                )
            })?;
        if sha256(token).ct_eq(&s.token_hash).into() {
            Ok(s)
        } else {
            Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "The session token is missing or wrong.",
            ))
        }
    }

    /// Starts a flip (allowed in `sealed` with confirmed funds).
    pub fn begin_flip(&self, id: &str) -> Result<Session, ApiError> {
        let s = self
            .sessions
            .get(id)
            .map_err(Self::internal)?
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::NOT_FOUND,
                    "session_not_found",
                    "There is no session with that id.",
                )
            })?;
        if s.state != S::Sealed {
            return Err(state_error(
                "A vault can only be flipped while it is sealed.",
            ));
        }
        if !s.confirmed {
            return Err(state_error(
                "The vault's funds are not confirmed yet. Wait for the confirmation, then flip.",
            ));
        }
        if s.flips >= self.cfg.max_flips {
            return Err(state_error(
                "This session has used all of its flips. Wait for the unlock.",
            ));
        }
        let moved = self
            .sessions
            .transition(
                id,
                S::Sealed,
                S::Flipping,
                Some("flip"),
                None,
                "Flipping: moving the funds to a new vault.",
            )
            .map_err(Self::internal)?;
        if !moved {
            return Err(state_error("The session changed state. Check its status."));
        }
        self.sessions
            .get(id)
            .map_err(Self::internal)?
            .ok_or_else(|| Self::internal(()))
    }

    /// Settles a session after the unlock.
    pub fn begin_settle(
        &self,
        id: &str,
        action: &str,
        to: Option<&str>,
        now: u64,
    ) -> Result<Session, ApiError> {
        let s = self
            .sessions
            .get(id)
            .map_err(Self::internal)?
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::NOT_FOUND,
                    "session_not_found",
                    "There is no session with that id.",
                )
            })?;
        if s.state != S::Unlocked {
            return Err(state_error(
                "A vault can only be settled after it has unlocked.",
            ));
        }
        let moved = match action {
            "return" => self.sessions.transition(
                id,
                S::Unlocked,
                S::Settling,
                Some("return"),
                None,
                "Returning the funds to the demo float.",
            ),
            "send" => {
                let to = to
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| bad_request("A send needs the address in `to`."))?;
                if !self.chain.valid_address(to) {
                    return Err(ApiError::new(
                        StatusCode::BAD_REQUEST,
                        "network_mismatch",
                        "That is not a valid address for this network.",
                    ));
                }
                if !self.chain.is_shielded(to) {
                    return Err(bad_request(
                        "Payouts must go to a shielded address (Sapling, or a unified address with a shielded receiver). A transparent address would show the amount publicly.",
                    ));
                }
                self.sessions.transition(
                    id,
                    S::Unlocked,
                    S::Settling,
                    Some("send"),
                    Some(to),
                    "Sending the funds to your address.",
                )
            }
            "keep" => {
                let kept = self
                    .sessions
                    .count_kept_since(day_of(now) * 86_400)
                    .map_err(Self::internal)?;
                if kept >= self.cfg.max_keeps_per_day {
                    return Err(ApiError::new(
                        StatusCode::TOO_MANY_REQUESTS,
                        "daily_cap_reached",
                        "The playground has reached today's limit of kept vaults. Send the funds to your address or return them.",
                    ));
                }
                self.sessions.mark_kept(
                    id,
                    now,
                    "Kept. Restore the seed in your own wallet; an unclaimed vault returns to the float after 24 hours.",
                )
            }
            _ => return Err(bad_request("The action must be return, send or keep.")),
        }
        .map_err(Self::internal)?;
        if !moved {
            return Err(state_error("The session changed state. Check its status."));
        }
        self.sessions
            .get(id)
            .map_err(Self::internal)?
            .ok_or_else(|| Self::internal(()))
    }
}

fn pg(state: &AppState) -> Result<&Arc<Playground>, ApiError> {
    state.playground().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "playground_closed",
            "The playground is not enabled on this keeper.",
        )
    })
}

/// `GET /v1/playground/status`
pub async fn status(State(state): State<Arc<AppState>>) -> Json<Value> {
    match state.playground() {
        Some(p) => Json(p.status_json(unix_now())),
        None => Json(json!({ "enabled": false })),
    }
}

/// `POST /v1/playground/sessions`
pub async fn create(State(state): State<Arc<AppState>>, body: Bytes) -> Result<Response, ApiError> {
    let p = pg(&state)?;
    let req: CreateBody = parse_body(&body)?;
    let c = p
        .create_session(&req.judge_code, &req.heir_code, req.minutes, unix_now())
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": c.id,
            "token": c.token,
            "claim_link": c.claim_link,
            "network": p.cfg.network,
            "amount_zat": p.cfg.amount_zat,
            "minutes": c.minutes,
        })),
    )
        .into_response())
}

/// `GET /v1/playground/sessions/{id}`
pub async fn get_session(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let p = pg(&state)?;
    let s = p.authorize(&id, &headers)?;
    Ok(Json(p.session_json(&s)))
}

/// `POST /v1/playground/sessions/{id}/flip`
pub async fn flip(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let p = pg(&state)?;
    p.authorize(&id, &headers)?;
    let s = p.begin_flip(&id)?;
    Ok(Json(p.session_json(&s)))
}

/// `POST /v1/playground/sessions/{id}/settle`
pub async fn settle(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ApiError> {
    let p = pg(&state)?;
    p.authorize(&id, &headers)?;
    let req: SettleBody = parse_body(&body)?;
    let s = p.begin_settle(&id, &req.action, req.to.as_deref(), unix_now())?;
    Ok(Json(p.session_json(&s)))
}

/// Runs the state machine until the process ends.
pub fn spawn_worker(p: Arc<Playground>) {
    tokio::spawn(async move {
        loop {
            p.tick(unix_now()).await;
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });
}

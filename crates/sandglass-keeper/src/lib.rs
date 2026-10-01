//! The Sandglass keeper: blind mailboxes for kits (PLAN section 5.5).
//!
//! The keeper stores only the public part of kits and SHA-256 hashes of
//! tokens. It has no key that opens a kit. It keeps no request logs and no
//! analytics; rate limiting lives in memory only and is never persisted.

pub mod playground;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::{ConnectInfo, Path as UrlPath, Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use rand::{rngs::OsRng, RngCore};
use sandglass_core::mailbox::MAX_KIT_BYTES;
use sandglass_core::Kit;
use serde_json::json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

/// Mailboxes not updated for this long are deleted.
pub const RETENTION_SECS: u64 = 180 * 86_400;

/// Runtime configuration.
#[derive(Clone, Default)]
pub struct Config {
    /// Trust `X-Forwarded-For` (only behind a reverse proxy you control).
    pub trust_proxy: bool,
    /// Directory of static files served at `/`.
    pub web_dir: Option<PathBuf>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn sha256(text: &str) -> Vec<u8> {
    Sha256::digest(text.as_bytes()).to_vec()
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

/// SQLite storage.
pub struct Db {
    conn: Mutex<rusqlite::Connection>,
}

impl Db {
    /// Opens (and if needed creates) the database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = rusqlite::Connection::open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS mailboxes (
                 id TEXT PRIMARY KEY,
                 write_hash BLOB NOT NULL,
                 read_hash BLOB NOT NULL,
                 kit TEXT,
                 created INTEGER NOT NULL,
                 updated INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS mailboxes_updated ON mailboxes (updated);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn create(&self, id: &str, write_hash: &[u8], read_hash: &[u8], now: u64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO mailboxes (id, write_hash, read_hash, kit, created, updated)
             VALUES (?1, ?2, ?3, NULL, ?4, ?4)",
            rusqlite::params![id, write_hash, read_hash, now as i64],
        )?;
        Ok(())
    }

    /// `(write_hash, read_hash)` of a mailbox.
    fn hashes(&self, id: &str) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare_cached("SELECT write_hash, read_hash FROM mailboxes WHERE id = ?1")?;
        let mut rows = stmt.query([id])?;
        Ok(match rows.next()? {
            Some(r) => Some((r.get(0)?, r.get(1)?)),
            None => None,
        })
    }

    fn put_kit(&self, id: &str, kit: &str, now: u64) -> Result<()> {
        self.conn().execute(
            "UPDATE mailboxes SET kit = ?2, updated = ?3 WHERE id = ?1",
            rusqlite::params![id, kit, now as i64],
        )?;
        Ok(())
    }

    fn get_kit(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT kit FROM mailboxes WHERE id = ?1", [id], |r| {
                r.get::<_, Option<String>>(0)
            })?)
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM mailboxes WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Deletes mailboxes last updated before `cutoff`; returns how many.
    pub fn purge_before(&self, cutoff: u64) -> Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM mailboxes WHERE updated < ?1", [cutoff as i64])?)
    }

    /// Test helper: backdate a mailbox's last update.
    pub fn set_updated(&self, id: &str, updated: u64) -> Result<()> {
        self.conn().execute(
            "UPDATE mailboxes SET updated = ?2 WHERE id = ?1",
            rusqlite::params![id, updated as i64],
        )?;
        Ok(())
    }
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

/// In-memory token bucket per client address. Never persisted or logged.
struct Limiter {
    capacity: f64,
    per_sec: f64,
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
}

impl Limiter {
    fn new(capacity: f64, per_sec: f64) -> Self {
        Self {
            capacity,
            per_sec,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    fn allow(&self, ip: IpAddr) -> bool {
        let key = match ip {
            // One bucket per /64.
            IpAddr::V6(v6) => {
                let mut o = v6.octets();
                o[8..].fill(0);
                IpAddr::V6(o.into())
            }
            v4 => v4,
        };
        let now = Instant::now();
        let mut map = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        if map.len() > 50_000 {
            map.retain(|_, b| now.duration_since(b.last) < Duration::from_secs(3600));
            if map.len() > 50_000 {
                map.clear();
            }
        }
        let b = map.entry(key).or_insert(Bucket {
            tokens: self.capacity,
            last: now,
        });
        let elapsed = now.duration_since(b.last).as_secs_f64();
        b.tokens = (b.tokens + elapsed * self.per_sec).min(self.capacity);
        b.last = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Shared server state.
pub struct AppState {
    db: Arc<Db>,
    config: Config,
    general: Limiter,
    creation: Limiter,
    playground: OnceLock<Arc<playground::Playground>>,
}

impl AppState {
    /// Builds the state with the default rate limits.
    pub fn new(db: Db, config: Config) -> Arc<Self> {
        Arc::new(Self {
            db: Arc::new(db),
            config,
            playground: OnceLock::new(),
            // 60 requests in a burst, then 2 per second.
            general: Limiter::new(60.0, 2.0),
            // 10 new mailboxes in a burst, then 1 per minute.
            creation: Limiter::new(10.0, 1.0 / 60.0),
        })
    }

    /// The database, for maintenance and tests.
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// The database as a shared handle (the playground writes its mailboxes).
    pub fn db_arc(&self) -> Arc<Db> {
        self.db.clone()
    }

    /// The playground, when enabled.
    pub fn playground(&self) -> Option<&Arc<playground::Playground>> {
        self.playground.get()
    }

    /// Enables the playground (once).
    pub fn set_playground(&self, p: Arc<playground::Playground>) {
        let _ = self.playground.set(p);
    }
}

/// A JSON error: `{ "error": "<code>", "message": "<plain English>" }`.
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
        }
    }

    fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "The token is missing or wrong for this mailbox.",
        )
    }

    fn not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "mailbox_not_found",
            "There is no mailbox with that id, or it holds no kit yet.",
        )
    }

    fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "The keeper could not complete the request. Try again later.",
        )
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(_: anyhow::Error) -> Self {
        Self::internal()
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": self.code, "message": self.message })),
        )
            .into_response()
    }
}

#[derive(Clone, Copy)]
enum Token {
    Write,
    Read,
}

/// Checks the bearer token for a mailbox (constant-time on the hashes).
fn authorize(
    state: &AppState,
    id: &str,
    headers: &HeaderMap,
    which: Token,
) -> Result<(), ApiError> {
    if id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(ApiError::not_found());
    }
    let (write_hash, read_hash) = state.db.hashes(id)?.ok_or_else(ApiError::not_found)?;
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(ApiError::unauthorized)?;
    let expected = match which {
        Token::Write => write_hash,
        Token::Read => read_hash,
    };
    if sha256(token.trim()).ct_eq(&expected).into() {
        Ok(())
    } else {
        Err(ApiError::unauthorized())
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "version": sandglass_core::VERSION }))
}

async fn create_mailbox(State(state): State<Arc<AppState>>) -> Result<Response, ApiError> {
    let id = random_hex(16);
    let write_token = random_hex(32);
    let read_token = random_hex(32);
    state
        .db
        .create(&id, &sha256(&write_token), &sha256(&read_token), unix_now())?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": id, "write_token": write_token, "read_token": read_token })),
    )
        .into_response())
}

async fn put_kit(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &id, &headers, Token::Write)?;
    let too_big = || {
        ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "kit_invalid",
            "The kit is larger than 64 KiB.",
        )
    };
    let bytes = axum::body::to_bytes(body, MAX_KIT_BYTES)
        .await
        .map_err(|_| too_big())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "kit_invalid",
            "The kit is not valid UTF-8 JSON.",
        )
    })?;
    // Public part only: format, version, drand chain, round and time.
    if let Err(e) = Kit::from_json(text) {
        let (status, code, message) = match e {
            sandglass_core::KitError::VersionUnsupported(_) => (
                StatusCode::BAD_REQUEST,
                "kit_version_unsupported",
                "This kit has a version the keeper does not accept.",
            ),
            _ => (
                StatusCode::BAD_REQUEST,
                "kit_invalid",
                "This is not a valid Sandglass kit.",
            ),
        };
        return Err(ApiError::new(status, code, message));
    }
    state.db.put_kit(&id, text, unix_now())?;
    Ok(Json(json!({ "ok": true })))
}

async fn get_kit(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    authorize(&state, &id, &headers, Token::Read)?;
    let kit = state.db.get_kit(&id)?.ok_or_else(ApiError::not_found)?;
    Ok(([(CONTENT_TYPE, "application/json")], kit).into_response())
}

async fn delete_mailbox(
    State(state): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&state, &id, &headers, Token::Write)?;
    state.db.delete(&id)?;
    Ok(Json(json!({ "ok": true })))
}

fn client_ip(state: &AppState, req: &Request) -> Option<IpAddr> {
    if state.config.trust_proxy {
        // The proxy appends the address it saw; the last entry is the client.
        let forwarded = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse().ok());
        if forwarded.is_some() {
            return forwarded;
        }
    }
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())
}

async fn rate_limit(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    if let Some(ip) = client_ip(&state, &req) {
        let creating = req.method() == Method::POST
            && matches!(
                req.uri().path(),
                "/v1/mailboxes" | "/v1/playground/sessions"
            );
        let ok = state.general.allow(ip) && (!creating || state.creation.allow(ip));
        if !ok {
            return ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Too many requests. Wait a little and try again.",
            )
            .into_response();
        }
    }
    next.run(req).await
}

async fn unknown() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "not_found",
        "There is nothing at this address.",
    )
}

/// Builds the HTTP application.
pub fn app(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/mailboxes", post(create_mailbox))
        .route("/v1/mailboxes/{id}/kit", get(get_kit).put(put_kit))
        .route("/v1/mailboxes/{id}", delete(delete_mailbox))
        .route("/v1/playground/status", get(playground::status))
        .route("/v1/playground/sessions", post(playground::create))
        .route("/v1/playground/sessions/{id}", get(playground::get_session))
        .route("/v1/playground/sessions/{id}/flip", post(playground::flip))
        .route(
            "/v1/playground/sessions/{id}/settle",
            post(playground::settle),
        );
    let router = match &state.config.web_dir {
        Some(dir) => {
            api.fallback_service(ServeDir::new(dir).append_index_html_on_directories(true))
        }
        None => api.fallback(unknown),
    };
    router
        .layer(middleware::from_fn_with_state(state.clone(), rate_limit))
        // The heir page may live on another origin and sends the read token
        // in a header. No cookies or credentials are involved.
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([AUTHORIZATION, CONTENT_TYPE]),
        )
        .with_state(state)
}

/// Deletes stale mailboxes now and then. Runs until the process ends.
pub fn spawn_retention(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            let _ = state
                .db
                .purge_before(unix_now().saturating_sub(RETENTION_SECS));
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

/// Starts a keeper on a background thread with its own runtime (for tests and
/// embedding). Returns the address it listens on.
pub fn spawn_background(db_path: &Path, config: Config) -> Result<(SocketAddr, Arc<AppState>)> {
    let state = AppState::new(Db::open(db_path)?, config);
    let served = state.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(async move {
            let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:0").await else {
                return;
            };
            let Ok(addr) = listener.local_addr() else {
                return;
            };
            let _ = tx.send(addr);
            let _ = axum::serve(
                listener,
                app(served).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await;
        });
    });
    let addr = rx
        .recv_timeout(Duration::from_secs(10))
        .context("the keeper did not start")?;
    Ok((addr, state))
}

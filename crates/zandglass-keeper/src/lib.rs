//! The Zandglass keeper: blind mailboxes for kits (PLAN section 5.5).
//!
//! The keeper stores only the public part of kits and SHA-256 hashes of
//! tokens. It has no key that opens a kit. It keeps no request logs and no
//! analytics; rate limiting lives in memory only and is never persisted.

pub mod playground;

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::{ConnectInfo, Path as UrlPath, Request, State};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, REFERRER_POLICY};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use rand::{rngs::OsRng, RngCore};
use serde_json::json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use zandglass_core::mailbox::{MAX_BATCH, MAX_KIT_BYTES};
use zandglass_core::Kit;

/// Mailboxes not updated for this long are deleted.
pub const RETENTION_SECS: u64 = 180 * 86_400;

/// Mailboxes that never received a kit are deleted after this long.
pub const EMPTY_RETENTION_SECS: u64 = 86_400;

/// Mailboxes the whole keeper may hold by default.
pub const DEFAULT_MAX_MAILBOXES: u64 = 50_000;
/// Mailboxes the whole keeper creates per hour by default.
pub const DEFAULT_MAX_CREATES_PER_HOUR: usize = 2_000;

/// Runtime configuration.
#[derive(Clone)]
pub struct Config {
    /// Trust `CF-Connecting-IP` / `X-Forwarded-For` (only behind a reverse proxy you control).
    pub trust_proxy: bool,
    /// Directory of static files served at `/`.
    pub web_dir: Option<PathBuf>,
    /// Most mailboxes the keeper holds at once; creation fails with
    /// `rate_limited` beyond that.
    pub max_mailboxes: u64,
    /// Most mailboxes created per hour across all clients.
    pub max_creates_per_hour: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            trust_proxy: false,
            web_dir: None,
            max_mailboxes: DEFAULT_MAX_MAILBOXES,
            max_creates_per_hour: DEFAULT_MAX_CREATES_PER_HOUR,
        }
    }
}

/// The Content-Security-Policy of every response. Identical to
/// `web/vite.config.ts`.
pub const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
font-src 'self'; img-src 'self' data:; media-src 'self'; connect-src 'self' https://api.drand.sh \
https://api2.drand.sh https://drand.cloudflare.com; base-uri 'none'; form-action 'none'; \
frame-ancestors 'none'";

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
        // Created 0600; the WAL files inherit that mode.
        zandglass_wallet::fsutil::touch_private(path)?;
        let conn = rusqlite::Connection::open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        // Deleted mailboxes must not linger in free pages.
        conn.execute_batch(
            "PRAGMA secure_delete = ON;
             PRAGMA journal_mode = WAL;
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

    fn count(&self) -> Result<u64> {
        let n: i64 = self
            .conn()
            .query_row("SELECT COUNT(*) FROM mailboxes", [], |r| r.get(0))?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// Whether `PRAGMA secure_delete` is on for this connection.
    pub fn secure_delete_enabled(&self) -> bool {
        self.conn()
            .query_row("PRAGMA secure_delete", [], |r| r.get::<_, i64>(0))
            .map(|v| v != 0)
            .unwrap_or(false)
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

    /// Deletes mailboxes that never received a kit and were created before
    /// `cutoff`; returns how many.
    pub fn purge_empty_before(&self, cutoff: u64) -> Result<usize> {
        Ok(self.conn().execute(
            "DELETE FROM mailboxes WHERE kit IS NULL AND created < ?1",
            [cutoff as i64],
        )?)
    }

    /// Test helper: backdate a mailbox's creation.
    pub fn set_created(&self, id: &str, created: u64) -> Result<()> {
        self.conn().execute(
            "UPDATE mailboxes SET created = ?2 WHERE id = ?1",
            rusqlite::params![id, created as i64],
        )?;
        Ok(())
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

/// Most client buckets kept in memory.
const MAX_BUCKETS: usize = 50_000;

/// The bucket an address belongs to: IPv4 per address, IPv6 per /48 (the
/// smallest common end-site allocation), so one allocation cannot mint
/// buckets.
fn bucket_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => {
            let mut o = v6.octets();
            o[6..].fill(0);
            IpAddr::V6(o.into())
        }
        v4 => v4,
    }
}

/// In-memory token bucket per client address. Never persisted or logged.
struct Limiter {
    capacity: f64,
    per_sec: f64,
    max_entries: usize,
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
}

impl Limiter {
    fn new(capacity: f64, per_sec: f64) -> Self {
        Self::with_max_entries(capacity, per_sec, MAX_BUCKETS)
    }

    fn with_max_entries(capacity: f64, per_sec: f64, max_entries: usize) -> Self {
        Self {
            capacity,
            per_sec,
            max_entries,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.buckets.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    fn allow(&self, ip: IpAddr) -> bool {
        let key = bucket_key(ip);
        let now = Instant::now();
        let mut map = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        if map.len() >= self.max_entries && !map.contains_key(&key) {
            map.retain(|_, b| now.duration_since(b.last) < Duration::from_secs(3600));
            if map.len() >= self.max_entries {
                // Evict the oldest entries (a tenth, at least one) instead of
                // forgetting everyone: a flood of new addresses must not
                // reset the limits of the clients already being limited.
                let mut by_age: Vec<(Instant, IpAddr)> =
                    map.iter().map(|(k, b)| (b.last, *k)).collect();
                by_age.sort_unstable_by_key(|(last, _)| *last);
                let drop_n = (self.max_entries / 10).max(1);
                for (_, k) in by_age.into_iter().take(drop_n) {
                    map.remove(&k);
                }
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
    /// When the mailboxes created in the last hour were created.
    recent_creations: Mutex<VecDeque<Instant>>,
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
            recent_creations: Mutex::new(VecDeque::new()),
        })
    }

    /// Books `n` mailbox creations against the global hourly cap (all or
    /// nothing).
    fn allow_creation(&self, n: usize) -> bool {
        let now = Instant::now();
        let mut q = self
            .recent_creations
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        while q
            .front()
            .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(3600))
        {
            q.pop_front();
        }
        if q.len().saturating_add(n) > self.config.max_creates_per_hour {
            return false;
        }
        q.extend(std::iter::repeat_n(now, n));
        true
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
    Json(json!({ "ok": true, "version": zandglass_core::VERSION }))
}

/// `POST /v1/mailboxes`: one mailbox, or `{"count": N}` for a batch. A batch
/// counts once against the per-address limiter (in the middleware) and N
/// against the global caps; it is refused as a whole if it does not fit.
async fn create_mailbox(
    State(state): State<Arc<AppState>>,
    body: Body,
) -> Result<Response, ApiError> {
    let full = || {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "The keeper cannot take new mailboxes right now. Try again later.",
        )
    };
    let bad = || {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "The body must be empty or {\"count\": N} with N from 1 to 120.",
        )
    };
    let bytes = axum::body::to_bytes(body, 1024).await.map_err(|_| bad())?;
    let (count, batch) = if bytes.iter().all(u8::is_ascii_whitespace) {
        (1usize, false)
    } else {
        let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
        let n = v
            .get("count")
            .map_or(Some(1), |c| c.as_u64())
            .ok_or_else(bad)?;
        let n = usize::try_from(n).map_err(|_| bad())?;
        if !(1..=MAX_BATCH).contains(&n) {
            return Err(bad());
        }
        (n, n > 1)
    };
    let held = state.db.count()?;
    if held.saturating_add(count as u64) > state.config.max_mailboxes
        || !state.allow_creation(count)
    {
        return Err(full());
    }
    let now = unix_now();
    let mut made = Vec::with_capacity(count);
    for _ in 0..count {
        let id = random_hex(16);
        let write_token = random_hex(32);
        let read_token = random_hex(32);
        state
            .db
            .create(&id, &sha256(&write_token), &sha256(&read_token), now)?;
        made.push(json!({ "id": id, "write_token": write_token, "read_token": read_token }));
    }
    let body = if batch {
        json!({ "mailboxes": made })
    } else {
        made.remove(0)
    };
    Ok((StatusCode::CREATED, Json(body)).into_response())
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
    let stored = canonical_kit(text)?;
    state.db.put_kit(&id, &stored, unix_now())?;
    Ok(Json(json!({ "ok": true })))
}

/// The top-level fields of a kit. Anything else is refused, so the mailbox
/// cannot be used to park arbitrary data.
const KIT_FIELDS: [&str; 10] = [
    "format", "version", "kind", "network", "created", "plan", "epoch", "sequence", "unlock",
    "sealed",
];

/// Validates a kit and returns its canonical re-serialisation, which is what
/// the keeper stores (never the submitted text). Public part only: format,
/// version, drand chain, round and time.
fn canonical_kit(text: &str) -> Result<String, ApiError> {
    let invalid = || {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "kit_invalid",
            "This is not a valid Zandglass kit.",
        )
    };
    let kit = Kit::from_json(text).map_err(|e| match e {
        zandglass_core::KitError::VersionUnsupported(_) => ApiError::new(
            StatusCode::BAD_REQUEST,
            "kit_version_unsupported",
            "This kit has a version the keeper does not accept.",
        ),
        _ => invalid(),
    })?;
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| invalid())?;
    let fields = value.as_object().ok_or_else(invalid)?;
    if fields.keys().any(|k| !KIT_FIELDS.contains(&k.as_str())) {
        return Err(invalid());
    }
    kit.to_json().map_err(|_| invalid())
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
        // Cloudflare sets this to the connecting client and overwrites any
        // value the client sent.
        let cf = req
            .headers()
            .get("cf-connecting-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok());
        if cf.is_some() {
            return cf;
        }
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

/// Adds the security headers to every response: the CSP, no referrer, no
/// MIME sniffing, and `Cache-Control: no-store` on the API (tokens and kits
/// must never sit in a shared cache).
async fn security_headers(req: Request, next: Next) -> Response {
    let api = req.uri().path().starts_with("/v1/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(CSP),
    );
    h.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(
        HeaderName::from_static("strict-transport-security"),
        HeaderValue::from_static("max-age=31536000"),
    );
    h.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    if api {
        h.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    } else if !h.contains_key(CACHE_CONTROL) {
        // Proxies and CDNs must serve pages exactly as sent: no injected
        // scripts (such as automatic web analytics), no rewriting.
        h.insert(CACHE_CONTROL, HeaderValue::from_static("no-transform"));
    }
    res
}

/// Pages served at clean addresses: `/playground` serves `playground.html`.
const PAGES: [&str; 4] = ["playground", "heir", "claim", "owner"];

/// Clean page addresses for the static site. `/playground` is served from
/// `playground.html`; old `.html` addresses (and `/index.html`) redirect
/// permanently to the clean ones, keeping the query. Browsers keep the
/// `#fragment` across the redirect, so old claim links still work.
async fn clean_urls(mut req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let query = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let stem = path.strip_prefix('/').unwrap_or(path);
    if stem == "index.html" {
        return redirect(&format!("/{query}"));
    }
    if let Some(page) = stem.strip_suffix(".html") {
        if PAGES.contains(&page) {
            return redirect(&format!("/{page}{query}"));
        }
    }
    if PAGES.contains(&stem) {
        if let Ok(uri) = format!("/{stem}.html{query}").parse() {
            *req.uri_mut() = uri;
        }
    }
    next.run(req).await
}

fn redirect(to: &str) -> Response {
    let mut res = StatusCode::MOVED_PERMANENTLY.into_response();
    if let Ok(v) = HeaderValue::from_str(to) {
        res.headers_mut().insert(axum::http::header::LOCATION, v);
    }
    res
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
        Some(dir) => api
            .fallback_service(ServeDir::new(dir).append_index_html_on_directories(true))
            .layer(middleware::from_fn(clean_urls)),
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
        // Outermost: also covers rate-limit rejections and CORS preflights.
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

/// Deletes stale mailboxes now and then. Runs until the process ends.
pub fn spawn_retention(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            let _ = state
                .db
                .purge_before(unix_now().saturating_sub(RETENTION_SECS));
            // Empty mailboxes (never written) must not hold a slot for months.
            let _ = state
                .db
                .purge_empty_before(unix_now().saturating_sub(EMPTY_RETENTION_SECS));
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
#[cfg(test)]
mod tests {
    use super::*;

    fn ip(n: u32) -> IpAddr {
        IpAddr::V4(std::net::Ipv4Addr::from(n))
    }

    #[test]
    fn ipv6_is_bucketed_by_48() {
        let a: IpAddr = "2001:db8:1:1::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:ffff:1:2:3:4".parse().unwrap();
        let c: IpAddr = "2001:db8:2:1::1".parse().unwrap();
        assert_eq!(bucket_key(a), bucket_key(b));
        assert_ne!(bucket_key(a), bucket_key(c));
        let l = Limiter::new(1.0, 0.0);
        assert!(l.allow(a));
        assert!(!l.allow(b), "same /48 shares one bucket");
        assert!(l.allow(c));
    }

    #[test]
    fn full_limiter_evicts_the_oldest_not_everyone() {
        let l = Limiter::with_max_entries(1.0, 0.0, 10);
        // A client that is being limited: its bucket is empty.
        assert!(l.allow(ip(1)));
        assert!(!l.allow(ip(1)));
        // A flood of fresh addresses fills the table and overflows it.
        for n in 2..40 {
            l.allow(ip(n));
        }
        assert!(l.len() <= 10, "the table stays capped, got {}", l.len());
        // The limited client was oldest and may be evicted, but the table was
        // not wiped: the newest addresses are still tracked and limited.
        assert!(!l.allow(ip(39)));
        assert!(l.len() > 1);
    }
}

//! `zandglass ui`: the owner panel, a local web page for the owner's plans.
//!
//! The server binds to 127.0.0.1 only and serves the built web pages plus a
//! small JSON API. Every API call needs the random session token that is put
//! in the opened URL's fragment (never sent to any other server), and every
//! request must name this host and port (no DNS rebinding). Wallet work runs
//! on one worker thread, one job at a time, with the same code as the CLI.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use axum::body::Body;
use axum::extract::{Path as UrlPath, Request, State};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, HOST, REFERRER_POLICY};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::RngCore;
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use zandglass_wallet::ops::{self, network_name, now};
use zandglass_wallet::{
    connect, connect_any, default_urls, installments, remind, Client, Network, Plan, PlanStatus,
    Store,
};
use zeroize::Zeroizing;

/// The built web pages (`web/dist`). A build without them still compiles;
/// the panel then explains how to build them.
#[derive(RustEmbed)]
#[folder = "../../web/dist/"]
#[allow_missing = true]
struct Assets;

/// The public keeper used when the owner asks for a claim link.
pub const PUBLIC_KEEPER: &str = "https://zandglass.cash";

/// The unlocked store is locked again after this long without any request.
const IDLE_LOCK: std::time::Duration = std::time::Duration::from_secs(15 * 60);

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; \
img-src 'self' data:; media-src 'self'; connect-src 'self' https://api.drand.sh https://api2.drand.sh \
https://drand.cloudflare.com; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// Settings for the panel, taken from the command line.
pub struct UiConfig {
    pub network: Network,
    /// The base data directory; the network name is appended.
    pub base: PathBuf,
    /// A fixed lightwalletd server (`--server`), or the public list.
    pub server: Option<String>,
    pub accept_unaudited: bool,
    pub allow_transparent: bool,
    pub port: u16,
    pub open_browser: bool,
}

struct AppState {
    token: String,
    hosts: Vec<String>,
    jobs: mpsc::Sender<(Job, oneshot::Sender<JobResult>)>,
}

type JobResult = std::result::Result<Value, UiError>;

enum Job {
    State,
    Init(Zeroizing<String>),
    Unlock(Zeroizing<String>),
    Lock,
    Network(String),
    Create(CreateReq),
    CreateInstallments(InstallmentsReq),
    Split(String),
    Kits(String),
    Sync,
    Flip(String),
    Cancel(String, String),
    Receive(String),
    Kit(String),
    Calendar(String),
}

#[derive(Deserialize)]
struct PassReq {
    passphrase: String,
}

#[derive(Deserialize)]
struct CreateReq {
    heirs: Vec<String>,
    every: String,
    message: Option<String>,
    #[serde(default)]
    claim_link: bool,
    amount: Option<String>,
}

#[derive(Deserialize)]
struct NetworkReq {
    network: String,
}

#[derive(Deserialize)]
struct InstallmentsReq {
    payees: Vec<String>,
    /// ZEC per installment, as text (for example "0.05").
    amount: String,
    every: String,
    count: u32,
    /// RFC 3339 time, or a delay after the split (for example "1d").
    start: Option<String>,
    message: Option<String>,
    #[serde(default)]
    claim_link: bool,
}

#[derive(Deserialize)]
struct CancelReq {
    to: String,
}

struct UiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl UiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn from_anyhow(e: &anyhow::Error) -> Self {
        let code = crate::error_code(e).unwrap_or("error");
        let status = match code {
            "unauthorized" => StatusCode::UNAUTHORIZED,
            "error" => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
        };
        Self::new(status, code, crate::redact_tokens(&format!("{e:#}")))
    }
}

impl IntoResponse for UiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": self.code, "message": self.message })),
        )
            .into_response()
    }
}

/// Runs the panel until Ctrl-C.
pub async fn serve(cfg: UiConfig) -> Result<()> {
    let mut raw = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", cfg.port)).await?;
    let port = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{port}/owner#t={token}");
    let open_browser = cfg.open_browser;
    let network = cfg.network;

    let (tx, rx) = mpsc::channel(8);
    std::thread::Builder::new()
        .name("zandglass-ui-worker".into())
        .spawn(move || worker(cfg, rx))?;

    let state = Arc::new(AppState {
        token,
        hosts: vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")],
        jobs: tx,
    });
    let app = app(state);

    println!(
        "Zandglass owner panel ({}) is running on this computer only.",
        network_name(network)
    );
    println!("Open this address in your browser (it contains a one-time session key; do not share it):\n");
    println!("  {url}\n");
    println!(
        "The store locks itself after {} minutes without use. Press Ctrl-C to stop.",
        IDLE_LOCK.as_secs() / 60
    );
    if open_browser {
        open_in_browser(&url);
    }
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// The panel's HTTP application.
fn app(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/api/state", get(api_state))
        .route("/api/init", post(api_init))
        .route("/api/unlock", post(api_unlock))
        .route("/api/lock", post(api_lock))
        .route("/api/network", post(api_network))
        .route("/api/plans", post(api_create))
        .route("/api/sync", post(api_sync))
        .route("/api/installments", post(api_installments))
        .route("/api/plans/{id}/split", post(api_split))
        .route("/api/plans/{id}/kits", get(api_kits))
        .route("/api/plans/{id}/flip", post(api_flip))
        .route("/api/plans/{id}/cancel", post(api_cancel))
        .route("/api/plans/{id}/receive", get(api_receive))
        .route("/api/plans/{id}/kit", get(api_kit))
        .route("/api/plans/{id}/calendar", get(api_calendar))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token));
    api.fallback(static_file)
        .layer(middleware::from_fn_with_state(state.clone(), check_host))
        .layer(middleware::from_fn(headers))
        .with_state(state)
}

fn open_in_browser(url: &str) {
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    if r.is_err() {
        println!("(Could not open a browser by itself; copy the address above.)");
    }
}

/* ------------------------------------------------------------- middleware */

/// Rejects requests that do not name this server (DNS rebinding).
async fn check_host(State(s): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| s.hosts.iter().any(|x| x == h));
    if !ok {
        return UiError::new(StatusCode::FORBIDDEN, "forbidden", "Unknown host.").into_response();
    }
    next.run(req).await
}

/// API calls need the session token as a bearer token.
async fn require_token(State(s): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let given = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or("");
    let ok = given.len() == s.token.len()
        && given
            .bytes()
            .zip(s.token.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0;
    if !ok {
        return UiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or wrong session key. Open the address printed by `zandglass ui`.",
        )
        .into_response();
    }
    next.run(req).await
}

async fn headers(req: Request, next: Next) -> Response {
    let api = req.uri().path().starts_with("/api/");
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(CSP),
    );
    h.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        CACHE_CONTROL,
        HeaderValue::from_static(if api { "no-store" } else { "no-cache" }),
    );
    res
}

/* ------------------------------------------------------------- static */

async fn static_file(req: Request) -> Response {
    let path = req.uri().path().trim_start_matches('/');
    let name = match path {
        "" | "owner" => "owner.html".to_string(),
        "playground" | "heir" | "claim" => format!("{path}.html"),
        p => p.to_string(),
    };
    match Assets::get(&name) {
        Some(file) => {
            let mime = file.metadata.mimetype().to_string();
            Response::builder()
                .header(CONTENT_TYPE, mime)
                .body(Body::from(file.data.into_owned()))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        None if name == "owner.html" => (
            StatusCode::NOT_FOUND,
            "This build of zandglass has no web pages. Build them first: `cd web && npm ci && npm run build`, then rebuild the CLI.",
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/* ------------------------------------------------------------- api */

async fn run_job(s: &AppState, job: Job) -> Response {
    let (tx, rx) = oneshot::channel();
    if s.jobs.send((job, tx)).await.is_err() {
        return UiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "error",
            "The worker stopped.",
        )
        .into_response();
    }
    match rx.await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => e.into_response(),
        Err(_) => UiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "error",
            "The worker stopped.",
        )
        .into_response(),
    }
}

async fn api_state(State(s): State<Arc<AppState>>) -> Response {
    run_job(&s, Job::State).await
}

async fn api_init(State(s): State<Arc<AppState>>, Json(r): Json<PassReq>) -> Response {
    run_job(&s, Job::Init(Zeroizing::new(r.passphrase))).await
}

async fn api_unlock(State(s): State<Arc<AppState>>, Json(r): Json<PassReq>) -> Response {
    run_job(&s, Job::Unlock(Zeroizing::new(r.passphrase))).await
}

async fn api_lock(State(s): State<Arc<AppState>>) -> Response {
    run_job(&s, Job::Lock).await
}

async fn api_network(State(s): State<Arc<AppState>>, Json(r): Json<NetworkReq>) -> Response {
    run_job(&s, Job::Network(r.network)).await
}

async fn api_create(State(s): State<Arc<AppState>>, Json(r): Json<CreateReq>) -> Response {
    run_job(&s, Job::Create(r)).await
}

async fn api_installments(
    State(s): State<Arc<AppState>>,
    Json(r): Json<InstallmentsReq>,
) -> Response {
    run_job(&s, Job::CreateInstallments(r)).await
}

async fn api_split(State(s): State<Arc<AppState>>, UrlPath(id): UrlPath<String>) -> Response {
    run_job(&s, Job::Split(id)).await
}

async fn api_kits(State(s): State<Arc<AppState>>, UrlPath(id): UrlPath<String>) -> Response {
    run_job(&s, Job::Kits(id)).await
}

async fn api_sync(State(s): State<Arc<AppState>>) -> Response {
    run_job(&s, Job::Sync).await
}

async fn api_flip(State(s): State<Arc<AppState>>, UrlPath(id): UrlPath<String>) -> Response {
    run_job(&s, Job::Flip(id)).await
}

async fn api_cancel(
    State(s): State<Arc<AppState>>,
    UrlPath(id): UrlPath<String>,
    Json(r): Json<CancelReq>,
) -> Response {
    run_job(&s, Job::Cancel(id, r.to)).await
}

async fn api_receive(State(s): State<Arc<AppState>>, UrlPath(id): UrlPath<String>) -> Response {
    run_job(&s, Job::Receive(id)).await
}

async fn api_kit(State(s): State<Arc<AppState>>, UrlPath(id): UrlPath<String>) -> Response {
    run_job(&s, Job::Kit(id)).await
}

async fn api_calendar(State(s): State<Arc<AppState>>, UrlPath(id): UrlPath<String>) -> Response {
    run_job(&s, Job::Calendar(id)).await
}

/* ------------------------------------------------------------- worker */

struct Worker {
    cfg: UiConfig,
    store: Option<Store>,
    last_use: std::time::Instant,
}

fn worker(cfg: UiConfig, mut rx: mpsc::Receiver<(Job, oneshot::Sender<JobResult>)>) {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let mut w = Worker {
        cfg,
        store: None,
        last_use: std::time::Instant::now(),
    };
    loop {
        // An unlocked store left alone locks itself as soon as the idle time
        // is over, not only at the next request: the decrypted store (and the
        // passphrase in it) leaves memory.
        let wait = IDLE_LOCK.saturating_sub(w.last_use.elapsed());
        let next = rt.block_on(async { tokio::time::timeout(wait, rx.recv()).await });
        let (job, reply) = match next {
            Ok(Some(msg)) => msg,
            Ok(None) => break,
            Err(_) => {
                w.store = None;
                w.last_use = std::time::Instant::now();
                continue;
            }
        };
        if w.last_use.elapsed() > IDLE_LOCK {
            w.store = None;
        }
        w.last_use = std::time::Instant::now();
        let out = rt.block_on(w.handle(job));
        let _ = reply.send(out.map_err(|e| UiError::from_anyhow(&e)));
    }
}

impl Worker {
    fn store(&self) -> Result<&Store> {
        self.store
            .as_ref()
            .ok_or_else(|| zandglass_wallet::coded("unauthorized", "unlock the store first"))
    }

    fn root(&self) -> PathBuf {
        self.cfg.base.join(network_name(self.cfg.network))
    }

    fn kit_dir(&self) -> PathBuf {
        self.root().join("kits")
    }

    async fn client(&self) -> Result<Client> {
        match &self.cfg.server {
            Some(url) => connect(url).await,
            None => {
                let urls: Vec<String> = default_urls(self.cfg.network)
                    .iter()
                    .map(|u| (*u).to_string())
                    .collect();
                Ok(connect_any(&urls).await?.0)
            }
        }
    }

    async fn handle(&mut self, job: Job) -> Result<Value> {
        let network = self.cfg.network;
        match job {
            Job::State => self.state(),
            Job::Init(pass) => {
                if pass.chars().count() < 10 {
                    return Err(zandglass_wallet::coded(
                        "invalid_request",
                        "use a passphrase of at least 10 characters",
                    ));
                }
                self.store = Some(Store::init(&self.root(), &pass)?);
                self.state()
            }
            Job::Unlock(pass) => {
                self.store = Some(Store::open(&self.root(), &pass)?);
                self.state()
            }
            Job::Lock => {
                self.store = None;
                self.state()
            }
            Job::Network(name) => {
                if self.cfg.server.is_some() {
                    return Err(zandglass_wallet::coded(
                        "invalid_request",
                        "the panel was started with --server, which fixes the network",
                    ));
                }
                self.cfg.network = match name.as_str() {
                    "main" => Network::MainNetwork,
                    "test" => Network::TestNetwork,
                    _ => {
                        return Err(zandglass_wallet::coded(
                            "invalid_request",
                            "network must be main or test",
                        ))
                    }
                };
                // Each network has its own store; lock the old one.
                self.store = None;
                self.state()
            }
            Job::Create(r) => {
                let store = self.store()?;
                ops::check_heirs(&r.heirs)?;
                let new = ops::NewPlan {
                    heirs: r.heirs.clone(),
                    period_secs: ops::parse_duration(&r.every)?,
                    message: r.message.filter(|m| !m.trim().is_empty()),
                    mailbox_url: r.claim_link.then(|| PUBLIC_KEEPER.to_string()),
                    amount: r
                        .amount
                        .as_deref()
                        .filter(|a| !a.trim().is_empty())
                        .map(ops::parse_zec)
                        .transpose()?,
                };
                let mut client = self.client().await?;
                let c = ops::create_plan(
                    store,
                    network,
                    &mut client,
                    new,
                    &self.kit_dir(),
                    self.cfg.accept_unaudited,
                )
                .await?;
                Ok(json!({
                    "plan": summary(store, network, &c.plan),
                    "address": c.address,
                    "uri": c.uri,
                    "claim_link": c.claim_link.map(|l| l.to_string()),
                }))
            }
            Job::CreateInstallments(r) => {
                let store = self.store()?;
                let amount_zat = ops::parse_zec(&r.amount)?;
                let every_secs = ops::parse_duration(&r.every)?;
                let (start_at, start_after_secs) =
                    match r.start.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                        Some(s) => installments::parse_start(s)?,
                        None => (None, every_secs),
                    };
                installments::validate(&r.payees, amount_zat, r.count, start_at)?;
                let mut client = self.client().await?;
                let c = installments::create_installments(
                    store,
                    network,
                    &mut client,
                    installments::NewInstallments {
                        payees: r.payees,
                        amount_zat,
                        every_secs,
                        count: r.count,
                        start_at,
                        start_after_secs,
                        message: r.message.filter(|m| !m.trim().is_empty()),
                        mailbox_url: r.claim_link.then(|| PUBLIC_KEEPER.to_string()),
                        out_dir: Some(self.kit_dir().display().to_string()),
                    },
                    self.cfg.accept_unaudited,
                )
                .await?;
                Ok(json!({
                    "plan": summary(store, network, &c.plan),
                    "address": c.address,
                    "uri": c.uri,
                    "required_zat": c.required,
                    "claim_link": Value::Null,
                }))
            }
            Job::Split(id) => {
                let store = self.store()?;
                let plan = installments::require_installment_plan(store, Some(id.as_str()))?;
                let mut client = self.client().await?;
                let r = installments::split(
                    store,
                    network,
                    &mut client,
                    &plan.id,
                    &self.kit_dir(),
                    self.cfg.accept_unaudited,
                )
                .await?;
                let links = r
                    .links_path
                    .as_ref()
                    .and_then(|p| std::fs::read_to_string(p).ok());
                Ok(json!({
                    "plan": summary(store, network, &r.plan),
                    "txid": r.tx.txid,
                    "lines": r.lines.iter().map(|l| json!({
                        "index": l.index,
                        "unlock_time": l.unlock_time,
                        "kit_file": l.kit_file,
                    })).collect::<Vec<_>>(),
                    "links_file": r.plan.installments.as_ref().map(|_| format!("{}-claim-links.txt", r.plan.short_id())),
                    "links": links,
                }))
            }
            Job::Kits(id) => {
                let store = self.store()?;
                let plan = installments::require_installment_plan(store, Some(id.as_str()))?;
                let set = plan
                    .installments
                    .as_ref()
                    .ok_or_else(|| anyhow!("not an installment plan"))?;
                let mut files = Vec::new();
                for item in &set.items {
                    if let Ok(text) = std::fs::read_to_string(self.kit_dir().join(&item.kit_file)) {
                        files.push(json!({ "file_name": item.kit_file, "text": text }));
                    }
                }
                let links_name = format!("{}-claim-links.txt", plan.short_id());
                if let Ok(text) = std::fs::read_to_string(self.kit_dir().join(&links_name)) {
                    files.push(json!({ "file_name": links_name, "text": text }));
                }
                if files.is_empty() {
                    return Err(anyhow!(
                        "no kit files yet: the kits are written when the funding vault is split"
                    ));
                }
                Ok(json!({ "files": files }))
            }
            Job::Sync => {
                let store = self.store()?;
                let mut client = self.client().await?;
                for mut plan in store.plans()? {
                    if plan.status != PlanStatus::Active {
                        continue;
                    }
                    if plan.kind == "installments" {
                        installments::sync_all(store, network, &mut client, &plan).await?;
                        continue;
                    }
                    if plan.pending.is_some() {
                        ops::resolve_pending(store, network, &mut client, &mut plan, None).await?;
                    }
                    ops::open_vault(store, network, plan.current()?)?
                        .sync(&mut client)
                        .await?;
                }
                self.state()
            }
            Job::Flip(id) => {
                let store = self.store()?;
                let mut client = self.client().await?;
                let r = ops::flip(
                    store,
                    network,
                    &mut client,
                    &id,
                    &self.kit_dir(),
                    self.cfg.accept_unaudited,
                )
                .await?;
                Ok(json!({
                    "plan": summary(store, network, &r.plan),
                    "moved_zat": r.moved,
                    "adopted": r.adopted,
                    "txids": r.sweeps.iter().map(|s| s.txid.clone()).collect::<Vec<_>>(),
                    "mailbox_error": r.mailbox_error,
                }))
            }
            Job::Cancel(id, to) => {
                ops::check_destination(network, &to, self.cfg.allow_transparent)?;
                let store = self.store()?;
                let mut client = self.client().await?;
                let r = ops::cancel(store, network, &mut client, &id, &to).await?;
                Ok(json!({
                    "plan": summary(store, network, &r.plan),
                    "moved_zat": r.moved,
                    "txids": r.sweeps.iter().map(|s| s.txid.clone()).collect::<Vec<_>>(),
                }))
            }
            Job::Receive(id) => {
                let store = self.store()?;
                let plan = store.find_plan(Some(id.as_str()))?;
                let mut vault = ops::open_vault(store, network, plan.current()?)?;
                Ok(json!({ "address": vault.address()?, "uri": vault.zip321_uri(None)? }))
            }
            Job::Kit(id) => {
                let store = self.store()?;
                let plan = store.find_plan(Some(id.as_str()))?;
                if plan.kind == "installments" {
                    return Err(anyhow!(
                        "installment kits are written by the split; find them in the plan's folder"
                    ));
                }
                let name = ops::kit_file_name(&plan, plan.epoch);
                let text = std::fs::read_to_string(self.kit_dir().join(&name)).map_err(|_| {
                    anyhow!("the kit file {name} is not in this store's kits folder (it was written elsewhere, for example by the CLI with --out)")
                })?;
                Ok(json!({ "file_name": name, "text": text }))
            }
            Job::Calendar(id) => {
                let store = self.store()?;
                let plan = store.find_plan(Some(id.as_str()))?;
                Ok(json!({
                    "file_name": format!("zandglass-{}.ics", plan.short_id()),
                    "text": remind::plan_ics(&plan, now()),
                }))
            }
        }
    }

    fn state(&self) -> Result<Value> {
        let network = self.cfg.network;
        let store_state = if self.store.is_some() {
            "open"
        } else if self.root().join("keycheck.age").exists() {
            "locked"
        } else {
            "missing"
        };
        let plans: Vec<Value> = match &self.store {
            Some(s) => s.plans()?.iter().map(|p| summary(s, network, p)).collect(),
            None => Vec::new(),
        };
        Ok(json!({
            "network": network_name(network),
            "network_fixed": self.cfg.server.is_some(),
            "store": store_state,
            "now": now(),
            "cap_zat": ops::MAINNET_CAP_ZAT,
            "public_keeper": PUBLIC_KEEPER,
            "plans": plans,
        }))
    }
}

/// What the panel shows for one plan. Balances come from the local wallet
/// database (no network); the panel's Sync button refreshes them.
fn summary(store: &Store, network: Network, p: &Plan) -> Value {
    let current = p.current().ok();
    let balance = current
        .filter(|_| p.status == PlanStatus::Active)
        .and_then(|rec| ops::open_vault(store, network, rec).ok())
        .and_then(|v| v.balances().ok())
        .map(|b| {
            json!({
                "total_zat": b.total(),
                "spendable_zat": b.spendable(),
                "height": b.fully_scanned,
            })
        });
    json!({
        "id": p.id,
        "short_id": p.short_id(),
        "kind": p.kind,
        "status": match p.status { PlanStatus::Active => "active", PlanStatus::Cancelled => "cancelled" },
        "epoch": p.epoch,
        "heirs": p.heirs.len(),
        "period_secs": p.period_secs,
        "created": p.created,
        "unlock_time": current.map(|r| r.unlock_time),
        "epoch_created": current.map(|r| r.created),
        "mailbox": p.mailbox_url.is_some(),
        "pending": p.pending.is_some(),
        "balance": balance,
        "installments": p.installments.as_ref().map(|s| json!({
            "count": s.count,
            "amount_zat": s.amount_zat,
            "split": s.split_txid.is_some(),
            "unlocks": s.items.iter().map(|i| i.unlock_time).collect::<Vec<_>>(),
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A panel whose worker answers every job with the given state.
    fn test_app() -> Router {
        let (tx, mut rx) = mpsc::channel::<(Job, oneshot::Sender<JobResult>)>(8);
        tokio::spawn(async move {
            while let Some((_, reply)) = rx.recv().await {
                let _ = reply.send(Ok(json!({ "store": "locked" })));
            }
        });
        app(Arc::new(AppState {
            token: TOKEN.into(),
            hosts: vec!["127.0.0.1:9999".into(), "localhost:9999".into()],
            jobs: tx,
        }))
    }

    async fn status(host: &str, auth: Option<&str>, path: &str) -> (StatusCode, Option<String>) {
        let mut req = HttpRequest::builder().uri(path).header(HOST, host);
        if let Some(a) = auth {
            req = req.header(AUTHORIZATION, a);
        }
        let res = test_app()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let csp = res
            .headers()
            .get("content-security-policy")
            .map(|v| v.to_str().unwrap().to_string());
        (res.status(), csp)
    }

    #[tokio::test]
    async fn api_needs_the_session_key() {
        let bearer = format!("Bearer {TOKEN}");
        assert_eq!(
            status("127.0.0.1:9999", Some(&bearer), "/api/state")
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            status("localhost:9999", Some(&bearer), "/api/state")
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            status("127.0.0.1:9999", None, "/api/state").await.0,
            StatusCode::UNAUTHORIZED
        );
        let wrong = format!("Bearer {}", "f".repeat(TOKEN.len()));
        assert_eq!(
            status("127.0.0.1:9999", Some(&wrong), "/api/state").await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status("127.0.0.1:9999", Some("Bearer short"), "/api/state")
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn other_hosts_are_refused_even_with_the_key() {
        // DNS rebinding: a page on evil.example resolved to 127.0.0.1.
        let bearer = format!("Bearer {TOKEN}");
        assert_eq!(
            status("evil.example:9999", Some(&bearer), "/api/state")
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status("127.0.0.1:1234", Some(&bearer), "/api/state")
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status("evil.example", None, "/owner").await.0,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn every_response_carries_the_csp() {
        let (_, csp) = status("127.0.0.1:9999", None, "/api/state").await;
        assert!(csp.unwrap().starts_with("default-src 'none'"));
        let (_, csp) = status("127.0.0.1:9999", None, "/owner").await;
        assert!(csp.unwrap().contains("frame-ancestors 'none'"));
    }
}

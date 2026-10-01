//! `zandglass-keeper`: hosts blind mailboxes (it can never open a kit) and,
//! when enabled, the playground for judges.
//!
//! Configuration comes from the environment:
//! `ZANDGLASS_LISTEN` (default `127.0.0.1:8787`), `ZANDGLASS_DB` (default
//! `./keeper.sqlite`), `ZANDGLASS_TRUST_PROXY=1` (use `X-Forwarded-For` for
//! rate limiting), `ZANDGLASS_WEB_DIR` (static files served at `/`),
//! `ZANDGLASS_MAX_MAILBOXES` (default 50000) and
//! `ZANDGLASS_MAX_CREATES_PER_HOUR` (default 2000, all clients together).
//! The playground is enabled by `ZANDGLASS_PLAYGROUND_NETWORK`; see
//! `docs/PLAYGROUND_API.md` for its variables.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use zandglass_keeper::playground::{self, real::RealChain, PlaygroundEnv};
use zandglass_keeper::{app, spawn_retention, AppState, Config, Db};

#[tokio::main]
async fn main() -> Result<()> {
    let listen: SocketAddr = std::env::var("ZANDGLASS_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8787".into())
        .parse()
        .context("ZANDGLASS_LISTEN must look like 127.0.0.1:8787")?;
    let db_path = std::env::var("ZANDGLASS_DB").unwrap_or_else(|_| "./keeper.sqlite".into());
    let number = |name: &str| -> Result<Option<u64>> {
        std::env::var(name)
            .ok()
            .map(|v| {
                v.parse()
                    .with_context(|| format!("{name} must be a number"))
            })
            .transpose()
    };
    let defaults = Config::default();
    let config = Config {
        trust_proxy: std::env::var("ZANDGLASS_TRUST_PROXY").is_ok_and(|v| v == "1"),
        web_dir: std::env::var_os("ZANDGLASS_WEB_DIR").map(PathBuf::from),
        max_mailboxes: number("ZANDGLASS_MAX_MAILBOXES")?.unwrap_or(defaults.max_mailboxes),
        max_creates_per_hour: number("ZANDGLASS_MAX_CREATES_PER_HOUR")?
            .map_or(defaults.max_creates_per_hour, |n| {
                usize::try_from(n).unwrap_or(usize::MAX)
            }),
    };
    let db = Db::open(&PathBuf::from(db_path))?;
    let state = AppState::new(db, config);
    spawn_retention(state.clone());
    if let Some(env) = PlaygroundEnv::from_env()? {
        let sessions_path = env.real.data_dir.join("playground.sqlite");
        let network = env.config.network;
        let chain = RealChain::start(env.real)?;
        let pg = playground::Playground::new(env.config, chain, &sessions_path, state.db_arc())?;
        playground::spawn_worker(pg.clone());
        state.set_playground(pg);
        eprintln!("playground enabled on {network}");
    }
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("binding {listen}"))?;
    // Startup line only; requests are never logged.
    eprintln!(
        "zandglass-keeper {} listening on {listen}",
        zandglass_core::VERSION
    );
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

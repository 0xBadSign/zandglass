//! `sandglass-keeper`: hosts blind mailboxes (it can never open a kit) and,
//! when enabled, the playground for judges.
//!
//! Configuration comes from the environment:
//! `SANDGLASS_LISTEN` (default `127.0.0.1:8787`), `SANDGLASS_DB` (default
//! `./keeper.sqlite`), `SANDGLASS_TRUST_PROXY=1` (use `X-Forwarded-For` for
//! rate limiting) and `SANDGLASS_WEB_DIR` (static files served at `/`).
//! The playground is enabled by `SANDGLASS_PLAYGROUND_NETWORK`; see
//! `docs/PLAYGROUND_API.md` for its variables.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use sandglass_keeper::playground::{self, real::RealChain, PlaygroundEnv};
use sandglass_keeper::{app, spawn_retention, AppState, Config, Db};

#[tokio::main]
async fn main() -> Result<()> {
    let listen: SocketAddr = std::env::var("SANDGLASS_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8787".into())
        .parse()
        .context("SANDGLASS_LISTEN must look like 127.0.0.1:8787")?;
    let db_path = std::env::var("SANDGLASS_DB").unwrap_or_else(|_| "./keeper.sqlite".into());
    let config = Config {
        trust_proxy: std::env::var("SANDGLASS_TRUST_PROXY").is_ok_and(|v| v == "1"),
        web_dir: std::env::var_os("SANDGLASS_WEB_DIR").map(PathBuf::from),
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
        "sandglass-keeper {} listening on {listen}",
        sandglass_core::VERSION
    );
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

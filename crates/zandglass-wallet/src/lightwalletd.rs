//! lightwalletd gRPC connectivity, with fallback across public servers.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use tonic::transport::{Channel, ClientTlsConfig};
use zcash_client_backend::proto::service::compact_tx_streamer_client::CompactTxStreamerClient;
use zcash_client_backend::proto::service::ChainSpec;
use zcash_protocol::consensus::Network;

/// The gRPC client type used throughout the crate.
pub type Client = CompactTxStreamerClient<Channel>;

/// How long one server gets to connect and answer a health check.
const PROBE_TIMEOUT: Duration = Duration::from_secs(12);

/// Default public lightwalletd endpoint for a network.
pub fn default_url(network: Network) -> &'static str {
    default_urls(network)[0]
}

/// Public lightwalletd servers, in the order they are tried. Mainnet has
/// several independent operators; testnet currently has one public server.
pub fn default_urls(network: Network) -> &'static [&'static str] {
    match network {
        Network::MainNetwork => &[
            "https://zec.rocks:443",
            "https://na.zec.rocks:443",
            "https://eu.zec.rocks:443",
            "https://us.zec.stardust.rest:443",
            "https://eu.zec.stardust.rest:443",
            "https://zcash.mysideoftheweb.com:9067",
        ],
        Network::TestNetwork => &["https://testnet.zec.rocks:443"],
    }
}

/// Splits a comma-separated server list (as in `ZANDGLASS_LWD_URL`).
pub fn parse_urls(list: &str) -> Vec<String> {
    list.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Connects to a lightwalletd server over TLS (webpki roots).
pub async fn connect(url: &str) -> Result<Client> {
    // Installing twice is harmless: the second call just returns Err.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut endpoint = Channel::from_shared(url.to_string())
        .with_context(|| format!("invalid lightwalletd URL {url}"))?;
    if url.starts_with("https://") {
        endpoint = endpoint
            .tls_config(ClientTlsConfig::new().with_webpki_roots())
            .context("TLS configuration")?;
    }
    let channel = endpoint
        .connect_timeout(PROBE_TIMEOUT)
        .connect()
        .await
        .with_context(|| format!("connecting to {url}"))?;
    Ok(CompactTxStreamerClient::new(channel))
}

/// Connects to the first server in `urls` that connects and answers a
/// latest-block request in time. Returns the client and the URL used.
pub async fn connect_any(urls: &[String]) -> Result<(Client, String)> {
    let mut errors = Vec::new();
    for url in urls {
        match tokio::time::timeout(PROBE_TIMEOUT, probe(url)).await {
            Ok(Ok(client)) => return Ok((client, url.clone())),
            Ok(Err(e)) => errors.push(format!("{url}: {e:#}")),
            Err(_) => errors.push(format!("{url}: no answer in {}s", PROBE_TIMEOUT.as_secs())),
        }
    }
    Err(anyhow!(
        "no lightwalletd server could be reached ({})",
        errors.join("; ")
    ))
}

async fn probe(url: &str) -> Result<Client> {
    let mut client = connect(url).await?;
    client
        .get_latest_block(ChainSpec {})
        .await
        .with_context(|| format!("{url} did not answer"))?;
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_lists() {
        assert_eq!(default_url(Network::MainNetwork), "https://zec.rocks:443");
        assert!(default_urls(Network::MainNetwork).len() > 1);
        assert!(default_urls(Network::MainNetwork)
            .iter()
            .chain(default_urls(Network::TestNetwork))
            .all(|u| u.starts_with("https://")));
        assert_eq!(
            parse_urls(" https://a:443, ,https://b:443 "),
            vec!["https://a:443".to_string(), "https://b:443".to_string()]
        );
    }

    #[tokio::test]
    async fn connect_any_reports_every_failure() {
        let urls = vec!["https://127.0.0.1:1".to_string(), "not a url".to_string()];
        let Err(e) = connect_any(&urls).await else {
            panic!("must fail")
        };
        let text = format!("{e:#}");
        assert!(
            text.contains("127.0.0.1:1") && text.contains("not a url"),
            "{text}"
        );
    }
}

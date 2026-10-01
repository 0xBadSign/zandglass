//! lightwalletd gRPC connectivity.

use anyhow::{Context, Result};
use tonic::transport::{Channel, ClientTlsConfig};
use zcash_client_backend::proto::service::compact_tx_streamer_client::CompactTxStreamerClient;
use zcash_protocol::consensus::Network;

/// The gRPC client type used throughout the crate.
pub type Client = CompactTxStreamerClient<Channel>;

/// Default public lightwalletd endpoint for a network.
pub fn default_url(network: Network) -> &'static str {
    match network {
        Network::MainNetwork => "https://zec.rocks:443",
        Network::TestNetwork => "https://testnet.zec.rocks:443",
    }
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
        .connect()
        .await
        .with_context(|| format!("connecting to {url}"))?;
    Ok(CompactTxStreamerClient::new(channel))
}

//! The real chain: the float wallet and the session vaults, built on
//! `zandglass-wallet` (the same vault code as the CLI).
//!
//! Every operation takes one mutex (the wallet-operation queue), so
//! concurrent sessions never build conflicting transactions from the float.
//! Before building any transaction the wallet is re-synced (testnet blocks
//! come every ~18 s and a stale tip makes the transaction expire), and a
//! rejection for expiry is retried once after another sync.
//!
//! Seeds: each session vault gets a fresh seed, written encrypted (age
//! passphrase from `ZANDGLASS_PLAYGROUND_SECRET`, via the wallet `Store`) to
//! `<data>/vaults/<session>/e<epoch>/seed.age`. The float seed is read from
//! its file once, kept in memory only (zeroized) and never logged.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use tokio::sync::Mutex;
use zandglass_wallet::{coded, Client, Network, Store, Vault};
use zeroize::Zeroizing;

use super::chain::{Chain, NewVault, NotSent, VaultBalance};

const UNKNOWN: u64 = u64::MAX;

/// Settings of the real chain.
pub struct RealConfig {
    pub network: Network,
    /// lightwalletd servers, tried in order on every connection.
    pub lwd_urls: Vec<String>,
    /// `ZANDGLASS_PLAYGROUND_DATA`.
    pub data_dir: PathBuf,
    /// `ZANDGLASS_PLAYGROUND_SECRET`: encrypts the session seeds.
    pub secret: Zeroizing<String>,
    pub float_seed_file: PathBuf,
    pub float_birthday: u32,
    /// `ZANDGLASS_PLAYGROUND_MIN_CONFIRMATIONS` (default 1): confirmations
    /// before funds count as spendable, for the float and every session vault.
    pub min_confirmations: u32,
}

struct Inner {
    store: Store,
    float: Option<Vault>,
    float_address: Option<String>,
}

/// The real chain implementation.
pub struct RealChain {
    cfg: RealConfig,
    inner: Mutex<Inner>,
    ready: AtomicBool,
    float_spendable: AtomicU64,
}

/// A rejection that a fresh sync and one more try can fix.
pub fn is_expiry_error(e: &anyhow::Error) -> bool {
    let text = format!("{e:#}").to_lowercase();
    text.contains("expir")
}

fn error_code(e: &anyhow::Error) -> Option<&'static str> {
    e.downcast_ref::<zandglass_wallet::CodedError>()
        .map(|c| c.code)
}

impl RealChain {
    /// Opens the seed store and starts restoring the float in the background.
    pub fn start(cfg: RealConfig) -> Result<Arc<Self>> {
        let root = cfg.data_dir.join("vaults-store");
        zandglass_wallet::fsutil::create_private_dir(&cfg.data_dir)
            .context("creating the playground data directory")?;
        let store = if root.join("keycheck.age").exists() {
            Store::open(&root, &cfg.secret).context("opening the playground seed store")?
        } else {
            Store::init(&root, &cfg.secret).context("creating the playground seed store")?
        };
        let chain = Arc::new(Self {
            cfg,
            inner: Mutex::new(Inner {
                store,
                float: None,
                float_address: None,
            }),
            ready: AtomicBool::new(false),
            float_spendable: AtomicU64::new(UNKNOWN),
        });
        let bg = chain.clone();
        tokio::spawn(async move {
            loop {
                match bg.init_float().await {
                    Ok(()) => break,
                    Err(e) => {
                        eprintln!("playground: float not ready yet: {e:#}");
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                }
            }
            // Keep the float's view of the chain fresh while idle.
            loop {
                tokio::time::sleep(Duration::from_secs(120)).await;
                if let Err(e) = bg.refresh_float().await {
                    eprintln!("playground: float sync failed: {e:#}");
                }
            }
        });
        Ok(chain)
    }

    async fn client(&self) -> Result<Client> {
        Ok(zandglass_wallet::connect_any(&self.cfg.lwd_urls).await?.0)
    }

    async fn init_float(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        if inner.float.is_some() {
            return Ok(());
        }
        let seed = Zeroizing::new(
            std::fs::read_to_string(&self.cfg.float_seed_file)
                .context("reading the float seed file")?
                .trim()
                .to_string(),
        );
        let dir = self.cfg.data_dir.join("float");
        let mut client = self.client().await?;
        let mut vault = if dir.join("wallet.sqlite").exists() {
            Vault::open(&dir, self.cfg.network, seed)?
        } else {
            Vault::init(
                &dir,
                self.cfg.network,
                &mut client,
                seed,
                self.cfg.float_birthday,
            )
            .await?
        }
        .with_min_confirmations(self.cfg.min_confirmations)?;
        vault.sync(&mut client).await?;
        let address = vault.address()?;
        self.float_spendable
            .store(vault.balances()?.spendable(), Ordering::Relaxed);
        inner.float_address = Some(address);
        inner.float = Some(vault);
        self.ready.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn refresh_float(&self) -> Result<()> {
        let mut inner = self.inner.lock().await;
        let Some(float) = inner.float.as_mut() else {
            return Ok(());
        };
        let mut client = self.client().await?;
        float.sync(&mut client).await?;
        self.float_spendable
            .store(float.balances()?.spendable(), Ordering::Relaxed);
        Ok(())
    }

    fn session_rel(session: &str) -> Result<String> {
        if session.len() != 32 || !session.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(anyhow!("bad session id"));
        }
        Ok(format!("vaults/{session}"))
    }

    fn vault_rel(session: &str, epoch: u32) -> Result<String> {
        Self::session_rel(session)?;
        Ok(Store::vault_rel_dir(session, epoch))
    }
}

#[async_trait]
impl Chain for RealChain {
    fn float_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    fn float_spendable(&self) -> Option<u64> {
        match self.float_spendable.load(Ordering::Relaxed) {
            UNKNOWN => None,
            v => Some(v),
        }
    }

    fn valid_address(&self, address: &str) -> bool {
        zcash_keys::address::Address::decode(&self.cfg.network, address).is_some()
    }

    fn is_shielded(&self, address: &str) -> bool {
        use zcash_keys::address::Address;
        match Address::decode(&self.cfg.network, address) {
            Some(Address::Sapling(_)) => true,
            Some(Address::Unified(ua)) => ua.has_sapling() || ua.has_orchard(),
            _ => false,
        }
    }

    async fn float_address(&self) -> Result<String> {
        let inner = self.inner.lock().await;
        inner
            .float_address
            .clone()
            .ok_or_else(|| anyhow!("the float is not ready"))
    }

    async fn create_vault(&self, session: &str, epoch: u32) -> Result<NewVault> {
        let rel = Self::vault_rel(session, epoch)?;
        let inner = self.inner.lock().await;
        // Never delete or reuse an epoch directory: a sweep to it may have
        // been broadcast without us learning of it, and the directory holds
        // the only copy of the seed that controls those funds.
        if inner.store.path(&rel).exists() {
            return Err(anyhow!("epoch {epoch} already has a vault"));
        }
        let mut client = self.client().await?;
        let phrase = Vault::new_seed();
        let birthday = Vault::new_birthday(&mut client).await?;
        inner.store.write_seed(&rel, &phrase)?;
        let mut vault = Vault::init(
            &inner.store.path(&rel),
            self.cfg.network,
            &mut client,
            phrase.clone(),
            birthday,
        )
        .await?;
        let address = vault.address()?;
        Ok(NewVault {
            seed: phrase,
            birthday,
            address,
        })
    }

    async fn next_free_epoch(&self, session: &str, from: u32) -> Result<u32> {
        let inner = self.inner.lock().await;
        let mut epoch = from;
        while inner.store.path(&Self::vault_rel(session, epoch)?).exists() {
            epoch += 1;
        }
        Ok(epoch)
    }

    async fn fund(&self, to: &str, amount_zat: u64) -> Result<String> {
        let mut inner = self.inner.lock().await;
        let float = inner
            .float
            .as_mut()
            .ok_or_else(|| anyhow!("the float is not ready"))
            .context(NotSent)?;
        // Everything before `send` cannot reach the network: mark it so the
        // worker may retry. Errors from `send` itself are classified by
        // `chain::definitely_unsent`.
        let mut client = self.client().await.context(NotSent)?;
        let mut attempt = 0;
        loop {
            float.sync(&mut client).await.context(NotSent)?;
            match float.send(&mut client, to, amount_zat).await {
                Ok(reports) => {
                    if let Ok(b) = float.balances() {
                        self.float_spendable.store(b.spendable(), Ordering::Relaxed);
                    }
                    return reports
                        .into_iter()
                        .next()
                        .map(|r| r.txid)
                        .ok_or_else(|| anyhow!("no transaction was created"));
                }
                Err(e) if attempt == 0 && is_expiry_error(&e) => attempt += 1,
                Err(e) => return Err(e),
            }
        }
    }

    async fn balance(&self, session: &str, epoch: u32) -> Result<VaultBalance> {
        let rel = Self::vault_rel(session, epoch)?;
        let inner = self.inner.lock().await;
        let phrase = inner.store.read_seed(&rel)?;
        let mut vault = Vault::open(&inner.store.path(&rel), self.cfg.network, phrase)?
            .with_min_confirmations(self.cfg.min_confirmations)?;
        let mut client = self.client().await?;
        vault.sync(&mut client).await?;
        let b = vault.balances()?;
        Ok(VaultBalance {
            total: b.total(),
            spendable: b.spendable(),
        })
    }

    async fn sweep(&self, session: &str, epoch: u32, to: &str) -> Result<String> {
        let rel = Self::vault_rel(session, epoch)?;
        let inner = self.inner.lock().await;
        let phrase = inner.store.read_seed(&rel).context(NotSent)?;
        let mut vault = Vault::open(&inner.store.path(&rel), self.cfg.network, phrase)
            .and_then(|v| v.with_min_confirmations(self.cfg.min_confirmations))
            .context(NotSent)?;
        let mut client = self.client().await.context(NotSent)?;
        let mut attempt = 0;
        loop {
            vault.sync(&mut client).await.context(NotSent)?;
            let b = vault.balances().context(NotSent)?;
            if b.total() == 0 {
                return Err(coded("vault_empty", "The vault holds no funds."));
            }
            if b.spendable() < b.total() {
                return Err(coded(
                    "insufficient_funds",
                    "The funds are not confirmed yet.",
                ));
            }
            match vault.sweep(&mut client, to).await {
                Ok(reports) => {
                    return reports
                        .into_iter()
                        .next()
                        .map(|r| r.txid)
                        .ok_or_else(|| anyhow!("no transaction was created"));
                }
                Err(e) if attempt == 0 && is_expiry_error(&e) => attempt += 1,
                Err(e) => return Err(e),
            }
        }
    }

    async fn epochs(&self, session: &str) -> Result<Vec<u32>> {
        let rel = Self::session_rel(session)?;
        let inner = self.inner.lock().await;
        let dir = inner.store.path(&rel);
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("listing {}", dir.display())),
        };
        let mut list = Vec::new();
        for entry in entries {
            let name = entry?.file_name();
            let name = name.to_string_lossy();
            // Anything that is not an epoch directory makes the list unknown,
            // so the caller archives instead of deleting.
            let epoch = name
                .strip_prefix('e')
                .and_then(|n| n.parse::<u32>().ok())
                .ok_or_else(|| anyhow!("unexpected entry {name} in a session directory"))?;
            list.push(epoch);
        }
        list.sort_unstable();
        Ok(list)
    }

    async fn forget(&self, session: &str) {
        let Ok(rel) = Self::session_rel(session) else {
            return;
        };
        let inner = self.inner.lock().await;
        let _ = std::fs::remove_dir_all(inner.store.path(&rel));
    }

    async fn archive(&self, session: &str) -> Result<()> {
        let rel = Self::session_rel(session)?;
        let inner = self.inner.lock().await;
        let src = inner.store.path(&rel);
        if !src.exists() {
            return Ok(());
        }
        let root = inner.store.root().join("archive");
        zandglass_wallet::fsutil::create_private_dir(&root)
            .context("creating the playground archive directory")?;
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let dst = root.join(format!("{session}-{at}"));
        std::fs::rename(&src, &dst).with_context(|| format!("archiving {}", src.display()))?;
        Ok(())
    }
}

/// Maps an error to its shared code, for the worker.
pub fn code_of(e: &anyhow::Error) -> Option<&'static str> {
    error_code(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_rejections_are_recognised() {
        assert!(is_expiry_error(&anyhow!(
            "broadcast rejected (-1): tx-expiring-soon"
        )));
        assert!(is_expiry_error(&anyhow!("transaction expired")));
        assert!(!is_expiry_error(&anyhow!("insufficient funds")));
        assert!(!is_expiry_error(&anyhow!(
            "broadcast rejected (-1): bad-txns-inputs-spent"
        )));
    }

    #[test]
    fn session_ids_are_checked_before_touching_the_disk() {
        assert!(RealChain::session_rel("../../etc").is_err());
        assert!(RealChain::session_rel(&"a".repeat(32)).is_ok());
        assert!(RealChain::vault_rel(&"a".repeat(31), 1).is_err());
    }
}

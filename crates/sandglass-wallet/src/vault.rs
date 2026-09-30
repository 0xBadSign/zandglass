//! A vault: one BIP 39 seed, account 0 only, backed by a SQLite wallet DB.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use bip0039::{Count, English, Mnemonic};
use rand::rngs::OsRng;
use rusqlite::Connection;
use secrecy::SecretVec;
use zcash_address::ZcashAddress;
use zcash_client_backend::data_api::wallet::input_selection::GreedyInputSelectorError;
use zcash_client_backend::{
    data_api::{
        wallet::{
            create_proposed_transactions, propose_send_max_transfer, ConfirmationsPolicy,
            SpendingKeys,
        },
        AccountBirthday, MaxSpendMode, WalletRead, WalletWrite,
    },
    proto::service::{BlockId, RawTransaction},
    sync,
    wallet::OvkPolicy,
};
use zcash_client_sqlite::{util::SystemClock, wallet::init::init_wallet_db, AccountUuid, WalletDb};
use zcash_keys::keys::{UnifiedAddressRequest, UnifiedSpendingKey};
use zcash_primitives::transaction::fees::{zip317, FeeRule};
use zcash_proofs::prover::LocalTxProver;
use zcash_protocol::{consensus::Network, ShieldedPool};
use zeroize::Zeroizing;

use crate::{blockcache::MemBlockCache, lightwalletd::Client};

/// The wallet database type used by vaults.
pub type Db = WalletDb<Connection, Network, SystemClock, OsRng>;

const DB_FILE: &str = "wallet.sqlite";
const SYNC_BATCH_SIZE: u32 = 1000;
/// Birthday offset below the chain tip for freshly created vaults.
pub const NEW_VAULT_BIRTHDAY_LOOKBACK: u32 = 100;

/// Balance of one shielded pool, in zatoshi.
#[derive(Debug, Clone, Copy)]
pub struct PoolBalance {
    pub spendable: u64,
    pub total: u64,
}

/// Per-pool balances of the vault account.
#[derive(Debug, Clone, Copy)]
pub struct Balances {
    pub sapling: PoolBalance,
    pub orchard: PoolBalance,
    pub ironwood: PoolBalance,
    pub chain_tip: u32,
    pub fully_scanned: u32,
}

impl Balances {
    /// Total spendable value across all shielded pools.
    pub fn spendable(&self) -> u64 {
        self.sapling.spendable + self.orchard.spendable + self.ironwood.spendable
    }

    /// Total value across all shielded pools, including unconfirmed funds.
    pub fn total(&self) -> u64 {
        self.sapling.total + self.orchard.total + self.ironwood.total
    }
}

/// What a sweep produced, for verifying that it looks like an ordinary transaction.
#[derive(Debug, Clone)]
pub struct SweepReport {
    pub txid: String,
    pub target_height: u32,
    pub anchor_height: u32,
    pub expiry_height: u32,
    pub fee: Option<u64>,
    pub raw_len: usize,
}

/// An open vault wallet.
pub struct Vault {
    dir: PathBuf,
    network: Network,
    db: Db,
    account: AccountUuid,
    seed: Zeroizing<String>,
}

fn open_db(dir: &Path, network: Network) -> Result<Db> {
    let mut db = WalletDb::for_path(dir.join(DB_FILE), network, SystemClock, OsRng)
        .context("opening wallet database")?;
    init_wallet_db(&mut db, None).map_err(|e| anyhow!("wallet migration failed: {e:?}"))?;
    Ok(db)
}

fn parse_phrase(phrase: &str) -> Result<Mnemonic<English>> {
    Mnemonic::<English>::from_phrase(phrase.trim()).map_err(|e| anyhow!("invalid seed phrase: {e}"))
}

fn seed_bytes(mnemonic: &Mnemonic<English>) -> SecretVec<u8> {
    SecretVec::new(mnemonic.to_seed("").to_vec())
}

async fn birthday_at(client: &mut Client, height: u32) -> Result<AccountBirthday> {
    let treestate = client
        .get_tree_state(BlockId {
            height: u64::from(height.saturating_sub(1)),
            hash: vec![],
        })
        .await
        .context("fetching birthday tree state")?
        .into_inner();
    AccountBirthday::from_treestate(treestate, None)
        .map_err(|e| anyhow!("invalid birthday tree state: {e:?}"))
}

impl Vault {
    /// Generates a fresh random 24-word seed.
    pub fn new_seed() -> Zeroizing<String> {
        Zeroizing::new(Mnemonic::<English>::generate(Count::Words24).into_phrase())
    }

    /// The chain tip height, for choosing a birthday.
    pub async fn chain_tip(client: &mut Client) -> Result<u32> {
        let tip = client
            .get_latest_block(zcash_client_backend::proto::service::ChainSpec::default())
            .await
            .context("fetching chain tip")?
            .into_inner()
            .height;
        u32::try_from(tip).context("chain tip out of range")
    }

    /// The birthday height for a vault created now.
    pub async fn new_birthday(client: &mut Client) -> Result<u32> {
        Ok(Self::chain_tip(client)
            .await?
            .saturating_sub(NEW_VAULT_BIRTHDAY_LOOKBACK)
            .max(1))
    }

    /// Creates the wallet database for `phrase` in `dir` (which must not hold a
    /// wallet yet). The seed itself is never written here: the caller keeps it
    /// (the `Store` encrypts it).
    pub async fn init(
        dir: &Path,
        network: Network,
        client: &mut Client,
        phrase: Zeroizing<String>,
        birthday_height: u32,
    ) -> Result<Self> {
        let mnemonic = parse_phrase(&phrase)?;
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        if dir.join(DB_FILE).exists() {
            bail!("{} already contains a wallet", dir.display());
        }
        let birthday = birthday_at(client, birthday_height).await?;
        let mut db = open_db(dir, network)?;
        let (account, _usk) = db
            .create_account("vault", &seed_bytes(&mnemonic), &birthday, None)
            .map_err(|e| anyhow!("creating account: {e:?}"))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            network,
            db,
            account,
            seed: phrase,
        })
    }

    /// Opens an existing vault directory with its seed phrase.
    pub fn open(dir: &Path, network: Network, phrase: Zeroizing<String>) -> Result<Self> {
        parse_phrase(&phrase)?;
        let db = open_db(dir, network)?;
        let account = db
            .get_account_ids()
            .map_err(|e| anyhow!("listing accounts: {e:?}"))?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("wallet has no account"))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            network,
            db,
            account,
            seed: phrase,
        })
    }

    /// The vault's seed phrase (for sealing into a kit).
    pub fn seed_phrase(&self) -> &str {
        &self.seed
    }

    /// The vault directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Returns the vault's shielded-only unified address (Orchard receiver, no
    /// transparent receiver) as a string.
    pub fn address(&mut self) -> Result<String> {
        let existing = self
            .db
            .get_last_generated_address_matching(self.account, UnifiedAddressRequest::ORCHARD)
            .map_err(|e| anyhow!("reading address: {e:?}"))?;
        let ua = match existing {
            Some(ua) => ua,
            None => {
                self.db
                    .get_next_available_address(self.account, UnifiedAddressRequest::ORCHARD)
                    .map_err(|e| anyhow!("deriving address: {e:?}"))?
                    .ok_or_else(|| anyhow!("no address available"))?
                    .0
            }
        };
        Ok(ua.encode(&self.network))
    }

    /// Builds a ZIP 321 payment request URI for this vault's address.
    pub fn zip321_uri(&mut self, amount_zat: Option<u64>) -> Result<String> {
        let addr: ZcashAddress = self
            .address()?
            .parse()
            .map_err(|e| anyhow!("address parse: {e:?}"))?;
        let amount = amount_zat
            .map(zcash_protocol::value::Zatoshis::from_u64)
            .transpose()
            .map_err(|e| anyhow!("amount: {e:?}"))?;
        let payment = zip321::Payment::new(addr, amount, None, None, None, vec![])
            .map_err(|e| anyhow!("payment: {e:?}"))?;
        let request = zip321::TransactionRequest::new(vec![payment])
            .map_err(|e| anyhow!("request: {e:?}"))?;
        Ok(request.to_uri())
    }

    /// Syncs the wallet with lightwalletd until it is up to date.
    pub async fn sync(&mut self, client: &mut Client) -> Result<()> {
        let cache = MemBlockCache::new();
        sync::run(client, &self.network, &cache, &mut self.db, SYNC_BATCH_SIZE)
            .await
            .map_err(|e| anyhow!("sync failed: {e}"))
    }

    /// Reads the per-pool balances (Sapling, Orchard, Ironwood).
    pub fn balances(&self) -> Result<Balances> {
        let summary = self
            .db
            .get_wallet_summary(ConfirmationsPolicy::default())
            .map_err(|e| anyhow!("reading balances: {e:?}"))?
            .ok_or_else(|| anyhow!("wallet has not been synced yet"))?;
        let acct = summary
            .account_balances()
            .get(&self.account)
            .ok_or_else(|| anyhow!("account missing from summary"))?;
        let pool = |b: &zcash_client_backend::data_api::Balance| PoolBalance {
            spendable: u64::from(b.spendable_value()),
            total: u64::from(b.total()),
        };
        Ok(Balances {
            sapling: pool(acct.sapling_balance()),
            orchard: pool(acct.orchard_balance()),
            ironwood: pool(acct.ironwood_balance()),
            chain_tip: u32::from(summary.chain_tip_height()),
            fully_scanned: u32::from(summary.fully_scanned_height()),
        })
    }

    /// Sends the whole spendable shielded balance (minus the ZIP 317 fee) to
    /// `to_address`, then broadcasts the transaction.
    ///
    /// Uses the standard flow: `propose_send_max_transfer`, then
    /// `create_proposed_transactions` with default expiry, default anchor,
    /// the ZIP 317 fee rule and `OvkPolicy::Discard`.
    pub async fn sweep(
        &mut self,
        client: &mut Client,
        to_address: &str,
    ) -> Result<Vec<SweepReport>> {
        // Validate the recipient against the vault's network.
        zcash_keys::address::Address::decode(&self.network, to_address)
            .ok_or_else(|| anyhow!("recipient is not a valid address for this network"))?;
        let recipient: ZcashAddress = to_address
            .parse()
            .map_err(|e| anyhow!("recipient parse: {e:?}"))?;

        let policy = ConfirmationsPolicy::default();
        let (target, anchor) = self
            .db
            .get_target_and_anchor_heights(policy.trusted())
            .map_err(|e| anyhow!("reading heights: {e:?}"))?
            .ok_or_else(|| anyhow!("wallet has not been synced yet"))?;
        let target_height = u32::from(target);
        let anchor_height = u32::from(anchor);

        let proposal = propose_send_max_transfer::<
            _,
            _,
            _,
            zcash_client_sqlite::wallet::commitment_tree::Error,
        >(
            &mut self.db,
            &self.network,
            self.account,
            &[
                ShieldedPool::Sapling,
                ShieldedPool::Orchard,
                ShieldedPool::Ironwood,
            ],
            &zip317::FeeRule::standard(),
            recipient,
            None,
            MaxSpendMode::MaxSpendable,
            policy,
            &zcash_client_backend::data_api::wallet::input_selection::LockedInputPolicy::Exclude,
            None,
        )
        .map_err(|e| anyhow!("proposal failed: {e:?}"))?;

        let seed = Zeroizing::new(parse_phrase(&self.seed)?.to_seed(""));
        let usk = UnifiedSpendingKey::from_seed(&self.network, &seed[..], zip32::AccountId::ZERO)
            .map_err(|e| anyhow!("deriving spending key: {e:?}"))?;
        let prover = LocalTxProver::bundled();

        let txids = create_proposed_transactions::<
            _,
            _,
            GreedyInputSelectorError,
            _,
            <zip317::FeeRule as FeeRule>::Error,
            _,
        >(
            &mut self.db,
            &self.network,
            &prover,
            &prover,
            &SpendingKeys::from_unified_spending_key(usk),
            OvkPolicy::Discard,
            &proposal,
            None,
        )
        .map_err(|e| anyhow!("building transaction failed: {e:?}"))?;

        let mut reports = Vec::new();
        for txid in txids {
            let tx = self
                .db
                .get_transaction(txid)
                .map_err(|e| anyhow!("reading transaction: {e:?}"))?
                .ok_or_else(|| anyhow!("created transaction missing from wallet"))?;
            let mut raw = Vec::new();
            tx.write(&mut raw).context("serializing transaction")?;
            let resp = client
                .send_transaction(RawTransaction {
                    data: raw.clone(),
                    height: 0,
                })
                .await
                .context("SendTransaction RPC")?
                .into_inner();
            if resp.error_code != 0 {
                bail!(
                    "broadcast rejected ({}): {}",
                    resp.error_code,
                    resp.error_message
                );
            }
            let fee = tx
                .fee_paid(|_| Ok::<_, zcash_protocol::value::BalanceError>(None))
                .ok()
                .flatten()
                .map(u64::from);
            reports.push(SweepReport {
                txid: txid.to_string(),
                target_height,
                anchor_height,
                expiry_height: u32::from(tx.expiry_height()),
                fee,
                raw_len: raw.len(),
            });
        }
        Ok(reports)
    }
}

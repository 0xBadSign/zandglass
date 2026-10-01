//! A vault: one BIP 39 seed, account 0 only, backed by a SQLite wallet DB.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use bip0039::{Count, English, Mnemonic};
use rand::{rngs::OsRng, Rng};
use rusqlite::Connection;
use secrecy::SecretVec;
use zcash_address::ZcashAddress;
use zcash_client_backend::data_api::wallet::input_selection::{
    GreedyInputSelector, GreedyInputSelectorError, SpendPolicy,
};
use zcash_client_backend::fees::{
    standard::SingleOutputChangeStrategy, DustOutputPolicy, StandardFeeRule,
};
use zcash_client_backend::{
    data_api::{
        wallet::{
            create_proposed_transactions, propose_send_max_transfer, propose_transfer,
            ConfirmationsPolicy, SpendingKeys,
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
/// Extra random lookback (0 to this many blocks) added to the birthday.
pub const NEW_VAULT_BIRTHDAY_JITTER: u32 = 200;

/// `tip - 100 - extra`, never below 1.
fn jittered_birthday(tip: u32, extra: u32) -> u32 {
    tip.saturating_sub(NEW_VAULT_BIRTHDAY_LOOKBACK + extra)
        .max(1)
}

/// The pool for change when the wallet's own flows do not decide it (the
/// library only consults this for purely transparent flows, which vaults never
/// have). From NU6.3 the Orchard turnstile forbids new value in the Orchard
/// pool, so ordinary wallets use Ironwood; before that, Orchard.
pub(crate) fn fallback_change_pool(network: &Network, target_height: u32) -> ShieldedPool {
    use zcash_protocol::consensus::{BlockHeight, NetworkUpgrade, Parameters};
    if network.is_nu_active(NetworkUpgrade::Nu6_3, BlockHeight::from_u32(target_height)) {
        ShieldedPool::Ironwood
    } else {
        ShieldedPool::Orchard
    }
}

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
    policy: ConfirmationsPolicy,
}

fn open_db(dir: &Path, network: Network) -> Result<Db> {
    let mut db = WalletDb::for_path(dir.join(DB_FILE), network, SystemClock, OsRng)
        .context("opening wallet database")?;
    init_wallet_db(&mut db, None).map_err(|e| anyhow!("wallet migration failed: {e:?}"))?;
    Ok(db)
}

/// A symmetrical policy needing `min_confirmations` for every note.
pub fn confirmations_policy(min_confirmations: u32) -> Result<ConfirmationsPolicy> {
    let n = std::num::NonZeroU32::new(min_confirmations)
        .ok_or_else(|| anyhow!("minimum confirmations must be at least 1"))?;
    Ok(ConfirmationsPolicy::new_symmetrical(n))
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

    /// The birthday height for a vault created now: `tip - 100 - random(0..=200)`.
    ///
    /// The jitter keeps the first block a vault's wallet asks the server for
    /// from being a fixed offset below the tip, so two vaults (or a vault and
    /// its next epoch) cannot be linked by an identical start height. The
    /// birthday stays below the tip, hence below the funding height.
    pub async fn new_birthday(client: &mut Client) -> Result<u32> {
        let tip = Self::chain_tip(client).await?;
        Ok(jittered_birthday(
            tip,
            OsRng.gen_range(0..=NEW_VAULT_BIRTHDAY_JITTER),
        ))
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
        crate::fsutil::create_private_dir(dir)?;
        if dir.join(DB_FILE).exists() {
            bail!("{} already contains a wallet", dir.display());
        }
        let birthday = birthday_at(client, birthday_height).await?;
        // The wallet file is created 0600 before SQLite opens it; its journal
        // files inherit that mode.
        crate::fsutil::touch_private(&dir.join(DB_FILE))?;
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
            policy: ConfirmationsPolicy::default(),
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
            policy: ConfirmationsPolicy::default(),
        })
    }

    /// Sets how many confirmations funds need before this vault treats them
    /// as spendable (balances, proposals, sweeps). Trusted and untrusted
    /// notes use the same number. Without a call the library default applies
    /// (3 trusted, 10 untrusted). Lower values make funds available sooner
    /// and are meant for short-lived demo vaults.
    pub fn set_min_confirmations(&mut self, min_confirmations: u32) -> Result<()> {
        self.policy = confirmations_policy(min_confirmations)?;
        Ok(())
    }

    /// Builder form of [`Vault::set_min_confirmations`].
    pub fn with_min_confirmations(mut self, min_confirmations: u32) -> Result<Self> {
        self.set_min_confirmations(min_confirmations)?;
        Ok(self)
    }

    /// The confirmation policy in effect.
    pub fn confirmations_policy(&self) -> ConfirmationsPolicy {
        self.policy
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
            .get_wallet_summary(self.policy)
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

        let policy = self.policy;
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

        self.broadcast(client, txids, target_height, anchor_height)
            .await
    }

    /// Sends one ordinary transaction that pays every `(address, zatoshi)`
    /// pair (the installment split). The change returns to this vault.
    /// Uses `propose_transfer` with the ZIP 317 fee and `OvkPolicy::Discard`.
    pub async fn send_many(
        &mut self,
        client: &mut Client,
        payments: &[(String, u64)],
    ) -> Result<Vec<SweepReport>> {
        let mut list = Vec::new();
        for (addr, zat) in payments {
            zcash_keys::address::Address::decode(&self.network, addr)
                .ok_or_else(|| anyhow!("recipient is not a valid address for this network"))?;
            let to: ZcashAddress = addr
                .parse()
                .map_err(|e| anyhow!("recipient parse: {e:?}"))?;
            let amount = zcash_protocol::value::Zatoshis::from_u64(*zat)
                .map_err(|e| anyhow!("amount: {e:?}"))?;
            list.push(
                zip321::Payment::new(to, Some(amount), None, None, None, vec![])
                    .map_err(|e| anyhow!("payment: {e:?}"))?,
            );
        }
        let request =
            zip321::TransactionRequest::new(list).map_err(|e| anyhow!("request: {e:?}"))?;

        let policy = self.policy;
        let (target, anchor) = self
            .db
            .get_target_and_anchor_heights(policy.trusted())
            .map_err(|e| anyhow!("reading heights: {e:?}"))?
            .ok_or_else(|| anyhow!("wallet has not been synced yet"))?;
        let selector = GreedyInputSelector::<Db>::new();
        let change = SingleOutputChangeStrategy::<Db>::new(
            StandardFeeRule::Zip317,
            None,
            fallback_change_pool(&self.network, u32::from(target)),
            DustOutputPolicy::default(),
        );
        let proposal =
            propose_transfer::<_, _, _, _, zcash_client_sqlite::wallet::commitment_tree::Error>(
                &mut self.db,
                &self.network,
                self.account,
                &selector,
                &change,
                request,
                policy,
                &SpendPolicy::default(),
                None,
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
            <StandardFeeRule as FeeRule>::Error,
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
        self.broadcast(client, txids, u32::from(target), u32::from(anchor))
            .await
    }

    async fn broadcast(
        &mut self,
        client: &mut Client,
        txids: impl IntoIterator<Item = zcash_primitives::transaction::TxId>,
        target_height: u32,
        anchor_height: u32,
    ) -> Result<Vec<SweepReport>> {
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

mod send;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn birthday_is_jittered_below_tip() {
        assert_eq!(jittered_birthday(10_000, 0), 9_900);
        assert_eq!(jittered_birthday(10_000, 200), 9_700);
        assert_eq!(jittered_birthday(50, 200), 1);
        for extra in 0..=NEW_VAULT_BIRTHDAY_JITTER {
            assert!(jittered_birthday(10_000, extra) < 10_000 - 99);
        }
    }

    #[test]
    fn confirmations_policy_is_symmetrical_and_nonzero() {
        assert!(confirmations_policy(0).is_err());
        let one = confirmations_policy(1).unwrap();
        assert_eq!(one.trusted().get(), 1);
        assert_eq!(one.untrusted().get(), 1);
        let five = confirmations_policy(5).unwrap();
        assert_eq!((five.trusted().get(), five.untrusted().get()), (5, 5));
        // The library default is stricter for untrusted notes.
        assert!(ConfirmationsPolicy::default().untrusted().get() > 1);
    }

    #[test]
    fn change_pool_follows_nu6_3() {
        assert_eq!(
            fallback_change_pool(&Network::MainNetwork, 3_000_000),
            ShieldedPool::Orchard
        );
        assert_eq!(
            fallback_change_pool(&Network::MainNetwork, 3_428_143),
            ShieldedPool::Ironwood
        );
    }
}

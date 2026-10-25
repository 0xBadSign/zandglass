//! The chain operations the playground needs, behind a trait so the state
//! machine and the API can be tested offline with [`MockChain`].

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use zandglass_wallet::coded;
use zeroize::Zeroizing;

/// Zatoshi the ZIP 317 fee of a one-input, one-output sweep costs.
pub const SWEEP_FEE_ZAT: u64 = 10_000;

/// Context marker for a chain error that happened before anything could
/// reach the network (the float was not ready, no server answered, the
/// wallet could not sync). Wrap such errors with `.context(NotSent)`.
#[derive(Debug, Clone, Copy)]
pub struct NotSent;

impl std::fmt::Display for NotSent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("nothing was sent")
    }
}

/// Whether a `fund` or `sweep` error proves that no transaction reached the
/// network: it is marked [`NotSent`], the proposal or the transaction could
/// not be built, or the node explicitly rejected the broadcast (the same
/// rule as the wallet's own flip code). Anything else, such as a timeout or a
/// dropped connection during `SendTransaction`, may have happened after the
/// node accepted the transaction, so the caller must not send again.
pub fn definitely_unsent(e: &anyhow::Error) -> bool {
    if e.downcast_ref::<NotSent>().is_some() {
        return true;
    }
    let msg = format!("{e:#}");
    msg.contains("proposal failed")
        || msg.contains("building transaction failed")
        || msg.contains("broadcast rejected")
}

/// A freshly created session vault.
pub struct NewVault {
    /// The vault's 24-word seed. The caller seals it into a kit and drops it.
    pub seed: Zeroizing<String>,
    /// Block height to restore from.
    pub birthday: u32,
    /// The vault's shielded address.
    pub address: String,
}

/// A vault's balance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VaultBalance {
    /// Everything received, including unconfirmed funds.
    pub total: u64,
    /// What can be spent now.
    pub spendable: u64,
}

/// Chain operations. Implementations serialise them: concurrent sessions
/// must never build conflicting transactions from the float.
#[async_trait]
pub trait Chain: Send + Sync {
    /// The float wallet is restored and synced.
    fn float_ready(&self) -> bool;
    /// The float's last known spendable balance, if known.
    fn float_spendable(&self) -> Option<u64>;
    /// Whether `address` is a valid address for the playground network.
    fn valid_address(&self, address: &str) -> bool;
    /// Whether a valid `address` can receive shielded funds (Sapling, or a
    /// unified address with a shielded receiver). A transparent or Sprout
    /// destination would publish the payout amount, so sends refuse it.
    fn is_shielded(&self, address: &str) -> bool {
        let _ = address;
        true
    }
    /// The float's own address (where returned funds go).
    async fn float_address(&self) -> Result<String>;
    /// Creates the wallet of one session epoch and stores its seed
    /// (encrypted) on the server. It refuses an epoch that already has a
    /// vault: a sweep to an earlier attempt may have reached the network
    /// without the caller learning of it, so such a vault is never deleted
    /// or reused (see [`Chain::next_free_epoch`]).
    async fn create_vault(&self, session: &str, epoch: u32) -> Result<NewVault>;
    /// The lowest epoch number `>= from` that has no vault yet. Every flip
    /// attempt uses a fresh epoch, so an earlier attempt's vault keeps
    /// whatever it received.
    async fn next_free_epoch(&self, session: &str, from: u32) -> Result<u32>;
    /// Sends `amount_zat` from the float to `to` with an ordinary transaction.
    async fn fund(&self, to: &str, amount_zat: u64) -> Result<String>;
    /// Syncs a session vault and reads its balance.
    async fn balance(&self, session: &str, epoch: u32) -> Result<VaultBalance>;
    /// Sweeps a session vault to `to`; returns the txid. Errors carry the
    /// shared codes `vault_empty` and `insufficient_funds`.
    async fn sweep(&self, session: &str, epoch: u32, to: &str) -> Result<String>;
    /// Every epoch of a session that has a vault on the server, ascending.
    /// An error means the list is not known (never "no vaults").
    async fn epochs(&self, session: &str) -> Result<Vec<u32>>;
    /// Deletes everything stored for a session (seeds and wallets). Only
    /// for sessions whose every epoch was just seen empty.
    async fn forget(&self, session: &str);
    /// Moves everything stored for a session to an archive directory on the
    /// server. Nothing is deleted: the seeds stay recoverable by the operator.
    async fn archive(&self, session: &str) -> Result<()>;
}

/// What a [`MockChain`] holds for one vault.
#[derive(Default)]
struct MockVault {
    total: u64,
    spendable: u64,
}

#[derive(Default)]
struct MockState {
    float_ready: bool,
    float_spendable: Option<u64>,
    /// New funds are spendable at once (default) or only after `confirm_all`.
    manual_confirm: bool,
    vaults: HashMap<(String, u32), MockVault>,
    addresses: HashMap<String, (String, u32)>,
    next_tx: u64,
    /// Calls in order, for assertions (`fund`, `sweep:<session>:<epoch>`, ...).
    calls: Vec<String>,
    /// Make the next `n` sweeps fail with this message.
    fail_sweeps: Vec<String>,
    lose_reply_sweeps: Vec<String>,
    fail_funds: Vec<String>,
    lose_reply_funds: Vec<String>,
    /// Every `balance` call fails while set.
    balance_fails: bool,
    forgotten: Vec<String>,
    archived: Vec<String>,
}

/// An offline chain for tests.
pub struct MockChain {
    state: Mutex<MockState>,
}

impl Default for MockChain {
    fn default() -> Self {
        Self::new()
    }
}

impl MockChain {
    /// A ready float with plenty of funds.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MockState {
                float_ready: true,
                ..MockState::default()
            }),
        }
    }

    fn s(&self) -> std::sync::MutexGuard<'_, MockState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Sets whether the float counts as ready.
    pub fn set_float_ready(&self, ready: bool) {
        self.s().float_ready = ready;
    }

    /// Sets the float's known spendable balance.
    pub fn set_float_spendable(&self, v: Option<u64>) {
        self.s().float_spendable = v;
    }

    /// New funds stay unconfirmed until [`MockChain::confirm_all`].
    pub fn set_manual_confirm(&self, manual: bool) {
        self.s().manual_confirm = manual;
    }

    /// Confirms every pending balance.
    pub fn confirm_all(&self) {
        for v in self.s().vaults.values_mut() {
            v.spendable = v.total;
        }
    }

    /// Makes the next sweep fail with `message`.
    pub fn fail_next_sweep(&self, message: &str) {
        self.s().fail_sweeps.push(message.to_string());
    }

    /// Makes the next sweep move the funds but fail with `message`, like a
    /// broadcast that the node accepted and whose reply was lost.
    pub fn lose_reply_of_next_sweep(&self, message: &str) {
        self.s().lose_reply_sweeps.push(message.to_string());
    }

    /// Makes the next funding fail with `message`.
    pub fn fail_next_fund(&self, message: &str) {
        self.s().fail_funds.push(message.to_string());
    }

    /// Makes the next funding move the funds but fail with `message`, like a
    /// broadcast that the node accepted and whose reply was lost.
    pub fn lose_reply_of_next_fund(&self, message: &str) {
        self.s().lose_reply_funds.push(message.to_string());
    }

    /// Makes every `balance` call fail (an unreachable server) while set.
    pub fn set_balance_fails(&self, fails: bool) {
        self.s().balance_fails = fails;
    }

    /// Empties a vault as if the heir moved the funds by hand.
    pub fn drain(&self, session: &str, epoch: u32) {
        if let Some(v) = self.s().vaults.get_mut(&(session.to_string(), epoch)) {
            v.total = 0;
            v.spendable = 0;
        }
    }

    /// The recorded calls.
    pub fn calls(&self) -> Vec<String> {
        self.s().calls.clone()
    }

    /// Sessions passed to `forget`.
    pub fn forgotten(&self) -> Vec<String> {
        self.s().forgotten.clone()
    }

    /// Sessions passed to `archive`.
    pub fn archived(&self) -> Vec<String> {
        self.s().archived.clone()
    }

    /// A vault's `(total, spendable)`.
    pub fn vault(&self, session: &str, epoch: u32) -> Option<(u64, u64)> {
        self.s()
            .vaults
            .get(&(session.to_string(), epoch))
            .map(|v| (v.total, v.spendable))
    }
}

fn fake_txid(n: u64) -> String {
    format!("{n:064x}")
}

#[async_trait]
impl Chain for MockChain {
    fn float_ready(&self) -> bool {
        self.s().float_ready
    }

    fn float_spendable(&self) -> Option<u64> {
        self.s().float_spendable
    }

    fn valid_address(&self, address: &str) -> bool {
        address.starts_with("mock-") && address.len() > 5
    }

    fn is_shielded(&self, address: &str) -> bool {
        !address.starts_with("mock-t")
    }

    async fn float_address(&self) -> Result<String> {
        Ok("mock-float".into())
    }

    async fn create_vault(&self, session: &str, epoch: u32) -> Result<NewVault> {
        let mut s = self.s();
        if s.vaults.contains_key(&(session.to_string(), epoch)) {
            return Err(anyhow!("epoch {epoch} already has a vault"));
        }
        let address = format!("mock-vault-{session}-{epoch}");
        s.addresses
            .insert(address.clone(), (session.to_string(), epoch));
        s.vaults
            .insert((session.to_string(), epoch), MockVault::default());
        s.calls.push(format!("create:{session}:{epoch}"));
        Ok(NewVault {
            seed: Zeroizing::new("mock seed words".to_string()),
            birthday: 1,
            address,
        })
    }

    async fn next_free_epoch(&self, session: &str, from: u32) -> Result<u32> {
        let s = self.s();
        let mut e = from;
        while s.vaults.contains_key(&(session.to_string(), e)) {
            e += 1;
        }
        Ok(e)
    }

    async fn fund(&self, to: &str, amount_zat: u64) -> Result<String> {
        let mut s = self.s();
        s.calls.push(format!("fund:{amount_zat}"));
        if !s.fail_funds.is_empty() {
            let msg = s.fail_funds.remove(0);
            return Err(anyhow!(msg));
        }
        let lose_reply = if s.lose_reply_funds.is_empty() {
            None
        } else {
            Some(s.lose_reply_funds.remove(0))
        };
        let key = s
            .addresses
            .get(to)
            .cloned()
            .ok_or_else(|| anyhow!("unknown address"))?;
        let manual = s.manual_confirm;
        let v = s.vaults.entry(key).or_default();
        v.total += amount_zat;
        if !manual {
            v.spendable = v.total;
        }
        s.next_tx += 1;
        if let Some(msg) = lose_reply {
            return Err(anyhow!(msg));
        }
        Ok(fake_txid(s.next_tx))
    }

    async fn balance(&self, session: &str, epoch: u32) -> Result<VaultBalance> {
        let s = self.s();
        if s.balance_fails {
            return Err(anyhow!("no lightwalletd server answered"));
        }
        let v = s
            .vaults
            .get(&(session.to_string(), epoch))
            .ok_or_else(|| anyhow!("no such vault"))?;
        Ok(VaultBalance {
            total: v.total,
            spendable: v.spendable,
        })
    }

    async fn sweep(&self, session: &str, epoch: u32, to: &str) -> Result<String> {
        let mut s = self.s();
        s.calls.push(format!("sweep:{session}:{epoch}:{to}"));
        let lose_reply = if s.lose_reply_sweeps.is_empty() {
            None
        } else {
            Some(s.lose_reply_sweeps.remove(0))
        };
        if !s.fail_sweeps.is_empty() {
            let msg = s.fail_sweeps.remove(0);
            return Err(anyhow!(msg));
        }
        let key = (session.to_string(), epoch);
        let (total, spendable) = {
            let v = s.vaults.get(&key).ok_or_else(|| anyhow!("no such vault"))?;
            (v.total, v.spendable)
        };
        if total == 0 {
            return Err(coded("vault_empty", "The vault holds no funds."));
        }
        if spendable < total {
            return Err(coded("insufficient_funds", "Funds are not confirmed yet."));
        }
        let moved = total.saturating_sub(SWEEP_FEE_ZAT);
        if let Some(v) = s.vaults.get_mut(&key) {
            v.total = 0;
            v.spendable = 0;
        }
        if let Some(dest) = s.addresses.get(to).cloned() {
            let manual = s.manual_confirm;
            let v = s.vaults.entry(dest).or_default();
            v.total += moved;
            if !manual {
                v.spendable = v.total;
            }
        }
        s.next_tx += 1;
        if let Some(msg) = lose_reply {
            // The node took the transaction but the reply never arrived.
            return Err(anyhow!(msg));
        }
        Ok(fake_txid(s.next_tx))
    }

    async fn epochs(&self, session: &str) -> Result<Vec<u32>> {
        let s = self.s();
        let mut list: Vec<u32> = s
            .vaults
            .keys()
            .filter(|(id, _)| id == session)
            .map(|(_, e)| *e)
            .collect();
        list.sort_unstable();
        Ok(list)
    }

    async fn forget(&self, session: &str) {
        self.s().forgotten.push(session.to_string());
    }

    async fn archive(&self, session: &str) -> Result<()> {
        self.s().archived.push(session.to_string());
        Ok(())
    }
}

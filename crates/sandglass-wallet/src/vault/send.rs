//! Sending a fixed amount from a vault (the playground float funds sessions
//! with it). It uses the same standard proposal flow as [`Vault::sweep`]:
//! ZIP 317 fee, default expiry and anchor, `OvkPolicy::Discard`.

use anyhow::{anyhow, bail, Context, Result};
use zcash_client_backend::{
    data_api::{
        wallet::{
            create_proposed_transactions, input_selection::GreedyInputSelectorError,
            propose_standard_transfer_to_address, ConfirmationsPolicy, SpendingKeys,
        },
        WalletRead,
    },
    fees::StandardFeeRule,
    proto::service::RawTransaction,
    wallet::OvkPolicy,
};
use zcash_keys::keys::UnifiedSpendingKey;
use zcash_primitives::transaction::fees::FeeRule;
use zcash_proofs::prover::LocalTxProver;
use zcash_protocol::value::Zatoshis;
use zeroize::Zeroizing;

use super::{parse_phrase, SweepReport, Vault};
use crate::lightwalletd::Client;

impl Vault {
    /// Sends exactly `amount_zat` (plus the ZIP 317 fee, taken from the
    /// vault) to `to_address` and broadcasts the transaction. The vault must
    /// be synced.
    pub async fn send(
        &mut self,
        client: &mut Client,
        to_address: &str,
        amount_zat: u64,
    ) -> Result<Vec<SweepReport>> {
        let to = zcash_keys::address::Address::decode(&self.network, to_address)
            .ok_or_else(|| anyhow!("recipient is not a valid address for this network"))?;
        let amount = Zatoshis::from_u64(amount_zat).map_err(|_| anyhow!("invalid amount"))?;

        let policy = ConfirmationsPolicy::default();
        let (target, anchor) = self
            .db
            .get_target_and_anchor_heights(policy.trusted())
            .map_err(|e| anyhow!("reading heights: {e:?}"))?
            .ok_or_else(|| anyhow!("wallet has not been synced yet"))?;
        let target_height = u32::from(target);
        let anchor_height = u32::from(anchor);

        let proposal = propose_standard_transfer_to_address::<
            _,
            _,
            zcash_client_sqlite::wallet::commitment_tree::Error,
        >(
            &mut self.db,
            &self.network,
            StandardFeeRule::Zip317,
            self.account,
            policy,
            &to,
            amount,
            None,
            None,
            super::fallback_change_pool(&self.network, target_height),
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

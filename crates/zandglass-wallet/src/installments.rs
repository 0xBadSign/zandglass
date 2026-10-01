//! Installment plans: a funding vault, one split transaction into N
//! installment vaults, and one sealed kit per installment (PLAN section 4.4).
//!
//! Each installment vault holds the installment amount plus the ZIP 317 fee of
//! the payee's one-input sweep, so the payee receives exactly the amount.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use zandglass_core::drand;
use zandglass_core::kit::Sequence;
use zandglass_core::mailbox::{normalize_base, ClaimLink, Created, MailboxClient};

use crate::error::coded;
use crate::ops::{
    self, check_cap, check_heirs, delete_mailbox_parts, make_kit, network_name, now, random_id,
    sync_and_require_funds, total, write_kit_file, CancelReport,
};
use crate::store::{
    EpochRecord, InstallmentRecord, InstallmentSet, Plan, PlanStatus, Store, StoreError,
};
use crate::{Client, Network, SweepReport, Vault};

/// ZIP 317 conventional fee of a one-input, one-output shielded sweep.
pub const PAYEE_FEE_ZAT: u64 = 10_000;
/// ZIP 317 marginal fee per logical action.
const MARGINAL_FEE_ZAT: u64 = 5_000;
/// The most installments one plan may hold.
pub const MAX_INSTALLMENTS: u32 = 120;
/// The smallest installment amount (zatoshi).
pub const MIN_AMOUNT_ZAT: u64 = 10_000;

/// Zatoshi the funding vault needs for `count` installments of `amount`:
/// every installment vault, plus an upper estimate of the split fee.
pub fn required_funding(amount_zat: u64, count: u32) -> u64 {
    let n = u64::from(count);
    n * (amount_zat + PAYEE_FEE_ZAT) + MARGINAL_FEE_ZAT * (n + 2)
}

/// Parses a start given as an RFC 3339 UTC time (`2026-12-01T09:00:00Z`) or
/// as a duration from the split (`10m`, `1d`). Returns `(start_at, start_after_secs)`.
pub fn parse_start(text: &str) -> Result<(Option<u64>, u64)> {
    let text = text.trim();
    if text.contains('T') || text.contains('-') {
        let at = parse_rfc3339(text)?;
        return Ok((Some(at), 0));
    }
    Ok((None, ops::parse_duration(text)?))
}

/// Parses `YYYY-MM-DDTHH:MM:SSZ` (UTC only) into unix seconds.
pub fn parse_rfc3339(text: &str) -> Result<u64> {
    let bad = || {
        anyhow!("`{text}` is not a start time; use 2026-12-01T09:00:00Z or a duration such as 10m")
    };
    let t = text.strip_suffix('Z').ok_or_else(bad)?;
    let (date, time) = t.split_once('T').ok_or_else(bad)?;
    let d: Vec<&str> = date.split('-').collect();
    let c: Vec<&str> = time.split(':').collect();
    if d.len() != 3 || c.len() != 3 {
        return Err(bad());
    }
    let num = |s: &str| s.parse::<i64>().map_err(|_| bad());
    let (y, m, day) = (num(d[0])?, num(d[1])?, num(d[2])?);
    let (h, mi, s) = (num(c[0])?, num(c[1])?, num(c[2])?);
    if !(1970..=9999).contains(&y)
        || !(1..=12).contains(&m)
        || !(1..=31).contains(&day)
        || !(0..24).contains(&h)
        || !(0..60).contains(&mi)
        || !(0..60).contains(&s)
    {
        return Err(bad());
    }
    // Days from civil (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + s;
    u64::try_from(secs).map_err(|_| bad())
}

/// Inputs for [`create_installments`].
pub struct NewInstallments {
    /// 1 to 3 payee codes (any one of them can claim each installment).
    pub payees: Vec<String>,
    /// Zatoshi the payee receives per installment.
    pub amount_zat: u64,
    /// Seconds between installments.
    pub every_secs: u64,
    /// Number of installments.
    pub count: u32,
    /// Absolute start (unix), if given.
    pub start_at: Option<u64>,
    /// Otherwise, the delay between the split and the first unlock.
    pub start_after_secs: u64,
    /// Optional message for the payee, sealed into every kit.
    pub message: Option<String>,
    /// Optional keeper URL; one mailbox per installment is created at the split.
    pub mailbox_url: Option<String>,
    /// Directory for the kit files and the claim-links file.
    pub out_dir: Option<String>,
}

/// What [`create_installments`] produced.
pub struct InstallmentsCreated {
    /// The stored plan.
    pub plan: Plan,
    /// The funding vault's address.
    pub address: String,
    /// The ZIP 321 request for the funding vault.
    pub uri: String,
    /// Zatoshi the request asks for.
    pub required: u64,
}

/// Checks the arguments of a new installment plan (no network, no store).
pub fn validate(
    payees: &[String],
    amount_zat: u64,
    count: u32,
    start_at: Option<u64>,
) -> Result<()> {
    check_heirs(payees)?;
    if count == 0 || count > MAX_INSTALLMENTS {
        return Err(anyhow!("--count must be between 1 and {MAX_INSTALLMENTS}"));
    }
    if amount_zat < MIN_AMOUNT_ZAT {
        return Err(anyhow!(
            "--amount must be at least {} ZEC",
            ops::zec(MIN_AMOUNT_ZAT)
        ));
    }
    if let Some(at) = start_at {
        if at <= now() {
            return Err(anyhow!("--start is in the past"));
        }
    }
    Ok(())
}

/// Creates an installment plan and its funding vault.
pub async fn create_installments(
    store: &Store,
    network: Network,
    client: &mut Client,
    new: NewInstallments,
    accept_unaudited: bool,
) -> Result<InstallmentsCreated> {
    validate(&new.payees, new.amount_zat, new.count, new.start_at)?;
    let required = required_funding(new.amount_zat, new.count);
    check_cap(network, required, accept_unaudited)?;
    let mailbox_url = match &new.mailbox_url {
        Some(u) => {
            let u = normalize_base(u);
            crate::ops::require_secure_url(&u, "mailbox")?;
            Some(u)
        }
        None => None,
    };

    let now = now();
    let id = random_id();
    let phrase = Vault::new_seed();
    let birthday = Vault::new_birthday(client).await?;
    let rel = Store::vault_rel_dir(&id, 1);
    store.write_seed(&rel, &phrase)?;
    let mut vault = Vault::init(&store.path(&rel), network, client, phrase, birthday).await?;
    let plan = Plan {
        id,
        kind: "installments".into(),
        network: network_name(network).into(),
        heirs: new.payees,
        period_secs: new.every_secs,
        message: new.message,
        epoch: 1,
        status: PlanStatus::Active,
        created: now,
        mailbox_url,
        mailbox_id: None,
        mailbox_write_token: None,
        epochs: vec![EpochRecord {
            epoch: 1,
            dir: rel,
            birthday_height: birthday,
            unlock_round: 0,
            unlock_time: 0,
            created: now,
        }],
        installments: Some(InstallmentSet {
            amount_zat: new.amount_zat,
            fee_zat: PAYEE_FEE_ZAT,
            every_secs: new.every_secs,
            count: new.count,
            start_at: new.start_at,
            start_after_secs: new.start_after_secs,
            out_dir: new.out_dir,
            split_txid: None,
            items: Vec::new(),
        }),
        pending: None,
    };
    store.put_plan(&plan)?;
    let address = vault.address()?;
    let uri = vault.zip321_uri(Some(required))?;
    Ok(InstallmentsCreated {
        plan,
        address,
        uri,
        required,
    })
}

/// One line of the claim-links file.
pub struct LinkLine {
    /// 1-based installment index.
    pub index: u32,
    /// Unlock time (unix).
    pub unlock_time: u64,
    /// The claim link, when the plan has a keeper mailbox.
    pub link: Option<String>,
    /// The kit file that was written.
    pub kit_file: String,
}

/// What [`split`] did.
pub struct SplitReport {
    /// The updated plan.
    pub plan: Plan,
    /// The split transaction.
    pub tx: SweepReport,
    /// Per-installment details.
    pub lines: Vec<LinkLine>,
    /// The claim-links file, when the plan has a keeper mailbox.
    pub links_path: Option<PathBuf>,
    /// Zatoshi left in the funding vault (change).
    pub leftover_hint: u64,
}

fn installments_of(plan: &Plan) -> Result<&InstallmentSet> {
    plan.installments
        .as_ref()
        .ok_or_else(|| anyhow!("plan {} is not an installment plan", plan.short_id()))
}

/// Splits the funded funding vault into the installment vaults with one
/// ordinary transaction, then seals one kit per installment.
pub async fn split(
    store: &Store,
    network: Network,
    client: &mut Client,
    plan_id: &str,
    kit_dir: &Path,
    accept_unaudited: bool,
) -> Result<SplitReport> {
    let mut plan = store.find_plan(Some(plan_id))?;
    if plan.kind != "installments" {
        return Err(anyhow!(
            "plan {} is not an installment plan",
            plan.short_id()
        ));
    }
    if plan.status != PlanStatus::Active {
        return Err(anyhow!("plan {} is cancelled", plan.short_id()));
    }
    let set = installments_of(&plan)?.clone();
    if set.split_txid.is_some() {
        return Err(anyhow!(
            "plan {} was already split (txid {})",
            plan.short_id(),
            set.split_txid.unwrap_or_default()
        ));
    }
    let n = set.count;
    let need = u64::from(n) * set.vault_zat();
    check_cap(network, need, accept_unaudited)?;

    let funding_rec = plan.current()?.clone();
    let mut funding = ops::open_vault(store, network, &funding_rec)?;
    funding.sync(client).await?;
    let b = funding.balances()?;
    if b.total() == 0 {
        return Err(coded(
            "vault_empty",
            "The funding vault holds no funds yet. Pay the request shown by `zandglass receive` first.",
        ));
    }
    if b.spendable() < need + MARGINAL_FEE_ZAT * 2 {
        return Err(coded(
            "insufficient_funds",
            format!(
                "The funding vault has {} ZEC confirmed ({} ZEC in total), but the split needs about {} ZEC. \
                 Fund the request shown by `zandglass receive` and wait for confirmations.",
                ops::zec(b.spendable()),
                ops::zec(b.total()),
                ops::zec(required_funding(set.amount_zat, n))
            ),
        ));
    }

    let t0 = now();
    let start = match set.start_at {
        Some(at) => at,
        None => t0 + set.start_after_secs,
    };
    if start <= t0 + 30 {
        return Err(coded(
            "unlock_passed",
            "The start time is already (almost) here. Pick a later --start, or cancel the plan and make a new one.",
        ));
    }

    // Prepare everything (seeds, vaults, kits, mailboxes) before funds move.
    let kit_names: Vec<String> = (1..=n)
        .map(|k| format!("{}-i{k}of{n}.zandglass", plan.short_id()))
        .collect();
    for name in &kit_names {
        if kit_dir.join(name).exists() {
            return Err(anyhow!("{} already exists", kit_dir.join(name).display()));
        }
    }
    let links_name = format!("{}-claim-links.txt", plan.short_id());
    if plan.mailbox_url.is_some() && kit_dir.join(&links_name).exists() {
        return Err(anyhow!(
            "{} already exists",
            kit_dir.join(&links_name).display()
        ));
    }

    let birthday = Vault::new_birthday(client).await?;
    let mut items: Vec<InstallmentRecord> = Vec::new();
    let mut addresses: Vec<(String, u64)> = Vec::new();
    let mut written: Vec<PathBuf> = Vec::new();
    let mut links: Vec<Option<String>> = Vec::new();

    // One batch request for all mailboxes: the keeper's per-address creation
    // limiter would refuse the 11th of many single creates.
    let mut pre_created: Vec<Created> = Vec::new();
    let prepared: Result<()> = async {
        if let Some(url) = &plan.mailbox_url {
            pre_created = MailboxClient::new(url).create_many(n as usize)?;
        }
        for k in 1..=n {
            let phrase = Vault::new_seed();
            let rel = Store::installment_rel_dir(&plan.id, k);
            store.write_seed(&rel, &phrase)?;
            let mut v = Vault::init(&store.path(&rel), network, client, phrase, birthday).await?;
            addresses.push((v.address()?, set.vault_zat()));
            let round = drand::unlock_round(start + u64::from(k - 1) * set.every_secs);
            let kit = make_kit(
                &plan,
                1,
                v.seed_phrase(),
                birthday,
                round,
                Some(Sequence { index: k, count: n }),
                t0,
            )?;
            let mut rec = InstallmentRecord {
                index: k,
                dir: rel,
                birthday_height: birthday,
                unlock_round: round,
                unlock_time: drand::round_time(round),
                kit_file: kit_names[(k - 1) as usize].clone(),
                mailbox_id: None,
                mailbox_write_token: None,
            };
            let path = write_kit_file(kit_dir, &rec.kit_file, &kit)?;
            written.push(path);
            let mut link = None;
            let mut put_result = Ok(());
            if let Some(url) = &plan.mailbox_url {
                let mb = MailboxClient::new(url);
                let created = pre_created
                    .get((k - 1) as usize)
                    .ok_or_else(|| anyhow!("the keeper returned too few mailboxes"))?;
                rec.mailbox_id = Some(created.id.clone());
                rec.mailbox_write_token = Some(created.write_token.clone());
                put_result = mb.put(&created.id, &created.write_token, &kit.to_json()?);
                link = Some(ClaimLink::new(url, &created).to_string());
            }
            // Record the mailbox before checking the upload so a failure deletes it.
            items.push(rec);
            put_result?;
            links.push(link);
        }
        Ok(())
    }
    .await;

    let cleanup = |written: &[PathBuf]| {
        for p in written {
            let _ = fs::remove_file(p);
        }
        for k in 1..=n {
            let _ = fs::remove_dir_all(store.path(&Store::installment_rel_dir(&plan.id, k)));
        }
        if let Some(url) = &plan.mailbox_url {
            // Every mailbox the batch made, including ones not yet recorded.
            for c in &pre_created {
                let _ = delete_mailbox_parts(url, &c.id, &c.write_token);
            }
        }
    };
    if let Err(e) = prepared {
        cleanup(&written);
        return Err(e);
    }

    // The one transaction. Re-sync first: testnet tips move every ~20 s.
    funding.sync(client).await?;
    let sent = funding.send_many(client, &addresses).await;
    let reports = match sent {
        Ok(r) => r,
        Err(e) => {
            let msg = format!("{e:#}");
            if crate::ops::definitely_unsent(&e) {
                cleanup(&written);
                return Err(e);
            }
            return Err(anyhow!(
                "the split transaction may or may not have been broadcast ({msg}). \
                 The installment vaults and kits were kept in {} and {}; \
                 do not run the split again until you have checked the funding vault with `zandglass sync`.",
                store.root().display(),
                kit_dir.display()
            ));
        }
    };
    let tx = reports
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("the split produced no transaction"))?;

    plan.installments = Some(InstallmentSet {
        split_txid: Some(tx.txid.clone()),
        items: items.clone(),
        ..set
    });
    store.put_plan(&plan)?;

    let lines: Vec<LinkLine> = items
        .iter()
        .zip(links.iter())
        .map(|(it, link)| LinkLine {
            index: it.index,
            unlock_time: it.unlock_time,
            link: link.clone(),
            kit_file: it.kit_file.clone(),
        })
        .collect();
    let links_path = if plan.mailbox_url.is_some() {
        let mut text = format!(
            "Claim links for installment plan {} ({} installments). Send this file to the payee.\n\
             Each link opens nothing on its own: the payee also needs the heir phrase and must wait for the unlock time.\n\n",
            plan.short_id(),
            n
        );
        for l in &lines {
            text.push_str(&format!(
                "Installment {} of {n}, unlocks {}:\n{}\n\n",
                l.index,
                zandglass_core::kit::format_time(l.unlock_time),
                l.link.as_deref().unwrap_or("")
            ));
        }
        let path = kit_dir.join(&links_name);
        crate::fsutil::write_private_atomic(&path, text.as_bytes())?;
        Some(path)
    } else {
        None
    };
    Ok(SplitReport {
        plan,
        tx,
        lines,
        links_path,
        leftover_hint: b.total().saturating_sub(need),
    })
}

/// The state of one installment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallmentState {
    /// Before the unlock time.
    Locked,
    /// Unlock time passed and the vault still holds the funds.
    Unlocked,
    /// The vault is empty: claimed, or swept back by a cancel.
    ClaimedOrEmpty,
    /// Not known: the vault has not been synced (use `status --sync`).
    Unknown,
}

/// A row of `status` for an installment plan.
pub struct InstallmentRow {
    /// 1-based index.
    pub index: u32,
    /// Unlock time (unix).
    pub unlock_time: u64,
    /// What the payee receives.
    pub amount_zat: u64,
    /// Current state.
    pub state: InstallmentState,
}

/// Reads (and, with a client, first syncs) every installment vault.
pub async fn installment_rows(
    store: &Store,
    network: Network,
    plan: &Plan,
    mut client: Option<&mut Client>,
) -> Result<Vec<InstallmentRow>> {
    let set = installments_of(plan)?;
    let t = now();
    let mut rows = Vec::new();
    for it in &set.items {
        let mut state = InstallmentState::Unknown;
        let phrase = store.read_seed(&it.dir)?;
        let mut vault = Vault::open(&store.path(&it.dir), network, phrase)?;
        if let Some(c) = client.as_mut() {
            vault.sync(c).await?;
        }
        if let Ok(b) = vault.balances() {
            state = if t < it.unlock_time {
                InstallmentState::Locked
            } else if b.total() == 0 {
                InstallmentState::ClaimedOrEmpty
            } else {
                InstallmentState::Unlocked
            };
        } else if t < it.unlock_time {
            state = InstallmentState::Locked;
        }
        rows.push(InstallmentRow {
            index: it.index,
            unlock_time: it.unlock_time,
            amount_zat: set.amount_zat,
            state,
        });
    }
    Ok(rows)
}

/// Sweeps one vault to `to`, returning `(reports, moved)`. An empty vault
/// yields nothing; amounts at or below the fee are left alone.
async fn sweep_if_any(
    vault: &mut Vault,
    client: &mut Client,
    to: &str,
) -> Result<(Vec<SweepReport>, u64)> {
    match sync_and_require_funds(vault, client).await {
        Ok(b) => {
            let t = total(&b);
            if t <= PAYEE_FEE_ZAT {
                return Ok((Vec::new(), 0));
            }
            // Re-sync right before the sweep: stale tips cause expiry rejections.
            vault.sync(client).await?;
            Ok((vault.sweep(client, to).await?, t))
        }
        Err(e)
            if e.downcast_ref::<crate::error::CodedError>()
                .is_some_and(|c| c.code == "vault_empty") =>
        {
            Ok((Vec::new(), 0))
        }
        Err(e) => Err(e),
    }
}

/// Cancels an installment plan: sweeps every remaining installment vault and
/// the funding vault back to `to`, then deletes the mailboxes. Safe to run
/// again after a partial failure: empty vaults are skipped.
pub async fn cancel_installments(
    store: &Store,
    network: Network,
    client: &mut Client,
    mut plan: Plan,
    to: &str,
) -> Result<CancelReport> {
    let set = installments_of(&plan)?.clone();
    let mut sweeps = Vec::new();
    let mut moved = 0u64;
    let mut already_claimed = Vec::new();
    for it in &set.items {
        let phrase = store.read_seed(&it.dir)?;
        let mut vault = Vault::open(&store.path(&it.dir), network, phrase)?;
        match sweep_if_any(&mut vault, client, to).await {
            Ok((s, m)) => {
                sweeps.extend(s);
                moved += m;
            }
            // The payee's claim is in the mempool but not mined yet, so this
            // wallet still sees the note as unspent: the network refuses the
            // second spend. Nothing is left to take back.
            Err(e) if format!("{e:#}").contains("double-spend") => already_claimed.push(it.index),
            Err(e) => return Err(anyhow!("installment {} of {}: {e:#}", it.index, set.count)),
        }
    }
    let funding_rec = plan.current()?.clone();
    let mut funding = ops::open_vault(store, network, &funding_rec)?;
    let (s, m) = sweep_if_any(&mut funding, client, to)
        .await
        .map_err(|e| anyhow!("funding vault: {e:#}"))?;
    sweeps.extend(s);
    moved += m;

    plan.status = PlanStatus::Cancelled;
    store.put_plan(&plan)?;
    let mut errors = Vec::new();
    if let Some(url) = &plan.mailbox_url {
        for it in &set.items {
            if let (Some(id), Some(tok)) = (&it.mailbox_id, &it.mailbox_write_token) {
                if let Err(e) = delete_mailbox_parts(url, id, tok) {
                    errors.push(format!("{e:#}"));
                }
            }
        }
    }
    Ok(CancelReport {
        plan,
        sweeps,
        moved,
        mailbox_error: if errors.is_empty() {
            None
        } else {
            Some(errors.join("; "))
        },
        already_claimed,
    })
}

/// Syncs the funding vault and, after the split, every installment vault.
/// Returns `(label, total, spendable, height)` per vault.
pub async fn sync_all(
    store: &Store,
    network: Network,
    client: &mut Client,
    plan: &Plan,
) -> Result<Vec<(String, u64, u64, u32)>> {
    let mut out = Vec::new();
    let mut v = ops::open_vault(store, network, plan.current()?)?;
    v.sync(client).await?;
    let b = v.balances()?;
    out.push((
        "funding".to_string(),
        b.total(),
        b.spendable(),
        b.fully_scanned,
    ));
    if let Some(set) = &plan.installments {
        for it in &set.items {
            let phrase = store.read_seed(&it.dir)?;
            let mut v = Vault::open(&store.path(&it.dir), network, phrase)?;
            v.sync(client).await?;
            let b = v.balances()?;
            out.push((
                format!("installment {}", it.index),
                b.total(),
                b.spendable(),
                b.fully_scanned,
            ));
        }
    }
    Ok(out)
}

/// A store lookup helper that keeps the error type in one place.
pub fn require_installment_plan(store: &Store, id: Option<&str>) -> Result<Plan> {
    let plan = store.find_plan(id)?;
    if plan.kind != "installments" {
        return Err(anyhow!(StoreError::PlanNotFound(format!(
            "{} (not an installment plan)",
            plan.short_id()
        ))));
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z").unwrap(), 0);
        assert_eq!(
            parse_rfc3339("2026-12-01T09:00:00Z").unwrap(),
            1_796_115_600
        );
        assert!(parse_rfc3339("2026-12-01 09:00:00").is_err());
        assert!(parse_rfc3339("2026-13-01T09:00:00Z").is_err());
    }

    #[test]
    fn start_forms() {
        assert_eq!(parse_start("10m").unwrap(), (None, 600));
        assert_eq!(
            parse_start("2026-12-01T09:00:00Z").unwrap(),
            (Some(1_796_115_600), 0)
        );
        assert!(parse_start("soon").is_err());
    }

    #[test]
    fn funding_covers_vaults_and_fee() {
        let r = required_funding(1_000_000, 3);
        assert!(r >= 3 * 1_010_000 + 3 * MARGINAL_FEE_ZAT);
    }
}

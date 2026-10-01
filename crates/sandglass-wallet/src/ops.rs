//! Plan operations that combine the store, vaults, kits and sweeps:
//! create a plan, flip, cancel and claim-by-sweep.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use rand::{rngs::OsRng, RngCore};
use sandglass_core::drand;
use sandglass_core::heir::HeirCode;
use sandglass_core::kit::{BuildParams, Kind, Payload, Sequence};
use sandglass_core::mailbox::{normalize_base, ClaimLink, MailboxClient};
use sandglass_core::Kit;
use zeroize::Zeroizing;

use crate::error::coded;
use crate::store::{EpochRecord, Plan, PlanStatus, Store};
use crate::{Balances, Client, Network, SweepReport, Vault};

/// Mainnet plans are capped at 0.25 ZEC until an audit (decision D-003).
pub const MAINNET_CAP_ZAT: u64 = 25_000_000;

/// `main` or `test`.
pub fn network_name(network: Network) -> &'static str {
    match network {
        Network::MainNetwork => "main",
        Network::TestNetwork => "test",
    }
}

/// The kit-format network for a wallet network.
pub fn kit_network(network: Network) -> sandglass_core::kit::Network {
    match network {
        Network::MainNetwork => sandglass_core::kit::Network::Main,
        Network::TestNetwork => sandglass_core::kit::Network::Test,
    }
}

/// The wallet network for a kit-format network.
pub fn wallet_network(network: sandglass_core::kit::Network) -> Network {
    match network {
        sandglass_core::kit::Network::Main => Network::MainNetwork,
        sandglass_core::kit::Network::Test => Network::TestNetwork,
    }
}

/// Formats zatoshi as ZEC with eight decimals.
pub fn zec(zat: u64) -> String {
    format!("{}.{:08}", zat / 100_000_000, zat % 100_000_000)
}

/// Parses a decimal ZEC amount such as `0.01` into zatoshi.
pub fn parse_zec(text: &str) -> Result<u64> {
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    if frac.len() > 8
        || whole.is_empty() && frac.is_empty()
        || !whole
            .chars()
            .chain(frac.chars())
            .all(|c| c.is_ascii_digit())
    {
        return Err(anyhow!("`{text}` is not a valid ZEC amount"));
    }
    let whole: u64 = if whole.is_empty() { 0 } else { whole.parse()? };
    let frac: u64 = format!("{frac:0<8}").parse()?;
    whole
        .checked_mul(100_000_000)
        .and_then(|w| w.checked_add(frac))
        .ok_or_else(|| anyhow!("amount too large"))
}

/// Parses `90d`, `10m`, `3m`, `2h`, `45s`, `1w` into seconds (at least 30 s).
pub fn parse_duration(text: &str) -> Result<u64> {
    let text = text.trim();
    let split = text.find(|c: char| !c.is_ascii_digit()).ok_or_else(|| {
        anyhow!("`{text}` needs a unit: s, m, h, d or w (for example 90d or 10m)")
    })?;
    let (num, unit) = text.split_at(split);
    let n: u64 = num
        .parse()
        .map_err(|_| anyhow!("`{text}` is not a duration (for example 90d or 10m)"))?;
    let mult = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 604_800,
        _ => {
            return Err(anyhow!(
                "unknown duration unit `{unit}`; use s, m, h, d or w"
            ))
        }
    };
    let secs = n
        .checked_mul(mult)
        .ok_or_else(|| anyhow!("duration too large"))?;
    if secs < 30 {
        return Err(anyhow!("the period must be at least 30 seconds"));
    }
    Ok(secs)
}

/// Formats a number of seconds like `2d 3h 4m 5s`.
pub fn format_span(secs: u64) -> String {
    let (d, h, m, s) = (
        secs / 86_400,
        secs % 86_400 / 3600,
        secs % 3600 / 60,
        secs % 60,
    );
    let mut parts = Vec::new();
    if d > 0 {
        parts.push(format!("{d}d"));
    }
    if d > 0 || h > 0 {
        parts.push(format!("{h}h"));
    }
    if d > 0 || h > 0 || m > 0 {
        parts.push(format!("{m}m"));
    }
    parts.push(format!("{s}s"));
    parts.join(" ")
}

/// Current unix time in seconds.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub(crate) fn total(b: &Balances) -> u64 {
    b.total()
}

/// Opens the vault of one epoch of a plan.
pub fn open_vault(store: &Store, network: Network, rec: &EpochRecord) -> Result<Vault> {
    let phrase = store.read_seed(&rec.dir)?;
    Vault::open(&store.path(&rec.dir), network, phrase)
}

/// Syncs a vault and refuses unless it holds confirmed, spendable funds.
pub(crate) async fn sync_and_require_funds(
    vault: &mut Vault,
    client: &mut Client,
) -> Result<Balances> {
    vault.sync(client).await?;
    let b = vault.balances()?;
    let all = total(&b);
    if all == 0 {
        return Err(coded(
            "vault_empty",
            "The vault holds no funds. It was already moved, claimed or never funded.",
        ));
    }
    if b.spendable() < all {
        return Err(coded(
            "insufficient_funds",
            format!(
                "The vault has {} ZEC, but only {} ZEC is confirmed so far. \
                 Wait for the incoming transaction to confirm and try again.",
                zec(all),
                zec(b.spendable())
            ),
        ));
    }
    Ok(b)
}

pub(crate) fn check_cap(network: Network, amount: u64, accept_unaudited: bool) -> Result<()> {
    if network == Network::MainNetwork && amount > MAINNET_CAP_ZAT && !accept_unaudited {
        return Err(coded(
            "amount_over_cap",
            format!(
                "The vault holds {} ZEC, above the {} ZEC cap for unaudited mainnet plans. \
                 Pass --accept-unaudited to go above it.",
                zec(amount),
                zec(MAINNET_CAP_ZAT)
            ),
        ));
    }
    Ok(())
}

pub(crate) fn random_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub(crate) fn write_kit_file(dir: &Path, name: &str, kit: &Kit) -> Result<PathBuf> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(name);
    if path.exists() {
        return Err(anyhow!("{} already exists", path.display()));
    }
    fs::write(&path, kit.to_json()?).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

pub(crate) fn make_kit(
    plan: &Plan,
    epoch: u32,
    phrase: &str,
    birthday: u32,
    unlock_round: u64,
    sequence: Option<Sequence>,
    now: u64,
) -> Result<Kit> {
    let heirs = plan
        .heirs
        .iter()
        .map(|h| HeirCode::parse(h))
        .collect::<Result<Vec<_>, _>>()?;
    let payload = Payload {
        vault_seed: phrase.to_string(),
        birthday_height: birthday,
        network: kit_network(match plan.network.as_str() {
            "main" => Network::MainNetwork,
            _ => Network::TestNetwork,
        }),
        plan: plan.id.clone(),
        epoch,
        message: plan.message.clone(),
        restore_hints: Payload::default_hints(birthday),
    };
    Ok(Kit::build(
        &payload,
        &BuildParams {
            kind: if sequence.is_some() {
                Kind::Installment
            } else {
                Kind::Legacy
            },
            sequence,
            now,
            unlock_round,
            heirs: &heirs,
        },
    )?)
}

/// Uploads `kit` to the plan's mailbox, replacing the previous kit. Does
/// nothing when the plan has no mailbox.
pub fn upload_kit(plan: &Plan, kit: &Kit) -> Result<()> {
    if let (Some(url), Some(id), Some(token)) = (
        &plan.mailbox_url,
        &plan.mailbox_id,
        &plan.mailbox_write_token,
    ) {
        MailboxClient::new(url).put(id, token, &kit.to_json()?)?;
    }
    Ok(())
}

/// Deletes one mailbox by its parts.
pub fn delete_mailbox_parts(url: &str, id: &str, write_token: &str) -> Result<()> {
    MailboxClient::new(url).delete(id, write_token)?;
    Ok(())
}

/// Deletes the plan's mailbox. Does nothing when the plan has none.
pub fn delete_mailbox(plan: &Plan) -> Result<()> {
    if let (Some(url), Some(id), Some(token)) = (
        &plan.mailbox_url,
        &plan.mailbox_id,
        &plan.mailbox_write_token,
    ) {
        MailboxClient::new(url).delete(id, token)?;
    }
    Ok(())
}

/// Checks that there are 1 to 3 valid, distinct heir (or payee) codes.
pub fn check_heirs(heirs: &[String]) -> Result<()> {
    let max = sandglass_core::seal::MAX_HEIRS;
    if heirs.is_empty() {
        return Err(anyhow!(
            "a plan needs at least one heir code (give --heir or --payee 1 to {max} times)"
        ));
    }
    if heirs.len() > max {
        return Err(anyhow!(
            "a plan supports at most {max} heirs, but {} were given",
            heirs.len()
        ));
    }
    for (i, h) in heirs.iter().enumerate() {
        HeirCode::parse(h).map_err(|e| anyhow!("heir code #{}: {e}", i + 1))?;
        if heirs[..i].contains(h) {
            return Err(anyhow!("heir code #{} is a duplicate", i + 1));
        }
    }
    Ok(())
}

/// Inputs for [`create_plan`].
pub struct NewPlan {
    /// 1 to 3 heir codes.
    pub heirs: Vec<String>,
    /// Seconds between flips.
    pub period_secs: u64,
    /// Optional message for the heirs.
    pub message: Option<String>,
    /// Optional keeper mailbox URL.
    pub mailbox_url: Option<String>,
    /// Optional funding amount for the ZIP 321 request, in zatoshi.
    pub amount: Option<u64>,
}

/// What [`create_plan`] produced.
pub struct PlanCreated {
    /// The stored plan.
    pub plan: Plan,
    /// The epoch 1 kit file.
    pub kit_path: PathBuf,
    /// The vault's shielded address.
    pub address: String,
    /// The ZIP 321 funding request.
    pub uri: String,
    /// The link to give to the heir, when the plan has a mailbox. The read
    /// token is not stored anywhere, so this is the only time it is shown.
    pub claim_link: Option<ClaimLink>,
}

/// Creates a legacy plan with its first vault (epoch 1) and kit.
pub async fn create_plan(
    store: &Store,
    network: Network,
    client: &mut Client,
    new: NewPlan,
    kit_dir: &Path,
    accept_unaudited: bool,
) -> Result<PlanCreated> {
    check_heirs(&new.heirs)?;
    if let Some(a) = new.amount {
        check_cap(network, a, accept_unaudited)?;
    }
    // Create the mailbox first so a failure leaves no key material behind.
    let mailbox_url = match &new.mailbox_url {
        Some(u) => {
            let u = normalize_base(u);
            if !(u.starts_with("http://") || u.starts_with("https://")) {
                return Err(anyhow!(
                    "the mailbox URL must start with http:// or https://"
                ));
            }
            Some(u)
        }
        None => None,
    };
    let mailbox = match &mailbox_url {
        Some(u) => Some(MailboxClient::new(u).create()?),
        None => None,
    };
    let now = now();
    let id = random_id();
    let phrase = Vault::new_seed();
    let birthday = Vault::new_birthday(client).await?;
    let rel = Store::vault_rel_dir(&id, 1);
    store.write_seed(&rel, &phrase)?;
    let mut vault = Vault::init(&store.path(&rel), network, client, phrase, birthday).await?;

    let unlock_round = drand::unlock_round(now + new.period_secs);
    let plan = Plan {
        id,
        kind: "legacy".into(),
        network: network_name(network).into(),
        heirs: new.heirs,
        period_secs: new.period_secs,
        message: new.message,
        epoch: 1,
        status: PlanStatus::Active,
        created: now,
        mailbox_url,
        mailbox_id: mailbox.as_ref().map(|m| m.id.clone()),
        mailbox_write_token: mailbox.as_ref().map(|m| m.write_token.clone()),
        epochs: vec![EpochRecord {
            epoch: 1,
            dir: rel,
            birthday_height: birthday,
            unlock_round,
            unlock_time: drand::round_time(unlock_round),
            created: now,
        }],
        installments: None,
    };
    let kit = make_kit(
        &plan,
        1,
        vault.seed_phrase(),
        birthday,
        unlock_round,
        None,
        now,
    )?;
    let kit_path = write_kit_file(kit_dir, &format!("{}-e1.sandglass", plan.short_id()), &kit)?;
    if let Err(e) = upload_kit(&plan, &kit) {
        // Nothing was funded: drop the unused vault, kit and mailbox.
        let _ = fs::remove_file(&kit_path);
        let _ = fs::remove_dir_all(store.path(&Store::vault_rel_dir(&plan.id, 1)));
        let _ = delete_mailbox(&plan);
        return Err(e);
    }
    store.put_plan(&plan)?;
    let address = vault.address()?;
    let uri = vault.zip321_uri(new.amount)?;
    let claim_link = match (&plan.mailbox_url, &mailbox) {
        (Some(u), Some(m)) => Some(ClaimLink::new(u, m)),
        _ => None,
    };
    Ok(PlanCreated {
        plan,
        kit_path,
        address,
        uri,
        claim_link,
    })
}

/// What [`flip`] did.
pub struct FlipReport {
    /// The updated plan.
    pub plan: Plan,
    /// The new kit file.
    pub kit_path: PathBuf,
    /// The sweep transactions.
    pub sweeps: Vec<SweepReport>,
    /// Zatoshi that were in the old vault.
    pub moved: u64,
    /// Set when the plan has a mailbox and the new kit could not be uploaded.
    pub mailbox_error: Option<String>,
}

/// The owner's check-in: sweep the current vault into a new one and seal a
/// new kit with a later unlock round.
pub async fn flip(
    store: &Store,
    network: Network,
    client: &mut Client,
    plan_id: &str,
    kit_dir: &Path,
    accept_unaudited: bool,
) -> Result<FlipReport> {
    let mut plan = store.find_plan(Some(plan_id))?;
    if plan.status != PlanStatus::Active {
        return Err(anyhow!("plan {} is cancelled", plan.short_id()));
    }
    if plan.kind != "legacy" {
        return Err(anyhow!(
            "plan {} is an installment plan; it has nothing to flip. Use `sandglass cancel` to stop it.",
            plan.short_id()
        ));
    }
    let old = plan.current()?.clone();
    let mut old_vault = open_vault(store, network, &old)?;
    let balances = sync_and_require_funds(&mut old_vault, client).await?;
    let moved = total(&balances);
    check_cap(network, moved, accept_unaudited)?;

    let epoch = plan.epoch + 1;
    let now = now();
    let phrase: Zeroizing<String> = Vault::new_seed();
    let birthday = Vault::new_birthday(client).await?;
    let rel = Store::vault_rel_dir(&plan.id, epoch);
    let unlock_round = drand::unlock_round(now + plan.period_secs);
    // Everything the heirs and the owner need is written before any funds move.
    store.write_seed(&rel, &phrase)?;
    let mut new_vault = Vault::init(&store.path(&rel), network, client, phrase, birthday).await?;
    let kit = make_kit(
        &plan,
        epoch,
        new_vault.seed_phrase(),
        birthday,
        unlock_round,
        None,
        now,
    )?;
    let kit_path = write_kit_file(
        kit_dir,
        &format!("{}-e{epoch}.sandglass", plan.short_id()),
        &kit,
    )?;
    let address = new_vault.address()?;
    drop(new_vault);

    let sweeps = match old_vault.sweep(client, &address).await {
        Ok(s) => s,
        Err(e) => {
            // Nothing moved: drop the unused epoch and its kit.
            let _ = fs::remove_file(&kit_path);
            let _ = fs::remove_dir_all(store.path(&rel));
            return Err(e);
        }
    };

    plan.epoch = epoch;
    plan.epochs.push(EpochRecord {
        epoch,
        dir: rel,
        birthday_height: birthday,
        unlock_round,
        unlock_time: drand::round_time(unlock_round),
        created: now,
    });
    store.put_plan(&plan)?;
    // The funds have moved, so a failed upload is reported, not fatal: the
    // mailbox still holds the old kit, which now opens an empty vault.
    let mailbox_error = match upload_kit(&plan, &kit) {
        Ok(()) => None,
        Err(e) => Some(format!("{e:#}")),
    };
    Ok(FlipReport {
        plan,
        kit_path,
        sweeps,
        moved,
        mailbox_error,
    })
}

/// What [`cancel`] did.
pub struct CancelReport {
    /// The updated plan.
    pub plan: Plan,
    /// The sweep transactions (empty if the vault held nothing).
    pub sweeps: Vec<SweepReport>,
    /// Zatoshi that were in the vault.
    pub moved: u64,
    /// Set when the plan has a mailbox and it could not be deleted.
    pub mailbox_error: Option<String>,
    /// Installments whose funds the payee had already spent, so there was
    /// nothing to take back (installment plans only).
    pub already_claimed: Vec<u32>,
}

/// Sweeps the current vault to `to_address` and marks the plan cancelled.
pub async fn cancel(
    store: &Store,
    network: Network,
    client: &mut Client,
    plan_id: &str,
    to_address: &str,
) -> Result<CancelReport> {
    let mut plan = store.find_plan(Some(plan_id))?;
    if plan.status == PlanStatus::Cancelled {
        return Err(anyhow!("plan {} is already cancelled", plan.short_id()));
    }
    zcash_keys::address::Address::decode(&network, to_address).ok_or_else(|| {
        coded(
            "network_mismatch",
            format!("That is not a valid {} address.", network_name(network)),
        )
    })?;
    if plan.kind == "installments" {
        return crate::installments::cancel_installments(store, network, client, plan, to_address)
            .await;
    }
    let rec = plan.current()?.clone();
    let mut vault = open_vault(store, network, &rec)?;
    let (sweeps, moved) = match sync_and_require_funds(&mut vault, client).await {
        Ok(b) => (vault.sweep(client, to_address).await?, total(&b)),
        Err(e)
            if e.downcast_ref::<crate::error::CodedError>()
                .is_some_and(|c| c.code == "vault_empty") =>
        {
            (Vec::new(), 0)
        }
        Err(e) => return Err(e),
    };
    plan.status = PlanStatus::Cancelled;
    store.put_plan(&plan)?;
    // Funds are already safe; a failed delete is reported, not fatal. The
    // keeper removes the mailbox by itself after 180 days.
    let mailbox_error = delete_mailbox(&plan).err().map(|e| format!("{e:#}"));
    Ok(CancelReport {
        plan,
        sweeps,
        moved,
        mailbox_error,
        already_claimed: Vec::new(),
    })
}

/// Restores a vault from its seed in a throwaway wallet directory, syncs and
/// sweeps everything to `to_address`. The seed is never written to disk.
pub async fn claim_sweep(
    network: Network,
    client: &mut Client,
    phrase: Zeroizing<String>,
    birthday: u32,
    to_address: &str,
    work_dir: &Path,
) -> Result<(Vec<SweepReport>, u64)> {
    zcash_keys::address::Address::decode(&network, to_address).ok_or_else(|| {
        coded(
            "network_mismatch",
            format!("That is not a valid {} address.", network_name(network)),
        )
    })?;
    let result = async {
        let mut vault = Vault::init(work_dir, network, client, phrase, birthday).await?;
        let b = sync_and_require_funds(&mut vault, client).await?;
        let sweeps = vault.sweep(client, to_address).await?;
        Ok((sweeps, total(&b)))
    }
    .await;
    let _ = fs::remove_dir_all(work_dir);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("90d").unwrap(), 90 * 86_400);
        assert_eq!(parse_duration("3m").unwrap(), 180);
        assert_eq!(parse_duration("1w").unwrap(), 604_800);
        assert!(parse_duration("3").is_err());
        assert!(parse_duration("m").is_err());
        assert!(parse_duration("5x").is_err());
        assert!(parse_duration("10s").is_err());
    }

    #[test]
    fn amounts() {
        assert_eq!(parse_zec("0.25").unwrap(), 25_000_000);
        assert_eq!(parse_zec("1").unwrap(), 100_000_000);
        assert_eq!(parse_zec(".5").unwrap(), 50_000_000);
        assert!(parse_zec("0.123456789").is_err());
        assert!(parse_zec("abc").is_err());
        assert!(parse_zec("").is_err());
        assert_eq!(zec(25_000_000), "0.25000000");
    }

    #[test]
    fn spans() {
        assert_eq!(format_span(5), "5s");
        assert_eq!(format_span(185), "3m 5s");
        assert_eq!(format_span(90_061), "1d 1h 1m 1s");
    }

    #[test]
    fn cap() {
        assert!(check_cap(Network::MainNetwork, MAINNET_CAP_ZAT, false).is_ok());
        let err = check_cap(Network::MainNetwork, MAINNET_CAP_ZAT + 1, false).unwrap_err();
        assert_eq!(
            err.downcast_ref::<crate::error::CodedError>().unwrap().code,
            "amount_over_cap"
        );
        assert!(check_cap(Network::MainNetwork, MAINNET_CAP_ZAT + 1, true).is_ok());
        assert!(check_cap(Network::TestNetwork, u64::MAX, false).is_ok());
    }
}

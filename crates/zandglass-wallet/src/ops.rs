//! Plan operations that combine the store, vaults, kits and sweeps:
//! create a plan, flip, cancel and claim-by-sweep.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use rand::{rngs::OsRng, RngCore};
use zandglass_core::drand;
use zandglass_core::heir::HeirCode;
use zandglass_core::kit::{BuildParams, Kind, Payload, Sequence};
use zandglass_core::mailbox::{normalize_base, ClaimLink, MailboxClient};
use zandglass_core::Kit;
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
pub fn kit_network(network: Network) -> zandglass_core::kit::Network {
    match network {
        Network::MainNetwork => zandglass_core::kit::Network::Main,
        Network::TestNetwork => zandglass_core::kit::Network::Test,
    }
}

/// The wallet network for a kit-format network.
pub fn wallet_network(network: zandglass_core::kit::Network) -> Network {
    match network {
        zandglass_core::kit::Network::Main => Network::MainNetwork,
        zandglass_core::kit::Network::Test => Network::TestNetwork,
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
    crate::fsutil::write_new_private(&path, kit.to_json()?.as_bytes())?;
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

/// Accepts `https://` URLs, and `http://` only for the local machine
/// (`localhost`, `127.0.0.1`, `[::1]`): kit uploads, claim links and
/// lightwalletd traffic must not cross the network in clear text.
pub fn require_secure_url(url: &str, what: &str) -> Result<()> {
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("https://") {
        return Ok(());
    }
    if let Some(rest) = lower.strip_prefix("http://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = if let Some(v6) = authority.strip_prefix('[') {
            v6.split(']').next().map(|h| format!("[{h}]"))
        } else {
            authority.rsplit('@').next().and_then(|a| {
                // `user@host` is not a local address in disguise.
                if authority.contains('@') {
                    None
                } else {
                    a.split(':').next().map(str::to_string)
                }
            })
        };
        if matches!(host.as_deref(), Some("localhost" | "127.0.0.1" | "[::1]")) {
            return Ok(());
        }
        return Err(anyhow!(
            "the {what} URL uses plain http://, which would send data unencrypted. Use https:// (http:// is only accepted for localhost)."
        ));
    }
    Err(anyhow!("the {what} URL must start with https://"))
}

/// Refuses a transparent destination unless `allow_transparent` is set:
/// sending there moves the funds out of the shielded pool and makes the
/// amount and the address public on chain.
pub fn check_destination(network: Network, address: &str, allow_transparent: bool) -> Result<()> {
    use zcash_keys::address::Address;
    match Address::decode(&network, address) {
        Some(Address::Sapling(_)) | Some(Address::Unified(_)) | None => Ok(()),
        Some(_) if allow_transparent => {
            eprintln!(
                "WARNING: {address} is a transparent address; the amount and the address will be public on chain."
            );
            Ok(())
        }
        Some(_) => Err(anyhow!(
            "{address} is a transparent address. Sending there makes the amount and the address public on chain. Use a shielded address, or pass --allow-transparent to do it anyway."
        )),
    }
}

/// Checks that there are 1 to 3 valid, distinct heir (or payee) codes.
pub fn check_heirs(heirs: &[String]) -> Result<()> {
    let max = zandglass_core::seal::MAX_HEIRS;
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
            require_secure_url(&u, "mailbox")?;
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
        pending: None,
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
    let kit_path = write_kit_file(kit_dir, &format!("{}-e1.zandglass", plan.short_id()), &kit)?;
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
    /// True when no new sweep happened: an earlier flip's pending epoch was
    /// found to hold the funds and was adopted.
    pub adopted: bool,
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
            "plan {} is an installment plan; it has nothing to flip. Use `zandglass cancel` to stop it.",
            plan.short_id()
        ));
    }
    // A previous flip may have swept without learning the outcome: settle
    // that epoch first, and never create a second one on top of it.
    match resolve_pending(store, network, client, &mut plan, Some(kit_dir)).await? {
        PendingOutcome::Adopted(report) => return Ok(*report),
        PendingOutcome::Waiting(msg) => return Err(anyhow!(msg)),
        PendingOutcome::None | PendingOutcome::Discarded => {}
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
    let record = EpochRecord {
        epoch,
        dir: rel.clone(),
        birthday_height: birthday,
        unlock_round,
        unlock_time: drand::round_time(unlock_round),
        created: now,
    };
    // Everything the heirs and the owner need is written before any funds
    // move, and the epoch is recorded as pending: from here on it is only
    // deleted when the sweep provably never reached the network.
    store.write_seed(&rel, &phrase)?;
    plan.pending = Some(record.clone());
    store.put_plan(&plan)?;
    let mut created_kit: Option<PathBuf> = None;
    let prepared: Result<(Kit, PathBuf, String)> = async {
        let mut new_vault =
            Vault::init(&store.path(&rel), network, client, phrase, birthday).await?;
        let kit = make_kit(
            &plan,
            epoch,
            new_vault.seed_phrase(),
            birthday,
            unlock_round,
            None,
            now,
        )?;
        let kit_path = write_kit_file(kit_dir, &kit_file_name(&plan, epoch), &kit)?;
        created_kit = Some(kit_path.clone());
        let address = new_vault.address()?;
        Ok((kit, kit_path, address))
    }
    .await;
    let (kit, kit_path, address) = match prepared {
        Ok(p) => p,
        Err(e) => {
            // Nothing was sent yet: drop the unused epoch.
            abandon_pending(store, &mut plan, created_kit.as_deref());
            return Err(e);
        }
    };

    let sweeps = match old_vault.sweep(client, &address).await {
        Ok(s) => s,
        Err(e) => return Err(settle_failed_sweep(store, &mut plan, Some(&kit_path), e)),
    };

    plan.epoch = epoch;
    plan.epochs.push(record);
    plan.pending = None;
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
        adopted: false,
    })
}

/// File name of a legacy plan's kit for one epoch.
pub fn kit_file_name(plan: &Plan, epoch: u32) -> String {
    format!("{}-e{epoch}.zandglass", plan.short_id())
}

/// Whether a sweep error proves that nothing reached the network: the
/// proposal or the transaction could not be built, or the node explicitly
/// rejected the broadcast. Anything else (a timeout or a dropped connection
/// during `SendTransaction`) may have happened after the node accepted the
/// transaction, so the funds may already be in the new vault.
pub(crate) fn definitely_unsent(e: &anyhow::Error) -> bool {
    let msg = format!("{e:#}");
    msg.contains("proposal failed")
        || msg.contains("building transaction failed")
        || msg.contains("broadcast rejected")
}

/// Drops a pending epoch that never received funds: its vault directory, its
/// kit file (only when this call created it) and the plan's pending record.
fn abandon_pending(store: &Store, plan: &mut Plan, kit_path: Option<&Path>) {
    if let Some(p) = plan.pending.take() {
        let _ = fs::remove_dir_all(store.path(&p.dir));
        if let Some(k) = kit_path {
            let _ = fs::remove_file(k);
        }
        let _ = store.put_plan(plan);
    }
}

/// Handles a failed sweep. A definite pre-broadcast failure cleans the new
/// epoch up and returns the original error. Any other failure keeps the new
/// epoch (recorded as pending in the plan) and says what to do.
pub(crate) fn settle_failed_sweep(
    store: &Store,
    plan: &mut Plan,
    kit_path: Option<&Path>,
    e: anyhow::Error,
) -> anyhow::Error {
    if definitely_unsent(&e) {
        abandon_pending(store, plan, kit_path);
        return e;
    }
    let epoch = plan.pending.as_ref().map_or(0, |p| p.epoch);
    anyhow!(
        "the sweep to the new vault (epoch {epoch}) may or may not have been broadcast ({e:#}). \
         The new vault and its kit were kept. Run `zandglass sync` or `zandglass status --sync` \
         to check: if the funds arrived, the new vault is adopted automatically, \
         and `zandglass flip` will not start another sweep until this one is settled."
    )
}

/// What [`resolve_pending`] found.
pub enum PendingOutcome {
    /// The plan has no pending epoch.
    None,
    /// The funds are in the pending epoch, which is now the current one.
    Adopted(Box<FlipReport>),
    /// The sweep never happened (the old vault is spendable again after the
    /// transaction expired); the pending epoch was removed.
    Discarded,
    /// The outcome is not known yet; the message says what to do.
    Waiting(String),
}

/// What to do with a pending epoch, given what the two vaults show.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PendingDecision {
    Adopt,
    Discard,
    Wait,
}

/// A pending sweep is only declared dead after the default transaction
/// expiry (40 blocks, under an hour) has certainly passed.
const PENDING_DEAD_AFTER_SECS: u64 = 2 * 3600;

pub(crate) fn decide_pending(
    new_total: u64,
    old_total: u64,
    old_spendable: u64,
    age_secs: u64,
) -> PendingDecision {
    if new_total > 0 {
        PendingDecision::Adopt
    } else if old_total > 0 && old_spendable >= old_total && age_secs > PENDING_DEAD_AFTER_SECS {
        PendingDecision::Discard
    } else {
        PendingDecision::Wait
    }
}

/// Settles a pending epoch left by a flip whose sweep outcome was unknown.
/// Funds in the pending vault adopt it (the kit is rebuilt and uploaded);
/// funds back in the old vault long after the attempt discard it; anything
/// else waits. With `kit_dir`, a missing kit file is rewritten there.
pub async fn resolve_pending(
    store: &Store,
    network: Network,
    client: &mut Client,
    plan: &mut Plan,
    kit_dir: Option<&Path>,
) -> Result<PendingOutcome> {
    let Some(pend) = plan.pending.clone() else {
        return Ok(PendingOutcome::None);
    };
    let mut new_vault = open_vault(store, network, &pend)?;
    new_vault.sync(client).await?;
    let nb = new_vault.balances()?;
    let new_total = total(&nb);
    let (old_total, old_spendable) = if new_total == 0 {
        let mut old_vault = open_vault(store, network, plan.current()?)?;
        old_vault.sync(client).await?;
        let ob = old_vault.balances()?;
        (total(&ob), ob.spendable())
    } else {
        (0, 0)
    };
    let age = now().saturating_sub(pend.created);
    match decide_pending(new_total, old_total, old_spendable, age) {
        PendingDecision::Adopt => {
            plan.epoch = pend.epoch;
            plan.epochs.push(pend.clone());
            plan.pending = None;
            store.put_plan(plan)?;
            let kit = make_kit(
                plan,
                pend.epoch,
                new_vault.seed_phrase(),
                pend.birthday_height,
                pend.unlock_round,
                None,
                pend.created,
            )?;
            let name = kit_file_name(plan, pend.epoch);
            let dir = kit_dir.unwrap_or_else(|| Path::new("."));
            let kit_path = dir.join(&name);
            if kit_dir.is_some() && !kit_path.exists() {
                write_kit_file(dir, &name, &kit)?;
            }
            let mailbox_error = match upload_kit(plan, &kit) {
                Ok(()) => None,
                Err(e) => Some(format!("{e:#}")),
            };
            Ok(PendingOutcome::Adopted(Box::new(FlipReport {
                plan: plan.clone(),
                kit_path,
                sweeps: Vec::new(),
                moved: new_total,
                mailbox_error,
                adopted: true,
            })))
        }
        PendingDecision::Discard => {
            let kit_path = kit_dir.map(|d| d.join(kit_file_name(plan, pend.epoch)));
            abandon_pending(store, plan, kit_path.as_deref());
            Ok(PendingOutcome::Discarded)
        }
        PendingDecision::Wait => Ok(PendingOutcome::Waiting(format!(
            "A previous flip (epoch {}) is not settled: neither vault shows the funds as moved yet. \
             Wait a few blocks, then run `zandglass sync`; do not flip again until it is settled.",
            pend.epoch
        ))),
    }
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

/// How long after a flip an empty-looking new vault may still be waiting for
/// the flip transaction to be mined (its expiry is 40 blocks, about 50 minutes
/// on mainnet; doubled for slow blocks).
pub const FLIP_SETTLE_SECS: u64 = 2 * 3600;

/// Whether the current epoch was created by a flip so recently that its
/// funds may still be unconfirmed (and so invisible to a light client).
pub fn flip_may_be_unconfirmed(plan: &Plan, rec: &EpochRecord, now: u64) -> bool {
    plan.epoch > 1 && now.saturating_sub(rec.created) < FLIP_SETTLE_SECS
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
    // Funds may sit in an epoch whose flip outcome was never learned.
    if let PendingOutcome::Waiting(msg) =
        resolve_pending(store, network, client, &mut plan, None).await?
    {
        return Err(anyhow!(msg));
    }
    let rec = plan.current()?.clone();
    let mut vault = open_vault(store, network, &rec)?;
    let (sweeps, moved) = match sync_and_require_funds(&mut vault, client).await {
        Ok(b) => (vault.sweep(client, to_address).await?, total(&b)),
        Err(e)
            if e.downcast_ref::<crate::error::CodedError>()
                .is_some_and(|c| c.code == "vault_empty") =>
        {
            // A light client does not see unmined transactions: right after
            // a flip the new vault looks empty although the funds are on
            // their way. Cancelling then would strand them in a cancelled plan.
            if flip_may_be_unconfirmed(&plan, &rec, now()) {
                return Err(coded(
                    "insufficient_funds",
                    "The last flip moved the funds into this vault, but that transaction is not confirmed yet. Wait a few minutes, then cancel again.",
                ));
            }
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

    const ID: &str = "0123456789abcdef0123456789abcdef";

    /// A store with a plan whose epoch 2 is pending, plus the files a flip
    /// writes before it sweeps.
    fn pending_fixture() -> (tempfile::TempDir, Store, Plan, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init_with(&dir.path().join("s"), "x", 10).unwrap();
        let rec = |epoch| EpochRecord {
            epoch,
            dir: Store::vault_rel_dir(ID, epoch),
            birthday_height: 5,
            unlock_round: 9,
            unlock_time: 10,
            created: 1,
        };
        let plan = Plan {
            id: ID.into(),
            kind: "legacy".into(),
            network: "test".into(),
            heirs: vec![],
            period_secs: 180,
            message: None,
            epoch: 1,
            status: PlanStatus::Active,
            created: 1,
            mailbox_url: None,
            mailbox_id: None,
            mailbox_write_token: None,
            epochs: vec![rec(1)],
            installments: None,
            pending: Some(rec(2)),
        };
        store.put_plan(&plan).unwrap();
        store.write_seed(&rec(2).dir, "seed words").unwrap();
        let kit = dir.path().join("kit-e2.zandglass");
        fs::write(&kit, "{}").unwrap();
        (dir, store, plan, kit)
    }

    #[test]
    fn a_fresh_flip_blocks_cancel_of_an_empty_looking_vault() {
        let (_d, _store, mut plan, _) = pending_fixture();
        let rec = EpochRecord {
            epoch: 2,
            dir: "v".into(),
            birthday_height: 1,
            unlock_round: 1,
            unlock_time: 0,
            created: 1_000_000,
        };
        plan.epoch = 2;
        assert!(flip_may_be_unconfirmed(&plan, &rec, 1_000_000 + 60));
        assert!(!flip_may_be_unconfirmed(
            &plan,
            &rec,
            1_000_000 + FLIP_SETTLE_SECS
        ));
        // A first epoch was funded from outside, not by a flip.
        plan.epoch = 1;
        assert!(!flip_may_be_unconfirmed(&plan, &rec, 1_000_000 + 60));
    }

    #[test]
    fn plain_http_is_only_for_localhost() {
        for ok in [
            "https://keeper.example",
            "HTTPS://keeper.example/x",
            "http://localhost:8787",
            "http://127.0.0.1:8787/",
            "http://[::1]:8787",
        ] {
            assert!(require_secure_url(ok, "keeper").is_ok(), "{ok}");
        }
        for bad in [
            "http://keeper.example",
            "http://127.0.0.1.evil.example",
            "http://localhost.evil.example",
            "http://localhost@evil.example",
            "http://10.0.0.5:8787",
            "ftp://x",
            "keeper.example",
        ] {
            assert!(require_secure_url(bad, "keeper").is_err(), "{bad}");
        }
    }

    #[test]
    fn transparent_destinations_need_a_flag() {
        let net = Network::MainNetwork;
        use zcash_address::ToAddress;
        let t = zcash_address::ZcashAddress::from_transparent_p2pkh(
            zcash_protocol::consensus::NetworkType::Main,
            [7; 20],
        )
        .encode();
        assert!(check_destination(net, &t, false).is_err());
        assert!(check_destination(net, &t, true).is_ok());
        // Anything that does not decode is left to the later validation.
        assert!(check_destination(net, "not an address", false).is_ok());
    }

    #[test]
    fn only_definite_errors_are_unsent() {
        assert!(definitely_unsent(&anyhow!("proposal failed: x")));
        assert!(definitely_unsent(&anyhow!(
            "building transaction failed: x"
        )));
        assert!(definitely_unsent(&anyhow!("broadcast rejected (-1): bad")));
        // Ambiguous: the node may have accepted the transaction.
        let timeout = anyhow!("deadline has elapsed").context("SendTransaction RPC");
        assert!(!definitely_unsent(&timeout));
        assert!(!definitely_unsent(&anyhow!("connection reset by peer")));
        assert!(!definitely_unsent(&anyhow!("reading transaction: x")));
    }

    #[test]
    fn rejected_sweep_cleans_up_the_new_epoch() {
        let (_d, store, mut plan, kit) = pending_fixture();
        let seed_dir = store.path(&Store::vault_rel_dir(ID, 2));
        let e = settle_failed_sweep(
            &store,
            &mut plan,
            Some(&kit),
            anyhow!("broadcast rejected (-26): bad-txns"),
        );
        assert!(format!("{e:#}").contains("broadcast rejected"));
        assert!(!seed_dir.exists());
        assert!(!kit.exists());
        assert!(plan.pending.is_none());
        assert!(store.find_plan(Some(ID)).unwrap().pending.is_none());
    }

    #[test]
    fn ambiguous_sweep_keeps_the_new_epoch_pending() {
        let (_d, store, mut plan, kit) = pending_fixture();
        let seed_dir = store.path(&Store::vault_rel_dir(ID, 2));
        let e = settle_failed_sweep(
            &store,
            &mut plan,
            Some(&kit),
            anyhow!("deadline has elapsed").context("SendTransaction RPC"),
        );
        let msg = format!("{e:#}");
        assert!(msg.contains("may or may not"), "{msg}");
        assert!(msg.contains("zandglass sync"), "{msg}");
        assert!(seed_dir.join("seed.age").exists());
        assert!(kit.exists());
        assert_eq!(plan.pending.as_ref().unwrap().epoch, 2);
        // The pending record survives on disk for the next flip or status.
        assert_eq!(store.find_plan(Some(ID)).unwrap().pending.unwrap().epoch, 2);
    }

    #[test]
    fn pending_epoch_decisions() {
        use PendingDecision::*;
        // Funds in the new vault: adopt, whatever the old one shows.
        assert_eq!(decide_pending(5, 0, 0, 0), Adopt);
        // Old vault spendable again, but too soon to call the tx dead.
        assert_eq!(decide_pending(0, 5, 5, 60), Wait);
        assert_eq!(
            decide_pending(0, 5, 5, PENDING_DEAD_AFTER_SECS + 1),
            Discard
        );
        // Old vault locked by a pending spend, or empty: keep waiting.
        assert_eq!(decide_pending(0, 5, 0, 99_999), Wait);
        assert_eq!(decide_pending(0, 0, 0, 99_999), Wait);
    }

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

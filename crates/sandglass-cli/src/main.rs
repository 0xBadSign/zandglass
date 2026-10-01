//! `sandglass`: time-locked inheritance for shielded Zcash.
//!
//! Commands are documented in `docs/PLAN.md` (T1.4). Every failure prints
//! `error[<code>]: <message>` using the shared codes from PLAN section 5.10.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use sandglass_core::heir::{generate_phrase, identity_from_phrase};
use sandglass_core::kit::{self, Kit};
use sandglass_core::mailbox::{ClaimLink, MailboxClient, MailboxError};
use sandglass_core::KitError;
use sandglass_wallet::installments;
use sandglass_wallet::ops::{self, format_span, network_name, now, zec};
use sandglass_wallet::{
    coded, connect, default_url, Client, CodedError, Network, PlanStatus, Store, StoreError,
};
use zeroize::Zeroizing;

#[derive(Clone, Copy, ValueEnum)]
enum Net {
    Main,
    Test,
}

#[derive(Parser)]
#[command(
    name = "sandglass",
    version,
    about = "Time-locked inheritance for shielded Zcash"
)]
struct Cli {
    /// Zcash network (mainnet is the default).
    #[arg(long, global = true, value_enum)]
    network: Option<Net>,
    /// Base data directory (default: ~/.sandglass). The network name is appended.
    #[arg(long, global = true, env = "SANDGLASS_HOME")]
    data_dir: Option<PathBuf>,
    /// lightwalletd server URL (default: a public server for the network).
    #[arg(long, global = true)]
    server: Option<String>,
    /// Allow plans above the 0.25 ZEC mainnet cap (the software is unaudited).
    #[arg(long, global = true)]
    accept_unaudited: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Heir key tools.
    #[command(subcommand)]
    Heir(HeirCmd),
    /// Create the encrypted store (asks for a passphrase).
    Init,
    /// Create plans.
    #[command(subcommand)]
    Plan(PlanCmd),
    /// Show the funding address and ZIP 321 request of a plan's current vault.
    Receive {
        /// Plan id (prefix). Optional when only one plan is active.
        plan: Option<String>,
    },
    /// Sync the current vault of every active plan.
    Sync,
    /// Show balances, unlock time and time left for every plan.
    Status {
        /// Sync before showing balances.
        #[arg(long)]
        sync: bool,
    },
    /// Check in: move the funds to a new vault and seal a new kit.
    Flip {
        /// Plan id (prefix). Optional when only one plan is active.
        plan: Option<String>,
        /// Directory for the new kit file.
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
    /// Sweep a plan's funds to an address and cancel the plan.
    Cancel {
        /// Plan id (prefix).
        plan: String,
        /// Address that receives the funds.
        #[arg(long)]
        to: String,
    },
    /// Heir or payee side: open unlocked kits and move the funds.
    Claim {
        /// `*.sandglass` kit files, or claim links the owner sent you
        /// (`https://.../claim.html#m=...&r=...`). Several are fine (installments).
        kits: Vec<String>,
        /// A text file with claim links or kit paths (the owner's
        /// `<plan>-claim-links.txt` works as it is).
        #[arg(long)]
        links_file: Option<PathBuf>,
        /// File holding the heir's 24-word phrase.
        #[arg(long)]
        phrase_file: PathBuf,
        /// Sweep the vault to this address. Without it the seed is printed.
        #[arg(long)]
        to: Option<String>,
        /// Wait until the kit unlocks instead of refusing.
        #[arg(long)]
        wait: bool,
        /// Wait for every kit in unlock order and sweep each one when it
        /// opens. A kit that turns out empty (a cancelled plan) is reported
        /// and skipped. Needs --to.
        #[arg(long)]
        watch: bool,
    },
    /// Installment plans.
    #[command(subcommand)]
    Installments(InstallmentsCmd),
    /// Kit tools.
    #[command(subcommand)]
    Kit(KitCmd),
}

#[derive(Subcommand)]
enum HeirCmd {
    /// Generate a heir key: prints the 24-word phrase once and the heir code.
    New {
        /// Also write the phrase to this file (owner-only permissions).
        #[arg(long)]
        save: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum InstallmentsCmd {
    /// Send the funded funding vault into the installment vaults (one
    /// ordinary transaction) and seal one kit per installment.
    Split {
        /// Plan id (prefix). Optional when only one plan is active.
        plan: Option<String>,
        /// Directory for the kit files and the claim-links file (default:
        /// the plan's --out).
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum PlanCmd {
    /// Scheduled payments: N installments, each unlocking for the payee on
    /// its due date; cancel sweeps the rest back.
    Installments {
        /// Payee code (age1...). Repeat for up to 3 payees; any one can claim.
        #[arg(long = "payee")]
        payees: Vec<String>,
        /// ZEC the payee receives per installment (for example 0.5).
        #[arg(long)]
        amount: String,
        /// Time between installments (for example 30d or 2m).
        #[arg(long)]
        every: String,
        /// Number of installments.
        #[arg(long)]
        count: u32,
        /// When the first installment unlocks: an RFC 3339 UTC time
        /// (2026-12-01T09:00:00Z), or a delay after the split (10m, 1d).
        /// Default: one period after the split.
        #[arg(long)]
        start: Option<String>,
        /// A message for the payee, sealed into every kit.
        #[arg(long)]
        message: Option<String>,
        /// Keeper URL: one mailbox per installment; the split writes all
        /// claim links to <plan>-claim-links.txt.
        #[arg(long)]
        mailbox: Option<String>,
        /// Directory for the kit files and the claim-links file.
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
    /// A legacy plan: funds go to the heirs if the owner stops flipping.
    Legacy {
        /// Heir code (age1...). Repeat for up to 3 heirs; any one can claim.
        #[arg(long = "heir")]
        heirs: Vec<String>,
        /// How long each kit stays locked, and how often to flip (for example 90d or 10m).
        #[arg(long)]
        every: String,
        /// A message for the heirs, sealed into the kit.
        #[arg(long)]
        message: Option<String>,
        /// Keeper URL: creates a mailbox, uploads every kit there and prints
        /// the claim link for the heir.
        #[arg(long)]
        mailbox: Option<String>,
        /// Amount in ZEC to put in the funding request.
        #[arg(long)]
        amount: Option<String>,
        /// Directory for the kit file.
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
}

#[derive(Subcommand)]
enum KitCmd {
    /// Print a kit's public part (no secrets are involved).
    Inspect {
        /// The `*.sandglass` kit file.
        file: PathBuf,
    },
}

fn network_of(cli: &Cli) -> Network {
    match cli.network {
        Some(Net::Test) => Network::TestNetwork,
        _ => Network::MainNetwork,
    }
}

fn store_root(cli: &Cli, network: Network) -> Result<PathBuf> {
    let base = match &cli.data_dir {
        Some(d) => d.clone(),
        None => {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .ok_or_else(|| anyhow!("cannot find your home directory; pass --data-dir"))?;
            PathBuf::from(home).join(".sandglass")
        }
    };
    Ok(base.join(network_name(network)))
}

fn passphrase(confirm: bool) -> Result<Zeroizing<String>> {
    if let Ok(p) = std::env::var("SANDGLASS_PASSPHRASE") {
        return Ok(Zeroizing::new(p));
    }
    let first = Zeroizing::new(rpassword::prompt_password("Store passphrase: ")?);
    if confirm {
        let again = Zeroizing::new(rpassword::prompt_password("Repeat passphrase: ")?);
        if *first != *again {
            return Err(anyhow!("the passphrases do not match"));
        }
    }
    Ok(first)
}

fn open_store(cli: &Cli, network: Network) -> Result<Store> {
    let root = store_root(cli, network)?;
    // Fail on a missing store before asking for a passphrase.
    if !root.join("keycheck.age").exists() {
        return Err(StoreError::NotInitialised(root.display().to_string()).into());
    }
    Store::open(&root, &passphrase(false)?)
}

async fn lightd(cli: &Cli, network: Network) -> Result<Client> {
    let url = cli
        .server
        .clone()
        .unwrap_or_else(|| default_url(network).to_string());
    connect(&url).await
}

fn warn_unaudited(network: Network) {
    if network == Network::MainNetwork {
        eprintln!(
            "WARNING: Sandglass is unaudited software on Zcash mainnet. \
             Plans are capped at {} ZEC unless --accept-unaudited is given.",
            zec(ops::MAINNET_CAP_ZAT)
        );
    }
}

fn unix_to_text(t: u64) -> String {
    kit::format_time(t)
}

fn time_left(unlock: u64) -> String {
    let t = now();
    if t >= unlock {
        format!(
            "UNLOCKED {} ago: heirs can claim now",
            format_span(t - unlock)
        )
    } else {
        format!("in {}", format_span(unlock - t))
    }
}

fn print_txids(sweeps: &[sandglass_wallet::SweepReport]) {
    for s in sweeps {
        println!("  txid {}", s.txid);
    }
}

fn read_kit(path: &Path) -> Result<Kit> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))
        .map_err(|e| coded("kit_invalid", format!("{e:#}")))?;
    Ok(Kit::from_json(&text)?)
}

async fn run(cli: Cli) -> Result<()> {
    let network = network_of(&cli);
    match &cli.cmd {
        Cmd::Heir(HeirCmd::New { save }) => {
            let phrase = generate_phrase();
            let code = identity_from_phrase(&phrase)?.code();
            if let Some(path) = save {
                write_secret_file(path, &phrase)?;
            }
            println!("Heir phrase (24 words). Write it down now; it is shown only once:\n");
            println!("  {}\n", phrase.as_str());
            println!("Heir code (give this to the owner; it is safe to share):\n");
            println!("  {code}\n");
            println!("The owner needs only the heir code. Keep the phrase safe: with the kit file and the phrase, the heir can claim after the unlock time.");
            if let Some(path) = save {
                println!("The phrase was also saved to {}.", path.display());
            }
        }
        Cmd::Init => {
            let root = store_root(&cli, network)?;
            Store::init(&root, &passphrase(true)?)?;
            println!(
                "Created the {} store at {}.",
                network_name(network),
                root.display()
            );
            println!("Vault seeds are encrypted with your passphrase. If you lose the passphrase, the vault seeds cannot be recovered; heirs are not affected because their kits hold the seeds too.");
        }
        Cmd::Plan(PlanCmd::Legacy {
            heirs,
            every,
            message,
            mailbox,
            amount,
            out,
        }) => {
            ops::check_heirs(heirs)?;
            warn_unaudited(network);
            let store = open_store(&cli, network)?;
            let new = ops::NewPlan {
                heirs: heirs.clone(),
                period_secs: ops::parse_duration(every)?,
                message: message.clone(),
                mailbox_url: mailbox.clone(),
                amount: amount.as_deref().map(ops::parse_zec).transpose()?,
            };
            let mut client = lightd(&cli, network).await?;
            let c = ops::create_plan(&store, network, &mut client, new, out, cli.accept_unaudited)
                .await?;
            let unlock = c.plan.current()?.unlock_time;
            println!("Plan {} created on {}.", c.plan.id, network_name(network));
            println!("Vault epoch 1 is ready. Kit file: {}", c.kit_path.display());
            println!(
                "The kit unlocks at {} ({}).",
                unix_to_text(unlock),
                time_left(unlock)
            );
            println!("\nFund the vault from any Zcash wallet with this payment request:\n");
            println!("  {}\n", c.uri);
            println!("Vault address: {}", c.address);
            match &c.claim_link {
                Some(link) => {
                    println!("\nThe kit was uploaded to your keeper mailbox. Send this claim link to your heir now. It is shown only once:\n");
                    println!("  {link}\n");
                    println!("The link alone opens nothing: the heir also needs the heir phrase and must wait for the unlock time.");
                    println!("Run `sandglass flip {}` before the unlock time to keep the funds yours; the mailbox is updated automatically.", c.plan.short_id());
                }
                None => println!("\nGive the kit file to your heirs. Run `sandglass flip {}` before the unlock time to keep the funds yours.", c.plan.short_id()),
            }
        }
        Cmd::Plan(PlanCmd::Installments {
            payees,
            amount,
            every,
            count,
            start,
            message,
            mailbox,
            out,
        }) => {
            let amount_zat = ops::parse_zec(amount)?;
            let every_secs = ops::parse_duration(every)?;
            let (start_at, start_after_secs) = match start {
                Some(s) => installments::parse_start(s)?,
                None => (None, every_secs),
            };
            installments::validate(payees, amount_zat, *count, start_at)?;
            warn_unaudited(network);
            let store = open_store(&cli, network)?;
            let out_abs = std::path::absolute(out).unwrap_or_else(|_| out.clone());
            let mut client = lightd(&cli, network).await?;
            let c = installments::create_installments(
                &store,
                network,
                &mut client,
                installments::NewInstallments {
                    payees: payees.clone(),
                    amount_zat,
                    every_secs,
                    count: *count,
                    start_at,
                    start_after_secs,
                    message: message.clone(),
                    mailbox_url: mailbox.clone(),
                    out_dir: Some(out_abs.display().to_string()),
                },
                cli.accept_unaudited,
            )
            .await?;
            let id = c.plan.short_id().to_string();
            println!(
                "Installment plan {} created on {}: {} installments of {} ZEC.",
                c.plan.id,
                network_name(network),
                count,
                zec(amount_zat)
            );
            println!(
                "Each installment vault will hold {} ZEC: the {} ZEC installment plus {} ZEC for the payee's sweep fee (ZIP 317, one input), so the payee receives exactly {} ZEC.",
                zec(amount_zat + installments::PAYEE_FEE_ZAT),
                zec(amount_zat),
                zec(installments::PAYEE_FEE_ZAT),
                zec(amount_zat)
            );
            println!(
                "\nStep 1. Fund the funding vault with {} ZEC ({} installments plus the fees) from any Zcash wallet:\n",
                zec(c.required),
                count
            );
            println!("  {}\n", c.uri);
            println!("Funding address: {}", c.address);
            println!(
                "\nStep 2. When the funds have confirmed, run `sandglass installments split {id}`. \
                 It sends one ordinary transaction into the {count} installment vaults and writes the kits{}.",
                if mailbox.is_some() { " and the claim-links file" } else { "" }
            );
            match &start_at {
                Some(at) => println!(
                    "The first installment unlocks at {}, then every {}.",
                    unix_to_text(*at),
                    format_span(every_secs)
                ),
                None => println!(
                    "The schedule starts at the split: the first installment unlocks {} later, then every {}.",
                    format_span(start_after_secs),
                    format_span(every_secs)
                ),
            }
            println!("`sandglass cancel {id} --to <address>` sweeps everything that is left back to you.");
        }
        Cmd::Installments(InstallmentsCmd::Split { plan, out }) => {
            warn_unaudited(network);
            let store = open_store(&cli, network)?;
            let p = installments::require_installment_plan(&store, plan.as_deref())?;
            let kit_dir = match out {
                Some(o) => o.clone(),
                None => PathBuf::from(
                    p.installments
                        .as_ref()
                        .and_then(|s| s.out_dir.clone())
                        .unwrap_or_else(|| ".".into()),
                ),
            };
            let mut client = lightd(&cli, network).await?;
            let r = installments::split(
                &store,
                network,
                &mut client,
                &p.id,
                &kit_dir,
                cli.accept_unaudited,
            )
            .await?;
            let set = r.plan.installments.as_ref().expect("installment plan");
            println!(
                "Split plan {}: one transaction funded {} installment vaults of {} ZEC each ({} ZEC for the payee, {} ZEC for the payee's sweep fee).",
                r.plan.short_id(),
                set.count,
                zec(set.vault_zat()),
                zec(set.amount_zat),
                zec(set.fee_zat)
            );
            println!("  txid {}", r.tx.txid);
            for l in &r.lines {
                println!(
                    "  installment {} of {}: unlocks {} ({}), kit {}",
                    l.index,
                    set.count,
                    unix_to_text(l.unlock_time),
                    time_left(l.unlock_time),
                    kit_dir.join(&l.kit_file).display()
                );
            }
            match &r.links_path {
                Some(p) => println!(
                    "\nThe kits were uploaded to the keeper (one mailbox each). Send the payee this file with all claim links: {}\nThe links open nothing without the payee's phrase and the unlock time. The payee runs: sandglass claim --watch --links-file <file> --phrase-file <phrase file> --to <address>",
                    p.display()
                ),
                None => println!("\nGive the kit files to the payee (any order; each opens at its own time)."),
            }
            println!(
                "Run `sandglass cancel {} --to <address>` to stop the schedule and take back what is left.",
                r.plan.short_id()
            );
        }
        Cmd::Receive { plan } => {
            let store = open_store(&cli, network)?;
            let plan = store.find_plan(plan.as_deref())?;
            let mut vault = ops::open_vault(&store, network, plan.current()?)?;
            println!("Plan {} (epoch {})", plan.short_id(), plan.epoch);
            println!("Vault address: {}", vault.address()?);
            println!("Payment request: {}", vault.zip321_uri(None)?);
            if network == Network::MainNetwork {
                println!(
                    "Reminder: keep the plan under {} ZEC while Sandglass is unaudited.",
                    zec(ops::MAINNET_CAP_ZAT)
                );
            }
        }
        Cmd::Sync => {
            let store = open_store(&cli, network)?;
            let mut client = lightd(&cli, network).await?;
            for plan in store.plans()? {
                if plan.status != PlanStatus::Active {
                    continue;
                }
                if plan.kind == "installments" {
                    for (label, total, spendable, height) in
                        installments::sync_all(&store, network, &mut client, &plan).await?
                    {
                        println!(
                            "Plan {} {label}: synced to height {height}. Balance {} ZEC ({} confirmed).",
                            plan.short_id(),
                            zec(total),
                            zec(spendable)
                        );
                    }
                    continue;
                }
                let mut vault = ops::open_vault(&store, network, plan.current()?)?;
                vault.sync(&mut client).await?;
                let b = vault.balances()?;
                println!(
                    "Plan {}: synced to height {}. Balance {} ZEC ({} confirmed).",
                    plan.short_id(),
                    b.fully_scanned,
                    zec(b.total()),
                    zec(b.spendable())
                );
            }
        }
        Cmd::Status { sync } => {
            let store = open_store(&cli, network)?;
            let mut client = if *sync {
                Some(lightd(&cli, network).await?)
            } else {
                None
            };
            let plans = store.plans()?;
            if plans.is_empty() {
                println!("No plans yet. Create one with `sandglass plan legacy`.");
            }
            for plan in plans {
                if plan.kind == "installments" {
                    installments_status(&store, network, &plan, client.as_mut()).await?;
                    continue;
                }
                let rec = plan.current()?;
                println!(
                    "Plan {} ({}, {}) {}",
                    plan.id,
                    plan.kind,
                    plan.network,
                    match plan.status {
                        PlanStatus::Active => "active",
                        PlanStatus::Cancelled => "cancelled",
                    }
                );
                println!("  heirs:   {}", plan.heirs.len());
                println!("  epoch:   {}", plan.epoch);
                if plan.status == PlanStatus::Active {
                    println!(
                        "  unlocks: {} ({})",
                        unix_to_text(rec.unlock_time),
                        time_left(rec.unlock_time)
                    );
                    println!("  flip:    every {}", format_span(plan.period_secs));
                    let mut vault = ops::open_vault(&store, network, rec)?;
                    if let Some(c) = client.as_mut() {
                        vault.sync(c).await?;
                    }
                    match vault.balances() {
                        Ok(b) => {
                            println!(
                                "  balance: {} ZEC ({} confirmed), as of height {}",
                                zec(b.total()),
                                zec(b.spendable()),
                                b.fully_scanned
                            );
                            if plan.network == "main"
                                && b.total() > ops::MAINNET_CAP_ZAT
                                && !cli.accept_unaudited
                            {
                                println!("  warning: above the {} ZEC cap; flip will refuse without --accept-unaudited", zec(ops::MAINNET_CAP_ZAT));
                            }
                        }
                        Err(_) => println!("  balance: not synced yet (run `sandglass sync`)"),
                    }
                }
            }
        }
        Cmd::Flip { plan, out } => {
            warn_unaudited(network);
            let store = open_store(&cli, network)?;
            let plan = store.find_plan(plan.as_deref())?;
            let unlock = plan.current()?.unlock_time;
            if now() >= unlock {
                eprintln!("Note: the current kit is already unlocked, so an heir could claim at any moment. Flipping now only works if the funds are still in the vault.");
            }
            let mut client = lightd(&cli, network).await?;
            let r = ops::flip(
                &store,
                network,
                &mut client,
                &plan.id,
                out,
                cli.accept_unaudited,
            )
            .await?;
            let unlock = r.plan.current()?.unlock_time;
            println!(
                "Flipped plan {} to epoch {}: moved {} ZEC into a new vault.",
                r.plan.short_id(),
                r.plan.epoch,
                zec(r.moved)
            );
            print_txids(&r.sweeps);
            println!("New kit: {}", r.kit_path.display());
            println!(
                "It unlocks at {} ({}). The old kit now opens an empty vault.",
                unix_to_text(unlock),
                time_left(unlock)
            );
            match (&r.plan.mailbox_url, &r.mailbox_error) {
                (None, _) => println!("Give the new kit to your heirs in place of the old one."),
                (Some(_), None) => {
                    println!("The new kit replaced the old one in your keeper mailbox; the heir's claim link still works.")
                }
                (Some(_), Some(e)) => eprintln!(
                    "WARNING: the funds moved, but the new kit could not be uploaded to the keeper ({e}). \
                     The mailbox still holds the old, empty kit. Give your heirs {} directly.",
                    r.kit_path.display()
                ),
            }
        }
        Cmd::Cancel { plan, to } => {
            warn_unaudited(network);
            let store = open_store(&cli, network)?;
            let mut client = lightd(&cli, network).await?;
            let r = ops::cancel(&store, network, &mut client, plan, to).await?;
            println!(
                "Plan {} cancelled. Moved {} ZEC to {} ({} ZEC was in the vaults; network fees {} ZEC).",
                r.plan.short_id(),
                zec(r.moved.saturating_sub(r.sweeps.iter().filter_map(|s| s.fee).sum::<u64>())),
                to,
                zec(r.moved),
                zec(r.sweeps.iter().filter_map(|s| s.fee).sum::<u64>())
            );
            print_txids(&r.sweeps);
            if r.plan.kind == "installments" {
                if !r.already_claimed.is_empty() {
                    let list: Vec<String> = r.already_claimed.iter().map(u32::to_string).collect();
                    println!(
                        "Installment(s) {} were already claimed by the payee (their claim is on the network), so there was nothing to take back.",
                        list.join(", ")
                    );
                }
                println!("Every installment vault and the funding vault were emptied; kits not yet claimed now open empty vaults.");
                match &r.mailbox_error {
                    None if r.plan.mailbox_url.is_some() => {
                        println!("The keeper mailboxes were deleted.")
                    }
                    None => {}
                    Some(e) => eprintln!(
                        "WARNING: some keeper mailboxes could not be deleted ({e}); the keeper removes them after 180 days."
                    ),
                }
            } else if r.plan.mailbox_id.is_some() {
                match &r.mailbox_error {
                    None => println!("The keeper mailbox was deleted."),
                    Some(e) => eprintln!(
                        "WARNING: the keeper mailbox could not be deleted ({e}); the keeper removes it after 180 days."
                    ),
                }
            }
        }
        Cmd::Claim {
            kits,
            links_file,
            phrase_file,
            to,
            wait,
            watch,
        } => {
            let mut sources = kits.clone();
            if let Some(f) = links_file {
                sources.extend(read_links_file(f)?);
            }
            if sources.is_empty() {
                return Err(anyhow!(
                    "give at least one kit file or claim link (or --links-file)"
                ));
            }
            if *watch && to.is_none() {
                return Err(anyhow!(
                    "--watch sweeps each kit as it unlocks, so it needs --to <address>"
                ));
            }
            claim_all(
                &cli,
                &sources,
                phrase_file,
                to.as_deref(),
                *wait || *watch,
                *watch,
            )
            .await?
        }
        Cmd::Kit(KitCmd::Inspect { file }) => inspect(file)?,
    }
    Ok(())
}

fn inspect(file: &Path) -> Result<()> {
    let kit = read_kit(file)?;
    let unlock = kit.unlock_time();
    println!("Kit {}", file.display());
    println!("  format:   {} v{}", kit.format, kit.version);
    println!("  kind:     {:?}", kit.kind);
    println!("  network:  {}", kit.network);
    println!("  plan:     {}", kit.plan);
    println!("  epoch:    {}", kit.epoch);
    println!("  created:  {}", kit.created);
    println!(
        "  unlocks:  {} (drand round {}, {})",
        kit.unlock.time,
        kit.unlock.round,
        time_left(unlock)
    );
    println!("  chain:    {}", kit.unlock.drand_chain);
    println!(
        "  sealed:   {} bytes (readable only by the heir after the unlock)",
        kit.sealed.len()
    );
    Ok(())
}

fn write_secret_file(path: &Path, text: &str) -> Result<()> {
    if path.exists() {
        return Err(anyhow!("{} already exists", path.display()));
    }
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Claim links and kit paths found in a text file (other words are ignored).
fn read_links_file(path: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))
        .map_err(|e| coded("kit_invalid", format!("{e:#}")))?;
    Ok(text
        .split_whitespace()
        .filter(|t| ClaimLink::parse(t).is_some() || Path::new(t).is_file())
        .map(str::to_string)
        .collect())
}

fn load_kit(source: &str) -> Result<Kit> {
    match ClaimLink::parse(source) {
        Some(link) if !Path::new(source).exists() => {
            let text = MailboxClient::new(&link.base).get(&link.id, &link.read_token)?;
            Ok(Kit::from_json(&text)?)
        }
        _ => read_kit(Path::new(source)),
    }
}

/// Claims every kit in unlock order. With `watch`, a kit that fails (for
/// example an empty vault after a cancel) is reported and the next one is
/// still waited for; the exit status is then non-zero.
async fn claim_all(
    cli: &Cli,
    sources: &[String],
    phrase_file: &Path,
    to: Option<&str>,
    wait: bool,
    watch: bool,
) -> Result<()> {
    let mut kits = Vec::new();
    for s in sources {
        kits.push((s.clone(), load_kit(s)?));
    }
    kits.sort_by_key(|(_, k)| k.unlock_time());
    let total = kits.len();
    let mut failed = 0usize;
    for (i, (source, kit)) in kits.iter().enumerate() {
        if total > 1 {
            match kit.sequence {
                Some(s) => println!(
                    "\n== Kit {} of {total}: installment {} of {} ==",
                    i + 1,
                    s.index,
                    s.count
                ),
                None => println!("\n== Kit {} of {total} ==", i + 1),
            }
        }
        match claim(cli, kit, phrase_file, to, wait).await {
            Ok(()) => {}
            Err(e) if watch => {
                failed += 1;
                match error_code(&e) {
                    Some(code) => eprintln!("error[{code}]: {source}: {e:#}"),
                    None => eprintln!("error: {source}: {e:#}"),
                }
            }
            Err(e) => return Err(e),
        }
    }
    if failed > 0 {
        return Err(anyhow!("{failed} of {total} kits could not be claimed"));
    }
    Ok(())
}

async fn installments_status(
    store: &Store,
    network: Network,
    plan: &sandglass_wallet::Plan,
    mut client: Option<&mut Client>,
) -> Result<()> {
    let set = plan
        .installments
        .as_ref()
        .ok_or_else(|| anyhow!("plan {} has no installment data", plan.short_id()))?;
    println!(
        "Plan {} (installments, {}) {}",
        plan.id,
        plan.network,
        match plan.status {
            PlanStatus::Active => "active",
            PlanStatus::Cancelled => "cancelled",
        }
    );
    println!("  payees:  {}", plan.heirs.len());
    println!(
        "  amount:  {} ZEC per installment x {}, every {} (each vault holds {} ZEC: the amount plus {} ZEC for the payee's sweep fee)",
        zec(set.amount_zat),
        set.count,
        format_span(set.every_secs),
        zec(set.vault_zat()),
        zec(set.fee_zat)
    );
    if plan.status == PlanStatus::Cancelled {
        return Ok(());
    }
    let mut funding = ops::open_vault(store, network, plan.current()?)?;
    if let Some(c) = client.as_mut() {
        funding.sync(c).await?;
    }
    match funding.balances() {
        Ok(b) => println!(
            "  funding: {} ZEC ({} confirmed), as of height {}",
            zec(b.total()),
            zec(b.spendable()),
            b.fully_scanned
        ),
        Err(_) => println!("  funding: not synced yet (run `sandglass status --sync`)"),
    }
    match &set.split_txid {
        None => println!(
            "  split:   not done. Fund the vault ({} ZEC needed), then run `sandglass installments split {}`.",
            zec(installments::required_funding(set.amount_zat, set.count)),
            plan.short_id()
        ),
        Some(txid) => println!("  split:   done, txid {txid}"),
    }
    if set.items.is_empty() {
        return Ok(());
    }
    let rows = installments::installment_rows(store, network, plan, client).await?;
    for r in rows {
        let state = match r.state {
            installments::InstallmentState::Locked => "locked",
            installments::InstallmentState::Unlocked => "unlocked (the payee can claim now)",
            installments::InstallmentState::ClaimedOrEmpty => "claimed or empty",
            installments::InstallmentState::Unknown => "unknown (run `sandglass status --sync`)",
        };
        println!(
            "  {} of {}: {} ZEC, unlocks {} ({}), {state}",
            r.index,
            set.count,
            zec(r.amount_zat),
            unix_to_text(r.unlock_time),
            time_left(r.unlock_time)
        );
    }
    Ok(())
}

async fn claim(
    cli: &Cli,
    kit: &Kit,
    phrase_file: &Path,
    to: Option<&str>,
    wait: bool,
) -> Result<()> {
    let kit_net = ops::wallet_network(kit.network);
    if let Some(n) = cli.network {
        let asked = match n {
            Net::Main => kit::Network::Main,
            Net::Test => kit::Network::Test,
        };
        if asked != kit.network {
            return Err(KitError::NetworkMismatch {
                kit: kit.network,
                wanted: asked,
            }
            .into());
        }
    }

    let unlock = kit.unlock_time();
    if now() < unlock {
        if !wait {
            return Err(coded(
                "not_yet_unlocked",
                format!(
                    "This kit is still locked. It unlocks at {} ({}). Run again then, or use --wait.",
                    kit.unlock.time,
                    time_left(unlock)
                ),
            ));
        }
        println!("Waiting for the unlock at {} ...", kit.unlock.time);
        while now() < unlock {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    let phrase = Zeroizing::new(
        std::fs::read_to_string(phrase_file)
            .with_context(|| format!("reading {}", phrase_file.display()))?,
    );
    let identity =
        identity_from_phrase(&phrase).map_err(|e| coded("wrong_heir_key", format!("{e}")))?;

    // The beacon can lag the clock by a few seconds; retry briefly.
    let mut attempt = 0;
    let payload = loop {
        match kit.open_online(&identity, now()) {
            Ok(p) => break p,
            Err(e @ KitError::BeaconUnavailable(_)) | Err(e @ KitError::NotYetUnlocked { .. })
                if attempt < 10 =>
            {
                attempt += 1;
                let _ = e;
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            Err(e) => return Err(e.into()),
        }
    };

    println!(
        "Kit opened: plan {} epoch {} on {}.",
        payload.plan, payload.epoch, payload.network
    );
    if let Some(m) = &payload.message {
        println!("Message from the owner: {m}");
    }
    match to {
        None => {
            println!(
                "\nVault seed (keep it secret):\n\n  {}\n",
                payload.vault_seed
            );
            println!("Birthday height: {}", payload.birthday_height);
            println!("Network: {}", payload.network);
            println!("\nTo restore the funds:");
            for h in &payload.restore_hints {
                println!("  - {h}");
            }
        }
        Some(addr) => {
            let mut client = lightd(cli, kit_net).await?;
            let work = std::env::temp_dir().join(format!(
                "sandglass-claim-{}-{}",
                &payload.plan[..8],
                now()
            ));
            // When the caller is waiting, the unlock can come before the funds
            // are spendable (the owner's split still confirming): keep trying.
            let started = now();
            let (sweeps, moved) = loop {
                let attempt = ops::claim_sweep(
                    kit_net,
                    &mut client,
                    Zeroizing::new(payload.vault_seed.clone()),
                    payload.birthday_height,
                    addr,
                    &work,
                )
                .await;
                match attempt {
                    Ok(r) => break r,
                    Err(e) if wait => {
                        let patience = match error_code(&e) {
                            Some("insufficient_funds") => 1800,
                            Some("vault_empty") => 120,
                            _ => 0,
                        };
                        if patience == 0 || now().saturating_sub(started) >= patience {
                            return Err(e);
                        }
                        println!("The funds are not spendable yet ({e}); trying again in 20 s ...");
                        tokio::time::sleep(Duration::from_secs(20)).await;
                    }
                    Err(e) => return Err(e),
                }
            };
            let fee: u64 = sweeps.iter().filter_map(|s| s.fee).sum();
            println!(
                "Moved {} ZEC to {addr} (the vault held {} ZEC; network fee {} ZEC).",
                zec(moved.saturating_sub(fee)),
                zec(moved),
                zec(fee)
            );
            print_txids(&sweeps);
        }
    }
    Ok(())
}

fn error_code(e: &anyhow::Error) -> Option<&'static str> {
    if let Some(c) = e.downcast_ref::<CodedError>() {
        return Some(c.code);
    }
    if let Some(k) = e.downcast_ref::<KitError>() {
        return Some(k.code());
    }
    if let Some(m) = e.downcast_ref::<MailboxError>() {
        return match m.code.as_str() {
            "mailbox_not_found" => Some("mailbox_not_found"),
            "unauthorized" => Some("unauthorized"),
            "kit_invalid" => Some("kit_invalid"),
            "kit_version_unsupported" => Some("kit_version_unsupported"),
            _ => None,
        };
    }
    match e.downcast_ref::<StoreError>() {
        Some(StoreError::WrongPassphrase) => Some("unauthorized"),
        _ => None,
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            match error_code(&e) {
                Some(code) => eprintln!("error[{code}]: {e:#}"),
                None => eprintln!("error: {e:#}"),
            }
            ExitCode::FAILURE
        }
    }
}

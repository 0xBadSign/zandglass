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
use sandglass_core::KitError;
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
    /// Heir side: open an unlocked kit and move the funds.
    Claim {
        /// The `*.sandglass` kit file.
        kit: PathBuf,
        /// File holding the heir's 24-word phrase.
        #[arg(long)]
        phrase_file: PathBuf,
        /// Sweep the vault to this address. Without it the seed is printed.
        #[arg(long)]
        to: Option<String>,
        /// Wait until the kit unlocks instead of refusing.
        #[arg(long)]
        wait: bool,
    },
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
enum PlanCmd {
    /// A legacy plan: funds go to the heirs if the owner stops flipping.
    Legacy {
        /// Heir code (age1...). Repeat for up to 3 heirs.
        #[arg(long = "heir", required = true)]
        heirs: Vec<String>,
        /// How long each kit stays locked, and how often to flip (for example 90d or 10m).
        #[arg(long)]
        every: String,
        /// A message for the heirs, sealed into the kit.
        #[arg(long)]
        message: Option<String>,
        /// Keeper mailbox URL to remember for this plan.
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
            println!("\nGive the kit file to your heirs. Run `sandglass flip {}` before the unlock time to keep the funds yours.", c.plan.short_id());
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
                "It unlocks at {} ({}). Give it to your heirs in place of the old kit; the old kit now opens an empty vault.",
                unix_to_text(unlock),
                time_left(unlock)
            );
        }
        Cmd::Cancel { plan, to } => {
            warn_unaudited(network);
            let store = open_store(&cli, network)?;
            let mut client = lightd(&cli, network).await?;
            let r = ops::cancel(&store, network, &mut client, plan, to).await?;
            println!(
                "Plan {} cancelled. Moved {} ZEC to {}.",
                r.plan.short_id(),
                zec(r.moved),
                to
            );
            print_txids(&r.sweeps);
        }
        Cmd::Claim {
            kit: kit_path,
            phrase_file,
            to,
            wait,
        } => claim(&cli, kit_path, phrase_file, to.as_deref(), *wait).await?,
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

async fn claim(
    cli: &Cli,
    kit_path: &Path,
    phrase_file: &Path,
    to: Option<&str>,
    wait: bool,
) -> Result<()> {
    let kit = read_kit(kit_path)?;
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
            let (sweeps, moved) = ops::claim_sweep(
                kit_net,
                &mut client,
                Zeroizing::new(payload.vault_seed.clone()),
                payload.birthday_height,
                addr,
                &work,
            )
            .await?;
            println!("Moved {} ZEC to {addr}.", zec(moved));
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

//! Feasibility spike for the Zandglass vault wallet. See `spike_vault_README.md`.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use zandglass_wallet::{connect, default_url, Balances, Network, Vault};
use zcash_client_backend::proto::service::{ChainSpec, Empty};
use zeroize::Zeroizing;

const USAGE: &str = "usage: spike_vault [--network test|main] [--url URL] <command>
commands:
  info
  new-vault <dir>
  sync <dir>
  sweep <dir> <to_address>
  restore <dir> <seed-file> <birthday>";

/// Spike only: seeds live in plaintext files. The product encrypts them.
fn read_seed(path: &Path) -> Result<Zeroizing<String>> {
    Ok(Zeroizing::new(
        std::fs::read_to_string(path)?.trim().to_string(),
    ))
}

fn open_vault(dir: &str, network: Network) -> Result<Vault> {
    let dir = PathBuf::from(dir);
    Vault::open(&dir, network, read_seed(&dir.join("seed.txt"))?)
}

fn zec(zat: u64) -> String {
    format!("{}.{:08}", zat / 100_000_000, zat % 100_000_000)
}

fn print_balances(b: &Balances) {
    println!(
        "chain tip {}, fully scanned {}",
        b.chain_tip, b.fully_scanned
    );
    for (name, p) in [
        ("sapling", b.sapling),
        ("orchard", b.orchard),
        ("ironwood", b.ironwood),
    ] {
        println!(
            "{name:<9} spendable {} ZEC, total {} ZEC",
            zec(p.spendable),
            zec(p.total)
        );
    }
    println!("spendable (all pools): {} ZEC", zec(b.spendable()));
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut network = Network::TestNetwork;
    let mut url: Option<String> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--network" => {
                network = match args.next().as_deref() {
                    Some("test") => Network::TestNetwork,
                    Some("main") => Network::MainNetwork,
                    _ => bail!("--network takes test or main\n{USAGE}"),
                }
            }
            "--url" => url = Some(args.next().ok_or_else(|| anyhow!("--url needs a value"))?),
            _ => rest.push(arg),
        }
    }
    let url = url.unwrap_or_else(|| default_url(network).to_string());
    let cmd = rest.first().map(String::as_str).unwrap_or("");
    let arg = |i: usize| -> Result<&str> {
        rest.get(i)
            .map(String::as_str)
            .ok_or_else(|| anyhow!("missing argument\n{USAGE}"))
    };

    match cmd {
        "info" => {
            let mut client = connect(&url).await?;
            let info = client.get_lightd_info(Empty {}).await?.into_inner();
            let tip = client.get_latest_block(ChainSpec {}).await?.into_inner();
            println!("server:           {url}");
            println!("chain name:       {}", info.chain_name);
            println!("block height:     {}", info.block_height);
            println!("latest block:     {}", tip.height);
            println!("server version:   {} ({})", info.version, info.vendor);
            println!("branch id:        {}", info.consensus_branch_id);
            println!(
                "upgrade:          {} at {}",
                info.upgrade_name, info.upgrade_height
            );
            println!("protocol version: {}", info.lightwallet_protocol_version);
        }
        "new-vault" => {
            let mut client = connect(&url).await?;
            let dir = PathBuf::from(arg(1)?);
            let phrase = Vault::new_seed();
            let birthday = Vault::new_birthday(&mut client).await?;
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("seed.txt"), phrase.as_bytes())?;
            let mut vault = Vault::init(&dir, network, &mut client, phrase, birthday).await?;
            println!("vault created in {}", vault.dir().display());
            println!("birthday height: {birthday}");
            println!("address: {}", vault.address()?);
            println!("zip321:  {}", vault.zip321_uri(None)?);
        }
        "sync" => {
            let mut client = connect(&url).await?;
            let mut vault = open_vault(arg(1)?, network)?;
            vault.sync(&mut client).await?;
            print_balances(&vault.balances()?);
        }
        "sweep" => {
            let mut client = connect(&url).await?;
            let mut vault = open_vault(arg(1)?, network)?;
            vault.sync(&mut client).await?;
            print_balances(&vault.balances()?);
            for r in vault.sweep(&mut client, arg(2)?).await? {
                println!("txid:          {}", r.txid);
                println!("target height: {}", r.target_height);
                println!("anchor height: {}", r.anchor_height);
                println!("expiry height: {}", r.expiry_height);
                match r.fee {
                    Some(f) => println!("fee:           {f} zat"),
                    None => println!("fee:           unknown"),
                }
                println!("raw size:      {} bytes", r.raw_len);
            }
        }
        "restore" => {
            let mut client = connect(&url).await?;
            let birthday: u32 = arg(3)?.parse()?;
            let dir = PathBuf::from(arg(1)?);
            let phrase = read_seed(Path::new(arg(2)?))?;
            let mut vault = Vault::init(&dir, network, &mut client, phrase, birthday).await?;
            std::fs::write(dir.join("seed.txt"), vault.seed_phrase())?;
            println!("vault restored in {}", vault.dir().display());
            println!("address: {}", vault.address()?);
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
}

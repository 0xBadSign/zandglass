//! Feasibility spike for the Sandglass vault wallet. See `spike_vault_README.md`.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use sandglass_wallet::{connect, default_url, Balances, Network, Vault};
use zcash_client_backend::proto::service::{ChainSpec, Empty};

const USAGE: &str = "usage: spike_vault [--network test|main] [--url URL] <command>
commands:
  info
  new-vault <dir>
  sync <dir>
  sweep <dir> <to_address>
  restore <dir> <seed-file> <birthday>";

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
            let (mut vault, birthday) =
                Vault::create(&PathBuf::from(arg(1)?), network, &mut client).await?;
            println!("vault created in {}", vault.dir().display());
            println!("birthday height: {birthday}");
            println!("address: {}", vault.address()?);
            println!("zip321:  {}", vault.zip321_uri()?);
        }
        "sync" => {
            let mut client = connect(&url).await?;
            let mut vault = Vault::open(&PathBuf::from(arg(1)?), network)?;
            vault.sync(&mut client).await?;
            print_balances(&vault.balances()?);
        }
        "sweep" => {
            let mut client = connect(&url).await?;
            let mut vault = Vault::open(&PathBuf::from(arg(1)?), network)?;
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
            let mut vault = Vault::restore(
                &PathBuf::from(arg(1)?),
                network,
                &mut client,
                &PathBuf::from(arg(2)?),
                birthday,
            )
            .await?;
            println!("vault restored in {}", vault.dir().display());
            println!("address: {}", vault.address()?);
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
}

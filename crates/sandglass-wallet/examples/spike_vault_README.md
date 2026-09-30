# spike_vault

Feasibility spike for the Sandglass vault wallet. It exercises the standard
`zcash_client_backend` flow against a public lightwalletd server.

```
spike_vault [--network test|main] [--url URL] <command>
  info
  new-vault <dir>
  sync <dir>
  sweep <dir> <to_address>
  restore <dir> <seed-file> <birthday>
```

Defaults: network `test`, URL `https://testnet.zec.rocks:443` (test) or
`https://zec.rocks:443` (main).

Run it from the repository root:

```
cargo run -p sandglass-wallet --example spike_vault -- info
cargo run -p sandglass-wallet --example spike_vault -- new-vault /tmp/vault1
```

## What it does

- `info` prints GetLightdInfo and the latest block height.
- `new-vault` generates a 24-word BIP 39 seed, creates a wallet DB with account
  0 (birthday = chain tip minus 100), and prints a shielded-only unified
  address (Orchard receiver, no transparent receiver) and a ZIP 321 URI.
- `sync` downloads compact blocks and prints spendable and total balances for
  Sapling, Orchard and Ironwood.
- `sweep` calls `propose_send_max_transfer` over all shielded pools, then
  `create_proposed_transactions` (default expiry, default anchor, ZIP 317
  fee, `OvkPolicy::Discard`), then broadcasts with `SendTransaction`. It
  prints the txid, target and anchor heights, expiry height and fee.
- `restore` creates a wallet from an existing seed file and birthday.

## Security note (spike only)

In this spike the example writes the seed in plaintext to `<dir>/seed.txt` (the library never does). The product stores vault
seeds encrypted (plan section 5.6). Keep spike data directories outside the
repository, and do not use this example with funds you cannot lose.

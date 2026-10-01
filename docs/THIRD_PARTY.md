# Third-party software

Sandglass is licensed MIT OR Apache-2.0. All third-party code arrives as libraries through package managers (crates.io and npm). There are no vendored copies, no patched forks and no `[patch]` sections in the workspace. License data below comes from `cargo metadata --locked` and from `web/package-lock.json` as of 2026-10-01; regenerate it when dependencies change.

Counts: the Rust lockfile holds 513 third-party packages and the web lockfile 161 (55 of them production dependencies, the rest are build and test tools).

## Main Rust crates (direct dependencies of the workspace)

| Crate | Version | License | Used for |
|---|---|---|---|
| `zcash_client_backend` | 0.24.0 | MIT OR Apache-2.0 | Transaction proposals, scanning, lightwalletd client |
| `zcash_client_sqlite` | 0.22.0 | MIT OR Apache-2.0 | Light-client wallet database |
| `zcash_primitives` | 0.30.1 | MIT OR Apache-2.0 | Transactions, ZIP 317 fee rule |
| `zcash_protocol` | 0.10.6 | MIT OR Apache-2.0 | Network and value types |
| `zcash_keys` | 0.16.1 | MIT OR Apache-2.0 | Unified keys and addresses |
| `zcash_address` | 0.13.0 | MIT OR Apache-2.0 | Address parsing |
| `zcash_proofs` | 0.30.0 | MIT OR Apache-2.0 | Local prover (`bundled-prover` feature ships the Sapling parameters inside the crate) |
| `orchard` | 0.15.5 | MIT OR Apache-2.0 | Orchard shielded pool |
| `zip321` | 0.9.0 | MIT OR Apache-2.0 | ZIP 321 payment requests |
| `zip32` | 0.2.1 | MIT OR Apache-2.0 | HD key derivation |
| `age` | 0.12.1 | MIT OR Apache-2.0 | age encryption (heir layer, store passphrase) |
| `tlock_age` | 0.0.10 | MIT | drand timelock encryption in age format |
| `drand_core` | 0.0.19 | MIT | drand beacons and verification |
| `bip0039` | 0.12.0 | MIT/Apache-2.0 | BIP 39 mnemonics |
| `hkdf`, `sha2` | 0.12.4, 0.10.9 | MIT OR Apache-2.0 | Heir key derivation, token hashes |
| `bech32` | 0.11.1 | MIT | age identity encoding |
| `zeroize`, `secrecy` | 1.9.0, 0.8.0 | Apache-2.0 OR MIT | Secret handling |
| `subtle` | 2.6.1 | BSD-3-Clause | Constant-time token comparison |
| `rand` | 0.8.8 | MIT OR Apache-2.0 | Randomness |
| `tonic` | 0.14.6 | MIT | gRPC to lightwalletd |
| `rustls` | 0.23.45 | Apache-2.0 OR ISC OR MIT | TLS |
| `tokio` | 1.53.1 | MIT | Async runtime |
| `axum` | 0.8.9 | MIT | Keeper HTTP server |
| `tower-http` | 0.6.11 | MIT | CORS and static files |
| `rusqlite` | 0.37.0 | MIT | SQLite (the keeper bundles SQLite) |
| `ureq` | 3.4.2 | MIT OR Apache-2.0 | HTTP client (drand, mailbox) |
| `clap` | 4.6.7 | MIT OR Apache-2.0 | CLI |
| `rpassword` | 7.5.4 | Apache-2.0 | Passphrase prompt |
| `serde`, `serde_json` | 1.0.229, 1.0.151 | MIT OR Apache-2.0 | Serialization |
| `anyhow`, `thiserror`, `hex`, `async-trait` | various | MIT OR Apache-2.0 | Utilities |

## Main npm packages (`web/`)

| Package | Version | License | Kind | Used for |
|---|---|---|---|---|
| `tlock-js` | 0.9.0 | Apache-2.0 OR MIT | prod | Timelock decryption in the browser |
| `drand-client` | 1.4.2 | Apache-2.0 OR MIT | prod | Fetching and verifying beacons |
| `age-encryption` | 0.3.1 | BSD-3-Clause | prod | age decryption in the browser |
| `@noble/hashes` | 2.4.0 | MIT | prod | HKDF, SHA-256 for the heir key |
| `@scure/base` | 2.4.0 | MIT | prod | Bech32 |
| `@scure/bip39` | 2.4.0 | MIT | listed as dev, bundled by the build | BIP 39 phrases |
| `qrcode-generator` | 2.0.4 | MIT | prod | QR codes |
| `@fontsource-variable/bodoni-moda`, `@fontsource-variable/sora`, `@fontsource-variable/martian-mono` | 5.3.0 | OFL-1.1 | prod | Self-hosted fonts |
| `vite` | 7.3.6 | MIT | dev | Build |
| `typescript` | 5.9.3 | Apache-2.0 | dev | Type checking |
| `vitest` | 3.2.7 | MIT | dev | Unit tests |
| `@playwright/test` | 1.63.0 | Apache-2.0 | dev | Browser tests |
| `@axe-core/playwright` | 4.13.0 | MPL-2.0 | dev | Accessibility tests |

## Reference tools (tests and documentation only, not shipped)

`age` (1.2.1) and `tle` from the drand `tlock` project (v1.2.0) are used by `scripts/interop-kit.sh` and are described in `docs/KIT_FORMAT.md`. They are installed by the user and not bundled; see their repositories for their licenses.

## Licenses that are not plain MIT, Apache-2.0, BSD or ISC

- **OFL-1.1 (SIL Open Font License)**: the three font packages. The fonts are served with the web build. The license allows redistribution and embedding; the license text must stay with the font files, which the packages include.
- **MPL-2.0**: `axe-core` (a dependency of `@axe-core/playwright`, two packages in the lockfile). Development and test only; it is not part of the shipped pages and is unmodified.
- **Dual licenses with a copyleft option**: `self_cell` 1.3.0 (Apache-2.0 OR GPL-2.0-only) and `r-efi` 6.0.0 (MIT OR Apache-2.0 OR LGPL-2.1-or-later), both transitive. We use them under the permissive alternative (Apache-2.0 and MIT).
- **Compound licenses**: `ring` 0.17.14 (Apache-2.0 AND ISC) and `matchit` 0.8.4 (MIT AND BSD-3-Clause), both transitive; both are permissive and require the notices to be kept.
- **Proving parameters**: `zcash_proofs` with `bundled-prover` embeds the Sapling parameters. They are distributed inside that crate under its license.

Everything else in the Rust lockfile is licensed under 0BSD, Apache-2.0, BSD-1/2/3-Clause, BSL-1.0, CC0-1.0, CDLA-Permissive-2.0, ISC, MIT, MIT-0, Unicode-3.0, Unlicense, Zlib or a combination of these (with the LLVM exception where stated). In the npm lockfile the remaining licenses are MIT, ISC, BSD-2-Clause and BSD-3-Clause.

## External services

These are not code in the repository but the product talks to them: drand quicknet relays (`api.drand.sh`, `api2.drand.sh`, `drand.cloudflare.com`) and the public lightwalletd servers `zec.rocks` (mainnet) and `testnet.zec.rocks` (testnet).

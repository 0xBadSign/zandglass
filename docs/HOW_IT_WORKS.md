# How Zandglass works

This document describes what the code in this repository does. Where `docs/PLAN.md` differs, the code is described here and the difference is noted.

## 1. Keys and objects

| Object | What it is | Where it lives |
|---|---|---|
| **Vault seed** | A BIP 39 24-word mnemonic generated per epoch. The wallet uses account 0 only. The vault address is a shielded-only unified address with an Orchard receiver and no transparent receiver. | The owner's store, encrypted: `~/.zandglass/<network>/vaults/<plan>/e<N>/seed.age` (age passphrase encryption, scrypt). Also inside every kit, sealed. |
| **Owner signing key** | A random Ed25519 key per plan (new plans only). It signs every kit of the plan so the heir's page can tell when a later kit was not sealed by the same owner (`docs/KIT_FORMAT.md` section 8). It opens no funds and no kit. | The owner's store, encrypted like the seeds: `vaults/<plan>/owner_key.age`. The public half is in every kit. |
| **Birthday height** | The block height a wallet restore starts scanning from. New vaults use the chain tip minus 100 blocks. | In the plan record and in the kit payload. |
| **Heir key** | 24 BIP 39 words, turned into an age X25519 identity (see `docs/KIT_FORMAT.md`). Created by the heir only (`zandglass heir new`, or the heir page in the browser). | The heir's head, paper or password manager. Never sent anywhere. |
| **Heir code** | The matching public age recipient string (`age1...`). The owner's tool accepts heir codes and nothing else. | Plan record (public). |
| **Plan** | Heir codes (1 to 3), a period, an optional message, the current epoch, optional mailbox data. | `plans.json` in the store. It holds no seeds. |
| **Epoch** | One vault of a plan. Epoch 1 is the first vault; every flip creates the next. | One directory per epoch. |
| **Kit** | A JSON file `<plan>-e<epoch>.zandglass`: public metadata plus the sealed vault seed. | Given to the heir, or uploaded to the keeper mailbox. |
| **Mailbox** | A slot on a keeper that holds the latest kit of a plan. The heir's claim link points to it. | The keeper (optional). |

The owner's store is created by `zandglass init`. It is protected by a passphrase (prompted, or the `ZANDGLASS_PASSPHRASE` environment variable for automation). A store exists per network (`main` or `test`).

## 2. The nested seal

```
payload   = JSON { vault_seed, birthday_height, network, plan, epoch, message, restore_hints }
inner     = age_encrypt(payload, to = 1..3 heir codes)          // X25519 recipients, age v1 format
sealed    = tlock_encrypt(inner, drand quicknet round R)        // age file with a tlock stanza, ASCII armour
kit       = { public metadata..., "sealed": sealed }
```

- **Inner layer (heir-gated).** The payload is encrypted with the `age` crate to up to three heir codes. Any one of the heirs can open it.
- **Outer layer (time-gated).** The age file is encrypted with `tlock_age` to a round of drand quicknet. The result is an ASCII-armoured age file whose header carries the round number and the chain hash. Nobody can compute the key before drand publishes that round's beacon (its BLS signature). A threshold of the League of Entropy drand nodes could in theory publish early; that would remove only the time gate, not the heir gate.
- Both layers are standard formats. `tle --decrypt` followed by `age --decrypt -i <identity>` opens a kit without any Zandglass code (`docs/KIT_FORMAT.md`).
- Opening: the library requires the beacon of exactly round R. A beacon of a later round is rejected (`WrongBeaconRound`), because the tlock key is round-specific.
- The seal is built in memory; the plaintext payload and the vault seed are zeroized after use where the libraries allow.

## 3. Flips and epochs

`zandglass flip` is the owner's check-in. In order:

1. Sync the current vault and require that it holds funds, all of them confirmed (`vault_empty`, `insufficient_funds` otherwise).
2. On mainnet, refuse if the balance is above 0.25 ZEC and `--accept-unaudited` was not given (`amount_over_cap`).
3. Create the next vault: a new random seed, a new birthday, an encrypted seed file, a new wallet database.
4. Build the new kit (seed of the new vault, unlock round for `now + period`) and write the kit file. Everything the heir and owner need exists on disk before any funds move.
5. Sweep the old vault to the new vault's address: one ordinary shielded transaction that sends the whole spendable balance minus the ZIP 317 fee, built with the standard `zcash_client_backend` proposal flow (default expiry, default anchor) and `OvkPolicy::Discard`. If this fails, the new epoch and its kit are removed and nothing has moved.
6. Record the new epoch in `plans.json`.
7. If the plan has a mailbox, replace the kit there. If that upload fails, the funds have already moved and the CLI warns; the mailbox still holds the old kit, which opens an empty vault, and the owner must hand the new kit file to the heirs.

Properties:

- **Revocation.** After a flip, an older kit still opens (if its time has come) but its vault is empty. `claim` reports `vault_empty`. This was shown on testnet (`docs/evidence/EVIDENCE.md`).
- **`OvkPolicy::Discard`.** Outputs of a sweep are not encrypted to the sender's outgoing viewing key. Someone who later learns an old vault's seed (the heir holding an old kit, say) can see that vault's own incoming notes and that they were spent, but cannot recover where the funds went.
- **No marker on chain.** The sweep has the same shape as any wallet's transaction. See the fingerprint analysis in `docs/PRIVACY.md`.
- **The owner can race.** After the unlock time the funds still sit in a vault whose seed the owner also knows. The owner can move them until the heir does. This is intentional and the same as Bitcoin timelock recovery.
- **Flip after unlock.** The CLI prints a note that the heir could already claim, and flips only if the funds are still there.

`zandglass cancel <plan> --to <address>` sweeps the current vault to an address you choose, marks the plan cancelled and deletes the mailbox (the keeper also deletes it by itself, 180 days after the unlock). The plan text says the funds may go to "a fresh vault"; the code sends them to the address you give.

## 4. Installments

Scheduled payments (an allowance, a salary) reuse the same mechanism, one vault per installment. `zandglass plan installments --payee <heir code> --amount <ZEC> --every <period> --count <N>` creates the plan and a funding request; once the funding vault is funded, `zandglass installments split` sends it into N installment vaults in one ordinary transaction and seals one kit per installment (kit `kind: "installment"` with a `sequence` index and count), each unlocking one period after the previous one. The payee runs `zandglass claim --watch --links-file <plan>-claim-links.txt --to <address>`, which waits for each unlock and sweeps every installment as it opens. `zandglass cancel` sweeps the installments that are still locked back to the owner; their kits then open empty vaults. The owner panel offers the same steps (evidence: `docs/evidence/EVIDENCE.md`, 2026-10-01 rows).

### Savings lock

A savings lock is a commitment device: the owner locks shielded ZEC until a fixed date and cannot touch it before then. `zandglass plan lock --phrase-file <file with your 24 heir words> --until <RFC 3339 time or duration such as 90d> --i-have-my-heir-words` (or "Savings lock" in the owner panel) does this. The lock is irreversible, so it is made only to a heir code whose words you can show: the CLI derives the code from `--phrase-file` (an optional `--heir <code>` must then match it, and a pasted code alone is refused), and the panel asks you to type the 24 words, derives the code in the browser and sends only that public code to the local panel (the words are never sent or stored, and the box is emptied). A code swapped by clipboard malware or a typo therefore cannot lock the funds to a key you do not hold:

1. It creates a vault, as for any plan, and seals one kit to the owner's own heir code (made on `/heir` like any heir key) with the fixed unlock time. The kit's public `kind` is `legacy`: kit format and claim page are unchanged. The plan is stored with kind `lock`, the vault address, and the same per-plan owner signing key as other plans.
2. The vault seed is never written to the store: it exists in memory only while the kit is sealed (and zeroized afterwards); the scratch wallet directory used to derive the address holds viewing keys only and is removed. So there is no `seed.age` for a lock, and only the kit plus the owner's 24 heir words can open the vault after the unlock. The vault address and birthday stay in the plan so the owner can fund it (QR request) any time before the unlock.
3. Before the plan is recorded and anything is funded, the kit file is written and read back (`Kit::from_json` and `validate`, same round, owner signature present), and it is uploaded to the mailbox if one was asked for (the keeper keeps a mailbox until 180 days after the kit unlocks). Any failure removes the kit and the owner key and creates nothing.
4. The caller must confirm explicitly: the CLI flag `--i-have-my-heir-words`, or in the panel a checkbox plus typing `LOCK`. Without it nothing is created. The mainnet funding cap applies to `--amount` as for other plans (the balance of a lock cannot be shown or checked: there is no seed, and the wallet is not kept).
5. `flip` and `cancel` refuse with: "This is a savings lock: nobody can move the funds before <date>, not even you. After the unlock, open the kit on zandglass.cash/claim or with `zandglass claim`." `status` shows the unlock date, the vault address and "seed deleted"; `sync` skips the plan.

Losing the 24 heir words or the kit means losing the funds; nobody can recover them. On chain the lock is an ordinary funding transaction into an ordinary shielded address (`docs/PRIVACY.md` is unchanged).

## 5. Claim paths

| Path | What happens |
|---|---|
| `zandglass claim <kit file or claim link> --phrase-file <file> --to <address>` | Refuses before the unlock time unless `--wait` is given. Derives the heir identity from the phrase file, fetches the beacon of round R (retrying for about 30 seconds if the beacon lags the clock), opens both layers, restores the vault seed into a throwaway wallet directory under the system temp directory, syncs, sweeps everything to `<address>` and deletes the directory. The seed is not written to disk. |
| `zandglass claim` without `--to` | Opens the kit and prints the owner's message, the vault seed, the birthday height, the network and restore hints. The heir restores the seed in any Zcash wallet, setting the birthday height. |
| Heir page (`/claim`) | Loads a kit from a claim link (`#m=<id>&r=<read token>&o=<owner key hint>`, optional `&k=<keeper origin>`) or a file, checks the owner signature (an altered kit is refused; when the link carries the owner hint `o` the kit must be signed by exactly that key or it is refused as a possible decoy; without `o` a kit signed by another key or plan than this mailbox or plan served before, or not at all, gets a warning, and an unsigned kit from a mailbox is flagged; kit files and paper kits have no link and keep the warning-only behaviour; unsigned kits stay valid, see `docs/KIT_FORMAT.md` section 8), shows a countdown and a pulse line (when the owner last made a kit, from the kit's public `created` and `epoch`, with a local `.ics` file for the unlock), fetches the beacon from drand and decrypts in the browser, then shows the seed, birthday and restore steps. It does not sweep for the heir. |
| Paper kit (`/claim`, scan) | The owner panel prints the kit as numbered QR codes (`ZGK1:<i>/<n>:<8 hex of the SHA-256>:<chunk>`; a short kit is one plain code). The claim page reads them from photos or the camera, in any order, in the browser (`web/src/lib/paper.ts`), checks the hash, rebuilds the kit and continues as for a file. Nothing is uploaded. |
| Practice kit (`/heir`, then `/claim`) | The heir page seals a practice kit to the heir's own code in the browser, unlocking in about 60 seconds, with an empty seed (`docs/KIT_FORMAT.md` section 9). The heir opens it on `/claim` with their own 24 words. No funds, no upload, no pulse line. |
| By hand | `tle --decrypt` then `age --decrypt -i`, see `docs/KIT_FORMAT.md`. |

Errors use shared codes: `kit_invalid`, `kit_version_unsupported`, `not_yet_unlocked`, `beacon_unavailable`, `wrong_heir_key`, `decrypt_failed`, `vault_empty`, `network_mismatch`, `amount_over_cap`, `insufficient_funds`, `mailbox_not_found`, `unauthorized`. The CLI prints `error[<code>]: <message>`. (`sync_required`, listed in the plan, is not used by the code.)

The claim sweep is not subject to the 0.25 ZEC cap; the cap applies to creating a funded request (`--amount`) and to flips.

## 6. Network upgrades

A kit holds a seed, not a transaction. Every transaction (funding aside, which comes from any wallet) is built when it is sent, by the wallet code of the day, with the current consensus branch, anchor and expiry. A network upgrade therefore cannot make a kit stale. This is the main reason the pre-signed design was rejected (`docs/PRIVACY.md`).

What an upgrade does require is a wallet-library bump. This repository builds on `zcash_client_backend` 0.25 and `zcash_protocol` 0.11, which know NU7 (mainnet activation height 3,543,405, around 2026-11-05); the bump was tested on testnet, which already runs NU7 (`docs/evidence/EVIDENCE.md`, 2026-10-25). Balances are read, and sweeps taken, from the Sapling, Orchard and Ironwood pools.

## 7. Time math

drand quicknet: chain hash `52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971`, genesis time 1692803367 (unix), period 3 seconds, scheme `bls-unchained-g1-rfc9380`. These are pinned in `zandglass-core::drand` and were checked against the network's `/info` endpoint (D-012).

```
round_at(t)      = floor((t - genesis) / 3) + 1        // the round published at time t (0 before genesis)
unlock_round(t)  = ceil((t - genesis) / 3) + 1         // the first round published at or after t
round_time(R)    = genesis + (R - 1) * 3               // when round R is published
```

- A new kit for a plan with period P is sealed to `unlock_round(now + P)`. The kit's `unlock.time` is `round_time(R)` in UTC, which is at or after `now + P` and less than 3 seconds later.
- Unlocking is wall-clock time, not block height. The change of Zcash block time at NU7 does not matter.
- Example: the test vector's round 32665869 is published at `1692803367 + 32665868 * 3 = 1790800971`, which is 2026-09-30T20:42:51Z.
- Beacons are fetched over HTTPS from `api.drand.sh` and `drand.cloudflare.com` in the Rust code (the web pages also try `api2.drand.sh`), tried in order, and verified against the pinned public key. `Kit::open_online` refuses without any network call while the local clock is before the unlock round, so a wrong local clock gives `not_yet_unlocked`.
- If drand stops publishing, kits cannot open until it resumes; the funds stay in the owner's vault.

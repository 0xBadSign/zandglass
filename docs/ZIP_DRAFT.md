```
ZIP: Unassigned
Title: Time-Locked Recovery Kits for Shielded Wallets
Owners: Zandglass contributors
Status: Draft
Category: Wallet
Created: 2026-10-01
License: MIT
Discussions-To: (to be opened on the Zcash Community Forum)
```

# Terminology

The key words "MUST", "MUST NOT", "SHOULD", "SHOULD NOT" and "MAY" are to be interpreted as described in BCP 14 [^bcp14] when, and only when, they appear in all capitals.

- **Owner**: the person whose funds are protected.
- **Heir**: a person who may receive the funds after the unlock time. "Payee" is used for scheduled payments.
- **Heir key**: a BIP 39 [^bip39] phrase held by the heir. **Heir code**: the public age [^age] X25519 recipient derived from it.
- **Vault**: a dedicated wallet seed (BIP 39, 24 words) that holds the protected funds. Each vault is used for one **epoch**.
- **Kit**: a file that contains a vault seed encrypted first to the heir codes, then to a future drand [^drand] round.
- **Flip**: moving all funds of the current vault to a fresh vault and sealing a new kit with a later unlock round.
- **Keeper**: an optional, untrusted mailbox server that stores kits and serves the latest one to the heir.

# Abstract

This proposal specifies how a Zcash wallet can offer inheritance and scheduled payments for shielded funds without consensus changes, scripts, custody or any on-chain marker. The owner keeps funds in a dedicated vault whose seed is sealed twice: to the heir's public key, and to a future time with drand timelock encryption. While the owner is active, the wallet periodically moves the funds to a fresh vault ("flip"), which makes every older kit open an empty vault. If the owner stops, the newest kit becomes openable by the heir alone at the chosen time. Every transaction involved is an ordinary shielded transaction, built by the standard wallet code at the time it is sent.

# Motivation

Shielded Zcash has no timelocks and no scripts, so there is no private way to pass funds on. Heirs of shielded funds often cannot even discover that the funds exist. The options in use today are poor:

- writing the seed into a will or giving it to a lawyer exposes it now, to people who should only get it later;
- custodial services hold the keys and usually know the user's identity;
- transparent-chain inheritance schemes publish the recovery policy and the heir's claim on chain;
- pre-signed transactions need unusual expiry heights and old anchors, which are on-chain fingerprints, and they go stale at network upgrades.

A wallet-level standard lets any Zcash wallet offer the feature, lets kits made by one wallet be opened by another, and keeps the privacy properties users expect from shielded Zcash.

# Requirements

- No consensus change, no new transaction type, no transparent component.
- Nothing on chain distinguishes a protected vault, a flip or a claim from ordinary shielded use.
- Before the unlock time nobody, including the heir and any server, can open a kit. After it, only a listed heir can.
- A kit opens with widely available reference tools (`tle` and `age`), without the wallet that made it.
- The heir can restore the vault in any wallet that accepts a BIP 39 seed and a birthday height.
- Revocation needs no cooperation from the heir: an owner can always make old kits worthless.

# Non-requirements

- Hiding from the heir, after the unlock, the balance and history of the vault they receive.
- Protection against an owner who gives the heir the vault seed directly.
- Proof of death or legal enforceability. This is a dead man's switch, not a will.

# Specification

## Heir keys

An heir key is a 24-word BIP 39 English phrase. The heir identity is derived as:

```
seed      = BIP39-Seed(phrase, passphrase = "")                    (64 bytes)
secret    = HKDF-SHA256(ikm = seed, salt = "", info = "zandglass-heir-v1", L = 32)
identity  = Bech32("age-secret-key-", secret), upper-cased           (AGE-SECRET-KEY-1...)
heir code = the age X25519 recipient of that identity                (age1...)
```

Wallets MUST show the heir code as the only thing the heir shares, and MUST NOT ask the owner for the heir phrase. Wallets SHOULD let the heir confirm a few words of the phrase before showing the heir code.

## Kits

A kit is the JSON document specified in [Kit format v1](KIT_FORMAT.md). In short:

```
sealed = ASCII-armour( tlock( age( payload, to 1..3 heir codes ), drand quicknet, round ) )
```

The payload holds the vault seed, its birthday height, the network, the plan id, the epoch and an optional message. The public part holds only the format, version, kind, network, plan id, epoch and unlock round. Wallets MUST NOT put amounts, addresses or transaction ids in the public part.

A kit MAY carry two more public fields, `owner_key` and `owner_sig`: a random per-plan Ed25519 public key and its signature over a canonical text form of the public part and the hash of `sealed` ([KIT_FORMAT.md](KIT_FORMAT.md) section 8). Readers MUST accept kits without them and MUST reject a kit whose owner fields are present but invalid. A reader that has seen a signed kit of a plan SHOULD warn when a later kit of that plan is unsigned or signed by another key. The key is random, belongs to the plan only and is not derived from any vault or wallet key.

The unlock round MUST be on drand quicknet (chain hash `52db9ba7…e971`, genesis 1692803367, period 3 s). The round for a target time `t` is `ceil((t - genesis) / 3) + 1`.

## Vault lifecycle

1. **Create.** The wallet generates a fresh 24-word vault seed, uses account 0 only, records a birthday height, seals a kit for epoch 1 and shows a ZIP 321 [^zip321] payment request for the vault's shielded address. The vault address MUST NOT have a transparent receiver.
2. **Fund.** Any wallet pays the request.
3. **Flip.** The wallet creates vault `k+1` (fresh seed), seals its kit with a later unlock round, writes or uploads the new kit, and only then sweeps all funds of vault `k` to vault `k+1`. If the outcome of the sweep is unknown (for example a lost connection), the wallet MUST keep both epochs until it learns whether the transaction was mined or expired.
4. **Claim.** After the unlock round's beacon is published, the heir opens the newest kit, restores the vault seed with its birthday height in any wallet, and moves the funds.
5. **Cancel.** The owner sweeps the current vault to an address of their choice and deletes the keeper mailbox if there is one.

Scheduled payments use the same objects: a funding vault is split in one transaction into `N` vaults, each sealed to the payee with its own unlock round; cancelling sweeps every vault that has not been claimed.

## Transaction rules

Every funding, flip, split, claim and cancel transaction MUST be built by the wallet's standard proposal flow at the time it is sent, with:

- the default expiry height (target height + 40);
- the default anchor;
- the ZIP 317 [^zip317] conventional fee;
- `OvkPolicy::Discard` for sweeps out of a vault, so that an heir who later learns a vault seed cannot see where earlier funds went;
- shielded inputs and outputs only.

Wallets MUST NOT pre-sign transactions for later broadcast as part of this scheme.

## Keeper (optional)

A keeper is a blind mailbox: the owner creates a mailbox, receives a write token, uploads each new kit, and gives the heir one claim link `https://<keeper>/claim#m=<id>&r=<read token>`. The keeper stores only kits and token hashes and serves the newest kit to the read token. The HTTP API is specified in [Keeper API](KEEPER_API.md). The fragment of the claim link is never sent to a server by browsers. Wallets MUST work without a keeper (kit files handed over directly).

## Reminders

Wallets SHOULD remind the owner to flip before each unlock. Reminders SHOULD be local (notifications, or an iCalendar file the owner imports) rather than a server that would learn the schedule.

# Privacy considerations

| Party | Learns | Does not learn |
|---|---|---|
| Chain observers | Ordinary shielded transactions with standard fields | That a plan exists, the heir, amounts, unlock times, links between vault epochs |
| drand | That someone fetched a public beacon | Anything about a kit |
| Keeper | That a mailbox exists and when its kit changes | Owner, heir, amounts, addresses, kit contents |
| lightwalletd | Usual light-client metadata | Keys and balances |
| Heir after the unlock | The current vault's seed, balance and incoming transfer | Earlier vaults' destinations, the owner's other funds |

Residual metadata: the flip cadence is visible to a keeper; the number of outputs of a split is visible on chain; birthday-height queries can correlate the owner and the heir at one lightwalletd server (wallets SHOULD randomise the birthday a little and MAY use different servers). Details: [PRIVACY.md](PRIVACY.md).

# Security considerations

- **drand.** Opening early requires a threshold of the drand League of Entropy to collude. A drand outage delays claims; it never enables them early.
- **Lost heir phrase.** The kit cannot be opened; the owner can re-seal to a new heir code at the next flip.
- **Decoy kit.** Anyone who knows an heir code can seal a kit to it, so a party that controls where the heir fetches the kit (a keeper operator) could substitute a kit with a different seed. Signed kits (above) let the heir's software detect a kit that does not come from the same owner as an earlier one; the first key seen is trusted on first use, and the owner can confirm the key fingerprint out of band.
- **Stolen kit.** Useless without a listed heir's phrase. **Stolen heir phrase.** Useless without the newest kit, and only after the unlock.
- **Late flip.** Once the unlock round is published, the heir can claim at any moment; a flip only helps if the funds have not moved yet. Wallets MUST warn the owner when the unlock is near.
- **Software risk.** Implementations SHOULD cap amounts until audited.

# Rationale

- **Seal the seed, not a transaction.** A sealed seed survives network upgrades, needs no unusual transaction fields and lets the heir claim with any wallet.
- **Fresh vault per epoch.** Revocation by sweeping is the only revocation that needs nothing from the heir or any server.
- **age and tlock.** Both have independent reference implementations and stable formats, so kits stay openable even if every Zandglass implementation disappears.
- **drand quicknet.** Public, unchained beacons every 3 seconds, operated by independent organisations.

# Deployment

This is a wallet-level convention. It needs no network upgrade and can be adopted by one wallet at a time. Kits made by one implementation open in any other that follows this document.

# Reference implementation

Zandglass: [github.com/0xBadSign/zandglass](https://github.com/0xBadSign/zandglass) (Rust: `zandglass-core`, `zandglass-wallet`, the `zandglass` CLI and owner panel; TypeScript: the heir and claim pages). Interoperability with `tle` and `age` is checked by `scripts/interop-kit.sh`.

# Test vectors

`crates/zandglass-core/tests/vectors/` holds a test-only heir phrase, its identity and heir code, a kit sealed to it at drand round 32665869, the expected payload and the recorded beacon. See [KIT_FORMAT.md](KIT_FORMAT.md) section 7.

# References

[^bcp14]: [Information on BCP 14: "RFC 2119" and "RFC 8174"](https://www.rfc-editor.org/info/bcp14)
[^bip39]: [BIP 39: Mnemonic code for generating deterministic keys](https://github.com/bitcoin/bips/blob/master/bip-0039.mediawiki)
[^age]: [age encryption format v1](https://age-encryption.org/v1)
[^drand]: [drand and timelock encryption (tlock)](https://drand.love/docs/timelock-encryption)
[^zip321]: [ZIP 321: Payment Request URIs](https://zips.z.cash/zip-0321)
[^zip317]: [ZIP 317: Proportional Transfer Fee Mechanism](https://zips.z.cash/zip-0317)

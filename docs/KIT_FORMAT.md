# Kit format v1

A kit is a UTF-8 JSON file, conventionally named `<plan>-e<epoch>.sandglass`. It has a public part and one secret part, the `sealed` field, which is a nested ciphertext (drand timelock around an age file around the payload). This document is the specification the Rust code (`crates/sandglass-core/src/kit.rs`, `seal.rs`) and the TypeScript code (`web/src/lib/kit.ts`) implement.

## 1. Public part

```json
{
  "format": "sandglass-kit",
  "version": 1,
  "kind": "legacy",
  "network": "test",
  "created": "2026-09-30T20:30:00Z",
  "plan": "0123456789abcdef0123456789abcdef",
  "epoch": 2,
  "unlock": {
    "drand_chain": "52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971",
    "round": 32665869,
    "time": "2026-09-30T20:42:51Z"
  },
  "sealed": "-----BEGIN AGE ENCRYPTED FILE-----\n...\n-----END AGE ENCRYPTED FILE-----\n"
}
```

The example is illustrative: the `sealed` value is abbreviated and is not a real ciphertext.

| Field | Type | Meaning |
|---|---|---|
| `format` | string | Always `sandglass-kit`. |
| `version` | integer | Always `1` in this release. |
| `kind` | string | `legacy` or `installment`. |
| `network` | string | `main` or `test`. |
| `created` | string | Creation time, RFC 3339 UTC, `YYYY-MM-DDTHH:MM:SSZ`. |
| `plan` | string | Random 128-bit plan id: exactly 32 lowercase hex characters. The same for every epoch of a plan. |
| `epoch` | integer | Vault epoch, at least 1. One more per flip. |
| `sequence` | object, optional | `{ "index": i, "count": n }`, 1-based, for installment kits. Omitted for legacy kits. |
| `unlock.drand_chain` | string | drand chain hash. Must be the quicknet hash shown above. |
| `unlock.round` | integer | The drand round whose beacon opens the outer layer. |
| `unlock.time` | string | That round's publication time, RFC 3339 UTC. |
| `sealed` | string | The armoured nested ciphertext (section 3). |

The public part never contains amounts, addresses, seeds or transaction ids.

## 2. Payload (inside `sealed`)

```json
{
  "vault_seed": "<24 words>",
  "birthday_height": 4423623,
  "network": "test",
  "plan": "0123456789abcdef0123456789abcdef",
  "epoch": 2,
  "message": "optional words from the owner",
  "restore_hints": ["...", "..."]
}
```

| Field | Meaning |
|---|---|
| `vault_seed` | The vault's BIP 39 24-word mnemonic. Account 0. |
| `birthday_height` | Block height to start scanning from when restoring. |
| `network` | `main` or `test`; must equal the public `network`. |
| `plan`, `epoch` | Must equal the public `plan` and `epoch`. |
| `message` | Optional text from the owner. Omitted when absent. |
| `restore_hints` | Human steps for restoring in common wallets and with `sandglass claim`. Default empty list. |

## 3. The sealed layers

```
sealed = ASCII armour( tlock_age( age( payload JSON, to 1..3 heir codes ), chain, round ) )
```

- **Inner:** a binary age v1 file encrypted to one to three X25519 recipients (the heir codes, `age1...`). Any one recipient can open it.
- **Outer:** the age file encrypted with the tlock scheme to `unlock.round` on drand quicknet, written as PEM-style ASCII armour (`-----BEGIN AGE ENCRYPTED FILE-----`). The armoured file's header holds a `tlock` stanza with the round and the chain hash, so the round can be read without any key.

## 4. Validation rules

`Kit::from_json` and `Kit::validate` (offline, no network, no secrets) reject a kit with `kit_invalid` unless:

- the JSON parses and `format` is `sandglass-kit`;
- `plan` is 32 lowercase hex characters and `epoch >= 1`;
- if `sequence` is present, `1 <= index <= count`;
- `unlock.drand_chain` is the pinned quicknet hash;
- `unlock.time` equals the time of `unlock.round` (`genesis + (round - 1) * 3`, formatted as above);
- the `sealed` armour parses, uses the pinned chain, and is locked to the same round as `unlock.round`.

A `version` other than 1 gives `kit_version_unsupported`; a missing `version` gives `kit_invalid`. Unknown fields are ignored, in the public part and in the payload. When opening, the payload's `network`, `plan` and `epoch` must match the public part, otherwise `decrypt_failed`. The keeper applies the same validation to every kit that is put into a mailbox and accepts at most 64 KiB.

The web pages also accept a bare armoured `sealed` string in place of the JSON file.

## 5. Versioning

`version` is an integer. A reader must reject a version it does not know. New optional fields may be added within version 1 because readers ignore unknown fields; any change that alters the meaning of an existing field, the layering or the validation rules needs version 2. Installment kits (`kind: installment`, `sequence`) are part of v1 and are validated, but the wallet does not create them yet (see `docs/HOW_IT_WORKS.md`).

## 6. Open a kit by hand with the reference tools

You need the kit, the heir's age identity file, and network access to the drand relays (the reference `tle` fetches the beacon itself). This is exactly what `scripts/interop-kit.sh` does, using the versions recorded in `docs/PROGRESS.md` (age 1.2.1, `tle` v1.2.0), and it works whether or not Sandglass is installed.

```bash
# 1. Take the sealed field out of the JSON kit (any JSON tool will do)
jq -r .sealed kit.sandglass > sealed.armor

# 2. Remove the timelock layer: needs the beacon of the kit's round, so it fails before the unlock time
tle --decrypt < sealed.armor > inner.age

# 3. Remove the heir layer with the heir's identity
age --decrypt -i identity.txt < inner.age > payload.json
```

`payload.json` is the payload of section 2. (A bare armoured file, such as the test vector `crates/sandglass-core/tests/vectors/kit.txt`, goes straight into step 2: `tle --decrypt < kit.txt | age --decrypt -i identity.txt`.)

`identity.txt` holds one line, the heir's age identity (`AGE-SECRET-KEY-1...`). If you only have the 24 words, derive it as in section 7.

Sealing by hand works the other way round (this is step 4 of `scripts/interop-kit.sh`; `<round>` is a drand quicknet round, which `sandglass` would put in `unlock.round`):

```bash
age -r <heir code> < payload.json | tle --encrypt -r <round> --force --armor > sealed.armor
```

The result is a valid `sealed` value. A full kit also needs the public part of section 1 (the `sandglass` tools open and inspect kits through it; `tle` and `age` alone do not need it).

## 7. How the heir identity derives from the 24 words

```
phrase (24 BIP 39 English words)
  -> BIP 39 seed   : 64 bytes, PBKDF2-HMAC-SHA512, empty passphrase
  -> HKDF-SHA256   : salt empty, info = "sandglass-heir-v1", output 32 bytes
  -> X25519 secret : the 32 bytes
  -> age identity  : Bech32 (HRP "age-secret-key-"), upper-cased: AGE-SECRET-KEY-1...
heir code = the matching age recipient string (age1...)
```

The same steps in Rust are `sandglass_core::heir::identity_from_phrase`, and in TypeScript `web/src/lib/heir.ts`. A stand-alone Node snippet with `@scure/bip39`, `@noble/hashes` and `@scure/base` (the packages the web pages use). It reads the phrase from a file so that the words never appear on a command line:

```js
import { readFileSync } from 'node:fs';
import { bech32 } from '@scure/base';
import { hkdf } from '@noble/hashes/hkdf.js';
import { sha256 } from '@noble/hashes/sha2.js';
import { mnemonicToSeedSync } from '@scure/bip39';

const phrase = readFileSync(process.argv[2], 'utf8').trim().split(/\s+/).join(' ');
const seed = mnemonicToSeedSync(phrase, '');
const secret = hkdf(sha256, seed, undefined, new TextEncoder().encode('sandglass-heir-v1'), 32);
console.log(bech32.encode('age-secret-key-', bech32.toWords(secret), 1023).toUpperCase());
```

Run it as `node identity.mjs heir.phrase > identity.txt` and keep the output as secret as the phrase.

**Test vector.** The files in `crates/sandglass-core/tests/vectors/` hold a test-only phrase and its identity (`TEST_ONLY_phrase.txt`, `TEST_ONLY_identity.txt`; never use them for anything real). The snippet above turns the first into the second, byte for byte, and the heir code for that phrase is:

```
age1y78z72dgwullphvyfstg8lregpf6377rge6wjg760aq032r5sgkqn7rvpa
```

The same directory holds a kit sealed to that heir code (`kit.txt`, locked to drand round 32665869), the expected payload (`payload.txt`) and the recorded beacon (`beacon.json`), so the full chain can be checked offline with `cargo test -p sandglass-core` and online with `bash scripts/interop-kit.sh`.

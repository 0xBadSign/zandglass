# Kit format v1

A kit is a UTF-8 JSON file, conventionally named `<plan>-e<epoch>.zandglass`. It has a public part and one secret part, the `sealed` field, which is a nested ciphertext (drand timelock around an age file around the payload). This document is the specification the Rust code (`crates/zandglass-core/src/kit.rs`, `seal.rs`) and the TypeScript code (`web/src/lib/kit.ts`) implement.

## 1. Public part

```json
{
  "format": "zandglass-kit",
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
| `format` | string | Always `zandglass-kit`. |
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
| `owner_key` | string, optional | The plan's Ed25519 public key: exactly 64 lowercase hex characters (section 8). |
| `owner_sig` | string, optional | Ed25519 signature over the public part and `sealed`: exactly 128 lowercase hex characters (section 8). Present if and only if `owner_key` is. |

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
  "practice": true,
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
| `practice` | Optional boolean. `true` only in a practice kit (section 9); omitted otherwise. |
| `restore_hints` | Human steps for restoring in common wallets and with `zandglass claim`. Default empty list. |

## 3. The sealed layers

```
sealed = ASCII armour( tlock_age( age( payload JSON, to 1..3 heir codes ), chain, round ) )
```

- **Inner:** a binary age v1 file encrypted to one to three X25519 recipients (the heir codes, `age1...`). Any one recipient can open it.
- **Outer:** the age file encrypted with the tlock scheme to `unlock.round` on drand quicknet, written as PEM-style ASCII armour (`-----BEGIN AGE ENCRYPTED FILE-----`). The armoured file's header holds a `tlock` stanza with the round and the chain hash, so the round can be read without any key.

## 4. Validation rules

`Kit::from_json` and `Kit::validate` (offline, no network, no secrets) reject a kit with `kit_invalid` unless:

- the JSON parses and `format` is `zandglass-kit`;
- `plan` is 32 lowercase hex characters and `epoch >= 1`;
- if `sequence` is present, `1 <= index <= count`;
- `unlock.drand_chain` is the pinned quicknet hash;
- `unlock.time` equals the time of `unlock.round` (`genesis + (round - 1) * 3`, formatted as above);
- the `sealed` armour parses, uses the pinned chain, and is locked to the same round as `unlock.round`;
- `owner_key` and `owner_sig` are either both absent or both well-formed, and the signature verifies (section 8).

A `version` other than 1 gives `kit_version_unsupported`; a missing `version` gives `kit_invalid`. Unknown fields are ignored, in the public part and in the payload. When opening, the payload's `network`, `plan` and `epoch` must match the public part, otherwise `decrypt_failed`. The keeper applies the same validation to every kit that is put into a mailbox and accepts at most 64 KiB.

The web pages also accept a bare armoured `sealed` string in place of the JSON file.

## 5. Versioning

`version` is an integer. A reader must reject a version it does not know. New optional fields may be added within version 1 because readers ignore unknown fields; any change that alters the meaning of an existing field, the layering or the validation rules needs version 2. `owner_key` and `owner_sig` (section 8) were added this way: kits without them stay valid. Installment kits (`kind: installment`, `sequence`) are part of v1; `zandglass installments split` and the owner panel create them (see `docs/INSTALLMENTS.md`).

## 6. Open a kit by hand with the reference tools

You need the kit, the heir's age identity file, and network access to the drand relays (the reference `tle` fetches the beacon itself). This is exactly what `scripts/interop-kit.sh` does, using the versions recorded in `docs/PROGRESS.md` (age 1.2.1, `tle` v1.2.0), and it works whether or not Zandglass is installed.

```bash
# 1. Take the sealed field out of the JSON kit (any JSON tool will do)
jq -r .sealed kit.zandglass > sealed.armor

# 2. Remove the timelock layer: needs the beacon of the kit's round, so it fails before the unlock time
tle --decrypt < sealed.armor > inner.age

# 3. Remove the heir layer with the heir's identity
age --decrypt -i identity.txt < inner.age > payload.json
```

`payload.json` is the payload of section 2. (A bare armoured file, such as the test vector `crates/zandglass-core/tests/vectors/kit.txt`, goes straight into step 2: `tle --decrypt < kit.txt | age --decrypt -i identity.txt`.)

`identity.txt` holds one line, the heir's age identity (`AGE-SECRET-KEY-1...`). If you only have the 24 words, derive it as in section 7.

Sealing by hand works the other way round (this is step 4 of `scripts/interop-kit.sh`; `<round>` is a drand quicknet round, which `zandglass` would put in `unlock.round`):

```bash
age -r <heir code> < payload.json | tle --encrypt -r <round> --force --armor > sealed.armor
```

The result is a valid `sealed` value. A full kit also needs the public part of section 1 (the `zandglass` tools open and inspect kits through it; `tle` and `age` alone do not need it).

## 7. How the heir identity derives from the 24 words

```
phrase (24 BIP 39 English words)
  -> BIP 39 seed   : 64 bytes, PBKDF2-HMAC-SHA512, empty passphrase
  -> HKDF-SHA256   : salt empty, info = "zandglass-heir-v1", output 32 bytes
  -> X25519 secret : the 32 bytes
  -> age identity  : Bech32 (HRP "age-secret-key-"), upper-cased: AGE-SECRET-KEY-1...
heir code = the matching age recipient string (age1...)
```

The same steps in Rust are `zandglass_core::heir::identity_from_phrase`, and in TypeScript `web/src/lib/heir.ts`. A stand-alone Node snippet with `@scure/bip39`, `@noble/hashes` and `@scure/base` (the packages the web pages use). It reads the phrase from a file so that the words never appear on a command line:

```js
import { readFileSync } from 'node:fs';
import { bech32 } from '@scure/base';
import { hkdf } from '@noble/hashes/hkdf.js';
import { sha256 } from '@noble/hashes/sha2.js';
import { mnemonicToSeedSync } from '@scure/bip39';

const phrase = readFileSync(process.argv[2], 'utf8').trim().split(/\s+/).join(' ');
const seed = mnemonicToSeedSync(phrase, '');
const secret = hkdf(sha256, seed, undefined, new TextEncoder().encode('zandglass-heir-v1'), 32);
console.log(bech32.encode('age-secret-key-', bech32.toWords(secret), 1023).toUpperCase());
```

Run it as `node identity.mjs heir.phrase > identity.txt` and keep the output as secret as the phrase.

**Test vector.** The files in `crates/zandglass-core/tests/vectors/` hold a test-only phrase and its identity (`TEST_ONLY_phrase.txt`, `TEST_ONLY_identity.txt`; never use them for anything real). The snippet above turns the first into the second, byte for byte, and the heir code for that phrase is:

```
age16rj3u4ertnvr8yhntwpq9kfaq0k3cyscv5xkrtgjlne9dtzjdc4s5awu04
```

The same directory holds a kit sealed to that heir code (`kit.txt`, locked to drand round 32665869), the expected payload (`payload.txt`) and the recorded beacon (`beacon.json`), so the full chain can be checked offline with `cargo test -p zandglass-core` and online with `bash scripts/interop-kit.sh`.

## 8. Owner authentication (optional)

A mailbox operator, or anyone else who can write to the place an heir fetches the kit from, can put a decoy kit there: a kit sealed to the same heir code with a different vault seed. Anyone can seal a kit to a public heir code, so the kit alone proves nothing about who made it. To let the heir's page notice a swapped kit, every plan gets one random **Ed25519 signing key**, created together with the plan, and each kit of the plan carries the public key and a signature.

### Fields

- `owner_key`: the 32-byte Ed25519 public key, 64 lowercase hex characters.
- `owner_sig`: the 64-byte Ed25519 signature, 128 lowercase hex characters.

Both are public and of fixed length, so a signed kit is a constant number of bytes larger than an unsigned one of the same plan type (about 250 bytes) and the kit's size still reveals nothing about the plan. The key is random and belongs to the plan: it is not derived from the vault seeds or the owner's wallet and is the same for every epoch of one plan, which the plan id already links (see `docs/PRIVACY.md`, R9).

### What is signed

The signature covers this exact byte string, in UTF-8. It is not a JSON serialisation. Every line ends in a line feed (`\n`), including the last one; there is no other whitespace.

```
zandglass-kit-sig-v1
format=<format>
version=<version>
kind=<kind>
network=<network>
created=<created>
plan=<plan>
epoch=<epoch>
sequence=<index>/<count>        (or  sequence=none  when the kit has no sequence)
unlock.drand_chain=<unlock.drand_chain>
unlock.round=<unlock.round>
unlock.time=<unlock.time>
owner_key=<owner_key>
sealed_sha256=<lowercase hex SHA-256 of the UTF-8 bytes of the sealed string>
```

Integers are written in decimal without padding. Every other value is the field's string exactly as it appears in the kit; a value that contains a line feed makes the signature invalid. `owner_sig` is not part of the message. `sealed` is covered through its SHA-256, so changing even one character of the ciphertext breaks the signature. Ed25519 is the pure scheme of RFC 8032 (no context, no prehash); verifiers use strict verification (reject non-canonical encodings).

The reference message for the test vector is `crates/zandglass-core/tests/vectors/TEST_ONLY_owner_message.txt`, the signed kit is `TEST_ONLY_owner_kit.zandglass` and the key is described in `TEST_ONLY_owner_key.txt`. The Rust test (`crates/zandglass-core/tests/owner_sig.rs`) regenerates all three and fails if a committed file differs (Ed25519 signatures are deterministic); the TypeScript test (`web/src/lib/owner-sig.test.ts`) builds the same message and verifies the same kit.

### Fingerprint

The fingerprint of an owner key is the first eight bytes of the SHA-256 of the 32 key bytes, as lowercase hex in four groups of four: `xxxx-xxxx-xxxx-xxxx`. It is for people to compare; software that pins a key pins the whole public key.

### Verification rules

- A kit with neither field is **unsigned**. It is valid, and old kits and kits of plans created before this feature stay that way.
- A kit with exactly one field, a malformed field (wrong length, upper case, not hex) or a signature that does not verify is **invalid**: parsers reject it with `kit_invalid`. The keeper applies the same rule when a kit is uploaded, and `zandglass claim` refuses it.
- A valid signature proves that the holder of the plan's signing key sealed this exact kit. It does not say who that holder is: the signature is only useful by comparison with a key seen earlier.
- **Pinning.** An heir page or tool that has seen a signed kit of a plan remembers its `owner_key` (the web page in localStorage, per plan id). A later kit of the same plan that is signed by another key, or that carries no signature at all, was not produced by the same owner and must be shown as a warning, not opened silently. The first signed kit seen is trusted on first use: an attacker who controls the very first kit an heir sees is not detected. The owner can close that gap by giving the heir the key fingerprint out of band (`zandglass kit inspect` and `zandglass claim` print it).
- **Pinning by mailbox.** The plan id is written by whoever made the kit, so a decoy can bring a fresh plan id and slip past a pin keyed on it. A page that loads a kit from a claim link therefore also pins the key and plan id per mailbox id (which comes from the link). A different key, or the same key under a different plan id, on a mailbox seen before is a mismatch warning, and an unsigned kit on a mailbox that served a signed one counts as stripped. An unsigned kit from a mailbox with no earlier signed kit is shown as a visible warning, not as a neutral note.
- Unknown versions, unknown fields and the kit layering are unchanged: this is an additive part of version 1.

### Claim link owner hint

Trust on first use leaves one gap: the first kit an heir sees. For a signed plan the claim link closes it. The link `https://<keeper>/claim#m=<id>&r=<read token>&o=<hint>` carries `o`, the first 16 bytes of the SHA-256 of the 32 owner public key bytes as 32 lowercase hex characters. The owner's tool builds it from the plan's key when the plan is created (installment plans: one link per installment, all with the same hint). The hint lives in the fragment, so it is never sent to the keeper or any server, and it reveals nothing the kit does not already show (the key is public in every kit and the hint is a hash of it).

When `o` is present the loaded kit MUST be signed (valid signature) by a key whose hint equals `o`. An unsigned kit, a kit signed by another key, or a malformed `o` is a hard error: the claim page says "This kit was not sealed by the owner who sent you this link", drops the kit and offers no way to open it, and `zandglass claim` refuses it with `kit_invalid`. When `o` is absent (links made before this feature, plans without a key) the pinning rules above apply. Paper kits and kit files come without a link and are unaffected.

### Where the key lives

The owner's store keeps the signing key in `vaults/<plan>/owner_key.age`, encrypted with the store passphrase like the vault seeds (`seed.age`); it is never written in plaintext and never leaves the owner's machine. The key opens no funds and no kit: a stolen key lets a thief sign a decoy kit that the heirs' pages would treat as genuine for that plan, so it is protected like the seeds. `plans.json` holds no key material; it stays HMAC-authenticated. A plan without the file (created by an older version) keeps working and produces unsigned kits.

## 9. Practice kits

A practice kit is an ordinary kit used for the heir fire drill, so that an heir can rehearse the claim long before it matters. The heir page (`/heir`) makes it in the browser (`sealPracticeKit` in `web/src/lib/kit.ts`), and the claim page opens it.

- The payload has an empty `vault_seed` (`""`) and `practice: true` (the optional field of section 2); the message says that it holds no funds.
- It is sealed to the heir's own heir code, with an unlock about 60 seconds after it was made.
- It is unsigned (no `owner_key`, no `owner_sig`), holds no funds and no seed, and is never uploaded: it stays in the browser (and in a file if the heir downloads it).
- The public part is the same as for any other `legacy` kit and nothing in it marks the kit as a practice kit; only the payload does, so the claim page shows a practice banner after opening it. Readers that do not know `practice` ignore it and would show an empty seed.

# Security review (2026-10-25)

An independent, read-only review of the whole code base (Rust crates, web pages, keeper and playground, release workflows) by a separate AI auditor that had not written the code. The question it was given first: can anybody other than the designated heir open a vault, or open it early?

## Result

**No critical finding.** The review found no way for anyone but the heir to decrypt a kit or learn a vault seed, no way to open a kit before its drand round, and no way to read another user's mailbox or playground session. Areas checked and found sound:

- Kit sealing: drand timelock outside, age to the heir codes inside, always three recipients with padding, payload checked against the public part; heir key derivation identical in Rust and TypeScript; the phrase is never stored or sent; no `innerHTML`.
- Early opening: beacons are verified against the pinned quicknet key and round; the clock is used only for display.
- Keeper: 128-bit ids, 256-bit tokens stored hashed and compared in constant time, separate read and write tokens, kits canonicalised.
- Owner panel: Host-header check, 192-bit bearer token, strict CSP, idle lock.
- Store and fund logic: scrypt (2^18), HMAC'd plans, private file modes; funds only ever move to the plan's new vault or to an address the user gives.
- On-chain privacy: default fields, ZIP 317 fee, no memos, `OvkPolicy::Discard`.
- Dependencies: no git or patched dependencies; `--locked` and `npm ci`.

## Findings and fixes

| ID | Severity | Finding | Fix |
|---|---|---|---|
| H1 | High | The keeper deleted a mailbox 180 days after its last update even if its kit had not unlocked yet (long flip periods, late installments). | Retention is unlock-aware: a mailbox is kept until 180 days after its kit unlocks (existing databases migrate on start). |
| M1 | Medium | The owner-key pin was keyed on the plan id, which a decoy kit controls. | Claim links carry an owner-key hint (`&o=`, in the URL fragment); a kit not signed by that key is refused by the claim page and the CLI. Pins are also kept per mailbox; an unsigned kit from a mailbox is shown as a warning. |
| M2 | Medium | Playground funding could repeat after a database error or an unknown broadcast outcome. | Funding is recorded before it is sent, never retried after an unknown outcome, and never sent to a vault that already holds funds. |
| M3 | Medium | The playground could delete a session's vault seeds while a late payment sat in them. | Before forgetting a session every vault is synced; funds go back to the demo float, and anything not provably empty is archived, never deleted. An uncertain flip keeps tracking the new vault. |
| M4 | Medium | A savings lock accepted a pasted heir code with no proof the owner holds its words. | A lock is created from the owner's 24 words (derived locally, never sent); a code fingerprint is shown for every heir code. |
| M5 | Medium | One address could exhaust the keeper's mailbox capacity. | Batches are charged per mailbox, with a per-address hourly quota and a separate cap for empty mailboxes. |
| M6 | Medium | Release workflow actions were not pinned. | Every action is pinned to a commit; release builds use no cache; artifacts carry a build-provenance attestation. |
| L3 | Low | Restore hints inside a kit were shown without the caution note given to the message. | The caution note now covers both. |

Other low items are documented: the keeper's origin is reachable only through the Cloudflare tunnel (it listens on 127.0.0.1), and the playground judge code is long and rate limited.

All fixes are covered by tests (Rust, vitest and Playwright; 92 browser tests) and were exercised on the real networks: a testnet savings lock created from the owner's words with a link whose wrong owner hint is refused, and a full mainnet playground run (`docs/evidence/EVIDENCE.md`, 2026-10-26 rows).

Zandglass remains unaudited by a human security firm; mainnet plans are capped.

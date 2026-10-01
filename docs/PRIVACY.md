# Privacy and security model

Sandglass is judged, first of all, on whether it leaks. This document states the threat model, who learns what, what metadata remains, and what the design trusts. It is written to be checked against the code, the evidence file and the tests it cites. The software is unaudited.

## 1. Goal and threat model

**Goal.** Let a named person (an heir) claim shielded funds only after a chosen time, unless the owner keeps checking in, without anything on chain that marks a transaction as Sandglass, and without any party we operate being able to open a kit.

**Adversaries considered**

- Anyone reading the Zcash chain.
- The drand network and its relays.
- The keeper operator (a service that stores kits).
- The lightwalletd operator.
- The heir, before and after the unlock.
- A thief of a kit file or of a claim link.
- A thief of the heir phrase.
- A thief of the owner's device.

**Out of scope.** A compromised owner or heir device while the secret is in use; coercion; a break of Zcash shielded pools, age (X25519) or BLS12-381; a threshold of drand nodes colluding to publish early (it would remove the time gate only, never the heir gate).

## 2. Who learns what

| Party | Learns | Does not learn |
|---|---|---|
| Chain observers | Ordinary shielded transactions that look like any wallet's: funding, flips, a claim sweep | That a plan exists, the heir, amounts, unlock times, any link between vault epochs (each epoch is a fresh seed and address; `OvkPolicy::Discard` on every sweep) |
| drand network and relays | That someone fetched a public beacon, and when (and the fetcher's IP, as any HTTPS server sees) | Anything about a kit. The beacon is public data; the kit is never sent |
| Keeper | That a mailbox exists; update times; the public part of every kit it stores (kind, network, plan id, epoch, created time, unlock round and time); the sealed ciphertext; hashes of the tokens | Owner or heir identity (no accounts), amounts, addresses, kit contents. It never has a key that opens a kit, before or after the unlock. It logs no requests |
| lightwalletd | The light client's IP and sync timing; the transactions the wallet broadcasts | Keys and balances (compact blocks are scanned locally). It can withhold or serve stale data, not spend |
| Heir, after the unlock | The current vault's seed, so its balance and its one incoming transfer (intended) | Where earlier vaults sent funds (`OvkPolicy::Discard`), the owner's other funds |
| Thief of a kit (or claim link) | The public part. Nothing else, before or after the unlock | The payload: the heir key is also required |
| Thief of the heir phrase | Nothing without a kit. With a kit, the vault, after the unlock | |
| Thief of the owner's store files | `plans.json`: heir codes (public), plan ids, mailbox write token (opens no funds and no kit). Each vault's `wallet.sqlite`: its viewing key and transaction history (amounts, times). Files are owner-only (0600/0700) but not encrypted | Seeds (encrypted with the passphrase: age, scrypt), unless the passphrase is weak or known |

## 3. Residual metadata

- **R1 Light-wallet network metadata.** lightwalletd sees the IP and sync timing, as with every Zcash light wallet. Mitigation: `--server <url>` selects your own lightwalletd. Tor is not implemented.
- **R2 Mailbox metadata.** The keeper sees when a kit is replaced (flip times) and the public part of the kit, including the epoch number, so it can count flips of a mailbox, and the unlock time. The keeper never sees an identity, and keeps no request logs (its own code; a reverse proxy in front of it is the operator's responsibility). Mitigation: skip the mailbox and hand kit files to heirs directly.
- **R3 drand fetch timing.** A claimant fetches the beacon of round R, which is public and cacheable. The fetch reveals to a relay (and to whoever watches the heir's network) that someone is opening a kit around that time. The heir page and the CLI try several relays in order.
- **R4 Owner race after the unlock.** By design. Mitigation: flip on time. Reminders are not implemented.
- **R5 Pool transitions (reviewed).** Vault addresses carry an Orchard receiver. After NU6.3 the library delivers payments to that receiver through the Ironwood bundle and keeps change in the pool of the inputs, so a flip spends Ironwood notes and creates Ironwood notes, like any up-to-date wallet. Sandglass's own fallback change pool follows NU6.3 (Ironwood from activation on). The funding transaction from an older wallet that still holds Orchard notes crosses the turnstile like any other payment from that wallet.
- **R6 Installment splits.** One transaction creates N+1 outputs, which shows as a larger action count (amounts stay hidden). Ordinary batch payments look the same; prefer small N.
- **R7 Birthday queries.** The owner and the heir both ask lightwalletd for the tree state at the vault's birthday. Sandglass randomises the birthday (tip - 100 - up to 200 blocks) so the query is not an exact link; using different lightwalletd servers or Tor removes it.
- **Playground.** In the hosted playground the server plays the owner, so it holds each session vault's seed (encrypted on disk) for the length of the session, and it learns the destination address if the visitor chooses "send to my address". This is the main mock and it is labelled as such in the UI (see `docs/PLAYGROUND_API.md`).

## 4. Fingerprint analysis

Every Sandglass transaction is built by the standard `zcash_client_backend` flow (`propose_send_max_transfer` or `propose_standard_transfer_to_address`, then `create_proposed_transactions`) at the moment it is sent. The code sets no expiry, anchor, fee or memo of its own. Fields from the testnet flip in `docs/evidence/EVIDENCE.md` (tx `cbc4dcbab10480fa6dc7b67e575fe439a6288292191f391e844245abbe66e0c3`):

| Field | Sandglass transaction | A pre-signed transaction (rejected design) |
|---|---|---|
| Expiry height | 4423823 = target height 4423783 + 40, the library default | 0 (no expiry), which is rare |
| Anchor | Height 4423780, 3 blocks below the target height: the default confirmations policy, a recent anchor | Old, fixed at signing time |
| Fee | 10,000 zat, the ZIP 317 fee for the minimum two logical actions | Whatever was fixed at signing time |
| Outputs recoverable by the sender's keys | No (`OvkPolicy::Discard`) | Depends |
| Memo | None added by Sandglass | |
| Broadcast time | When the owner (or heir) runs the command | Anyone, any time after unlock |
| Consensus branch | The current one at send time | Fixed at signing time, so stale after an upgrade |

The end-to-end run (plan `3166779e`) logs the same standard fields for its funding transaction (expiry = target + 40, fee 10,000 zat), and lists its flip and claim transactions. A systematic comparison of many transactions against ordinary wallets (task T4.1) has not been done yet. By construction the fields come from the same library code as any other wallet that uses `zcash_client_backend`; the claim is that Sandglass adds nothing of its own, not that the default values are shared by every wallet.

## 5. Why the pre-signed design was rejected

The first plan sealed fully signed transactions with `nExpiryHeight = 0`, to be broadcast by anyone after the unlock. That design was dropped (decision D-009) for two reasons:

1. **On-chain fingerprints.** Expiry 0 is rare and the anchor is old by the time the transaction is broadcast. Such a transaction is recognisable as pre-signed, which is exactly the kind of leak the project must not have.
2. **Staleness.** The signature commits to the consensus branch ID, so every network upgrade invalidates the transaction unless variants are prepared. That is fragile over years.

The sealed-vault design stores keys, so nothing about it needs to be pre-built. The idea may return later only as an opt-in "auto-delivery" mode with the trade-off stated; it is not implemented.

## 6. Trust assumptions

- **drand (quicknet).** Threshold honesty: a colluding threshold could publish a beacon early, opening the timelock layer but not the heir layer. Liveness: if drand halts, kits cannot open until it resumes; the funds stay in the owner's vault. The chain hash and public key are pinned in the code and in the web build.
- **Devices.** The security of the owner's and the heir's devices and of the heir's backup of 24 words. A phrase thief who also gets the kit can take the vault after the unlock; several heirs are possible but any one of them can claim.
- **lightwalletd.** Any server will do (`https://zec.rocks:443` by default); it learns network metadata (R1) but cannot spend.
- **Cryptography.** age (X25519, ChaCha20-Poly1305), tlock over BLS12-381, Zcash shielded protocols, and the libraries that implement them. None of it has been audited for this project.
- **The keeper is untrusted for secrecy of kits.** It stores only ciphertext and the public part. It is trusted only for availability: if it disappears or deletes a mailbox, the owner still has the kit files and can give them to the heirs.
- **Whoever serves the web pages is trusted for the page code.** The heir types the phrase into the heir page. If the keeper also serves the pages, a malicious operator could ship code that exfiltrates the phrase. Heirs who do not want to trust the host can use the CLI (`sandglass claim`), a self-built copy of `web/`, or the reference tools (`tle`, `age`).

## 7. What the web pages send

The landing page, the heir page and the claim page are static pages. They send requests only to:

- their own origin (static assets);
- the drand relays `api.drand.sh`, `api2.drand.sh` and `drand.cloudflare.com` (a GET for a public beacon);
- the keeper, for the claim page (`GET /v1/mailboxes/<id>/kit` with the read token in an `Authorization` header) and for the playground (`/v1/playground/*`).

The heir phrase, the heir identity and the decrypted vault seed are created and used in the browser only. The claim link keeps the mailbox id and read token in the URL fragment, which browsers do not send to servers on page load. Requests use `credentials: 'omit'` and `referrerPolicy: 'no-referrer'`. The pages have no analytics or third-party scripts, and the Content-Security-Policy, served by the keeper on every response and mirrored in `web/vite.config.ts` for previews, is `default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'` plus the three drand hosts. The playground stores the judge code in `sessionStorage` for the tab only.

The playground create request carries only `heir_code` (public), `judge_code` and `minutes`; settle requests carry only `action` and, for "send", the destination address.

**Playwright leak tests (`web/e2e/`).** These record every request the pages make and fail on:

- `app.spec.ts` (heir and claim pages): any host outside this origin and the three drand hosts; any word of the heir phrase or of the decrypted seed found in any request URL, query, header or body (same-origin static asset paths are excluded from the word search, the rest is searched in full).
- `playground.spec.ts` (playground flow with a mocked keeper and a recorded drand beacon): any host outside the allowlist; the string `age-secret-key`; any three consecutive words of the phrase or seed in any request; a create body with keys other than `heir_code`, `judge_code` and `minutes`; a settle body with keys other than `action` and `to`.
- `landing.spec.ts`: the landing page contacts nothing but its own origin.

These tests show that the pages do not send secrets in the flows they exercise. They are not a proof for flows they do not exercise.

## 8. Software security notes

- Vault seeds are encrypted at rest with an age passphrase (scrypt, work factor 18) and written with owner-only permissions on Unix. A claim restores into a throwaway directory and deletes it.
- Secrets are wrapped in zeroizing types where the libraries allow. The CLI prints the heir phrase once and writes it to a file only with `--save` (mode 600).
- The mailbox read and write tokens are 256-bit random values; the keeper stores only SHA-256 hashes and compares in constant time.
- No telemetry, no accounts, no IP logging in the keeper. Rate limiting is in memory and never persisted.
- Mainnet is capped at 0.25 ZEC per plan unless `--accept-unaudited` is given (enforced at `--amount` on plan creation and at flip; see `docs/HOW_IT_WORKS.md`).

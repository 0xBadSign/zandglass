# Playground API v1 (contract between the keeper and the web UI)

The playground lets a judge try Sandglass on a real network in a few minutes.
The server plays the **owner** with a small float wallet. The visitor plays the
**heir**: the heir key is created in the visitor's browser and never leaves it.
All endpoints are under the keeper origin and return JSON. Errors use
`{ "error": "<code>", "message": "<plain English>" }` (PLAN 5.10 codes plus
`judge_code_invalid`, `playground_closed`, `playground_busy`, `daily_cap_reached`,
`session_not_found`, `invalid_state`, `invalid_request`, `heir_code_invalid`).

## Configuration (environment, keeper side)

| Variable | Meaning |
|---|---|
| `SANDGLASS_PLAYGROUND_NETWORK` | `main` or `test`; unset disables the playground |
| `SANDGLASS_PLAYGROUND_FLOAT_SEED_FILE` | file with the float wallet's 24 words (root-only, read once, kept in memory only) |
| `SANDGLASS_PLAYGROUND_FLOAT_BIRTHDAY` | float wallet birthday height |
| `SANDGLASS_PLAYGROUND_DATA` | directory for the float wallet, the session vaults and the session database |
| `SANDGLASS_PLAYGROUND_SECRET` | passphrase (at least 12 characters) that encrypts the session vault seeds on disk |
| `SANDGLASS_PLAYGROUND_JUDGE_CODE_SHA256` | hex SHA-256 of the judge code |
| `SANDGLASS_PUBLIC_ORIGIN` | public origin of the site, used in claim links (`https://...`) |
| `SANDGLASS_EXPLORER_TX_URL` | optional block explorer template containing `{txid}`; without it `explorer` is `null` |
| `SANDGLASS_PLAYGROUND_AMOUNT_ZAT` | per session, default 20000 (0.0002 ZEC) |
| `SANDGLASS_PLAYGROUND_MAX_ACTIVE` | default 10 |
| `SANDGLASS_PLAYGROUND_MAX_PER_DAY` | default 60 (UTC day) |
| `SANDGLASS_PLAYGROUND_MAX_KEEPS_PER_DAY` | default 5 (sessions whose funds the visitor keeps) |
| `SANDGLASS_PLAYGROUND_MAX_FLIPS` | default 3, flips per session |
| `SANDGLASS_PLAYGROUND_MIN_CONFIRMATIONS` | default 1, at least 1. Confirmations before funds count as spendable, for the float and every session vault. The wallet library's own default (3 trusted, 10 untrusted, about 12 minutes on testnet) is too slow for a demo; the CLI keeps that default |
| `SANDGLASS_LWD_URL` | lightwalletd URL (default per network) |

## Behaviour notes

- **Funding amount.** The vault receives `amount_zat` plus one ZIP 317 sweep fee
  (10000 zat) for every allowed flip and one more for the final sweep, so with
  the defaults 20000 + 4 x 10000 = 60000 zat. The float gets back what is left
  after the sweeps. `balance_zat` is the vault's real balance.
- **Seeds.** Each session vault has a fresh seed. The server keeps it on disk
  encrypted with an age passphrase (`SANDGLASS_PLAYGROUND_SECRET`, via the wallet
  store) until the session is purged, because the server must be able to sweep
  the vault. The float seed is never written by the keeper.
- **Wallet queue.** All wallet operations run one at a time behind a single
  lock. The wallet is re-synced before every transaction and an expiry
  rejection is retried once.
- **State timing.** `funding` lasts until the funding transaction is confirmed
  (spendable). Then the state is `sealed`, or `unlocked` if the drand round has
  already passed. `sealed` becomes `unlocked` when the round passes.
- **Flip** needs `sealed`, confirmed funds and fewer than `max_flips` flips. The
  state becomes `flipping`, then `sealed` again with `epoch + 1`, a new
  `unlock_round` and `confirmed: false` until the swept funds confirm. The
  `claim_link` does not change; the kit behind it is replaced. If the flip
  fails three times the session returns to `sealed` unchanged.
- **Settle** needs `unlocked`; the state becomes `settling` and the worker
  sweeps as soon as the funds are spendable. `keep` is immediate. When the daily
  keep cap is reached `keep` answers `daily_cap_reached` and the visitor can
  still `return` or `send`. A kept vault that nobody emptied is returned to the
  float after 24 hours. If a settle finds the vault already empty (the visitor
  moved it with the seed) the session ends as `kept`.
- **Failure.** `failed` means the funding or the sweeps could not be completed;
  `error` explains in plain English. Failed rows are kept 7 days for manual
  recovery.
- **Privacy.** The session row holds the public heir code, the claim link,
  timestamps and the state. A `send` destination is kept only until the sweep is
  done. No IPs and no request logs. Settled rows (and their wallets and
  mailboxes) are purged 7 days after settlement.

## Endpoints

### `GET /v1/playground/status`
`{ "enabled": true, "network": "main", "amount_zat": 20000, "active": 2, "max_active": 10, "today": 7, "max_per_day": 60, "float_ready": true }`
When the playground is disabled: `{ "enabled": false }`.

### `POST /v1/playground/sessions`
Body: `{ "judge_code": "...", "heir_code": "age1...", "minutes": 3 | 5 | 10 }`
Creates a vault (epoch 1), seals the kit to the heir code with the chosen timer
and stores it in a keeper mailbox. The funding transaction is sent by the
background worker.
Response `201`:
```json
{ "id": "hex", "token": "hex (session bearer token)", "claim_link": "https://<origin>/claim#m=..&r=..",
  "network": "main", "amount_zat": 20000, "minutes": 5 }
```
Errors: `403 judge_code_invalid`, `400 heir_code_invalid`, `400 invalid_request`
(bad JSON or timer), `503 playground_busy` (active cap, float not ready or out
of funds), `429 daily_cap_reached`, `503 playground_closed` (not enabled),
`429 rate_limited` (per IP, in memory only).

### `GET /v1/playground/sessions/{id}` (header `Authorization: Bearer <token>`)
```json
{ "id": "hex", "state": "funding | sealed | flipping | unlocked | settling | returned | sent | kept | failed",
  "epoch": 1, "unlock_time": "2026-10-02T12:05:00Z", "unlock_round": 32700000,
  "balance_zat": 60000, "confirmed": true, "flips": 0, "max_flips": 3,
  "txs": [ { "kind": "fund | flip | return | send", "txid": "hex", "explorer": "https://... or null" } ],
  "claim_link": "https://...", "message": "plain English status line", "error": null }
```
Errors: `401 unauthorized`, `404 session_not_found`.

### `POST /v1/playground/sessions/{id}/flip`
Moves the funds to a new vault, re-seals with a fresh timer of the same length and
replaces the mailbox kit. Allowed in `sealed` (funds confirmed). Returns the session
(state `flipping`; poll until `sealed`). Errors: `409 invalid_state`.

### `POST /v1/playground/sessions/{id}/settle`
Body: `{ "action": "return" }` sweeps the vault back to the float;
`{ "action": "send", "to": "<unified address>" }` sweeps to the visitor (`400
network_mismatch` for an invalid address);
`{ "action": "keep" }` marks the session as kept (the visitor restores the seed
in their own wallet; counted against the daily keep cap; after 24 h an unclaimed
vault may be returned).
Allowed in `unlocked`. Returns the session. Errors: `409 invalid_state`,
`400 invalid_request`, `429 daily_cap_reached` (keep only).

Sessions auto-return to the float 30 minutes after unlock if not settled.
Nothing about the visitor is stored beyond the session row (no IPs).

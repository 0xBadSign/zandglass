# Playground API v1 (contract between the keeper and the web UI)

The playground lets a judge try Sandglass on a real network in a few minutes.
The server plays the **owner** with a small float wallet. The visitor plays the
**heir**: the heir key is created in the visitor's browser and never leaves it.
All endpoints are under the keeper origin and return JSON. Errors use
`{ "error": "<code>", "message": "<plain English>" }` (PLAN 5.10 codes plus
`judge_code_invalid`, `playground_closed`, `playground_busy`, `daily_cap_reached`,
`session_not_found`, `invalid_state`).

## Configuration (environment, keeper side)

| Variable | Meaning |
|---|---|
| `SANDGLASS_PLAYGROUND_NETWORK` | `main` or `test`; unset disables the playground |
| `SANDGLASS_PLAYGROUND_FLOAT_SEED_FILE` | file with the float wallet's 24 words (root-only, read once) |
| `SANDGLASS_PLAYGROUND_FLOAT_BIRTHDAY` | float wallet birthday height |
| `SANDGLASS_PLAYGROUND_DATA` | directory for the float and session vault wallets |
| `SANDGLASS_PLAYGROUND_JUDGE_CODE_SHA256` | hex SHA-256 of the judge code |
| `SANDGLASS_PLAYGROUND_AMOUNT_ZAT` | per session, default 20000 (0.0002 ZEC) |
| `SANDGLASS_PLAYGROUND_MAX_ACTIVE` | default 10 |
| `SANDGLASS_PLAYGROUND_MAX_PER_DAY` | default 60 |
| `SANDGLASS_PLAYGROUND_MAX_KEEPS_PER_DAY` | default 5 (sessions whose funds the visitor keeps) |
| `SANDGLASS_LWD_URL` | lightwalletd URL (default per network) |

## Endpoints

### `GET /v1/playground/status`
`{ "enabled": true, "network": "main", "amount_zat": 20000, "active": 2, "max_active": 10, "today": 7, "max_per_day": 60, "float_ready": true }`

### `POST /v1/playground/sessions`
Body: `{ "judge_code": "...", "heir_code": "age1...", "minutes": 3 | 5 | 10 }`
Creates a vault (epoch 1), funds it from the float, seals the kit to the heir
code with the chosen timer and stores it in a keeper mailbox.
Response `201`:
```json
{ "id": "hex", "token": "hex (session bearer token)", "claim_link": "https://<origin>/claim.html#m=..&r=..",
  "network": "main", "amount_zat": 20000, "minutes": 5 }
```
Funding happens in the background; poll the session.

### `GET /v1/playground/sessions/{id}` (header `Authorization: Bearer <token>`)
```json
{ "id": "hex", "state": "funding | sealed | flipping | unlocked | settling | returned | sent | kept | failed",
  "epoch": 1, "unlock_time": "2026-10-02T12:05:00Z", "unlock_round": 32700000,
  "balance_zat": 20000, "confirmed": true,
  "txs": [ { "kind": "fund | flip | return | send", "txid": "hex", "explorer": "https://..." } ],
  "claim_link": "https://...", "message": "plain English status line", "error": null }
```
`state` is `unlocked` once the drand round has passed (the visitor can now open the kit).

### `POST /v1/playground/sessions/{id}/flip`
Moves the funds to a new vault, re-seals with a fresh timer of the same length and
replaces the mailbox kit. Allowed in `sealed` (funds confirmed). Returns the session.

### `POST /v1/playground/sessions/{id}/settle`
Body: `{ "action": "return" }` sweeps the vault back to the float;
`{ "action": "send", "to": "<unified address>" }` sweeps to the visitor;
`{ "action": "keep" }` marks the session as kept (the visitor restores the seed
in their own wallet; counted against the daily keep cap; after 24 h an unclaimed
vault may be returned).
Allowed after `unlocked`. Returns the session.

Sessions auto-return to the float 30 minutes after unlock if not settled.
Nothing about the visitor is stored beyond the session row (no IPs).

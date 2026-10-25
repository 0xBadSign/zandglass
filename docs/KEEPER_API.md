# Keeper API v1 (blind mailbox)

The keeper (`zandglass-keeper`) stores the latest kit of a plan so that an heir can open a claim link without the owner handing over a file. It is **blind**: it stores the kit as it received it and has no key that opens one, before or after the unlock. Using a keeper is optional; kits are ordinary files.

The playground endpoints (`/v1/playground/*`) are documented in [PLAYGROUND_API.md](PLAYGROUND_API.md). This document covers the mailbox endpoints and the server itself.

## Endpoints

All bodies are JSON. Tokens go in `Authorization: Bearer <token>`.

| Method and path | Auth | Purpose | Success |
|---|---|---|---|
| `GET /v1/health` | none | Liveness | `200 { "ok": true, "version": "0.1.0" }` |
| `POST /v1/mailboxes` | none | Create a mailbox, or a batch with an optional body `{ "count": N }` (1 to 120) | `201 { "id", "write_token", "read_token" }`; for `count` above 1: `201 { "mailboxes": [ { "id", "write_token", "read_token" }, ... ] }` |
| `PUT /v1/mailboxes/{id}/kit` | write token | Replace the mailbox's kit (the previous kit is overwritten) | `200 { "ok": true }` |
| `GET /v1/mailboxes/{id}/kit` | read token | The latest kit, exactly as stored (`application/json`) | `200` |
| `DELETE /v1/mailboxes/{id}` | write token | Delete the mailbox | `200 { "ok": true }` |

- `id` is 128 random bits as 32 hex characters; each token is 256 random bits as 64 hex characters. The keeper stores only SHA-256 hashes of the tokens and compares in constant time.
- The **write token** stays with the owner (the CLI stores it in the plan). The **read token** goes to the heir inside the claim link. Neither token opens funds or a kit.
- A claim link is `<keeper>/claim#m=<id>&r=<read token>` (the heir page). Older links ending in `/claim.html#...` are also accepted, and the keeper redirects `/claim.html` (and the other `.html` pages) to the clean address. The heir page also accepts an optional `&k=<keeper origin>` when the page and the keeper live on different origins. Links of signed plans carry a further `&o=<owner key hint>` (the first 16 bytes of the SHA-256 of the plan's owner public key, 32 lowercase hex characters, see `docs/KIT_FORMAT.md` section 8): the claim page and `zandglass claim` then refuse any kit that is not signed by that key. The fragment is not sent to any server by the browser; the page uses it to build the `GET` request, so the keeper never sees `r`, `o` or `k`.
- `PUT` validates the kit's public part offline, with the same rules as `docs/KIT_FORMAT.md` section 4 (format, version, plan id, drand chain, round and time consistency, header round, and the optional `owner_key`/`owner_sig` pair: both absent, or both present with a valid Ed25519 signature). It cannot and does not look inside `sealed`. Kits are limited to 64 KiB; a signed kit is about 250 bytes larger than an unsigned one.
- A mailbox that has been created but never filled answers `GET` with `404 mailbox_not_found`. A mailbox that never receives a kit is deleted after 24 hours.
- A batch request (`count` above 1) counts once against the per-address request limiter, but N times against the per-address hourly quota (300 mailboxes per client bucket and hour; one installment plan needs at most 120) and N times against the keeper-wide limits (mailboxes per hour, mailboxes held, and never-filled mailboxes held, 10000 at most so empty mailboxes cannot crowd out filled ones). It is refused as a whole with `429 rate_limited` when it does not fit. A `count` outside 1 to 120, or a body that is not `{ "count": N }`, answers `400 invalid_request`. Without a body, or with `count` 1, the reply is the single-mailbox shape, so existing clients keep working.

Example:

```bash
curl -s -X POST https://<keeper>/v1/mailboxes
# {"id":"...","write_token":"...","read_token":"..."}
curl -s -X PUT  https://<keeper>/v1/mailboxes/<id>/kit -H "Authorization: Bearer <write_token>" \
     -H "Content-Type: application/json" --data-binary @kit.zandglass
curl -s https://<keeper>/v1/mailboxes/<id>/kit -H "Authorization: Bearer <read_token>"
```

The CLI does this for you: `zandglass plan legacy --mailbox https://<keeper> ...` creates the mailbox and prints the claim link once; every `flip` replaces the kit; `cancel` deletes the mailbox; `claim <claim link>` reads it.

## Errors

Errors are `{ "error": "<code>", "message": "<plain English>" }`.

| HTTP | `error` | When |
|---|---|---|
| 400 | `kit_invalid` | The body is not UTF-8 JSON, or not a valid kit (public part) |
| 400 | `kit_version_unsupported` | The kit has a `version` the keeper does not accept |
| 400 | `invalid_request` | The mailbox-creation body is not `{ "count": N }` with N from 1 to 120 |
| 401 | `unauthorized` | Missing or wrong token for this mailbox |
| 404 | `mailbox_not_found` | No such mailbox, a malformed id, or no kit stored yet |
| 404 | `not_found` | Unknown path (when no static web directory is configured) |
| 413 | `kit_invalid` | The kit is larger than 64 KiB |
| 429 | `rate_limited` | Too many requests |
| 500 | `internal_error` | The keeper could not complete the request |

## Limits

- Kit size: 64 KiB (`MAX_KIT_BYTES`).
- Rate limit per client address, in memory only (IPv6 addresses are grouped by /48): a burst of 60 requests, then 2 per second. Creating a mailbox (and a playground session) has its own bucket: a burst of 10, then 1 per minute.
- With `ZANDGLASS_TRUST_PROXY=1` the client address is taken from `CF-Connecting-IP` when present (Cloudflare), otherwise from the last entry of `X-Forwarded-For`. Enable it only behind a reverse proxy you control, and make sure the origin is reachable only through that proxy (the production keeper listens on 127.0.0.1 behind a cloudflared tunnel): otherwise a client could send its own `CF-Connecting-IP` and pick its rate-limit bucket.

Every response carries `Strict-Transport-Security: max-age=31536000`.

## CORS

Any origin is allowed (`Access-Control-Allow-Origin: *`), methods `GET` and `POST`, headers `Authorization` and `Content-Type`. No cookies or credentials are involved. Browser pages need only `GET` (the claim page) and `POST` (the playground); `PUT` and `DELETE` are used by the CLI, which is not subject to CORS.

## Retention

A mailbox that holds a kit is deleted when both hold: it was last updated more than 180 days ago, and its kit unlocked more than 180 days ago, so heirs keep half a year after the unlock to fetch it. The keeper reads the unlock time from the kit when it is stored (a kit that unlocks more than 31 years from now is refused with `400 kit_invalid`); a kit that has not unlocked yet is never deleted for age. A background task checks hourly. Databases from older keepers are migrated on start (an `unlock_time` column is added and filled from the stored kits). A mailbox that never receives a kit is deleted after 24 hours (see above). The owner can delete earlier with `DELETE`. The keeper keeps only the latest kit; replacing it removes the old one.

## Privacy

- Stored per mailbox: the id, the two token hashes, the latest kit (public part and sealed ciphertext), and the creation and update times.
- Not stored: accounts, names, IP addresses, request logs, analytics. The keeper writes only a startup line to its output. Rate-limit buckets live in memory.
- The keeper can therefore see when a kit is replaced (flip times), and the public part of kits, including plan id, epoch and unlock time (`docs/PRIVACY.md`, R2). It cannot read the payload. A reverse proxy in front of the keeper may log by itself; that is the operator's responsibility.

## Running the server

`zandglass-keeper` reads its configuration from the environment:

| Variable | Default | Meaning |
|---|---|---|
| `ZANDGLASS_LISTEN` | `127.0.0.1:8787` | Address to bind |
| `ZANDGLASS_DB` | `./keeper.sqlite` | SQLite database file |
| `ZANDGLASS_TRUST_PROXY` | off | `1` to use `CF-Connecting-IP` or `X-Forwarded-For` for rate limiting |
| `ZANDGLASS_MAX_MAILBOXES` | `50000` | Mailboxes held at once |
| `ZANDGLASS_MAX_CREATES_PER_HOUR` | `2000` | Mailboxes created per hour, all clients together |
| `ZANDGLASS_MAX_EMPTY_MAILBOXES` | `10000` | Never-filled mailboxes held at once |
| `ZANDGLASS_MAX_CREATES_PER_IP_PER_HOUR` | `300` | Mailboxes created per hour by one client bucket (a batch counts N) |
| `ZANDGLASS_WEB_DIR` | none | Directory of static files (the built `web/dist`) served at `/` |
| `ZANDGLASS_PLAYGROUND_*` | off | Enables the playground; see [PLAYGROUND_API.md](PLAYGROUND_API.md) |

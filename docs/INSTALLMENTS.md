# Installments (CLI)

Scheduled payments that stay cancelable. Each installment is its own vault, sealed to the payee and to its due time; nothing on chain links the installments.

## Owner

```
sandglass plan installments --payee <heir code> [--payee ...] \
    --amount <ZEC per installment> --every <30d | 2m | ...> --count <N> \
    [--start <2026-12-01T09:00:00Z | 10m>] [--message ...] \
    [--mailbox <keeper url>] [--out <dir>]
```

1. `plan installments` creates the funding vault and prints a ZIP 321 request for `N * (amount + 0.0001) + split fee`.
2. After the funds have confirmed, run `sandglass installments split <plan>`. This is an explicit step (not hidden in `sync`) so the owner decides when the schedule starts. It sends **one ordinary transaction** (`propose_transfer`, ZIP 317 fee, `OvkPolicy::Discard`) with N payments to N fresh installment vaults, then seals one kit per installment and writes `<plan8>-i<k>of<N>.sandglass` (kind `installment`, `sequence {index, count}`).
3. With `--mailbox`, each kit goes to its own keeper mailbox and the split writes every claim link to `<plan8>-claim-links.txt`. Send that file to the payee.
4. `sandglass status [--sync]` lists each installment: amount, unlock time and state (locked, unlocked, claimed or empty). `sandglass receive` shows the funding request.
5. `sandglass cancel <plan> --to <address>` sweeps every installment vault that still holds funds and the funding-vault leftover to the address, and deletes the mailboxes. Kits that are not claimed yet then open empty vaults. It can be run again after a partial failure.

`--start` is either an absolute RFC 3339 UTC time, or a delay counted from the split (default: one period). Installment k unlocks at `start + (k-1) * every`.

### Fees

Each installment vault holds `amount + 0.0001 ZEC`. The extra 10 000 zat is the ZIP 317 conventional fee of the payee's one-input sweep, so the payee receives exactly `amount`. The split transaction fee is paid from the funding vault; whatever is left there stays in it until `cancel`.

## Payee

```
sandglass claim --watch --phrase-file <phrase file> --to <address> \
    --links-file <claim-links.txt>          # or several kit files / links
```

Kits are processed in unlock order. `--watch` waits for each unlock and sweeps that vault to `--to`. If the funds are still confirming at the unlock time the claim retries (up to 30 minutes); an empty vault (a cancelled plan) is reported and skipped, and the exit status is then non-zero.

## Several heirs or payees

`plan legacy --heir a --heir b --heir c` and `plan installments --payee ...` accept 1 to 3 codes. Any one of them can open the kit. Zero, more than three, duplicate and malformed codes are rejected before anything is created.

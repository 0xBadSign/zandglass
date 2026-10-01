#!/usr/bin/env bash
# Evidence: an heir restores a Sandglass vault seed in a THIRD-PARTY wallet
# (ECC's zcash-devtool) and spends from it. Testnet.
#
#   FUNDER_DIR=<spike_vault dir with TAZ> RETURN_TO=<testnet UA> DEVTOOL=<path> \
#     bash scripts/e2e-thirdparty-restore.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/sandglass-target}"

: "${FUNDER_DIR:?}" "${RETURN_TO:?}" "${DEVTOOL:?}"
WORK="${WORK:-$HOME/sandglass-e2e/restore-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$WORK"
export SANDGLASS_HOME="$WORK/home" SANDGLASS_PASSPHRASE="e2e-only-passphrase"

cargo build -q -p sandglass-cli --bin sandglass -p sandglass-wallet --example spike_vault
SG="$CARGO_TARGET_DIR/debug/sandglass --network test"
SPIKE="$CARGO_TARGET_DIR/debug/examples/spike_vault --network test"
log() { printf '\n=== %s | %s\n' "$(date -u +%H:%M:%S)" "$*"; }

log "heir key, store, legacy plan (5 minute timer)"
$SG heir new --save "$WORK/heir.phrase" > "$WORK/heir.txt"
HEIR_CODE=$(grep -o 'age1[0-9a-z]*' "$WORK/heir.txt" | head -1)
$SG init
$SG plan legacy --heir "$HEIR_CODE" --every 5m --message "Restore me in any wallet" --out "$WORK" > "$WORK/plan.txt"
VAULT=$($SG receive | grep -o 'utest1[0-9a-z]*' | head -1)
KIT=$(ls "$WORK"/*-e1.sandglass)

log "fund the vault"
$SPIKE sweep "$FUNDER_DIR" "$VAULT" | tee "$WORK/fund.txt"

log "wait for the unlock, then open the kit WITHOUT sweeping (prints the seed for the heir)"
$SG claim "$KIT" --phrase-file "$WORK/heir.phrase" --wait > "$WORK/opened.txt"
SEED=$(grep -E "^ *([a-z]+ ){23}[a-z]+ *$" "$WORK/opened.txt" | head -1 | xargs)
BIRTHDAY=$(sed -n 's/^Birthday height: *\([0-9]*\).*/\1/p' "$WORK/opened.txt" | head -1)
[ -n "$SEED" ] && [ -n "$BIRTHDAY" ] || { echo "could not read seed/birthday"; cat "$WORK/opened.txt" | grep -v -i seed; exit 1; }
echo "birthday $BIRTHDAY (seed read, not printed)"

log "restore the seed in zcash-devtool (third-party wallet)"
DT_DIR="$WORK/devtool"
mkdir -p "$DT_DIR"
cd "$DT_DIR"
printf '%s\n' "$SEED" | "$DEVTOOL" wallet -w "$DT_DIR/wallet" restore-mnemonic --name heir --identity "$DT_DIR/id.txt" --birthday "$BIRTHDAY" --network test 2>&1 | tail -3
for _ in $(seq 1 60); do
    "$DEVTOOL" wallet -w "$DT_DIR/wallet" sync > "$WORK/dt-sync.txt" 2>&1 || true
    "$DEVTOOL" wallet -w "$DT_DIR/wallet" balance > "$WORK/dt-balance.txt" 2>&1 || true
    if grep -q -E 'Spendable[^0-9]*0\.0*[1-9]|Spendable[^0-9]*[1-9]' "$WORK/dt-balance.txt"; then break; fi
    sleep 30
done
cat "$WORK/dt-balance.txt"

log "spend from the third-party wallet: send 0.5 TAZ back"
ACCOUNT=$("$DEVTOOL" wallet -w "$DT_DIR/wallet" list-accounts 2>/dev/null | grep -o -E '[0-9a-f]{8}-[0-9a-f-]{27}' | head -1)
"$DEVTOOL" wallet -w "$DT_DIR/wallet" send --identity "$DT_DIR/id.txt" --address "$RETURN_TO" --value 50000000 "$ACCOUNT" 2>&1 | tee "$WORK/dt-send.txt" | tail -5
log "done; transcripts in $WORK"

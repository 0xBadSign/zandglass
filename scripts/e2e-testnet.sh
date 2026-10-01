#!/usr/bin/env bash
# End-to-end legacy flow on Zcash testnet with real transactions (T1.5).
#
#   FUNDER_DIR=<spike vault dir with TAZ> HEIR_TO=<testnet UA> bash scripts/e2e-testnet.sh
#
# Steps: heir key, store, legacy plan (short timer), fund the vault from a
# funder wallet, flip (the first kit must now open an empty vault), stay
# silent, then claim the second kit to HEIR_TO after the unlock.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/zandglass-target}"

: "${FUNDER_DIR:?set FUNDER_DIR to a funded spike_vault directory}"
: "${HEIR_TO:?set HEIR_TO to the testnet address that receives the claim}"
EVERY="${EVERY:-6m}"
WORK="${WORK:-$HOME/zandglass-e2e/$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$WORK"
export ZANDGLASS_HOME="$WORK/home" ZANDGLASS_PASSPHRASE="e2e-only-passphrase"

cargo build -q -p zandglass-cli --bin zandglass -p zandglass-wallet --example spike_vault
SG="$CARGO_TARGET_DIR/debug/zandglass --network test"
SPIKE="$CARGO_TARGET_DIR/debug/examples/spike_vault --network test"
log() { printf '\n=== %s | %s\n' "$(date -u +%H:%M:%S)" "$*"; }

wait_confirmed() { # wait until the plan's vault shows a confirmed balance
    for _ in $(seq 1 60); do
        confirmed=$($SG status --sync | sed -n 's/.*(\([0-9.]*\) confirmed).*/\1/p' | head -1)
        if awk -v c="${confirmed:-0}" 'BEGIN { exit !(c > 0) }'; then return 0; fi
        sleep 20
    done
    return 1
}

log "heir key"
$SG heir new --save "$WORK/heir.phrase" | tee "$WORK/heir.txt"
HEIR_CODE=$(grep -o 'age1[0-9a-z]*' "$WORK/heir.txt" | head -1)

log "store and plan (every $EVERY)"
$SG init
$SG plan legacy --heir "$HEIR_CODE" --every "$EVERY" --message "E2E test $(date -u +%F)" --out "$WORK" | tee "$WORK/plan.txt"
VAULT1=$($SG receive | grep -o 'utest1[0-9a-z]*' | head -1)
KIT1=$(ls "$WORK"/*-e1.zandglass)

log "fund vault epoch 1 from the funder"
$SPIKE sweep "$FUNDER_DIR" "$VAULT1" | tee "$WORK/fund.txt"
wait_confirmed || { echo "funding never confirmed"; exit 1; }

log "flip to epoch 2"
$SG flip --out "$WORK" | tee "$WORK/flip.txt"
KIT2=$(ls "$WORK"/*-e2.zandglass)

log "stay silent until the epoch 2 kit unlocks, then claim to $HEIR_TO"
$SG claim "$KIT2" --phrase-file "$WORK/heir.phrase" --to "$HEIR_TO" --wait | tee "$WORK/claim.txt"

log "the old epoch 1 kit must open an empty vault"
$SG claim "$KIT1" --phrase-file "$WORK/heir.phrase" --to "$HEIR_TO" --wait 2>&1 | tee "$WORK/claim-old.txt" || true
grep -q 'vault_empty' "$WORK/claim-old.txt" && echo "OK: old kit is empty"

log "done; transcripts in $WORK"
grep -h 'txid' "$WORK"/*.txt || true

#!/usr/bin/env bash
# Kit interop: the same kit must open with the Rust core, the reference CLIs
# (`tle` then `age`) and TypeScript (tlock-js + age-encryption), in both
# directions. Needs network (drand relays). Run inside WSL from the repo root:
#   bash scripts/interop-kit.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
export PATH="$HOME/.local/bin:$PATH"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/zandglass-target}"

V=crates/zandglass-core/tests/vectors
IDENTITY="$V/TEST_ONLY_identity.txt"
PAYLOAD="$V/payload.txt"
ROUND="$(tr -d '\n' < "$V/round.txt")"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

step() { printf '\n== %s\n' "$1"; }
same() { cmp -s "$1" "$2" && echo "   OK: payload matches ($3)" || { echo "   FAIL: $3"; exit 1; }; }
rust_open() { cargo run -q -p zandglass-core --example open_kit -- "$@"; }

command -v age >/dev/null || { echo "age not found"; exit 1; }
command -v tle >/dev/null || { echo "tle not found"; exit 1; }

step "1. Rust core opens the committed vector (offline, recorded beacon)"
rust_open "$V/kit.txt" "$IDENTITY" "$V/beacon.json" > "$TMP/out1"
same "$TMP/out1" "$PAYLOAD" "rust offline"

step "1b. Rust core opens the committed vector (live relays)"
rust_open "$V/kit.txt" "$IDENTITY" > "$TMP/out1b"
same "$TMP/out1b" "$PAYLOAD" "rust online"

step "2. Reference CLIs open the committed vector: tle --decrypt | age --decrypt"
tle --decrypt < "$V/kit.txt" > "$TMP/inner.age"
age --decrypt -i "$IDENTITY" < "$TMP/inner.age" > "$TMP/out2"
same "$TMP/out2" "$PAYLOAD" "tle + age"

step "3. TypeScript opens the committed vector, and seals a kit (tlock-js + age-encryption)"
( cd web && ZANDGLASS_NETWORK_TESTS=1 KIT_OUT="$TMP/ts.kit" npx vitest run src/kit-interop.test.ts --reporter=dot 2>&1 | grep -v 'beacon received' )
echo "   OK: vitest interop tests passed"

step "4. CLI-sealed kit: age -r <heir code> | tle --encrypt -r <round> --armor"
age -r "$(tr -d '\n' < "$V/heir_code.txt")" < "$PAYLOAD" \
    | tle --encrypt -r "$ROUND" --force --armor > "$TMP/cli.kit"
head -1 "$TMP/cli.kit"
rust_open "$TMP/cli.kit" "$IDENTITY" "$V/beacon.json" > "$TMP/out4"
same "$TMP/out4" "$PAYLOAD" "CLI-sealed, rust opens"
( cd web && KIT_IN="$TMP/cli.kit" npx vitest run src/kit-interop.test.ts --reporter=dot 2>&1 | grep -v 'beacon received' )
echo "   OK: CLI-sealed kit opens in TypeScript"

step "5. TypeScript-sealed kit opens with the CLIs and the Rust core"
tle --decrypt < "$TMP/ts.kit" | age --decrypt -i "$IDENTITY" > "$TMP/out5a"
same "$TMP/out5a" <(printf 'TEST ONLY payload sealed by TypeScript\n') "TS-sealed, tle + age opens"
rust_open "$TMP/ts.kit" "$IDENTITY" "$V/beacon.json" > "$TMP/out5b"
same "$TMP/out5b" <(printf 'TEST ONLY payload sealed by TypeScript\n') "TS-sealed, rust opens"

step "6. Rust network tests (ignored by default): live beacon fetch and vector"
cargo test -q -p zandglass-core -- --ignored 2>&1 | grep "test result"

printf '\nALL KIT INTEROP CHECKS PASSED\n'

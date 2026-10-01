#!/usr/bin/env bash
# The quality gate. Run inside WSL from the repository root:
#   bash scripts/check.sh          # everything
#   bash scripts/check.sh rust     # only the Rust checks
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/sandglass-target}"

ONLY="${1:-all}"
failed=()
run() {
    local name="$1"; shift
    echo "== $name"
    if "$@"; then echo "   ok"; else echo "   FAILED"; failed+=("$name"); fi
}

english_only() {
    # Repository prose must be English. Flag letters specific to other
    # languages we might slip into (Turkish in particular).
    local hits
    hits=$(git ls-files -co --exclude-standard \
        | grep -v -E '^(LICENSE-|web/package-lock\.json|scripts/check\.sh)' \
        | xargs -d '\n' grep -n -P '[çğışöüÇĞİŞÖÜ]' 2>/dev/null || true)
    [ -z "$hits" ] || { echo "$hits"; return 1; }
}

no_secrets() {
    # Nothing under .private may be tracked; no obvious secret material.
    if git ls-files | grep -q '^\.private/'; then echo ".private is tracked"; return 1; fi
    local hits
    hits=$(git ls-files -co --exclude-standard \
        | xargs -d '\n' grep -n -E '(BEGIN [A-Z ]*PRIVATE KEY|AGE-SECRET-KEY-1[0-9A-Z]{20,})' 2>/dev/null \
        | grep -v -E '^(crates/[^:]*/tests/vectors/|docs/KIT_FORMAT\.md)' || true)
    [ -z "$hits" ] || { echo "$hits"; return 1; }
    # Wallet seeds: no seed-like file names, and no line of 12 or 24 lowercase
    # words alone (a BIP 39 phrase). Marked test vectors are the only exception.
    hits=$(git ls-files -co --exclude-standard | grep -i -E '(^|/)[^/]*seed[^/]*\.txt$' || true)
    [ -z "$hits" ] || { echo "seed-like file: $hits"; return 1; }
    hits=$(git ls-files -co --exclude-standard | grep -v -E '(^|/)(TEST_ONLY_[^/]*|package-lock\.json|Cargo\.lock)$' \
        | xargs -d '\n' grep -l -E '^\s*([a-z]{3,8} ){11}([a-z]{3,8} ){0,12}[a-z]{3,8}\s*$' 2>/dev/null || true)
    [ -z "$hits" ] || { echo "possible seed phrase in: $hits"; return 1; }
    if command -v gitleaks >/dev/null; then gitleaks detect --no-banner --redact -q; fi
}

if [ "$ONLY" = all ] || [ "$ONLY" = rust ]; then
    run rust-fmt cargo fmt --all -- --check
    run rust-clippy cargo clippy --workspace --all-targets --quiet -- -D warnings
    run rust-test cargo test --workspace --quiet
fi
if [ "$ONLY" = all ] || [ "$ONLY" = web ]; then
    if [ -f web/package.json ]; then
        # Reinstall when the lockfile changed since the last install.
        run web-install bash -c 'cd web && { [ node_modules/.package-lock.json -nt package-lock.json ] || npm ci --silent --no-audit --no-fund; }'
        run web-types bash -c 'cd web && npx tsc --noEmit'
        run web-test bash -c 'cd web && npm test --silent'
    fi
fi
run english-only english_only
run secrets no_secrets

echo
if [ ${#failed[@]} -eq 0 ]; then echo "ALL CHECKS PASSED"; else echo "FAILED: ${failed[*]}"; exit 1; fi

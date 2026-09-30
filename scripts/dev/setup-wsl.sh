#!/usr/bin/env bash
# One-time developer toolchain setup inside WSL Ubuntu.
#
# System packages need root. From Windows, run first:
#   wsl -d Ubuntu -u root -- apt-get install -y age build-essential pkg-config protobuf-compiler libssl-dev golang-go
# then this script as the normal user.
set -euo pipefail

if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --component rustfmt,clippy
fi
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
rustup show active-toolchain

for tool in age go; do
    command -v "$tool" >/dev/null || { echo "missing $tool: install the system packages first (see header)"; exit 1; }
done

# drand's tle CLI, used by interoperability tests.
if [ ! -x "$HOME/.local/bin/tle" ]; then
    GOBIN="$HOME/.local/bin" go install github.com/drand/tlock/cmd/tle@v1.2.0
fi
"$HOME/.local/bin/tle" --help 2>&1 | head -1
echo "setup done"

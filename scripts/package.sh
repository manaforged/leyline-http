#!/usr/bin/env bash
# Package Leyline crates in crates.io publish order.
#
# `leyline` depends on `leyline-quiche` by version, so crates.io packaging
# cannot verify `leyline` until the matching `leyline-quiche` version is
# visible in the crates.io index. This script makes that dependency boundary
# explicit instead of hiding it behind a confusing cargo error.
#
# Usage:
#   ./scripts/package.sh [cargo-package-flags...]
#   ./scripts/package.sh --allow-dirty
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
if [[ -n "${USERPROFILE:-}" ]] && command -v cygpath >/dev/null 2>&1; then
    export PATH="$(cygpath "$USERPROFILE")/.cargo/bin:$PATH"
fi
export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo >/dev/null 2>&1 && command -v cargo.exe >/dev/null 2>&1; then
    cargo() { cargo.exe "$@"; }
fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo_root/target/package}"

version="$(awk -F'"' '/^version *= *"/{print $2; exit}' Cargo.toml)"
if [[ -z "$version" ]]; then
    echo "could not read workspace.package.version from Cargo.toml" >&2
    exit 2
fi

step() { printf '\n\033[1;34m== %s ==\033[0m\n' "$*"; }
ok()   { printf '\033[1;32m✓ %s\033[0m\n'  "$*"; }

step "cargo package -p leyline-quiche"
cargo package -p leyline-quiche "$@"
ok "leyline-quiche packages"

step "cargo package -p leyline"
log="$(mktemp)"
if cargo package -p leyline "$@" 2>&1 | tee "$log"; then
    ok "leyline packages"
    rm -f "$log"
    exit 0
fi

if grep -q "no matching package named.*leyline-quiche" "$log"; then
    rm -f "$log"
    cat >&2 <<EOF

leyline-quiche packaged successfully, but leyline cannot be packaged yet
because crates.io does not have leyline-quiche $version in its index.

Release order:
  1. cargo publish -p leyline-quiche
  2. wait for crates.io index propagation
  3. ./scripts/package.sh
  4. cargo publish -p leyline

EOF
    exit 3
fi

rm -f "$log"
exit 1

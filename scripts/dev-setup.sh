#!/usr/bin/env bash
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
if ! command -v rustc >/dev/null 2>&1 && command -v rustc.exe >/dev/null 2>&1; then
    rustc() { rustc.exe "$@"; }
fi

step() { printf '\n\033[1;34m== %s ==\033[0m\n' "$*"; }
ok()   { printf '\033[1;32mOK %s\033[0m\n' "$*"; }
warn() { printf '\033[1;33mWARN %s\033[0m\n' "$*"; }
need() {
    if ! command -v "$1" >/dev/null 2>&1; then
        warn "$1 was not found on PATH"
        return 1
    fi
    return 0
}

step "toolchain"
need cargo || {
    cat >&2 <<'EOF'
Install Rust with rustup first:
  https://rustup.rs
EOF
    exit 2
}
rustc --version
cargo --version
msrv="$(awk -F'"' '/^rust-version *= *"/{print $2; exit}' Cargo.toml)"
if [[ -n "$msrv" ]]; then
    echo "workspace MSRV: $msrv"
fi

step "native build prerequisites"
uname_s="$(uname -s 2>/dev/null || echo unknown)"
if command -v cmd.exe >/dev/null 2>&1 || command -v powershell.exe >/dev/null 2>&1; then
    uname_s="Windows"
fi

uname_m="$(uname -m 2>/dev/null || echo unknown)"
case "$uname_m" in
    arm64|aarch64) arch="aarch64" ;;
    x86_64|amd64)  arch="x86_64" ;;
    *)             arch="$uname_m" ;;
esac
case "$uname_s" in
    Darwin)            triple="$arch-apple-darwin" ;;
    Linux)             triple="$arch-unknown-linux-gnu" ;;
    Windows|MINGW*|MSYS*|CYGWIN*) triple="x86_64-pc-windows-msvc" ;;
    *)                 triple="" ;;
esac

case "$triple" in
    aarch64-apple-darwin|x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|x86_64-pc-windows-msvc)
        ok "supported target: $triple" ;;
    *)  warn "unsupported target '${triple:-$uname_s/$uname_m}'; the leyline-bssl-sys build will stop" ;;
esac
echo "  leyline-bssl-sys builds BoringSSL from source. It needs:"
case "$uname_s" in
    Darwin)  need clang || warn "install the Xcode Command Line Tools" ;;
    Linux)   need c++   || warn "Debian/Ubuntu: sudo apt-get install build-essential"
             need clang || warn "for scripts/regen-bssl-bindings.sh, Debian/Ubuntu: sudo apt-get install clang libclang-dev" ;;
    Windows|MINGW*|MSYS*|CYGWIN*)
             need clang || warn "for scripts/regen-bssl-bindings.sh, install LLVM: choco install llvm" ;;
    *)       need c++   || true ;;
esac
ok "prerequisite scan complete"

step "fast offline build"
cargo check -p leyline-http --all-features
ok "leyline-http all-features check passed"

step "offline smoke tests"
cargo test -p leyline-http --test it parity_builder::
cargo test -p leyline-http --features bench-internals --test it tls_happy_eyeballs::
ok "developer setup looks ready"

cat <<'EOF'

Next useful commands:
  ./scripts/verify.sh
  cargo test --workspace --exclude leyline-quiche
  cargo test -p leyline-http --features bench-internals --test it tls_peet:: -- --ignored
EOF

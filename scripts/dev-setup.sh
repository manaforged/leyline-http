#!/usr/bin/env bash
# Fresh-developer setup check for Leyline.
#
# This intentionally runs a small, offline build instead of the full release
# gate. Use scripts/verify.sh once the local toolchain is warm.
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
case "$uname_s" in
    Darwin)
        need cmake || warn "install with: brew install cmake"
        need perl || warn "install Xcode command line tools and Perl"
        ;;
    Linux)
        need cmake || warn "Debian/Ubuntu: sudo apt-get install cmake"
        need perl || warn "Debian/Ubuntu: sudo apt-get install perl"
        need pkg-config || warn "Debian/Ubuntu: sudo apt-get install pkg-config"
        ;;
    Windows|MINGW*|MSYS*|CYGWIN*)
        if [[ -f crates/btls-sys/native/x86_64-pc-windows-msvc/lib/ssl.lib ]]; then
            echo "Windows prebuilt BoringSSL shim found; CMake/Perl only needed to refresh it."
        else
            need cmake || warn "install Visual Studio Build Tools or CMake"
            need perl || warn "install Strawberry Perl"
        fi
        ;;
    *)
        need cmake || true
        need perl || true
        ;;
esac
ok "prerequisite scan complete"

step "fast offline build"
cargo check -p leyline --all-features
ok "leyline all-features check passed"

step "offline smoke tests"
cargo test -p leyline --test parity_builder
cargo test -p leyline --test tls_happy_eyeballs
ok "developer setup looks ready"

cat <<'EOF'

Next useful commands:
  ./scripts/verify.sh --quick
  cargo test --workspace --exclude leyline-quiche
  cargo test -p leyline --test tls_peet -- --ignored
EOF

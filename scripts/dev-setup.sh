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

# Map host OS/arch to the Rust target triple the prebuilt shim is keyed on,
# then check whether crates/leyline-bssl-sys ships a prebuilt for it. If it
# does, the in-workspace build links it directly — no CMake/Perl/libclang/Go.
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

prebuilt_dir="crates/leyline-bssl-sys/native/$triple/lib"
if [[ -n "$triple" ]] \
    && { [[ -f "$prebuilt_dir/ssl.lib" ]] || [[ -f "$prebuilt_dir/libssl.a" ]]; }; then
    ok "prebuilt BoringSSL shim found for $triple — no CMake/Perl/libclang/Go needed"
else
    warn "no prebuilt BoringSSL shim for target '${triple:-$uname_s/$uname_m}'"
    echo "  The in-workspace build will fail for this target. To proceed:"
    echo "    - set BORING_BSSL_PATH to a BoringSSL build for it, OR"
    echo "    - source-build via: (cd crates/leyline-bssl-sys && cargo build --features source-build)."
    echo "  Source build needs these tools:"
    case "$uname_s" in
        Darwin)  need cmake || warn "install with: brew install cmake"
                 need perl  || warn "install Xcode command line tools and Perl" ;;
        Linux)   need cmake || warn "Debian/Ubuntu: sudo apt-get install cmake"
                 need perl  || warn "Debian/Ubuntu: sudo apt-get install perl"
                 need pkg-config || warn "Debian/Ubuntu: sudo apt-get install pkg-config" ;;
        Windows|MINGW*|MSYS*|CYGWIN*)
                 need cmake || warn "install Visual Studio Build Tools or CMake"
                 need perl  || warn "install Strawberry Perl" ;;
        *)       need cmake || true; need perl || true ;;
    esac
fi
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

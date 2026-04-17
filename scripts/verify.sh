#!/usr/bin/env bash
# Leyline local verify: runs the release quality gates locally before
# tagging a release.
#
# Usage: ./scripts/verify.sh [--quick]
# --quick  skips live tls_peet tests and the smoke suite
set -euo pipefail

quick=0
if [[ "${1:-}" == "--quick" ]]; then
    quick=1
fi

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

step() { printf '\n\033[1;34m== %s ==\033[0m\n' "$*"; }
ok()   { printf '\033[1;32m✓ %s\033[0m\n'  "$*"; }
fail() { printf '\033[1;31m✗ %s\033[0m\n'  "$*" >&2; exit 1; }

# -- rustc toolchain sanity ----------------------------------------------
step "rust toolchain"
rustc --version
cargo --version
msrv="$(awk -F'"' '/^rust-version *= *"/{print $2}' Cargo.toml)"
if [[ -n "$msrv" ]]; then
    echo "workspace MSRV pinned to: $msrv"
    # If cargo-msrv is available, verify; otherwise advise.
    if command -v cargo-msrv >/dev/null; then
        cargo msrv verify --manifest-path Cargo.toml || fail "MSRV verify failed"
        ok "MSRV verified"
    else
        echo "  (cargo-msrv not installed; skipping active MSRV check)"
    fi
fi

# -- format --------------------------------------------------------------
step "cargo fmt --all --check"
cargo fmt --all -- --check || fail "rustfmt found formatting issues"
ok "format clean"

# -- clippy --------------------------------------------------------------
step "cargo clippy --workspace --all-targets"
cargo clippy --workspace --all-targets -- -D warnings || fail "clippy produced warnings"
ok "clippy clean"

# -- docs ----------------------------------------------------------------
step "cargo doc --workspace --no-deps"
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps || fail "rustdoc warnings"
ok "docs clean"

# -- tests ---------------------------------------------------------------
step "cargo test --workspace"
cargo test --workspace --exclude boring --exclude boring-sys --exclude tokio-boring || fail "tests failed"
ok "tests pass"

if [[ $quick -eq 0 ]]; then
    step "cargo test -p leyline --test tls_peet -- --ignored"
    cargo test -p leyline --test tls_peet -- --ignored || fail "live tls_peet tests failed"
    ok "live tls_peet pass"

    step "cargo run -p leyline --example smoke"
    cargo run --release -p leyline --example smoke || fail "smoke suite failed"
    ok "smoke pass"
fi

# -- supply chain --------------------------------------------------------
step "cargo deny --all-features check"
if ! command -v cargo-deny >/dev/null; then
    fail "cargo-deny not installed: cargo install --locked cargo-deny"
fi
cargo deny --all-features check || fail "cargo-deny found issues"
ok "cargo-deny clean"

# -- benches compile -----------------------------------------------------
step "benches compile (sub-workspace)"
( cd benches && cargo bench --no-run ) || fail "benches fail to compile"
ok "benches compile"

printf '\n\033[1;32mAll verify gates passed.\033[0m\n'

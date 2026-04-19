#!/usr/bin/env bash
# Leyline local verify: runs the release quality gates locally before
# tagging a release.
#
# Usage: ./scripts/verify.sh [--quick] [--fuzz [SECONDS]]
# --quick        skips live tls_peet tests and the smoke suite
# --fuzz [N]     run each cargo-fuzz target for N seconds (default 60)
#                on top of the existing corpus replay. Requires
#                `cargo install cargo-fuzz` and nightly rustc. Corpus
#                replay (fast, just runs the seeded inputs once) is
#                always performed in non-quick mode if cargo-fuzz is
#                available — it catches regressions on bugs already
#                caught-and-fixed without adding the nightly-rustc
#                requirement to a normal release gate.
set -euo pipefail

quick=0
fuzz=0
fuzz_seconds=60
while [[ $# -gt 0 ]]; do
    case "$1" in
        --quick) quick=1; shift ;;
        --fuzz)
            fuzz=1
            shift
            # Optional numeric seconds argument.
            if [[ "${1:-}" =~ ^[0-9]+$ ]]; then
                fuzz_seconds="$1"
                shift
            fi
            ;;
        *) echo "unknown flag: $1" >&2; exit 2 ;;
    esac
done

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
# Vendored forks of boring/boring-sys/tokio-boring carry upstream warnings
# we don't own. Gate clippy strictly on Leyline's own crates, with
# `--no-deps` so bindgen-generated code (function-pointer comparisons in
# boring's generated bindings.rs) doesn't pollute our gate.
step "cargo clippy (workspace, excluding vendored boring*, --no-deps)"
cargo clippy \
    --workspace \
    --exclude boring --exclude boring-sys --exclude tokio-boring \
    --all-targets --no-deps \
    -- -D warnings \
    || fail "clippy produced warnings"
ok "clippy clean"

# -- docs ----------------------------------------------------------------
step "cargo doc (workspace, excluding vendored boring*)"
RUSTDOCFLAGS="-D warnings" cargo doc \
    --workspace \
    --exclude boring --exclude boring-sys --exclude tokio-boring \
    --no-deps \
    || fail "rustdoc warnings"
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

# -- fuzz corpus replay --------------------------------------------------
# The four `cargo fuzz` targets have seeded
# corpora but no replay in the verify gate — the bugs they caught
# (HPACK overflow panics, cookie overflow, frame parser panics) could
# regress without any test breaking. Replay each corpus once to catch
# regressions. This is fast (seconds per target) and doesn't need
# nightly — libFuzzer's corpus replay is a deterministic byte-for-byte
# run of every seeded input through the target harness.
#
# The full time-bounded fuzzer (which DOES need nightly rustc for
# sanitizer support) is opt-in via `--fuzz [SECONDS]` because nightly
# is a heavier prerequisite for a normal release.
FUZZ_TARGETS=(hpack_integer hpack_header_block h2_frame cookie_set)

if [[ $quick -eq 0 ]]; then
    step "fuzz corpus replay (cargo fuzz, -runs=0)"
    if ! command -v cargo-fuzz >/dev/null; then
        echo "  (cargo-fuzz not installed; skipping corpus replay — install with"
        echo "   'cargo install --locked cargo-fuzz' to enable this gate)"
    elif ! command -v rustc >/dev/null || ! rustup toolchain list 2>/dev/null | grep -q nightly; then
        # cargo-fuzz insists on nightly even for `-runs=0` corpus replay
        # because the instrumentation layer is only stable on nightly.
        echo "  (nightly toolchain not installed; skipping corpus replay —"
        echo "   install with 'rustup toolchain install nightly')"
    else
        for target in "${FUZZ_TARGETS[@]}"; do
            echo "  replaying corpus/$target ..."
            ( cd fuzz && cargo +nightly fuzz run "$target" -- -runs=0 \
                </dev/null >/tmp/leyline-fuzz-$target.log 2>&1 ) \
                || { cat /tmp/leyline-fuzz-$target.log; fail "corpus replay failed for $target"; }
        done
        ok "fuzz corpus replay clean"
    fi
fi

if [[ $fuzz -eq 1 ]]; then
    step "fuzz (time-bounded, ${fuzz_seconds}s per target)"
    if ! command -v cargo-fuzz >/dev/null; then
        fail "cargo-fuzz not installed: cargo install --locked cargo-fuzz"
    fi
    if ! rustup toolchain list 2>/dev/null | grep -q nightly; then
        fail "nightly toolchain required: rustup toolchain install nightly"
    fi
    for target in "${FUZZ_TARGETS[@]}"; do
        echo "  fuzzing $target for ${fuzz_seconds}s ..."
        ( cd fuzz && cargo +nightly fuzz run "$target" -- \
            -max_total_time="$fuzz_seconds" </dev/null ) \
            || fail "fuzz run found a crash on $target — inspect fuzz/artifacts/"
    done
    ok "fuzz time-bounded runs clean"
fi

printf '\n\033[1;32mAll verify gates passed.\033[0m\n'

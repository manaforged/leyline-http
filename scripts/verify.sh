#!/usr/bin/env bash
set -euo pipefail

full=0
fuzz=0
fuzz_seconds=300
only=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --quick) shift ;; # compatibility: quick is now the default
        --full) full=1; shift ;;
        --only) only="${2:?--only needs a comma-separated gate list}"; shift 2 ;;
        --fuzz)
            full=1
            fuzz=1
            shift
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
if [[ -n "${USERPROFILE:-}" ]] && command -v cygpath >/dev/null 2>&1; then
    export PATH="$(cygpath "$USERPROFILE")/.cargo/bin:$PATH"
fi
export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo >/dev/null 2>&1 && command -v cargo.exe >/dev/null 2>&1; then
    cargo() { cargo.exe "$@"; }
fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo_root/target/verify}"

step() { printf '\n\033[1;34m== %s ==\033[0m\n' "$*"; }
ok()   { printf '\033[1;32m✓ %s\033[0m\n'  "$*"; }
fail() { printf '\033[1;31m✗ %s\033[0m\n'  "$*" >&2; exit 1; }

msrv="$(awk -F'"' '/^rust-version *= *"/{print $2; exit}' Cargo.toml)"

package_stage=""
package_root=""
cleanup() {
    [[ -z "${package_stage:-}" ]] || rm -rf "$package_stage"
    [[ -z "${package_root:-}" ]] || rm -rf "$package_root"
}
trap cleanup EXIT

g_comments() {
    step "rust comments (one line)"
    python3 scripts/check-comments.py || fail "comments must be one-line rustdoc or // SAFETY:"
    ok "comment lint"
}

g_msrv() {
    step "rust toolchain"
    rustc --version
    cargo --version
    if [[ -n "$msrv" ]]; then
        echo "workspace MSRV pinned to: $msrv"
        cargo +"$msrv" --version || fail "Rust $msrv is required; install it with rustup toolchain install $msrv"
    fi

    step "cargo +$msrv check"
    cargo +"$msrv" check --workspace || fail "MSRV cargo check failed"
    ok "MSRV compile sanity"
}

g_package() {
    step "cargo package (publishable Rust crates)"
    package_stage="$(mktemp -d)"
    git archive --format=tar HEAD | tar -xf - -C "$package_stage"
    node scripts/stage-package-workspace.mjs "$package_stage" \
        || fail "failed to stage the package workspace"
    rm -rf "$CARGO_TARGET_DIR/package"
    cargo package \
        --manifest-path "$package_stage/Cargo.toml" \
        --workspace \
        --no-verify \
        || fail "cargo package failed"
    ok "publishable crates packaged"

    step "packaged leyline consumer smoke check"
    package_root="$(mktemp -d)"
    for archive in "$CARGO_TARGET_DIR"/package/*.crate; do
        tar -xzf "$archive" -C "$package_root"
    done
    package_path() { find "$package_root" -maxdepth 1 -type d -name "$1-[0-9]*" -print -quit; }
    leyline_package="$(package_path leyline-http)"
    bssl_sys_package="$(package_path leyline-bssl-sys)"
    bssl_package="$(package_path leyline-bssl)"
    bssl_tokio_package="$(package_path leyline-bssl-tokio)"
    quiche_package="$(package_path leyline-quiche)"
    [[ -n "$leyline_package" && -n "$bssl_sys_package" && -n "$bssl_package" \
        && -n "$bssl_tokio_package" && -n "$quiche_package" ]] \
        || fail "packaged crate archive is missing"
    mkdir "$package_root/consumer"
    cat >"$package_root/consumer/Cargo.toml" <<EOF
[package]
name = "leyline-package-smoke"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
leyline-http = { path = "$leyline_package" }

[patch.crates-io]
leyline-bssl-sys = { path = "$bssl_sys_package" }
leyline-bssl = { path = "$bssl_package" }
leyline-bssl-tokio = { path = "$bssl_tokio_package" }
leyline-quiche = { path = "$quiche_package" }
EOF
    mkdir "$package_root/consumer/src"
    printf 'fn main() {}\n' >"$package_root/consumer/src/main.rs"
    cargo +"$msrv" check --manifest-path "$package_root/consumer/Cargo.toml" \
        || fail "packaged leyline consumer smoke check failed"
    ok "packaged leyline consumer smoke check"
}

g_fmt() {
    step "cargo fmt --all --check"
    cargo fmt --all -- --check || fail "rustfmt found formatting issues"
    ok "format clean"
}

g_clippy() {
    step "cargo clippy (workspace, excluding vendored leyline-quiche, --no-deps)"
    cargo clippy \
        --workspace \
        --exclude leyline-quiche \
        --all-targets --no-deps \
        -- -D warnings \
        || fail "clippy produced warnings"
    ok "clippy clean"
}

g_doc() {
    step "cargo doc (workspace, excluding vendored leyline-quiche)"
    RUSTDOCFLAGS="-D warnings" cargo doc \
        --workspace \
        --exclude leyline-quiche \
        --no-deps \
        || fail "rustdoc warnings"
    ok "docs clean"
}

g_api() {
    step "generated API reference"
    python3 scripts/generate-api.py --check || fail "run python3 scripts/generate-api.py"
    ok "API reference current"
}

g_book() {
    step "mdbook"
    mdbook build docs >/dev/null || fail "mdbook build docs"
    ok "book builds"
}

g_test() {
    step "cargo test --workspace --exclude leyline-quiche"
    cargo test --workspace --exclude leyline-quiche \
        --features leyline-http/full,leyline-http/bench-internals || fail "tests failed"
    ok "tests pass"
}

g_live() {
    step "cargo test -p leyline-http --test tls_peet --release -- --ignored"
    cargo test -p leyline-http --test tls_peet --release -- --ignored || fail "live tls_peet tests failed"
    ok "live tls_peet pass"

    step "cargo test -p leyline-http --test smoke -- --ignored --nocapture"
    cargo test -p leyline-http --test smoke -- --ignored --nocapture || fail "smoke suite failed"
    ok "smoke pass"
}

g_deny() {
    step "cargo deny --all-features check"
    if ! command -v cargo-deny >/dev/null; then
        fail "cargo-deny not installed: cargo install --locked cargo-deny"
    fi
    cargo deny --all-features check || fail "cargo-deny found issues"
    ok "cargo-deny clean"
}

g_semver() {
    step "cargo semver-checks check-release -p leyline-http"
    if ! command -v cargo-semver-checks >/dev/null; then
        echo "  (cargo-semver-checks not installed; skipping — install with"
        echo "   'cargo install --locked cargo-semver-checks')"
    elif [[ -z "$(git tag --list 'v*')" ]]; then
        semver_status="skipped (no v* tag to compare against)"
        echo "  (no v* tag exists yet; nothing to compare a release against)"
    else
        cargo semver-checks check-release -p leyline-http \
            || fail "cargo-semver-checks found a breaking change"
        semver_status="ran"
        ok "semver-checks clean"
    fi
}

g_external_types() {
    external_types_nightly="nightly-2026-06-20"
    step "cargo check-external-types leyline-http --features full ($external_types_nightly)"
    if ! command -v cargo-check-external-types >/dev/null; then
        echo "  (cargo-check-external-types not installed; skipping — install with"
        echo "   'cargo install --locked cargo-check-external-types')"
    elif ! rustup toolchain list 2>/dev/null | grep -q "^$external_types_nightly"; then
        external_types_status="skipped ($external_types_nightly toolchain not installed)"
        echo "  ($external_types_nightly not installed; skipping — rustdoc JSON format"
        echo "   57 requires this nightly. Install with"
        echo "   'rustup toolchain install $external_types_nightly')"
    else
        cargo "+$external_types_nightly" check-external-types \
            --manifest-path crates/leyline/Cargo.toml --features full \
            || fail "public API leaks unapproved external types"
        external_types_status="ran"
        ok "external types clean"
    fi
}

g_benches() {
    step "benches compile (sub-workspace)"
    if [[ -d benches ]]; then
        ( cd benches && cargo bench --no-run ) || fail "benches fail to compile"
        ok "benches compile"
    else
        echo "  (benches/ not present; skipping bench compile gate)"
    fi
}

FUZZ_TARGETS=(h2_frame hpack h2_continuation h1_head h1_chunked cookie connect_response qpack)

fuzz_run() {
    local host
    host="$(rustc -vV | sed -n 's/^host: //p')"
    ( cd fuzz && cargo +nightly fuzz run --target "$host" "$@" )
}

g_fuzz_replay() {
    step "fuzz corpus replay (cargo fuzz, -runs=0)"
    if [[ ! -d fuzz ]]; then
        echo "  (fuzz/ not present; skipping corpus replay)"
    elif ! command -v cargo-fuzz >/dev/null; then
        echo "  (cargo-fuzz not installed; skipping corpus replay — install with"
        echo "   'cargo install --locked cargo-fuzz' to enable this gate)"
    elif ! command -v rustc >/dev/null || ! rustup toolchain list 2>/dev/null | grep -q nightly; then
        echo "  (nightly toolchain not installed; skipping corpus replay —"
        echo "   install with 'rustup toolchain install nightly')"
    else
        for target in "${FUZZ_TARGETS[@]}"; do
            echo "  replaying corpus/$target ..."
            fuzz_run "$target" -- -runs=0 \
                </dev/null >/tmp/leyline-fuzz-$target.log 2>&1 \
                || { cat /tmp/leyline-fuzz-$target.log; fail "corpus replay failed for $target"; }
        done
        ok "fuzz corpus replay clean"
    fi
}

g_fuzz_timed() {
    step "fuzz (time-bounded, ${fuzz_seconds}s per target)"
    if [[ ! -d fuzz ]]; then
        fail "fuzz/ not present; cannot run --fuzz"
    fi
    if ! command -v cargo-fuzz >/dev/null; then
        fail "cargo-fuzz not installed: cargo install --locked cargo-fuzz"
    fi
    if ! rustup toolchain list 2>/dev/null | grep -q nightly; then
        fail "nightly toolchain required: rustup toolchain install nightly"
    fi
    for target in "${FUZZ_TARGETS[@]}"; do
        echo "  fuzzing $target for ${fuzz_seconds}s ..."
        fuzz_run "$target" -- -max_total_time="$fuzz_seconds" </dev/null \
            || fail "fuzz run found a crash on $target — inspect fuzz/artifacts/"
    done
    ok "fuzz time-bounded runs clean"
}

gate_order=(
    comments msrv package
    fmt clippy doc api book test live deny semver external-types benches
    fuzz-replay fuzz-timed
)
quick_gates=(comments msrv package)
full_gates=(comments msrv package fmt clippy doc api book test live
    deny semver external-types benches fuzz-replay)

if [[ -n "$only" ]]; then
    IFS=',' read -ra want <<<"$only"
    for g in "${want[@]}"; do
        [[ " ${gate_order[*]} " == *" $g "* ]] \
            || { echo "unknown gate: $g (gates: ${gate_order[*]})" >&2; exit 2; }
    done
elif [[ $full -eq 1 ]]; then
    want=("${full_gates[@]}")
else
    want=("${quick_gates[@]}")
fi
[[ $fuzz -eq 1 ]] && want+=(fuzz-timed)

semver_status="skipped (cargo install --locked cargo-semver-checks)"
external_types_status="skipped (cargo install --locked cargo-check-external-types)"

for g in "${gate_order[@]}"; do
    [[ " ${want[*]} " == *" $g "* ]] && "g_${g//-/_}"
done

if [[ -z "$only" && $full -eq 1 ]]; then
    step "pre-tag matrix parity (.github/workflows/matrix.yml)"
    printf '  %-22s %s\n' \
        'check (MSRV)'        "ran (cargo +$msrv check --workspace)" \
        'check (stable)'      'ran (this toolchain)' \
        'check (beta)'        'matrix only' \
        'each feature'        'matrix only (per-feature cargo check)' \
        'minimal versions'    'matrix only (cargo minimal-versions check)' \
        'semver'              "$semver_status" \
        'external types'      "$external_types_status" \
        'supply chain'        'ran (cargo deny --all-features check)' \
        'cross aarch64-macos' 'matrix only' \
        'cross x86_64-linux'  'matrix only' \
        'cross aarch64-linux' 'matrix only' \
        'cross x86_64-windows' 'matrix only'
    echo "  Run the rest with: gh workflow run matrix.yml"
fi

if [[ -z "$only" && $full -eq 0 ]]; then
    printf '\n\033[1;32mSanity check passed.\033[0m\n'
    exit 0
fi

printf '\n\033[1;32mAll verify gates passed.\033[0m\n'

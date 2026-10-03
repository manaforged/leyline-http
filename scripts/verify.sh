#!/usr/bin/env bash
set -euo pipefail

full=0
fuzz=0
fuzz_seconds=300
only=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --quick) shift ;;
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
skip() { [[ -z "${CI:-}" ]] || fail "$1"; printf '  (%s; skipping)\n' "$1"; }

msrv="$(awk -F'"' '/^rust-version *= *"/{print $2; exit}' Cargo.toml)"

package_stage=""
package_root=""
cleanup() {
    [[ -z "${package_stage:-}" ]] || rm -rf "$package_stage"
    [[ -z "${package_root:-}" ]] || rm -rf "$package_root"
}
trap cleanup EXIT

ensure_msrv() {
    [[ -n "$msrv" ]] || return 0
    rustup toolchain list | grep -q "^$msrv" || rustup toolchain install "$msrv" --profile minimal \
        || fail "could not install Rust $msrv"
}

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
        ensure_msrv
        cargo +"$msrv" --version || fail "Rust $msrv is required; install it with rustup toolchain install $msrv"
    fi

    step "cargo +$msrv check"
    cargo +"$msrv" check --workspace || fail "MSRV cargo check failed"
    step "cargo +$msrv check -p leyline-http --features full"
    cargo +"$msrv" check -p leyline-http --features full || fail "MSRV cargo check with feature full failed"
    ok "MSRV compile sanity"
}

stage_workspace() {
    [[ -z "$package_stage" ]] || return 0
    [[ -z "$(git status --porcelain --untracked-files=no)" ]] \
        || fail "the tree has uncommitted changes; packages build from HEAD"
    package_stage="$(mktemp -d)"
    git archive --format=tar HEAD | tar -xf - -C "$package_stage"
    git submodule foreach --quiet --recursive \
        "git archive --format=tar --prefix=\"\$displaypath/\" HEAD | tar -xf - -C \"$package_stage\"" \
        || fail "failed to stage the submodules"
    node scripts/stage-package-workspace.mjs "$package_stage" \
        || fail "failed to stage the package workspace"
}

g_package() {
    step "cargo package (publishable Rust crates)"
    ensure_msrv
    stage_workspace
    rm -rf "$CARGO_TARGET_DIR/package"
    cargo package \
        --manifest-path "$package_stage/Cargo.toml" \
        --workspace \
        --no-verify \
        || fail "cargo package failed"
    local archive
    for archive in "$CARGO_TARGET_DIR"/package/*.crate; do
        (( $(wc -c <"$archive") <= MAX_CRATE_BYTES )) \
            || fail "$(basename "$archive") is over the crates.io limit of $MAX_CRATE_BYTES bytes"
    done
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
    mkdir -p "$package_root/consumer/src"
    printf 'fn main() {}\n' >"$package_root/consumer/src/main.rs"
    local smoke_features smoke_set smoke_spec smoke_failed=()
    smoke_features="$(cargo metadata --manifest-path "$leyline_package/Cargo.toml" --no-deps --format-version 1 \
        | python3 -c '
import json, sys
pkg = next(p for p in json.load(sys.stdin)["packages"] if p["name"] == "leyline-http")
print(" ".join(sorted(f for f in pkg["features"] if f not in ("default", "full", "bench-internals", "test-util"))))
')" || fail "cargo metadata failed on the packaged leyline-http"
    for smoke_set in none default full $smoke_features; do
        case "$smoke_set" in
            none) smoke_spec='default-features = false' ;;
            default) smoke_spec='default-features = true' ;;
            *) smoke_spec="default-features = false, features = [\"$smoke_set\"]" ;;
        esac
        cat >"$package_root/consumer/Cargo.toml" <<EOF
[package]
name = "leyline-package-smoke"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
leyline-http = { path = "$leyline_package", $smoke_spec }

[patch.crates-io]
leyline-bssl-sys = { path = "$bssl_sys_package" }
leyline-bssl = { path = "$bssl_package" }
leyline-bssl-tokio = { path = "$bssl_tokio_package" }
leyline-quiche = { path = "$quiche_package" }
EOF
        printf '  consumer [%s]\n' "$smoke_set"
        cargo +"$msrv" check --manifest-path "$package_root/consumer/Cargo.toml" \
            || smoke_failed+=("$smoke_set")
    done
    (( ${#smoke_failed[@]} == 0 )) \
        || fail "packaged leyline consumer smoke check failed for: ${smoke_failed[*]}"
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

g_features() {
    step "feature matrix (clippy -p leyline-http per feature set)"
    scripts/feature-matrix.sh || fail "a feature set fails to build or lint"
    ok "feature matrix clean"
}

g_doc() {
    step "rustdoc as docs.rs runs it (DOCS_RS=1, feature full)"
    DOCS_RS=1 RUSTDOCFLAGS="-D warnings" CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}/docs-rs" cargo doc \
        --no-deps \
        --features leyline-http/full \
        -p leyline-http -p leyline-bssl -p leyline-bssl-tokio -p leyline-bssl-sys \
        || fail "rustdoc fails under docs.rs conditions"
    ok "docs.rs build clean"
}

g_api() {
    step "generated API reference"
    cargo truesight check || fail "run cargo truesight sync"
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
    step "cargo test -p leyline-http --features bench-internals --test tls_peet --release -- --ignored"
    cargo test -p leyline-http --features bench-internals --test tls_peet --release -- --ignored || fail "live tls_peet tests failed"
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
    step "cargo semver-checks check-release -p leyline-http (against crates.io)"
    if ! command -v cargo-semver-checks >/dev/null; then
        skip "cargo-semver-checks not installed: cargo install --locked cargo-semver-checks"
        return 0
    fi
    cargo semver-checks check-release -p leyline-http \
        || fail "cargo-semver-checks found a breaking change against the published release"
    semver_status="ran"
    ok "semver-checks clean"
}

g_external_types() {
    external_types_nightly="nightly-2026-06-20"
    step "cargo check-external-types leyline-http --features full ($external_types_nightly)"
    if ! command -v cargo-check-external-types >/dev/null; then
        echo "  (cargo-check-external-types not installed; skipping. Install with"
        echo "   'cargo install --locked cargo-check-external-types')"
    elif ! rustup toolchain list 2>/dev/null | grep -q "^$external_types_nightly"; then
        external_types_status="skipped ($external_types_nightly toolchain not installed)"
        echo "  ($external_types_nightly not installed; skipping. Rustdoc JSON format"
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
        skip "benches/ not present"
    fi
}

FUZZ_TARGETS=(h2_frame hpack h2_continuation h1_head h1_chunked cookie connect_response qpack html link)

fuzz_run() {
    local host
    host="$(rustc -vV | sed -n 's/^host: //p')"
    ( cd fuzz && cargo +nightly fuzz run --target "$host" "$@" )
}

g_fuzz_replay() {
    step "fuzz corpus replay (cargo fuzz, -runs=0)"
    if [[ ! -d fuzz ]]; then
        skip "fuzz/ not present"
    elif ! command -v cargo-fuzz >/dev/null; then
        skip "cargo-fuzz not installed: cargo install --locked cargo-fuzz"
    elif ! command -v rustc >/dev/null || ! rustup toolchain list 2>/dev/null | grep -q nightly; then
        skip "nightly toolchain not installed: rustup toolchain install nightly"
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
            || fail "fuzz run found a crash on $target; inspect fuzz/artifacts/"
    done
    ok "fuzz time-bounded runs clean"
}

MAX_CRATE_BYTES=10485760
CRATES_INDEX=https://index.crates.io
SUBCRATE_MANIFESTS=(
    crates/leyline-bssl-sys/Cargo.toml
    crates/leyline-bssl/Cargo.toml
    crates/leyline-bssl-tokio/Cargo.toml
)
PUBLISHED_MANIFESTS=(crates/leyline/Cargo.toml crates/leyline-quiche/Cargo.toml "${SUBCRATE_MANIFESTS[@]}")
PIN_MANIFESTS=(Cargo.toml "${PUBLISHED_MANIFESTS[@]}")

g_subcrates() {
    for manifest in "${SUBCRATE_MANIFESTS[@]}"; do
        step "cargo test --manifest-path $manifest"
        cargo test --manifest-path "$manifest" || fail "tests failed in $manifest"
    done
    ok "sub-workspace crates clean"
}

g_release() {
    step "release qualification (tag, manifests, changelog)"
    local tag="${GITHUB_REF_NAME:-}"
    [[ -n "$tag" ]] || fail "GITHUB_REF_NAME is unset; set it to the release tag (v<semver>)"
    [[ "$tag" =~ ^v([0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?)$ ]] \
        || fail "tag $tag is not v<semver>"
    local version="${BASH_REMATCH[1]}"
    local workspace_version
    workspace_version="$(awk -F'"' '/^\[/{section=$0} section=="[workspace.package]" && /^version *= *"/{print $2; exit}' Cargo.toml)"
    [[ "$workspace_version" == "$version" ]] \
        || fail "tag $tag does not match [workspace.package] version $workspace_version"
    local manifest pin crate_version
    for manifest in "${PUBLISHED_MANIFESTS[@]}"; do
        crate_version="$(awk -F'"' '/^\[/{section=$0} section=="[package]" && /^version\.workspace *= *true/{print "workspace"; exit} section=="[package]" && /^version *= *"/{print $2; exit}' "$manifest")"
        [[ "$crate_version" == workspace ]] && crate_version="$workspace_version"
        [[ "$crate_version" == "$version" ]] \
            || fail "$manifest is version ${crate_version:-unset}, expected $version"
    done
    for manifest in "${PIN_MANIFESTS[@]}"; do
        while read -r pin; do
            [[ "$pin" == "$version" ]] \
                || fail "$manifest pins a leyline crate to =$pin, expected =$version"
        done < <(grep -E '^leyline-[a-z-]+ *= *\{.*version *= *"=' "$manifest" \
            | sed -E 's/.*version *= *"=([^"]+)".*/\1/')
    done
    grep -qE "^## ${version//./\\.} - [0-9]{4}-[0-9]{2}-[0-9]{2}$" CHANGELOG.md \
        || fail "CHANGELOG.md has no '## $version - YYYY-MM-DD' heading"
    git fetch --quiet origin main 2>/dev/null || true
    git rev-parse --quiet --verify origin/main >/dev/null || fail "origin/main is unknown; fetch it before qualifying a release"
    git merge-base --is-ancestor HEAD origin/main || fail "tag $tag is on a commit that is not on main"
    mkdir -p "$CARGO_TARGET_DIR"
    awk -v head="## $version " 'index($0, head) == 1 {on = 1; next} on && /^## / {exit} on' CHANGELOG.md \
        >"$CARGO_TARGET_DIR/release-notes.md"
    ok "release $tag qualified"
}

published_on_crates_io() {
    local name="$1" version="$2" path
    case ${#name} in
        1) path="1/$name" ;;
        2) path="2/$name" ;;
        3) path="3/${name:0:1}/$name" ;;
        *) path="${name:0:2}/${name:2:2}/$name" ;;
    esac
    curl -fsS "$CRATES_INDEX/$path" 2>/dev/null | grep -q "\"vers\":\"$version\""
}

g_publish() {
    step "cargo publish (every publishable crate not yet on crates.io, in dependency order)"
    [[ -n "${CARGO_REGISTRY_TOKEN:-}" ]] || fail "CARGO_REGISTRY_TOKEN is unset; publish runs from the release workflow"
    stage_workspace
    local name version pending=() excluded=()
    while read -r name version; do
        if published_on_crates_io "$name" "$version"; then
            excluded+=(--exclude "$name")
        else
            pending+=("$name")
        fi
    done < <(cargo metadata --manifest-path "$package_stage/Cargo.toml" --no-deps --format-version 1 \
        | python3 -c 'import json, sys; [print(p["name"], p["version"]) for p in json.load(sys.stdin)["packages"] if p.get("publish") != []]')
    if (( ${#pending[@]} == 0 )); then
        ok "every crate is already on crates.io"
        return 0
    fi
    echo "  publishing: ${pending[*]}"
    rm -rf "$CARGO_TARGET_DIR/package"
    cargo package --manifest-path "$package_stage/Cargo.toml" --workspace --no-verify "${excluded[@]}" \
        || fail "cargo package failed"
    if [[ -n "${RELEASE_PACKAGES:-}" ]]; then
        local archive
        for archive in "$CARGO_TARGET_DIR"/package/*.crate; do
            cmp -s "$archive" "$RELEASE_PACKAGES/$(basename "$archive")" \
                || fail "$(basename "$archive") differs from the package the release job tested"
        done
    fi
    cargo publish --manifest-path "$package_stage/Cargo.toml" --workspace --no-verify "${excluded[@]}" \
        || fail "cargo publish failed"
    ok "crates published"
}

gate_order=(
    comments msrv package
    fmt clippy features doc api book test live deny semver external-types benches
    subcrates release publish
    fuzz-replay fuzz-timed
)
quick_gates=(comments msrv package)
full_gates=(comments msrv package fmt clippy features doc api book test live
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
        'minimal versions'    'matrix only (cargo minimal-versions check)' \
        'semver'              "$semver_status" \
        'external types'      "$external_types_status" \
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

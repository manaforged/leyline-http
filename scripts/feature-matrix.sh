#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

sets=(
    ""
    "default"
    "full"
    "full,bench-internals,socks,tower"
    "http3"
    "websocket"
    "html"
    "multipart"
    "test-util"
    "compression-gzip"
    "compression-brotli"
    "compression-deflate"
    "compression-zstd"
    "socks"
    "tower"
    "charset"
)

failed=()
for set in "${sets[@]}"; do
    case "$set" in
        default) args=() ;;
        "") args=(--no-default-features) ;;
        *) args=(--no-default-features --features "$set") ;;
    esac
    label="${set:-none}"
    case "$set" in
        default | full*) targets=(--all-targets) ;;
        *) targets=(--lib) ;;
    esac
    printf '\n== clippy -p leyline-http %s [%s] ==\n' "${targets[*]}" "$label"
    if cargo clippy -p leyline-http "${targets[@]}" --no-deps --locked "${args[@]}" -- -D warnings; then
        printf 'PASS %s\n' "$label"
    else
        printf 'FAIL %s\n' "$label"
        failed+=("$label")
    fi
done

printf '\n== test -p leyline-http --no-default-features --lib ==\n'
if cargo test -p leyline-http --no-default-features --lib --locked; then
    printf 'PASS no-default-features lib tests\n'
else
    printf 'FAIL no-default-features lib tests\n'
    failed+=("no-default-features-tests")
fi

if [[ ${#failed[@]} -gt 0 ]]; then
    printf 'feature sets failed: %s\n' "${failed[*]}" >&2
    exit 1
fi
printf 'all %d feature sets clean\n' "${#sets[@]}"

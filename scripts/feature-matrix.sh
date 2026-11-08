#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

sets=(
    ""
    "default"
    "full"
    "full,bench-internals,socks,tower"
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

if [[ ${#failed[@]} -gt 0 ]]; then
    printf 'feature sets failed: %s\n' "${failed[*]}" >&2
    exit 1
fi
printf 'all %d feature sets clean\n' "${#sets[@]}"

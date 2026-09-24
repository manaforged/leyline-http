#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

features="$(cargo metadata --no-deps --format-version 1 --locked | python3 -c '
import json, sys
pkg = next(p for p in json.load(sys.stdin)["packages"] if p["name"] == "leyline-http")
print(" ".join(sorted(f for f in pkg["features"] if f not in ("default", "full"))))
')"

sets=("" "default" "full" "full,bench-internals,socks,tower")
for f in $features; do
    sets+=("$f")
done

failed=()
for set in "${sets[@]}"; do
    case "$set" in
        default) args=() ;;
        "") args=(--no-default-features) ;;
        *) args=(--no-default-features --features "$set") ;;
    esac
    label="${set:-none}"
    printf '\n== clippy -p leyline-http [%s] ==\n' "$label"
    if cargo clippy -p leyline-http --all-targets --no-deps --locked "${args[@]}" -- -D warnings; then
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

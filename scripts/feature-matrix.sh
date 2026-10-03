#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

generated=()
while IFS= read -r set; do
    generated+=("$set")
done < <(cargo metadata --no-deps --format-version 1 --locked | python3 -c '
import itertools, json, sys
pkg = next(p for p in json.load(sys.stdin)["packages"] if p["name"] == "leyline-http")
features = pkg["features"]
default = sorted(features.get("default", []))
named = sorted(f for f in features if f not in ("default", "full"))
optional = [f for f in named if f not in default]
sets = list(named)
sets += [",".join(f for f in default if f != drop) for drop in default]
sets += [",".join(pair) for pair in itertools.combinations(optional, 2)]
print("\n".join(dict.fromkeys(sets)))
')
[[ ${#generated[@]} -gt 0 ]] || { echo "no feature sets read from cargo metadata" >&2; exit 1; }

sets=("" "default" "full" "full,bench-internals,socks,tower" "${generated[@]}")

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

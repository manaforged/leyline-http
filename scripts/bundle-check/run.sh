#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
version="$(sed -n 's/.*package = "leyline-http", version = "=\([0-9.]*\)".*/\1/p' "$here/Cargo.toml")"
status="$(curl -sS -o /dev/null -w '%{http_code}' -A "leyline-http bundle check" \
    "https://crates.io/api/v1/crates/leyline-http/$version")"
patch=()
case "$status" in
    200) echo "checking with leyline-http $version from crates.io" ;;
    404)
        echo "leyline-http $version is not on crates.io; checking with the crate source in this checkout" \
            | tee -a "${GITHUB_STEP_SUMMARY:-/dev/null}"
        patch=(--config "patch.crates-io.leyline-http.path=\"$root/crates/leyline\"")
        ;;
    *)
        echo "crates.io answered $status for leyline-http $version" >&2
        exit 1
        ;;
esac
check() { cargo run --quiet --release ${patch[@]+"${patch[@]}"} --manifest-path "$here/Cargo.toml" -- "$@"; }
check fixtures "$root/crates/leyline/tests/data/bundle"
if [[ "${1:-}" == official ]]; then
    renew="$(sed -n 's/^renew_secs = //p' "$root/crates/leyline/profiles/bundle.toml")"
    check official "$((renew / 2))"
else
    check "$@"
fi

#!/usr/bin/env bash
set -euo pipefail

package="leyline-bssl-sys"
generated="bindings.rs"

target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
dest_dir="$repo_root/crates/$package/bindings"
cd "$repo_root"

out_dir="$(
    cargo build --manifest-path "crates/$package/Cargo.toml" --features bindgen --target "$target" --message-format=json \
        | grep '"reason":"build-script-executed"' \
        | grep "$package" \
        | sed -n 's/.*"out_dir":"\([^"]*\)".*/\1/p' \
        | tail -n 1
)"
if [[ -z "$out_dir" || ! -f "$out_dir/$generated" ]]; then
    echo "no generated $generated for $target" >&2
    exit 1
fi

mkdir -p "$dest_dir"
cp "$out_dir/$generated" "$dest_dir/$target.rs"
echo "wrote crates/$package/bindings/$target.rs"

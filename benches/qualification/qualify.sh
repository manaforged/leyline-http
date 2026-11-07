#!/usr/bin/env bash
set -uo pipefail
cd "$(dirname "$0")"
root=$(cd ../.. && pwd)
fuzz_seconds=${FUZZ_SECONDS:-1800}
out="results/qualify/$(git rev-parse --short HEAD)"
mkdir -p "$out"
failed=()

step() {
    local name=$1
    shift
    echo "== $name"
    if "$@" >"$out/$name.txt" 2>&1; then
        echo "   pass"
    else
        echo "   FAIL, see $out/$name.txt"
        failed+=("$name")
    fi
}

fuzz_all() {
    local host targets pids=() status=0
    host=$(rustc -vV | sed -n 's/^host: //p')
    targets=$(sed -n 's/^name = "\(.*\)"$/\1/p' "$root/fuzz/Cargo.toml" | grep -v '^leyline-fuzz$')
    (cd "$root/fuzz" && cargo +nightly fuzz build --target "$host") || return 1
    for target in $targets; do
        (cd "$root/fuzz" && cargo +nightly fuzz run --target "$host" "$target" -- \
            -max_total_time="$fuzz_seconds" -print_final_stats=1 -close_fd_mask=1 </dev/null \
            >"$root/benches/qualification/$out/fuzz-$target.log" 2>&1) &
        pids+=($!)
    done
    for pid in "${pids[@]}"; do
        wait "$pid" || status=1
    done
    return $status
}

differential() {
    local go_flags=()
    if command -v go >/dev/null; then
        GO111MODULE=off go build -o target/go-differential differential/client.go && go_flags=(--go-client target/go-differential)
    fi
    python3 differential.py "${go_flags[@]}" --corpus "$root/fuzz/corpus" --out "$out/differential.json"
}

step full python3 run.py --tier full --force "$@"
step differential differential
step fuzz fuzz_all
step soak python3 run.py --tier soak --force "$@"
cp -r results/full results/soak "$out/" 2>/dev/null

if ((${#failed[@]})); then
    echo "qualification failed: ${failed[*]}"
    exit 1
fi
echo "qualification passed; receipts in $out"

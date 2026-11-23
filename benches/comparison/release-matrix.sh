#!/usr/bin/env bash
set -euo pipefail

C=$(cd "$(dirname "$0")" && pwd)
cd "$C"
ROOT=$(cd ../.. && pwd)

TAG_PREFIX=${TAG_PREFIX:-$(date -u +%F)}
RUNS=${RUNS:-3}
ROUNDS=${ROUNDS:-4}
PACED_ROUNDS=${PACED_ROUNDS:-3}
TARGET_MS=${TARGET_MS:-500}
STOP_REF=${STOP_REF:-leyline}
TIMEOUT=${TIMEOUT:-30s}
BUILD=${BUILD:-1}
SERVER_CPUS=${SERVER_CPUS:-}
CLIENT_CPUS=${CLIENT_CPUS:-}
RENDER_CHARTS=${RENDER_CHARTS:-1}

if [ -z "${CMP_CA:-}" ] && [ -f "$C/pki/ca.der" ]; then
  export CMP_CA=$C/pki/ca.der CMP_CERT=$C/pki/server.der CMP_KEY=$C/pki/server-key.der
fi
[ -n "${CMP_CA:-}" ] || { echo "set CMP_CA, CMP_CERT, and CMP_KEY (see BENCHMARKS.md#reproduce)" >&2; exit 1; }

if git -C "$ROOT" rev-parse --short HEAD >/dev/null 2>&1; then
  git -C "$ROOT" rev-parse --short HEAD > "$C/SOURCE"
fi

if [ "$BUILD" = 1 ]; then
  (cd .. && cargo build --release -q --example origin && cp target/release/examples/origin comparison/bin/server)
  (cd leyline-client && cargo build --release -q && cp target/release/leyline-cmp-client ../bin/leyline && cp target/release/leyline-blocking ../bin/leyline-blocking)
  for lib in hyper h2 reqwest wreq ureq curl; do
    (cd "$lib-client" && cargo build --release -q && b=$lib-cmp-client && { [ -f "target/release/$b" ] || b=$lib; } && cp "target/release/$b" "../bin/$lib")
  done
  (cd go && go build -o ../bin/nethttp ./nethttp && go build -o ../bin/fasthttp ./fasthttp && go build -o ../bin/tlsclient ./tlsclient)
  (cd "$ROOT" && { find crates -type f -not -path '*/target/*' -not -path '*/deps/boringssl/*'; echo Cargo.lock; echo benches/comparison/leyline-client/Cargo.toml; echo benches/comparison/leyline-client/Cargo.lock; } | LC_ALL=C sort | tr '\n' '\0' | xargs -0 cat | sha256sum | cut -d' ' -f1) > "$C/bin/leyline.source_hash"
fi

run_suite() {
  local tag=$1; shift
  echo "=== [$(date -u +%FT%TZ)] $tag ==="
  env BUILD=0 TIMEOUT="$TIMEOUT" SERVER_CPUS="$SERVER_CPUS" CLIENT_CPUS="$CLIENT_CPUS" \
    TAG="$tag" "$@" bash "$C/libs.sh" > "$C/results/$tag.log" 2>&1
}

# 1. General HTTP/2
CACHE="$C/results/${TAG_PREFIX}-clients-h2.sized"; : > "$CACHE"
for run in $(seq 1 "$RUNS"); do
  run_suite "${TAG_PREFIX}-clients-h2-r$run" \
    PROFILE=bare PROTO=h2 ROUNDS="$ROUNDS" TARGET_MS="$TARGET_MS" STOP_REF="$STOP_REF" \
    SIZED_CACHE="$CACHE" \
    LIBS="leyline leyline-cores hyper reqwest curl nethttp h2"
done
rm -f "$CACHE"
python3 "$C/libs-table.py" "$C"/results/"${TAG_PREFIX}"-clients-h2-r*/rows.txt --vs hyper \
  | tee "$C/results/${TAG_PREFIX}-clients-h2-r1/pooled.txt"

# 2. Browser profiles Chrome 146 HTTP/2
CACHE="$C/results/${TAG_PREFIX}-browser-h2.sized"; : > "$CACHE"
for run in $(seq 1 "$RUNS"); do
  run_suite "${TAG_PREFIX}-browser-h2-r$run" \
    PROFILE=chrome146 PROTO=h2 ROUNDS="$ROUNDS" TARGET_MS="$TARGET_MS" STOP_REF="$STOP_REF" \
    SIZED_CACHE="$CACHE" \
    LIBS="leyline wreq tlsclient"
done
rm -f "$CACHE"
python3 "$C/libs-table.py" "$C"/results/"${TAG_PREFIX}"-browser-h2-r*/rows.txt --vs tlsclient \
  | tee "$C/results/${TAG_PREFIX}-browser-h2-r1/pooled.txt"

# 3. General HTTP/1.1
CACHE="$C/results/${TAG_PREFIX}-clients-h1.sized"; : > "$CACHE"
for run in $(seq 1 "$RUNS"); do
  run_suite "${TAG_PREFIX}-clients-h1-r$run" \
    PROFILE=bare PROTO=h1 ROUNDS="$ROUNDS" TARGET_MS="$TARGET_MS" STOP_REF="$STOP_REF" \
    SIZED_CACHE="$CACHE" \
    LIBS="leyline leyline-cores leyline-blocking fasthttp nethttp hyper reqwest curl ureq"
done
rm -f "$CACHE"
python3 "$C/libs-table.py" "$C"/results/"${TAG_PREFIX}"-clients-h1-r*/rows.txt --vs fasthttp \
  | tee "$C/results/${TAG_PREFIX}-clients-h1-r1/pooled.txt"

# 4. Longer HTTP/1.1 (8 and 64 workers)
CACHE="$C/results/${TAG_PREFIX}-h1-long.sized"; : > "$CACHE"
for run in $(seq 1 "$RUNS"); do
  run_suite "${TAG_PREFIX}-h1-long-r$run" \
    PROFILE=bare PROTO=h1 ROUNDS="$ROUNDS" TARGET_MS="$TARGET_MS" STOP_REF="$STOP_REF" \
    SIZED_CACHE="$CACHE" CELLS="8par:1:8:1000000 64par:1:64:1000000" \
    LIBS="leyline leyline-cores leyline-blocking fasthttp nethttp hyper"
done
rm -f "$CACHE"
python3 "$C/libs-table.py" "$C"/results/"${TAG_PREFIX}"-h1-long-r*/rows.txt --vs fasthttp \
  | tee "$C/results/${TAG_PREFIX}-h1-long-r1/pooled.txt"

# 5. Open-loop paced HTTP/2
H2_PACED_ROWS=()
for run in $(seq 1 "$RUNS"); do
  run_suite "${TAG_PREFIX}-paced-h2-r$run-c2x16" \
    MODE=paced PACED_SECONDS="${PACED_LOW_SECONDS:-10}" PROFILE=bare PROTO=h2 ROUNDS="$PACED_ROUNDS" \
    CELLS="c2x16:2:16:2000" LIBS="leyline leyline-cores hyper reqwest wreq curl h2"
  run_suite "${TAG_PREFIX}-paced-h2-r$run-c8x256" \
    MODE=paced PACED_SECONDS="${PACED_HIGH_SECONDS:-5}" PROFILE=bare PROTO=h2 ROUNDS="$PACED_ROUNDS" \
    CELLS="c8x256:8:256:50000" LIBS="leyline leyline-cores hyper reqwest wreq curl h2"
  mkdir -p "$C/results/${TAG_PREFIX}-paced-h2-r$run"
  cat "$C/results/${TAG_PREFIX}-paced-h2-r$run-c2x16/rows.txt" \
      "$C/results/${TAG_PREFIX}-paced-h2-r$run-c8x256/rows.txt" \
      > "$C/results/${TAG_PREFIX}-paced-h2-r$run/rows.txt"
  cp "$C/results/${TAG_PREFIX}-paced-h2-r$run-c8x256/receipts.txt" \
     "$C/results/${TAG_PREFIX}-paced-h2-r$run/receipts.txt"
  H2_PACED_ROWS+=("$C/results/${TAG_PREFIX}-paced-h2-r$run/rows.txt")
done
python3 "$C/libs-table.py" "${H2_PACED_ROWS[@]}" \
  | tee "$C/results/${TAG_PREFIX}-paced-h2-pooled.txt"

# 6. Open-loop paced HTTP/1.1
H1_PACED_ROWS=()
for run in $(seq 1 "$RUNS"); do
  run_suite "${TAG_PREFIX}-paced-h1-r$run-c8" \
    MODE=paced PACED_SECONDS="${PACED_LOW_SECONDS:-10}" PROFILE=bare PROTO=h1 ROUNDS="$PACED_ROUNDS" \
    CELLS="c8:1:8:2000" LIBS="leyline leyline-cores hyper reqwest wreq ureq curl nethttp fasthttp"
  run_suite "${TAG_PREFIX}-paced-h1-r$run-c64" \
    MODE=paced PACED_SECONDS="${PACED_HIGH_SECONDS:-5}" PROFILE=bare PROTO=h1 ROUNDS="$PACED_ROUNDS" \
    CELLS="c64:64:64:20000" LIBS="leyline leyline-cores hyper reqwest wreq ureq curl nethttp fasthttp"
  mkdir -p "$C/results/${TAG_PREFIX}-paced-h1-r$run"
  cat "$C/results/${TAG_PREFIX}-paced-h1-r$run-c8/rows.txt" \
      "$C/results/${TAG_PREFIX}-paced-h1-r$run-c64/rows.txt" \
      > "$C/results/${TAG_PREFIX}-paced-h1-r$run/rows.txt"
  cp "$C/results/${TAG_PREFIX}-paced-h1-r$run-c64/receipts.txt" \
     "$C/results/${TAG_PREFIX}-paced-h1-r$run/receipts.txt"
  H1_PACED_ROWS+=("$C/results/${TAG_PREFIX}-paced-h1-r$run/rows.txt")
done
python3 "$C/libs-table.py" "${H1_PACED_ROWS[@]}" \
  | tee "$C/results/${TAG_PREFIX}-paced-h1-pooled.txt"

if [ "$RENDER_CHARTS" = 1 ]; then
  python3 "$C/render-charts.py" \
    --h2 "$C/results/${TAG_PREFIX}-clients-h2-r1/pooled.txt" \
    --browser "$C/results/${TAG_PREFIX}-browser-h2-r1/pooled.txt" \
    --paced "$C/results/${TAG_PREFIX}-paced-h2-pooled.txt" \
    --h1 "$C/results/${TAG_PREFIX}-h1-long-r1/pooled.txt" \
    --out-dir "$ROOT/docs/assets"
fi

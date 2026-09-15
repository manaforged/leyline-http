#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
ADDR="${ADDR:-127.0.0.1:0}"
WARM="${WARM:-2000}"
COLD="${COLD:-200}"
CONC="${CONC:-20000}"
CONCURRENCY="${CONCURRENCY:-64}"
WREQ_TARGET="${WREQ_TARGET:-target}"

mkdir -p bin

echo "== build go =="
( cd go && go build -o ../bin/server ./server && go build -o ../bin/tlsclient ./tlsclient && go build -o ../bin/azuretls ./azuretls )

echo "== build leyline client =="
( cd leyline-client && cargo build --release -q && cp target/release/leyline-cmp-client ../bin/leyline )

echo "== build wreq client (BoringSSL from source unless WREQ_TARGET reuses one) =="
( cd wreq-client && CARGO_TARGET_DIR="$WREQ_TARGET" cargo build --release -q && cp "$WREQ_TARGET/release/wreq-cmp-client" ../bin/wreq )

echo "== build reqwest client =="
( cd reqwest-client && CARGO_TARGET_DIR="$WREQ_TARGET" cargo build --release -q && cp "$WREQ_TARGET/release/reqwest-cmp-client" ../bin/reqwest )

echo "== start server =="
: > /tmp/cmp-server.log
./bin/server "$ADDR" >/tmp/cmp-server.log 2>&1 &
SRV=$!
trap 'kill $SRV 2>/dev/null || true' EXIT
URL=""
for _ in $(seq 1 100); do
  kill -0 "$SRV" 2>/dev/null || { echo "ERROR: server exited before listening:" >&2; cat /tmp/cmp-server.log >&2; exit 1; }
  URL=$(grep -oE 'https://[0-9.]+:[0-9]+/' /tmp/cmp-server.log | head -1 || true)
  [ -n "$URL" ] && break
  sleep 0.1
done
[ -n "$URL" ] || { echo "ERROR: server never printed a listen URL" >&2; cat /tmp/cmp-server.log >&2; exit 1; }
echo "server at $URL"

echo "== run (warm=$WARM conc=$CONC x$CONCURRENCY cold=$COLD) =="
./bin/leyline   "$URL" "$WARM" "$COLD" "$CONC" "$CONCURRENCY"
./bin/wreq      "$URL" "$WARM" "$COLD" "$CONC" "$CONCURRENCY"
./bin/reqwest   "$URL" "$WARM" "$COLD" "$CONC" "$CONCURRENCY"
./bin/tlsclient "$URL" "$WARM" "$COLD" "$CONC" "$CONCURRENCY"
./bin/azuretls  "$URL" "$WARM" "$COLD" "$CONC" "$CONCURRENCY"

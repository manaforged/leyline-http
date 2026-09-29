#!/usr/bin/env bash
# Independent reference ceiling for the comparison clients.
#
# Runs nghttp2's h2load against the same origin with the same request
# headers and the same connection/stream topology as the Rust comparison
# clients. Use its result as the ceiling for an absolute reading of a
# client's number, not just the peer delta that paired.sh reports.
#
# h2load does not verify peer certificates (OpenSSL's default is
# SSL_VERIFY_NONE), so the control measures an unverified TLS path and no
# CA is needed. The comparison clients do verify. Read the control as a
# transport reference only.
#
# Requirements: h2load (apt package nghttp2-client).
set -euo pipefail
cd "$(dirname "$0")"

H2LOAD="${H2LOAD:-h2load}"
REQUESTS="${REQUESTS:-524288}"
CONCURRENCY="${CONCURRENCY:-64}"
CLIENT_CPUS="${CLIENT_CPUS:-2-15}"
WARM_UP="${WARM_UP:-1}"
TARGET_URL="${TARGET_URL:-}"
ADDR="${ADDR:-127.0.0.1:0}"

if ! command -v "$H2LOAD" >/dev/null 2>&1; then
  echo "ERROR: h2load not found. Install nghttp2-client (Linux) or nghttp2 (macOS)." >&2
  exit 1
fi

# Same non-pseudo request headers that the Chrome 149 comparison clients
# send. The origin must do the same work for the control and the clients.
HEADERS=(
  'sec-ch-ua: "Google Chrome";v="149", "Chromium";v="149", "Not)A;Brand";v="24"'
  'sec-ch-ua-mobile: ?0'
  'sec-ch-ua-platform: "Windows"'
  'upgrade-insecure-requests: 1'
  'user-agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36'
  'accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7'
  'sec-fetch-site: none'
  'sec-fetch-mode: navigate'
  'sec-fetch-user: ?1'
  'sec-fetch-dest: document'
  'accept-encoding: gzip, deflate, br, zstd'
  'accept-language: en-US,en;q=0.9'
  'priority: u=0, i'
)

pin() { if [ -n "$CLIENT_CPUS" ] && command -v taskset >/dev/null 2>&1; then taskset -c "$CLIENT_CPUS" "$@"; else "$@"; fi; }

SRV=""
cleanup() { [ -n "$SRV" ] && kill "$SRV" 2>/dev/null || true; }
trap cleanup EXIT

if [ -n "$TARGET_URL" ]; then
  URL="$TARGET_URL"
else
  : > /tmp/cmp-control-server.log
  ./bin/server "$ADDR" >/tmp/cmp-control-server.log 2>&1 &
  SRV=$!
  URL=""
  for _ in $(seq 1 100); do
    kill -0 "$SRV" 2>/dev/null || { echo "ERROR: server exited" >&2; cat /tmp/cmp-control-server.log >&2; exit 1; }
    URL=$(grep -oE 'https://[0-9.]+:[0-9]+/' /tmp/cmp-control-server.log | head -1 || true)
    [ -n "$URL" ] && break
    sleep 0.1
  done
  [ -n "$URL" ] || { echo "ERROR: server never listened" >&2; exit 1; }
fi

run() { # label connections streams headers(yes/no)
  local label="$1" conns="$2" streams="$3" headers="$4"
  local args=(-n "$REQUESTS" -c "$conns" -m "$streams" -t "$conns" --warm-up-time="$WARM_UP")
  if [ "$headers" = "yes" ]; then
    local h
    for h in "${HEADERS[@]}"; do args+=(-H "$h"); done
  fi
  local out rate done_line latency
  out=$(pin "$H2LOAD" "${args[@]}" "$URL" 2>&1)
  rate=$(printf '%s\n' "$out" | grep -oE 'finished in [^,]+, [0-9.]+ req/s' | grep -oE '[0-9.]+ req/s' | head -1)
  done_line=$(printf '%s\n' "$out" | grep -oE '[0-9]+ succeeded, [0-9]+ failed' | head -1)
  latency=$(printf '%s\n' "$out" | awk '/time for request:/ {print "mean="$6}')
  printf '%-28s %-14s %-22s %s\n' "$label" "$rate" "$done_line" "$latency"
}

echo "== h2load reference: $URL requests=$REQUESTS ==" >&2
printf '%-28s %-14s %-22s %s\n' "config" "req/s" "requests" "latency"
run "light c1 m$CONCURRENCY" 1 "$CONCURRENCY" no
run "browser c1 m$CONCURRENCY" 1 "$CONCURRENCY" yes
run "browser c2 m$((CONCURRENCY / 2))" 2 "$((CONCURRENCY / 2))" yes
run "browser c8 m$((CONCURRENCY / 8))" 8 "$((CONCURRENCY / 8))" yes

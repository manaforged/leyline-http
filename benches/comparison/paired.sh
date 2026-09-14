#!/usr/bin/env bash
# Balanced paired comparison of the comparison clients against one origin.
#
# Each round runs both clients in alternating order, so a warm-up or drift
# effect cannot favour one side. Every round checks the exact response
# equivalence first. The summary reports the paired difference with a 95%
# interval, the winning rounds, and the concurrent p50/p99 latencies.
#
# Set CONTROL=0 to skip the h2load reference row. The row needs h2load but
# no CA: h2load does not verify peer certificates.
set -euo pipefail
cd "$(dirname "$0")"

ROUNDS="${ROUNDS:-20}"
CONC="${CONC:-20000}"
CONCURRENCY="${CONCURRENCY:-64}"
WREQ_TARGET="${WREQ_TARGET:-target}"
SERVER_CPUS="${SERVER_CPUS:-0-1}"
CLIENT_CPUS="${CLIENT_CPUS:-2-15}"
CONTROL="${CONTROL:-1}"

pin_client() { if [ -n "$CLIENT_CPUS" ] && command -v taskset >/dev/null 2>&1; then taskset -c "$CLIENT_CPUS" "$@"; else "$@"; fi; }

mkdir -p bin

echo "== build clients ==" >&2
( cd leyline-client && cargo build --release -q && cp target/release/leyline-cmp-client ../bin/leyline )
( cd wreq-client && CARGO_TARGET_DIR="$WREQ_TARGET" cargo build --release -q && cp "$WREQ_TARGET/release/wreq-cmp-client" ../bin/wreq )

SRV=""
cleanup() { [ -n "$SRV" ] && kill "$SRV" 2>/dev/null || true; }
trap cleanup EXIT

if [ -n "${TARGET_URL:-}" ]; then
  URL="$TARGET_URL"
  echo "== remote target $URL  proxy=${PROXY:-none} ==" >&2
else
  ( cd go && go build -o ../bin/server ./server )
  : > /tmp/cmp-paired-server.log
  if [ -n "$SERVER_CPUS" ] && command -v taskset >/dev/null 2>&1; then
    taskset -c "$SERVER_CPUS" ./bin/server "127.0.0.1:0" >/tmp/cmp-paired-server.log 2>&1 &
  else
    ./bin/server "127.0.0.1:0" >/tmp/cmp-paired-server.log 2>&1 &
  fi
  SRV=$!
  URL=""
  for _ in $(seq 1 100); do
    kill -0 "$SRV" 2>/dev/null || { echo "ERROR: server exited" >&2; cat /tmp/cmp-paired-server.log >&2; exit 1; }
    URL=$(grep -oE 'https://[0-9.]+:[0-9]+/' /tmp/cmp-paired-server.log | head -1 || true)
    [ -n "$URL" ] && break
    sleep 0.1
  done
  [ -n "$URL" ] || { echo "ERROR: server never listened" >&2; exit 1; }
  echo "== local server $URL ==" >&2
fi

echo "== equivalence gate ==" >&2
L_EQ=$(pin_client ./bin/leyline "$URL" equiv)
W_EQ=$(pin_client ./bin/wreq "$URL" equiv)
echo "$L_EQ" >&2
echo "$W_EQ" >&2
l_fp="${L_EQ#*status=}"; w_fp="${W_EQ#*status=}"
if [ "$l_fp" != "$w_fp" ]; then
  echo "EQUIVALENCE FAILED: leyline and wreq saw different responses — rps comparison is invalid." >&2
  exit 2
fi
echo "equivalence OK ($l_fp)" >&2

run_one() { # binary -> "rps p50 p99"
  local out rps p50 p99
  out=$(pin_client "$1" "$URL" 1 1 "$CONC" "$CONCURRENCY")
  rps=$(printf '%s' "$out" | grep -oE 'conc_rps=[0-9.]+' | cut -d= -f2)
  p50=$(printf '%s' "$out" | grep -oE 'conc_p50_us=[0-9.]+' | cut -d= -f2)
  p99=$(printf '%s' "$out" | grep -oE 'conc_p99_us=[0-9.]+' | cut -d= -f2)
  printf '%s %s %s' "${rps:-0}" "${p50:-0}" "${p99:-0}"
}

pairs="$(mktemp)"
echo "== $ROUNDS paired rounds (conc=$CONC x$CONCURRENCY, alternating order) ==" >&2
for r in $(seq 1 "$ROUNDS"); do
  if (( r % 2 )); then
    l=$(run_one ./bin/leyline); w=$(run_one ./bin/wreq); order="leyline,wreq"
  else
    w=$(run_one ./bin/wreq); l=$(run_one ./bin/leyline); order="wreq,leyline"
  fi
  echo "$l $w" >> "$pairs"
  echo "round $r ($order): leyline=$(echo "$l" | cut -d' ' -f1) wreq=$(echo "$w" | cut -d' ' -f1)" >&2
done

awk -v crit="$(awk -v n="$ROUNDS" 'BEGIN {
  split("12.706 4.303 3.182 2.776 2.571 2.447 2.365 2.306 2.262 2.228 2.201 2.179 2.160 2.145 2.131 2.120 2.110 2.101 2.093 2.086 2.080 2.074 2.069 2.064 2.060 2.056 2.052 2.048 2.045", t, " ");
  df = n - 1;
  if (df < 1) df = 1;
  if (df > 29) print "2.045"; else print t[df];
}')" '
{ l=$1; w=$4; d=l-w; n++; sl+=l; sw+=w; sd+=d; sdd+=d*d; if (l>w) wins++
  lp50+=$2; lp99+=$3; wp50+=$5; wp99+=$6 }
END {
  if (n<2) { print "need >= 2 rounds"; exit }
  ml=sl/n; mw=sw/n; md=sd/n;
  var=(sdd - n*md*md)/(n-1); se=sqrt(var/n);
  t = 0; if (se>0) t = md/se;
  lo=md-crit*se; hi=md+crit*se;
  printf("\n=== PAIRED RESULT (n=%d) ===\n", n);
  printf("leyline mean conc_rps : %.3f\n", ml);
  printf("wreq    mean conc_rps : %.3f\n", mw);
  printf("delta (leyline-wreq)  : %.3f  (%+.1f%%)\n", md, 100*md/mw);
  printf("95%% CI of delta       : [%.3f, %.3f]  (t=%.3f)\n", lo, hi, crit);
  printf("paired t              : %.2f  (|t|>%.2f ~ significant at 95%%)\n", t, crit);
  printf("sign test             : leyline faster in %d/%d rounds\n", wins, n);
  printf("mean round p50/p99 us : leyline %.0f/%.0f  wreq %.0f/%.0f\n", lp50/n, lp99/n, wp50/n, wp99/n);
}' "$pairs"
rm -f "$pairs"

if [ "$CONTROL" = "1" ] && command -v h2load >/dev/null 2>&1; then
  echo "" >&2
  echo "== h2load reference (independent client, same headers and topology; does not verify TLS) ==" >&2
  CMP_CA="${CMP_CA:-}" TARGET_URL="$URL" REQUESTS="$CONC" CONCURRENCY="$CONCURRENCY" \
    CLIENT_CPUS="$CLIENT_CPUS" ./control.sh
else
  echo "" >&2
  echo "== h2load reference skipped: install nghttp2-client, or set CONTROL=0 to silence ==" >&2
fi

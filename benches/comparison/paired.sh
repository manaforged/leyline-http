#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

ROUNDS="${ROUNDS:-20}"
CONC="${CONC:-20000}"
CONCURRENCY="${CONCURRENCY:-64}"
WREQ_TARGET="${WREQ_TARGET:-target}"
SERVER_CPUS="${SERVER_CPUS:-0-1}"
CLIENT_CPUS="${CLIENT_CPUS:-2-15}"

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

pairs="$(mktemp)"
echo "== $ROUNDS paired rounds (conc=$CONC x$CONCURRENCY) ==" >&2
for r in $(seq 1 "$ROUNDS"); do
  l=$(pin_client ./bin/leyline "$URL" 1 1 "$CONC" "$CONCURRENCY" | grep -oE 'conc_rps=[0-9]+' | cut -d= -f2)
  w=$(pin_client ./bin/wreq    "$URL" 1 1 "$CONC" "$CONCURRENCY" | grep -oE 'conc_rps=[0-9]+' | cut -d= -f2)
  echo "$l $w" >> "$pairs"
  echo "round $r: leyline=$l wreq=$w" >&2
done

awk '
{ l=$1; w=$2; d=l-w; n++; sl+=l; sw+=w; sd+=d; sdd+=d*d; if (l>w) wins++ }
END {
  if (n<2) { print "need >= 2 rounds"; exit }
  ml=sl/n; mw=sw/n; md=sd/n;
  var=(sdd - n*md*md)/(n-1); se=sqrt(var/n);
  t = (se>0)? md/se : 0;
  crit=2.09; lo=md-crit*se; hi=md+crit*se;
  printf("\n=== PAIRED RESULT (n=%d) ===\n", n);
  printf("leyline mean conc_rps : %.0f\n", ml);
  printf("wreq    mean conc_rps : %.0f\n", mw);
  printf("delta (leyline-wreq)  : %.0f  (%+.1f%%)\n", md, 100*md/mw);
  printf("95%% CI of delta       : [%.0f, %.0f]\n", lo, hi);
  printf("paired t              : %.2f  (|t|>2 ~ significant at 95%%)\n", t);
  printf("sign test             : leyline faster in %d/%d rounds\n", wins, n);
}' "$pairs"
rm -f "$pairs"

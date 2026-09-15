#!/usr/bin/env bash
# Balanced paired comparison of two comparison clients against one origin.
#
# Each round runs both clients in alternating order, so a warm-up or drift
# effect cannot favour one side. Every round checks the exact response
# equivalence first. The summary reports the paired difference with a 95%
# interval, the winning rounds, and the concurrent p50/p99 latencies.
#
# LEFT and RIGHT name the binaries in bin/ (default leyline and wreq).
# WARM and COLD set the per-round sequential counts (default 1 each, which
# keeps the loop concurrent-only; the published reference runs use
# WARM=2000 COLD=200). PAIRED_JSON writes the per-round observations as a
# JSON cell for results/.
#
# Set CONTROL=0 to skip the h2load reference row. The row needs h2load but
# no CA: h2load does not verify peer certificates.
set -euo pipefail
cd "$(dirname "$0")"

ROUNDS="${ROUNDS:-20}"
CONC="${CONC:-20000}"
CONCURRENCY="${CONCURRENCY:-64}"
WARM="${WARM:-1}"
COLD="${COLD:-1}"
WREQ_TARGET="${WREQ_TARGET:-target}"
SERVER_CPUS="${SERVER_CPUS:-0-1}"
CLIENT_CPUS="${CLIENT_CPUS:-2-15}"
CONTROL="${CONTROL:-1}"
LEFT="${LEFT:-leyline}"
RIGHT="${RIGHT:-wreq}"

pin_client() { if [ -n "$CLIENT_CPUS" ] && command -v taskset >/dev/null 2>&1; then taskset -c "$CLIENT_CPUS" "$@"; else "$@"; fi; }

mkdir -p bin

build_client() {
  case "$1" in
    leyline) ( cd leyline-client && cargo build --release -q && cp target/release/leyline-cmp-client ../bin/leyline ) ;;
    wreq) ( cd wreq-client && CARGO_TARGET_DIR="$WREQ_TARGET" cargo build --release -q && cp "$WREQ_TARGET/release/wreq-cmp-client" ../bin/wreq ) ;;
    reqwest) ( cd reqwest-client && CARGO_TARGET_DIR="$WREQ_TARGET" cargo build --release -q && cp "$WREQ_TARGET/release/reqwest-cmp-client" ../bin/reqwest ) ;;
    *) [ -x "bin/$1" ] || { echo "ERROR: no build rule for client '$1' and bin/$1 is missing" >&2; exit 1; } ;;
  esac
}

echo "== build clients ==" >&2
build_client "$LEFT"
build_client "$RIGHT"

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
L_EQ=$(pin_client "./bin/$LEFT" "$URL" equiv)
R_EQ=$(pin_client "./bin/$RIGHT" "$URL" equiv)
echo "$L_EQ" >&2
echo "$R_EQ" >&2
l_fp="${L_EQ#*status=}"; r_fp="${R_EQ#*status=}"
if [ "$l_fp" != "$r_fp" ]; then
  echo "EQUIVALENCE FAILED: $LEFT and $RIGHT saw different responses — rps comparison is invalid." >&2
  exit 2
fi
echo "equivalence OK ($l_fp)" >&2

run_one() { # binary -> "conc_rps p50 p99 warm_rps cold_rps"
  local out conc p50 p99 warm cold
  out=$(pin_client "$1" "$URL" "$WARM" "$COLD" "$CONC" "$CONCURRENCY")
  conc=$(printf '%s' "$out" | grep -oE 'conc_rps=[0-9.]+' | cut -d= -f2)
  p50=$(printf '%s' "$out" | grep -oE 'conc_p50_us=[0-9.]+' | cut -d= -f2)
  p99=$(printf '%s' "$out" | grep -oE 'conc_p99_us=[0-9.]+' | cut -d= -f2)
  warm=$(printf '%s' "$out" | grep -oE 'warm_rps=[0-9.]+' | cut -d= -f2)
  cold=$(printf '%s' "$out" | grep -oE 'cold_rps=[0-9.]+' | cut -d= -f2)
  printf '%s %s %s %s %s' "${conc:-0}" "${p50:-0}" "${p99:-0}" "${warm:-0}" "${cold:-0}"
}

pairs="$(mktemp)"
echo "== $ROUNDS paired rounds: $LEFT vs $RIGHT (warm=$WARM cold=$COLD conc=$CONC x$CONCURRENCY, alternating order) ==" >&2
for r in $(seq 1 "$ROUNDS"); do
  if (( r % 2 )); then
    l=$(run_one "./bin/$LEFT"); w=$(run_one "./bin/$RIGHT"); order="$LEFT,$RIGHT"
  else
    w=$(run_one "./bin/$RIGHT"); l=$(run_one "./bin/$LEFT"); order="$RIGHT,$LEFT"
  fi
  echo "$l $w" >> "$pairs"
  echo "round $r ($order): $LEFT=$(echo "$l" | cut -d' ' -f1) $RIGHT=$(echo "$w" | cut -d' ' -f1)" >&2
done

awk -v crit="$(awk -v n="$ROUNDS" 'BEGIN {
  split("12.706 4.303 3.182 2.776 2.571 2.447 2.365 2.306 2.262 2.228 2.201 2.179 2.160 2.145 2.131 2.120 2.110 2.101 2.093 2.086 2.080 2.074 2.069 2.064 2.060 2.056 2.052 2.048 2.045", t, " ");
  df = n - 1;
  if (df < 1) df = 1;
  if (df > 29) print "2.045"; else print t[df];
}')" -v L="$LEFT" -v R="$RIGHT" '
{ l=$1; w=$6; d=l-w; n++; sl+=l; sw+=w; sd+=d; sdd+=d*d; if (l>w) wins++
  lw=$4; rw=$9; dw=lw-rw; sdw+=dw; sddw+=dw*dw; slw+=lw; srw+=rw
  lc=$5; rc=$10; dc=lc-rc; sdc+=dc; sddc+=dc*dc; slc+=lc; src+=rc
  lp50+=$2; lp99+=$3; rp50+=$7; rp99+=$8 }
END {
  if (n<2) { print "need >= 2 rounds"; exit }
  ml=sl/n; mw=sw/n; md=sd/n;
  var=(sdd - n*md*md)/(n-1); se=sqrt(var/n);
  t = 0; if (se>0) t = md/se;
  lo=md-crit*se; hi=md+crit*se;
  printf("\n=== PAIRED RESULT (n=%d): %s vs %s ===\n", n, L, R);
  printf("%s mean conc_rps : %.3f\n", L, ml);
  printf("%s mean conc_rps : %.3f\n", R, mw);
  printf("delta (%s-%s)  : %.3f  (%+.1f%%)\n", L, R, md, 100*md/mw);
  printf("95%% CI of delta       : [%.3f, %.3f]  (t=%.3f)\n", lo, hi, crit);
  printf("paired t              : %.2f  (|t|>%.2f ~ significant at 95%%)\n", t, crit);
  printf("sign test             : %s faster in %d/%d rounds\n", L, wins, n);
  printf("mean round p50/p99 us : %s %.0f/%.0f  %s %.0f/%.0f\n", L, lp50/n, lp99/n, R, rp50/n, rp99/n);
  mdw=sdw/n; varw=(sddw - n*mdw*mdw)/(n-1); sew=sqrt(varw/n);
  printf("warm_rps delta        : %+.3f  (%+.1f%%)  95%% CI [%.3f, %.3f]\n", mdw, 100*mdw/(srw/n), mdw-crit*sew, mdw+crit*sew);
  mdc=sdc/n; varc=(sddc - n*mdc*mdc)/(n-1); sec=sqrt(varc/n);
  printf("cold_rps delta        : %+.3f  (%+.1f%%)  95%% CI [%.3f, %.3f]\n", mdc, 100*mdc/(src/n), mdc-crit*sec, mdc+crit*sec);
}' "$pairs"

if [ -n "${PAIRED_JSON:-}" ]; then
  LEFT="$LEFT" RIGHT="$RIGHT" WARM="$WARM" COLD="$COLD" CONC="$CONC" CONCURRENCY="$CONCURRENCY" \
    CELL_NAME="${CELL_NAME:-}" ORIGIN="${ORIGIN:-go}" CMP_BODY="${CMP_BODY:-}" \
    CMP_CONNECTIONS="${CMP_CONNECTIONS:-1}" LEYLINE_CHROME="${LEYLINE_CHROME:-}" \
    python3 - "$pairs" "$PAIRED_JSON" <<'PYEOF'
import hashlib, json, os, sys

pairs_path, out_path = sys.argv[1], sys.argv[2]
rounds = []
for i, line in enumerate(open(pairs_path), 1):
    f = line.split()
    rounds.append({
        "round": i,
        "left":  {"conc_rps": float(f[0]), "warm_rps": float(f[3]), "cold_rps": float(f[4])},
        "right": {"conc_rps": float(f[5]), "warm_rps": float(f[8]), "cold_rps": float(f[9])},
    })

def sha(b):
    try:
        return "sha256:" + hashlib.sha256(open(f"bin/{b}", "rb").read()).hexdigest()
    except OSError:
        return None

body = os.environ.get("CMP_BODY")
cell = {
    "name": os.environ.get("CELL_NAME") or f"{os.environ.get('ORIGIN', 'go')}-{body or '10'}b-{os.environ.get('CMP_CONNECTIONS', '1')}c",
    "origin": os.environ.get("ORIGIN", "go"),
    "body_bytes": int(body) if body else 10,
    "connections": int(os.environ.get("CMP_CONNECTIONS", "1")),
    "streams": int(os.environ.get("CONCURRENCY", "64")),
    "profile": os.environ.get("LEYLINE_CHROME", ""),
    "left": os.environ["LEFT"],
    "right": os.environ["RIGHT"],
    "left_sha256": sha(os.environ["LEFT"]),
    "right_sha256": sha(os.environ["RIGHT"]),
    "warm_count": int(os.environ.get("WARM", "1")),
    "cold_count": int(os.environ.get("COLD", "1")),
    "conc_count": int(os.environ.get("CONC", "20000")),
    "rounds": rounds,
}
with open(out_path, "w") as fh:
    json.dump(cell, fh, indent=1)
    fh.write("\n")
print(f"wrote {out_path}", file=sys.stderr)
PYEOF
fi
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

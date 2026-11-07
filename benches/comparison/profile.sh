#!/usr/bin/env bash
# profile.sh LIB...: per-request counters, cycles by layer, hot functions, allocations and (IBS=1) load sources.
set -uo pipefail
C=$(cd "$(dirname "$0")" && pwd); cd "$C"
PROTO=${PROTO:-h2}; export CMP_PROTO=$PROTO CMP_TLS=${CMP_TLS:-matched} CMP_HEADERS=${CMP_HEADERS:-$C/request-headers.tsv}
if [ "$PROTO" = h1 ]; then SHAPE=${SHAPE:-1:64:200000}; else SHAPE=${SHAPE:-8:8:200000}; fi
IFS=: read -r CONNS STREAMS N <<<"$SHAPE"
SERVER_CPUS=${SERVER_CPUS:-0-6,16-22}; CLIENT_CPUS=${CLIENT_CPUS:-8-15,24-31}
OUT=${OUT:-$C/results/$(date +%F-%H%M%S)-profile-$PROTO}; mkdir -p "$OUT" bin
[ "$#" -gt 0 ] || { echo "usage: profile.sh LIB... (leyline, h2, hyper, reqwest, ...)" >&2; exit 1; }

(cd .. && cargo build --release -q --example origin && cp target/release/examples/origin comparison/bin/server) || exit 1
cc -O2 -shared -fPIC -o "$OUT/alloc-count.so" alloc-count.c || exit 1
export CARGO_PROFILE_RELEASE_DEBUG=line-tables-only RUSTFLAGS="-C force-frame-pointers=yes"
for lib in "$@"; do
  (cd "$lib-client" && CARGO_TARGET_DIR=target-profile cargo build --release -q) || exit 1
  b=$(ls "$lib-client"/target-profile/release/{"$lib"-cmp-client,"$lib"} 2>/dev/null | head -1)
  cp "$b" "bin/$lib-profile"
done
unset RUSTFLAGS CARGO_PROFILE_RELEASE_DEBUG

taskset -c "$SERVER_CPUS" bin/server 127.0.0.1:0 > "$OUT/server.log" 2>&1 & SRV=$!
trap 'kill $SRV 2>/dev/null' EXIT
for _ in $(seq 100); do U=$(grep -oE "https://[0-9.]+:[0-9]+/" "$OUT/server.log" | head -1); [ -n "$U" ] && break; sleep 0.05; done
run() { CMP_CONNECTIONS=$CONNS "$@" taskset -c "$CLIENT_CPUS" "bin/$lib-profile" "$U" 1 0 "$n" "$STREAMS" </dev/null; }

for lib in "$@"; do
  d=$OUT/$lib; mkdir -p "$d"; n=$N
  run perf stat -x, -e cycles:u,instructions:u,ls_any_fills_from_sys.local_l2:u,ls_any_fills_from_sys.local_ccx:u -o "$d/stat.csv" > "$d/result.txt" 2>&1
  cyc=$(awk -F, -v n="$N" '/cycles/ {printf "%.0f", $1/n}' "$d/stat.csv")
  run perf record -q -g -e cycles:u -c 100003 -o "$d/cycles.data" > /dev/null 2>&1
  for n in $((N / 2)) "$N"; do run env LD_PRELOAD="$OUT/alloc-count.so" > /dev/null 2> "$d/allocs-$n.txt"; done
  if [ "${IBS:-0}" = 1 ]; then
    n=$N; run sudo -E perf record -q -e ibs_op/cnt_ctl=1,swfilt=1/ -c 50003 -d -o "$d/ibs.data" > /dev/null 2>&1
    sudo chown "$USER" "$d/ibs.data"
  fi
  {
    echo "== $lib $PROTO conns=$CONNS streams=$STREAMS requests=$N"
    grep -oE "conc_rps=[0-9.]+|conc_p50_us=[0-9]+|conc_p99_us=[0-9]+" "$d/result.txt" | tr '\n' ' '; echo
    awk -F, -v n="$N" '/^[0-9]/ {printf "%s %.1f  ", $3, $1/n} END {print ""}' "$d/stat.csv"
    perf script --no-inline -i "$d/cycles.data" -F ip,sym 2>/dev/null | python3 -I profile.py layers "$cyc"
    python3 -I profile.py allocs "$d/allocs-$((N / 2)).txt" "$d/allocs-$N.txt" $((N - N / 2))
    [ -f "$d/ibs.data" ] && perf script -i "$d/ibs.data" -F ip,addr,data_src 2>/dev/null | python3 -I profile.py loads "$N"
  } | tee "$d/summary.txt"
done
echo "raw data: $OUT"

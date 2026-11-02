#!/usr/bin/env bash
set -euo pipefail
C=$(cd "$(dirname "$0")" && pwd); cd "$C"
ulimit -n 200000
export CARGO_INCREMENTAL=1 CARGO_TARGET_DIR=$C/leyline-client/target-quick
(cd leyline-client && cargo build --release -q)
B=$CARGO_TARGET_DIR/release
export CMP_HEADERS=$C/request-headers.tsv LEYLINE_CHROME=bare
pids=()
trap 'kill "${pids[@]}" 2>/dev/null' EXIT
origin() {
  local log; log=$(mktemp -p "$C")
  CMP_PROTO=$1 taskset -c 0-6,16-22 bin/server 127.0.0.1:0 > "$log" 2>&1 & pids+=($!)
  for _ in $(seq 100); do grep -qoE "https://[0-9.]+:[0-9]+/" "$log" && break; sleep 0.05; done
  printf -v "$2" '%s' "$(grep -oE "https://[0-9.]+:[0-9]+/" "$log" | head -1)"; rm -f "$log"
}
origin h1 U1; origin h2 U2
declare -A BIN=([leyline]=$B/leyline-cmp-client [h2]=$C/bin/h2 [hyper]=$C/bin/hyper [pingora]=$C/bin/pingora)
CORES=(8 9 10 11 12 13 14 15 24 25 26 27 28 29 30 31)
rate() {
  local out; out=$(CMP_PROTO=$1 CMP_CONNECTIONS=$2 TOKIO_WORKER_THREADS=$3 taskset -c "$4" "${BIN[$5]}" "$6" 1 0 "$7" "$8" 2>/dev/null)
  grep -oE "conc_rps=[0-9]+" <<<"$out" | cut -d= -f2
}
median() { tr ' ' '\n' | sort -n | awk '{a[NR]=$1} END {print a[int((NR+1)/2)]}'; }

if [ -z "${SKIP_CORES:-}" ]; then
  echo "core scaling: h2, 8 connections x 256 in flight, req/s (median of ${REPS:-3})"
  printf '%-8s' cores; for lib in ${LIBS:-leyline h2 hyper pingora}; do printf '%12s' "$lib"; done; printf '%12s\n' per-core-ley
  for k in ${KS:-1 2 4 8 16}; do
    cpus=$(IFS=,; echo "${CORES[*]:0:$k}")
    printf '%-8s' "$k"
    for lib in ${LIBS:-leyline h2 hyper pingora}; do
      v=$(for _ in $(seq "${REPS:-3}"); do rate h2 8 "$k" "$cpus" "$lib" "$U2" $((50000 * k)) 256; done | paste -sd' ' | median)
      printf '%12s' "$v"; [ "$lib" = leyline ] && ley=$v
    done
    printf '%12s\n' "$((ley / k))"
  done
fi

if [ -z "${SKIP_MEM:-}" ]; then
  echo "memory: peak RSS growth per open connection, one request in flight each (KB)"
  printf '%-14s %10s %10s %12s\n' client rss_lo_kb rss_hi_kb per_conn_kb
  LO=${LO:-1000}; HI=${HI:-10000}
  for spec in h1:leyline h1:hyper h1:pingora h2:leyline h2:h2 h2:hyper h2:pingora; do
    IFS=: read -r proto lib <<<"$spec"; url=$U2; [ "$proto" = h1 ] && url=$U1
    rss() {
      local conns=1; [ "$proto" = h2 ] && conns=$1
      CMP_HOST_CONNS=$1 CMP_PROTO=$proto CMP_CONNECTIONS=$conns /usr/bin/time -f %M taskset -c 8-15,24-31 "${BIN[$lib]}" "$url" 1 0 $(( $1 * 5 )) "$1" 2>&1 >/dev/null | tail -1
    }
    lo=$(rss "$LO"); hi=$(rss "$HI")
    printf '%-14s %10s %10s %12.1f\n' "$proto-$lib" "$lo" "$hi" "$(echo "($hi - $lo) / ($HI - $LO)" | bc -l)"
  done
fi

#!/usr/bin/env bash
set -euo pipefail
C=$(cd "$(dirname "$0")" && pwd); cd "$C"
N=${N:-20000}
export CARGO_INCREMENTAL=1 CARGO_TARGET_DIR=$C/leyline-client/target-quick
export CMP_HEADERS=$C/request-headers.tsv CMP_CONNECTIONS=1
t0=$(date +%s%N)
(cd leyline-client && cargo build --release -q)
B=$CARGO_TARGET_DIR/release
build_ms=$(( ($(date +%s%N) - t0) / 1000000 ))
pids=()
trap 'kill "${pids[@]}" 2>/dev/null' EXIT
origin() {
  local log; log=$(mktemp -p "$C")
  CMP_PROTO=$1 taskset -c 0-6,16-22 bin/server 127.0.0.1:0 > "$log" 2>&1 & pids+=($!)
  for _ in $(seq 100); do grep -qoE "https://[0-9.]+:[0-9]+/" "$log" && break; sleep 0.05; done
  printf -v "$2" '%s' "$(grep -oE "https://[0-9.]+:[0-9]+/" "$log" | head -1)"; rm -f "$log"
}
origin h1 U1; origin h2 U2
one() {
  local st; st=$(mktemp -p "$C")
  local w0; w0=$(date +%s%N)
  local cpus=8 args=("$4" "$5" 0 1 1)
  [ "$6" != 0 ] && cpus=8-15,24-31 args=("$4" 1 0 "$5" "$6")
  CMP_PROTO=$1 LEYLINE_CHROME=$2 CMP_CONNECTIONS=$7 perf stat -x, -e instructions:u -o "$st" taskset -c "$cpus" "$3" "${args[@]}" >/dev/null 2>&1
  echo "$(grep instructions "$st" | cut -d, -f1) $(( $(date +%s%N) - w0 ))"; rm -f "$st"
}
last=$C/.quick.last; touch "$last"; : > "$last.new"
printf '%-12s %10s %7s %8s %10s %8s\n' path instr/req spread delta req/s delta
PATHS=${PATHS:-"h2-bare:h2:bare:$B/leyline-cmp-client:0:1 h2-chrome:h2:152:$B/leyline-cmp-client:0:1 h1-bare:h1:bare:$B/leyline-cmp-client:0:1 h1-blocking:h1:bare:$B/leyline-blocking:0:1"}
CONC="h1-64:h1:bare:$B/leyline-cmp-client:64:1 h1-64-blk:h1:bare:$B/leyline-blocking:64:1 h1-64-ureq:h1:bare:$C/bin/ureq:64:1 h2-8c8:h2:bare:$B/leyline-cmp-client:8:8 h2-8c8-crate:h2:bare:$C/bin/h2:8:8 h2-8c8-pingora:h2:bare:$C/bin/pingora:8:8"
[ -n "${CONC_ONLY:-}" ] && PATHS=$CONC
[ -n "${WITH_CONC:-}" ] && PATHS="$PATHS $CONC"
for spec in $PATHS; do
  IFS=: read -r name proto chrome bin streams conns <<<"$spec"; url=$U2; [ "$proto" = h1 ] && url=$U1
  n=$N; [ "$streams" != 0 ] && n=$((N * 10))
  samples=()
  for _ in $(seq "${REPS:-3}"); do
    read -r i1 w1 <<<"$(one "$proto" "$chrome" "$bin" "$url" "$n" "$streams" "$conns")"
    read -r i2 w2 <<<"$(one "$proto" "$chrome" "$bin" "$url" $((2 * n)) "$streams" "$conns")"
    samples+=("$(( (i2 - i1) / n )) $(( n * 1000000000 / (w2 - w1) ))")
  done
  read -r ins rps spread <<<"$(printf '%s\n' "${samples[@]}" | python3 -c 'import sys,statistics as s; r=[l.split() for l in sys.stdin]; i=[int(a) for a,_ in r]; q=[int(b) for _,b in r]; print(int(s.median(i)), int(s.median(q)), max(i)-min(i))')"
  read -r _ pins prps <<<"$(grep "^$name " "$last" || echo "$name $ins $rps")"
  printf '%-12s %10d %7d %+8d %10d %+7.1f%%\n' "$name" "$ins" "$spread" $((ins - pins)) "$rps" "$(echo "100*($rps-$prps)/$prps" | bc -l)"
  echo "$name $ins $rps" >> "$last.new"
done
[ "${KEEP:-1}" = 1 ] && mv "$last.new" "$last" || rm -f "$last.new"
echo "build ${build_ms} ms, total $(( ($(date +%s%N) - t0) / 1000000 )) ms"

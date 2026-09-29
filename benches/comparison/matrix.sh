#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
OUT="${OUT:-results/$(date +%F)}"
mkdir -p "$OUT"
export ORIGIN=hyper CONTROL=0 ROUNDS="${ROUNDS:-20}"
export SERVER_CPUS="${SERVER_CPUS:-0-6,16-22}" CLIENT_CPUS="${CLIENT_CPUS:-8-15,24-31}"
cell() {
  local name="$1" peer="$2" conns="$3" streams="$4" conc="$5" warm="$6" cold="$7"
  CELL_NAME="$name" RIGHT="$peer" CMP_CONNECTIONS="$conns" CONCURRENCY="$streams" \
    CONC="$conc" WARM="$warm" COLD="$cold" PAIRED_JSON="$OUT/$name-$peer.json" \
    ./paired.sh 2>&1 | tee "$OUT/$name-$peer.log"
}
for peer in ${PEERS:-wreq reqwest tlsclient}; do
  if [ "$peer" = tlsclient ]; then export LEYLINE_CHROME=152; else export LEYLINE_CHROME=149; fi
  cell hyper-1c64 "$peer" 1 64 400000 2000 200
  cell hyper-8c256 "$peer" 8 256 3000000 1 1
  cell hyper-8c8 "$peer" 8 8 200000 1 1
done

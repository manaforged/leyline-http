#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
PROFDATA_DIR=${PROFDATA_DIR:-/tmp/leyline-pgo}
CLIENT=benches/comparison/leyline-client
rm -rf "$PROFDATA_DIR" && mkdir -p "$PROFDATA_DIR"
RUSTFLAGS="-Cprofile-generate=$PROFDATA_DIR" cargo build --release -p leyline-http
echo "run a representative workload with the instrumented client, then:"
echo "  llvm-profdata merge -o merged.profdata $PROFDATA_DIR/*.profraw"
echo "  cargo rustc --release -- -Cprofile-use=merged.profdata"

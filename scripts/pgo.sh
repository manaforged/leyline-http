#!/usr/bin/env bash
# Profile-guided optimization — manual rustc flow (cargo-pgo's env handling
# clobbers .cargo/config.toml flags, which breaks the BoringSSL link).
#
#   1. RUSTFLAGS="-Cprofile-generate=DIR" cargo build --release
#   2. Run the comparison harness phases against the local server
#   3. llvm-profdata merge -o merged.profdata DIR/*.profraw
#   4. cargo rustc --release -- -Cprofile-use=merged.profdata
#      (cargo rustc APPENDS -C flags, preserving .cargo/config.toml)
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
PROFDATA_DIR=${PROFDATA_DIR:-/tmp/leyline-pgo}
CLIENT=benches/comparison/leyline-client
rm -rf "$PROFDATA_DIR" && mkdir -p "$PROFDATA_DIR"
RUSTFLAGS="-Cprofile-generate=$PROFDATA_DIR" cargo build --release -p leyline
echo "run a representative workload with the instrumented client, then:"
echo "  llvm-profdata merge -o merged.profdata $PROFDATA_DIR/*.profraw"
echo "  cargo rustc --release -- -Cprofile-use=merged.profdata"

#!/usr/bin/env bash
set -uo pipefail
cd "$(dirname "$0")"
URL="https://tls.peet.ws/api/all"
for c in leyline wreq tlsclient azuretls; do
  printf "%-12s " "$c"
  ./bin/"$c" "$URL" print 2>/dev/null | grep -oE '"ja4": *"[^"]+"' | head -1 || echo "(no ja4)"
done

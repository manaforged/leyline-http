#!/usr/bin/env bash
# Off-the-clock equivalence check: each client hits tls.peet.ws once and we
# print its JA4. Confirms every client really emits a Chrome-class fingerprint
# (so the perf numbers compare equal work). Requires network; the libraries
# track different Chrome majors, so the JA4s are Chrome-class but not identical.
set -uo pipefail
cd "$(dirname "$0")"
URL="https://tls.peet.ws/api/all"
for c in leyline wreq tlsclient azuretls; do
  printf "%-12s " "$c"
  ./bin/"$c" "$URL" print 2>/dev/null | grep -oE '"ja4": *"[^"]+"' | head -1 || echo "(no ja4)"
done

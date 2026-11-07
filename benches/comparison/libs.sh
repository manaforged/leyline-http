#!/usr/bin/env bash
set -uo pipefail
C=$(cd "$(dirname "$0")" && pwd); cd "$C"; ROOT=$(cd ../.. && pwd)
PROTO=${PROTO:-h2}; export CMP_PROTO=$PROTO; export CMP_TLS=${CMP_TLS:-matched}
PROFILE=${PROFILE:-bare}; TIMEOUT=${TIMEOUT:-default}; export CMP_TIMEOUT=$TIMEOUT; LOAD_BEFORE=$(cut -d' ' -f1-3 /proc/loadavg)
[[ $PROFILE =~ ^(bare|chrome[0-9]*)$ ]] || { echo "PROFILE must be bare, chrome or chrome<N>, got $PROFILE" >&2; exit 1; }
[[ $TIMEOUT =~ ^(default|none|[1-9][0-9]*s)$ ]] || { echo "TIMEOUT must be default, none or <N>s, got $TIMEOUT" >&2; exit 1; }
if [ "$PROFILE" = bare ]; then
  export CMP_HEADERS=${CMP_HEADERS:-$C/request-headers.tsv}
else
  unset CMP_HEADERS
  LIBS=${LIBS:-leyline wreq tlsclient}; kept=""
  for lib in $LIBS; do case $lib in leyline|leyline-nt|leyline-cores|leyline-blocking|leyline-python|leyline-node|wreq|tlsclient) kept="$kept $lib" ;; *) echo "$lib has no browser profile; left out" >&2 ;; esac; done
  LIBS=${kept# }
  [[ " $LIBS " == *" leyline "* ]] || { echo "PROFILE=$PROFILE needs leyline in LIBS: its headers are the reference" >&2; exit 1; }
fi
mkdir -p bin; R=${ROUNDS:-10}
source_hash() {
  (cd "$ROOT" && { find crates -type f -not -path '*/target/*' -not -path '*/deps/boringssl/*'; echo Cargo.lock; echo benches/comparison/leyline-client/Cargo.toml; echo benches/comparison/leyline-client/Cargo.lock; } | LC_ALL=C sort | tr '\n' '\0' | xargs -0 cat | sha256sum | cut -d' ' -f1)
}
DOTNET=${DOTNET:-dotnet}
PYTHON=${PYTHON:-python3}
if [ "$PROTO" = h1 ]; then
  LIBS=${LIBS:-floor leyline leyline-cores leyline-blocking fasthttp ureq hyper}
  CELLS=${CELLS:-seq:1:1:20000 8par:1:8:100000 64par:1:64:200000}
else
  LIBS=${LIBS:-leyline leyline-cores h2 hyper}
  CELLS=${CELLS:-seq:1:1:20000 1c64:1:64:200000 8c8:8:8:200000 8c256:8:256:400000}
fi
if [ "$PROFILE" = bare ]; then
  for lib in $LIBS; do
    case $lib in leyline-python|leyline-node) echo "$lib requires PROFILE=chrome or chrome<N>; bindings do not expose bare sessions" >&2; exit 1 ;; esac
  done
fi
if [ "$TIMEOUT" != default ]; then
  for lib in $LIBS; do
    case $lib in
      leyline|leyline-nt|leyline-cores|leyline-blocking|cand-*|reqwest|hyper|h2|curl|wreq|ureq|nethttp|tlsclient|fasthttp|pingora) ;;
      floor) [ "$TIMEOUT" = none ] || { echo "floor has no request timeout; it cannot honour TIMEOUT=$TIMEOUT" >&2; exit 1; } ;;
      *) echo "$lib does not read CMP_TIMEOUT; it cannot honour TIMEOUT=$TIMEOUT" >&2; exit 1 ;;
    esac
  done
fi
SPIN=(); [ -n "${SERVER_CPUS:-}" ] && SPIN=(taskset -c "$SERVER_CPUS")
[ -n "${SERVER_NETNS:-}" ] && SPIN=(sudo -E ip netns exec "$SERVER_NETNS" sudo -E -u "$USER" "${SPIN[@]}")
CPIN=(); [ -n "${CLIENT_CPUS:-}" ] && CPIN=(taskset -c "$CLIENT_CPUS")

if [ "${BUILD:-1}" = 1 ]; then
  (cd .. && cargo build --release -q --example origin && cp target/release/examples/origin comparison/bin/server) || exit 1
  for lib in $LIBS; do
    case $lib in
      leyline|leyline-nt|leyline-cores|leyline-blocking)
        (cd leyline-client && cargo build --release -q && cp target/release/leyline-cmp-client ../bin/leyline && cp target/release/leyline-blocking ../bin/leyline-blocking) && source_hash > bin/leyline.source_hash ;;
      reqwest|hyper|h2|curl|wreq|ureq|pingora)
        (cd "$lib-client" && cargo build --release -q && b=$lib-cmp-client && { [ -f "target/release/$b" ] || b=$lib; } && cp "target/release/$b" "../bin/$lib") ;;
      floor) (cd floor-client && cargo build --release -q && cp target/release/floor ../bin/floor) ;;
      nethttp|tlsclient|fasthttp) (cd go && go build -o "../bin/$lib" "./$lib") ;;
      dotnet-httpclient) (cd dotnet-client && $DOTNET publish -c Release -o bin --nologo -v q) ;;
      undici|axios) (cd node-client && npm install --silent) ;;
      okhttp|apache-httpclient) sh java-client/fetch-deps.sh >/dev/null ;;
    esac || exit 1
  done
fi

LCH=bare
if [ "$PROFILE" != bare ]; then
  versions() { case $1 in leyline|leyline-nt|leyline-cores|leyline-blocking) ls "$ROOT/crates/leyline/profiles/chrome" | sed -n 's/^\([0-9]*\)\.toml$/\1/p' ;; leyline-python) "$PYTHON" "$C/python-client/leyline_client.py" profiles ;; leyline-node) node "$C/node-client/leyline.mjs" profiles ;; *) "$C/bin/$1" profiles ;; esac; }
  if [ "$PROFILE" = chrome ]; then
    N=$(for lib in $LIBS; do versions "$lib" | sort -u; done | sort -n | uniq -c | awk -v n="$(wc -w <<<"$LIBS")" '$1 == n {v = $2} END {print v}')
    [ -n "$N" ] || { echo "no Chrome version is common to: $LIBS" >&2; exit 1; }
    PROFILE=chrome$N
  fi
  export CMP_CHROME=${PROFILE#chrome}; LCH=$CMP_CHROME
  for lib in $LIBS; do
    versions "$lib" | grep -qx "$CMP_CHROME" || { echo "$lib has no Chrome $CMP_CHROME profile; it has: $(versions "$lib" | sort -n | paste -sd' ')" >&2; exit 1; }
  done
fi
OUT=$C/results/${TAG:-$(date -u +%F)-libs-$PROTO-$PROFILE-timeout-$TIMEOUT}; mkdir -p "$OUT"

CP=$(ls "$C"/java-client/lib/*.jar 2>/dev/null | paste -sd:)
declare -A CMD=(
  [leyline]="env LEYLINE_CHROME=$LCH $C/bin/leyline" [leyline-nt]="env CMP_TIMEOUT=none LEYLINE_CHROME=$LCH $C/bin/leyline" [leyline-cores]="env CMP_CORES=auto LEYLINE_CHROME=$LCH $C/bin/leyline" [leyline-blocking]="env LEYLINE_CHROME=$LCH $C/bin/leyline-blocking" [floor]="$C/bin/floor"
  [reqwest]="$C/bin/reqwest" [hyper]="$C/bin/hyper" [pingora]="$C/bin/pingora"
  [h2]="$C/bin/h2" [curl]="$C/bin/curl" [wreq]="$C/bin/wreq" [nethttp]="$C/bin/nethttp" [tlsclient]="$C/bin/tlsclient"
  [undici]="node $C/node-client/undici.mjs" [node-http2]="node $C/node-client/http2.mjs"
  [httpx]="$PYTHON -I $C/python-client/httpx_client.py"
  [leyline-python]="$PYTHON $C/python-client/leyline_client.py" [leyline-node]="node $C/node-client/leyline.mjs"
  [jdk-httpclient]="java $C/java-client/JdkClient.java" [okhttp]="java -cp $CP $C/java-client/OkClient.java"
  [apache-httpclient]="java -cp $CP $C/java-client/ApacheClient.java" [dotnet-httpclient]="$DOTNET $C/dotnet-client/bin/dotnet-client.dll"
  [ureq]="$C/bin/ureq" [fasthttp]="$C/bin/fasthttp" [axios]="node $C/node-client/axios.mjs"
  [requests]="$PYTHON -I $C/python-client/requests_client.py" [aiohttp]="$PYTHON -I $C/python-client/aiohttp_client.py"
)
declare -A DIV=([httpx]=20 [requests]=20 [aiohttp]=10 [undici]=4 [node-http2]=4 [axios]=4 [jdk-httpclient]=2 [okhttp]=2 [apache-httpclient]=2)
for lib in $LIBS; do case $lib in cand-*) CMD[$lib]="env LEYLINE_CHROME=$LCH $C/bin/$lib" ;; esac; done

"${SPIN[@]}" bin/server "${SERVER_ADDR:-127.0.0.1:0}" > "$OUT/server.log" 2>&1 & SRV=$!
trap 'kill $SRV 2>/dev/null' EXIT
for _ in $(seq 1 100); do U=$(grep -oE "https://[0-9.]+:[0-9]+/" "$OUT/server.log" | head -1); [ -n "$U" ] && break; sleep 0.1; done
[ -n "$U" ] || { echo "origin never listened" >&2; exit 1; }

for lib in $LIBS; do
  ${CMD[$lib]} "$U" equiv 2>&1 | grep -q "status=200 fnv=5df04559e15e1bf4 len=10" || { echo "$lib failed the body check" >&2; exit 1; }
done

WANT=$(awk -F'\t' '{print tolower($1) ": " $2}' "${CMP_HEADERS:-/dev/null}" | LC_ALL=C sort | paste -sd'\n' -)
HOP='^(host|connection|keep-alive|te|upgrade|http2-settings|proxy-connection|transfer-encoding):'
NAMES='header names (hyper map order; pseudo-headers not shown)'
CMP_LOG_HEADERS=1 CMP_LOG_PROTO=1 bin/server 127.0.0.1:0 > "$OUT/audit.log" 2>&1 & AUDIT=$!
for _ in $(seq 1 100); do A=$(grep -oE "https://[0-9.]+:[0-9]+/" "$OUT/audit.log" | head -1); [ -n "$A" ] && break; sleep 0.1; done
[ -n "$A" ] || { echo "audit origin never listened" >&2; exit 1; }
: > "$OUT/headers.txt"; REF=""
audit=$LIBS; [ "$PROFILE" = bare ] || audit="leyline $(tr ' ' '\n' <<<"$LIBS" | grep -vx leyline | paste -sd' ' -)"
for lib in $audit; do
  seen=$(wc -l < "$OUT/audit.log")
  ${CMD[$lib]} "$A" equiv >/dev/null 2>&1
  new=$(tail -n +"$((seen + 1))" "$OUT/audit.log")
  got=$(grep '^HEADERS ' <<<"$new" | awk '!s[$0]++')
  [ "$(grep -c . <<<"$got")" = 1 ] || { printf '%s sent %s distinct header sets\n' "$lib" "$(grep -c . <<<"$got")" >&2; kill $AUDIT; exit 1; }
  fields=$(sed 's/^HEADERS //' <<<"$got" | tr '\037' '\n')
  { echo "$lib $NAMES: $(cut -d: -f1 <<<"$fields" | paste -sd' ' -)"; sed 's/^/  /' <<<"$fields"; echo "$lib $(grep -m1 '^TLS ' <<<"$new")"; } >> "$OUT/headers.txt"
  e2e=$(grep -vE "$HOP" <<<"$fields")
  if [ "$PROFILE" = bare ]; then
    [ "$(LC_ALL=C sort <<<"$e2e")" = "$WANT" ] || { printf '%s sent different headers:\n%s\nwant:\n%s\n' "$lib" "$e2e" "$WANT" >&2; kill $AUDIT; exit 1; }
    continue
  fi
  names=$(cut -d: -f1 <<<"$e2e" | LC_ALL=C sort -u)
  if [ "$lib" = leyline ]; then
    REF=$names; sed 's/: /\t/' <<<"$e2e" > "$OUT/profile-headers.tsv"
    CMD[tlsclient]="env CMP_HEADERS=$OUT/profile-headers.tsv $C/bin/tlsclient"
  elif [ "$names" != "$REF" ]; then
    printf '%s header names differ from leyline %s (< leyline, > %s):\n%s\n' "$lib" "$PROFILE" "$lib" "$(diff <(echo "$REF") <(echo "$names") | grep '^[<>]')" >&2; kill $AUDIT; exit 1
  fi
done
kill $AUDIT; wait $AUDIT 2>/dev/null
if [ "$PROFILE" = bare ]; then
  echo "all clients sent: $(paste -sd';' <<<"$WANT" | sed 's/;/; /g')" >> "$OUT/headers.txt"
else
  echo "name gate passed: $audit sent the same $(grep -c . <<<"$REF") header names as leyline $PROFILE: $(paste -sd' ' <<<"$REF")" >> "$OUT/headers.txt"
fi
cat "$OUT/headers.txt"

P=${U##*:}; P=${P%/}
: > "$OUT/conns.txt"
for cell in $CELLS; do IFS=: read -r name conns streams n <<<"$cell"; [ "$name" = seq ] && continue
  for lib in $LIBS; do
    CMP_CONNECTIONS=$conns "${CPIN[@]}" ${CMD[$lib]} "$U" 1 0 100000000 "$streams" </dev/null >/dev/null 2>&1 & X=$!
    want=$conns; [ "$PROTO" = h1 ] && want=$streams
    for _ in $(seq 30); do seen=$(ss -Htn state established "( dport = :$P )" | wc -l); [ "$seen" -ge "$want" ] && break; sleep 0.1; done
    echo "$name $lib $seen" >> "$OUT/conns.txt"
    pkill -P $X; kill $X; wait $X 2>/dev/null
  done
done

run() {
  local st=$OUT/stat.$$ w0 w1 args=("$U" 1 0 "$4" "$3")
  [ "$3" = 0 ] && args=("$U" "$4" 0 1 1)
  w0=$(date +%s%N)
  CMP_CONNECTIONS=$2 timeout 900 perf stat -x, -e instructions:u,task-clock -o "$st" "${CPIN[@]}" ${CMD[$1]} "${args[@]}" </dev/null >/dev/null 2>>"$OUT/stderr.$1" || return
  w1=$(date +%s%N)
  echo "$(grep instructions "$st" | cut -d, -f1) $(grep task-clock "$st" | cut -d, -f1) $((w1 - w0))"
}

: > "$OUT/rows.txt"
declare -A SIZED
for r in $(seq 1 "$R"); do
  for cell in $CELLS; do IFS=: read -r name conns streams n <<<"$cell"; [ "$name" = seq ] && streams=0
    order=$LIBS; (( r % 2 )) || order=$(tr " " "\n" <<<"$LIBS" | tac | tr "\n" " ")
    for lib in $order; do
      m=${SIZED[$name.$lib]:-$((n / ${DIV[$lib]:-1}))}; [ "$streams" -gt 0 ] && m=$((m / streams * streams))
      [ "$m" -gt 0 ] || { echo "$name: $lib gets no requests per stream" >&2; exit 1; }
      row="$r $name $lib $m $(run "$lib" "$conns" "$streams" "$m") $((2 * m)) $(run "$lib" "$conns" "$streams" $((2 * m)))"
      [ "$(wc -w <<<"$row")" = 11 ] || { echo "$lib failed in round $r of $name; see $OUT/stderr.$lib" >&2; exit 1; }
      echo "$row" | tee -a "$OUT/rows.txt"
      read -r -a f <<<"$row"
      SIZED[$name.$lib]=$(( m * ${TARGET_MS:-1000} * 1000000 / (f[10] - f[6] > 0 ? f[10] - f[6] : 1) ))
      [ "${SIZED[$name.$lib]}" -gt "$m" ] && SIZED[$name.$lib]=$m
      [ "${SIZED[$name.$lib]}" -ge "${streams:-1}" ] || SIZED[$name.$lib]=$(( streams > 0 ? streams : 1 ))
    done
  done
done

{
  echo "date_utc $(date -u +%FT%TZ)"; echo "kernel $(uname -r)"; echo "loadavg_before $LOAD_BEFORE"; echo "loadavg_after $(cut -d' ' -f1-3 /proc/loadavg)"
  echo "governor $(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null)"
  echo "cpu $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs)"
  echo "server_cpus ${SERVER_CPUS:-any} client_cpus ${CLIENT_CPUS:-any}"
  echo "server_addr ${SERVER_ADDR:-127.0.0.1:0} netns ${SERVER_NETNS:-none} link $( [ -n "${SERVER_NETNS:-}" ] && tc qdisc show | grep netem | head -2 | paste -sd";" - || echo loopback)"
  if [ -f bin/leyline.source_hash ]; then
    echo "leyline_source_hash $(cat bin/leyline.source_hash)"
    [ "$(source_hash)" = "$(cat bin/leyline.source_hash)" ] || echo "leyline_source_hash_now $(source_hash) (tree changed after bin/leyline was built)"
  else
    echo "leyline_source_hash unknown (bin/leyline was not built by libs.sh)"
  fi
  [ -f "$C/SOURCE" ] && echo "leyline_commit $(paste -sd' ' "$C/SOURCE")"
  echo "profile $PROFILE"; echo "timeout $TIMEOUT"; echo "leyline_chrome $LCH"; echo "tls $CMP_TLS"
  if [ -n "${CMP_CA:-}" ]; then
    echo "certificate_verification trusted-ca"
    echo "ca_sha256 $(sha256sum "$CMP_CA" | cut -d' ' -f1)"
  else
    echo "certificate_verification disabled"
    echo "ca_sha256 none"
  fi
  for lock in */Cargo.lock; do grep -A1 -E '^name = "(leyline-http|reqwest|hyper|hyper-util|h2|wreq|curl-sys|pingora-core)"$' "$lock" | paste -sd' ' | sed "s#^#$lock #"; done
  echo "go $(go version 2>/dev/null)"; echo "node $(node --version) undici $(node -p "require('$C/node-client/node_modules/undici/package.json').version")"
  echo "python $($PYTHON -V 2>&1) $($PYTHON -m pip freeze 2>/dev/null | grep -E '^(httpx|h2|httpcore)==' | paste -sd' ')"
  if [[ " $LIBS " == *" leyline-python "* ]]; then
    "$PYTHON" -c 'import hashlib,pathlib,sys,leyline,leyline._leyline as native,leyline._session as wrapper; print("leyline_python_version",leyline.__version__); print("leyline_python_native_sha256",hashlib.sha256(pathlib.Path(native.__file__).read_bytes()).hexdigest()); paths=[native.__file__,wrapper.__file__,*sys.argv[1:]]; print("leyline_python_artifact_sha256",hashlib.sha256(b"".join(hashlib.sha256(pathlib.Path(p).read_bytes()).digest() for p in paths)).hexdigest())' "$C/python-client/leyline_client.py" "$C/python-client/httpx_client.py"
  fi
  if [[ " $LIBS " == *" leyline-node "* ]]; then
    node -e 'const fs=require("node:fs"),crypto=require("node:crypto"),p=require("leyline-http"),digest=name=>crypto.createHash("sha256").update(fs.readFileSync(name)).digest(); console.log("leyline_node_version",p.version()); const native=Object.keys(require.cache).filter(n=>n.endsWith(".node")); for(const name of native) console.log("leyline_node_native_sha256",digest(name).toString("hex")); console.log("leyline_node_artifact_sha256",crypto.createHash("sha256").update(Buffer.concat([...native,require.resolve("leyline-http"),...process.argv.slice(1)].map(digest))).digest("hex"))' "$C/node-client/leyline.mjs" "$C/node-client/harness.mjs"
  fi
  echo "java $(java -version 2>&1 | head -1)"; echo "okhttp $(ls java-client/lib | paste -sd' ')"
  echo "dotnet $($DOTNET --version 2>/dev/null)"; echo "proto $PROTO"
  (cd bin && sha256sum server floor leyline leyline-blocking reqwest hyper h2 curl wreq ureq nethttp tlsclient fasthttp pingora 2>/dev/null)
} > "$OUT/receipts.txt"
python3 "$C/libs-table.py" "$OUT/rows.txt" | tee "$OUT/table.txt"

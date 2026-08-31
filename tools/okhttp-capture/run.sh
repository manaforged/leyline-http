#!/usr/bin/env bash
# Hit tls.peet.ws with Maven OkHttp 4.12.0 and 5.5.0. Needs a JDK.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${LEYLINE_ONESHOT_OUT:-${TMPDIR:-/tmp}/leyline-oneshot}"
cache="${LEYLINE_CFT_CACHE:-${HOME}/.cache/leyline-cft}/okhttp"
mkdir -p "$out" "$cache"
JAVA="${JAVA:-}"
if [[ -z "$JAVA" ]]; then
    if [[ -x /opt/homebrew/opt/openjdk@21/bin/java ]]; then
        JAVA=/opt/homebrew/opt/openjdk@21/bin/java
    elif command -v java >/dev/null 2>&1 && java -version >/dev/null 2>&1; then
        JAVA="$(command -v java)"
    fi
fi
if [[ -z "${JAVA}" ]] || ! "$JAVA" -version >/dev/null 2>&1; then
    echo "okhttp capture needs a JDK (macOS /usr/bin/java is a stub)." >&2
    exit 1
fi
JAVAC="${JAVA%java}javac"
MAVEN="https://repo1.maven.org/maven2"

fetch() {
    local dest="$1" url="$2"
    if [[ ! -s "$dest" ]]; then
        echo "downloading $url"
        curl -fL --max-time 60 -o "$dest" "$url"
    fi
}

fetch "$cache/okhttp-4.12.0.jar" "$MAVEN/com/squareup/okhttp3/okhttp/4.12.0/okhttp-4.12.0.jar"
fetch "$cache/okio-3.6.0.jar" "$MAVEN/com/squareup/okio/okio-jvm/3.6.0/okio-jvm-3.6.0.jar"
fetch "$cache/kotlin-stdlib-1.8.21.jar" "$MAVEN/org/jetbrains/kotlin/kotlin-stdlib/1.8.21/kotlin-stdlib-1.8.21.jar"
fetch "$cache/kotlin-stdlib-jdk8-1.8.21.jar" "$MAVEN/org/jetbrains/kotlin/kotlin-stdlib-jdk8/1.8.21/kotlin-stdlib-jdk8-1.8.21.jar"
fetch "$cache/kotlin-stdlib-jdk7-1.8.21.jar" "$MAVEN/org/jetbrains/kotlin/kotlin-stdlib-jdk7/1.8.21/kotlin-stdlib-jdk7-1.8.21.jar"
fetch "$cache/okhttp-jvm-5.5.0.jar" "$MAVEN/com/squareup/okhttp3/okhttp-jvm/5.5.0/okhttp-jvm-5.5.0.jar"
fetch "$cache/okio-jvm-3.18.1.jar" "$MAVEN/com/squareup/okio/okio-jvm/3.18.1/okio-jvm-3.18.1.jar"
fetch "$cache/kotlin-stdlib-2.1.21.jar" "$MAVEN/org/jetbrains/kotlin/kotlin-stdlib/2.1.21/kotlin-stdlib-2.1.21.jar"

"$JAVAC" -cp "$cache/okhttp-4.12.0.jar" -d "$cache" "$here/Capture.java"

run_one() {
    local name="$1" cp="$2"
    local dest="$out/okhttp-${name}.peet.json"
    echo "== okhttp ${name} =="
    "$JAVA" -cp "$cache:$cp" Capture "https://tls.peet.ws/api/all" >"$dest"
    python3 - "$dest" <<'PY'
import json, pathlib, sys
j = json.loads(pathlib.Path(sys.argv[1]).read_text())
print("ua\t", j.get("user_agent"))
print("ja4\t", (j.get("tls") or {}).get("ja4"))
print("akamai\t", (j.get("http2") or {}).get("akamai_fingerprint"))
print("ciphers\t", (j.get("tls") or {}).get("ciphers"))
PY
}

cp412="$cache/okhttp-4.12.0.jar:$cache/okio-3.6.0.jar:$cache/kotlin-stdlib-1.8.21.jar:$cache/kotlin-stdlib-jdk8-1.8.21.jar:$cache/kotlin-stdlib-jdk7-1.8.21.jar"
cp55="$cache/okhttp-jvm-5.5.0.jar:$cache/okio-jvm-3.18.1.jar:$cache/kotlin-stdlib-2.1.21.jar"
run_one "4.12.0" "$cp412"
run_one "5.5.0" "$cp55"
echo "wrote $out/okhttp-4.12.0.peet.json and $out/okhttp-5.5.0.peet.json"

#!/bin/sh
set -eu
repo=https://repo1.maven.org/maven2
lib="$(cd "$(dirname "$0")" && pwd)/lib"
mkdir -p "$lib"
for coord in \
    com/squareup/okhttp3/okhttp-jvm/5.5.1 \
    com/squareup/okio/okio-jvm/3.18.1 \
    org/jetbrains/kotlin/kotlin-stdlib/2.1.21 \
    org/jetbrains/annotations/13.0 \
    org/apache/httpcomponents/client5/httpclient5/5.6.5 \
    org/apache/httpcomponents/core5/httpcore5/5.4.4 \
    org/apache/httpcomponents/core5/httpcore5-h2/5.4.4 \
    org/slf4j/slf4j-api/1.7.36 \
    org/slf4j/slf4j-nop/1.7.36
do
    artifact=$(basename "$(dirname "$coord")")
    version=$(basename "$coord")
    jar="$artifact-$version.jar"
    curl -fsSL -o "$lib/$jar" "$repo/$coord/$jar"
    curl -fsSL -o "$lib/$jar.sha1" "$repo/$coord/$jar.sha1"
    want=$(cut -c1-40 "$lib/$jar.sha1")
    got=$( (sha1sum "$lib/$jar" 2>/dev/null || shasum -a 1 "$lib/$jar") | cut -c1-40)
    [ "$want" = "$got" ] || { echo "sha1 mismatch: $jar" >&2; exit 1; }
    rm "$lib/$jar.sha1"
done

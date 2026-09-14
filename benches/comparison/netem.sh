#!/usr/bin/env bash
# WAN emulation comparison: run the paired client comparison over a virtual
# link with a real round-trip delay. The origin runs in its own network
# namespace and the delay applies only to the veth pair, so no other
# service on the host is affected.
#
#     CMP_CA=ca.der CMP_CA_KEY=ca-key.pem RTT_MS=30 ./netem.sh
#
# Requirements: ip and tc (iproute2), openssl, sudo for the namespace
# setup, and a DER test CA in CMP_CA. The origin certificate must include
# the virtual link address in its SAN; set CMP_CA_KEY to mint one from the
# test CA, or set NETEM_CERT and NETEM_KEY to a prepared leaf.
set -euo pipefail
cd "$(dirname "$0")"

RTT_MS="${RTT_MS:-30}"
NS="${NS:-leyline-netem}"
HOST_IP="${HOST_IP:-192.0.2.1}"
NS_IP="${NS_IP:-192.0.2.2}"
ORIGIN="${ORIGIN:-server}"
SERVER_CPUS="${SERVER_CPUS:-0-1}"
SUDO="${SUDO:-sudo -n}"
LEAF="${NETEM_CERT:-}"
KEY="${NETEM_KEY:-}"

command -v ip >/dev/null 2>&1 || { echo "ERROR: ip (iproute2) required" >&2; exit 1; }
command -v tc >/dev/null 2>&1 || { echo "ERROR: tc (iproute2) required" >&2; exit 1; }
[ -n "${CMP_CA:-}" ] || { echo "ERROR: set CMP_CA to the DER test CA" >&2; exit 1; }

cleanup() {
  [ -n "${ORIGIN_PID:-}" ] && $SUDO kill "$ORIGIN_PID" 2>/dev/null || true
  $SUDO ip netns del "$NS" 2>/dev/null || true
  $SUDO ip link del veth-host 2>/dev/null || true
}
trap cleanup EXIT
cleanup

$SUDO ip netns add "$NS"
$SUDO ip link add veth-host type veth peer name veth-ns
$SUDO ip link set veth-ns netns "$NS"
$SUDO ip addr add "$HOST_IP/24" dev veth-host
$SUDO ip link set veth-host up
$SUDO ip netns exec "$NS" ip addr add "$NS_IP/24" dev veth-ns
$SUDO ip netns exec "$NS" ip link set veth-ns up
$SUDO ip netns exec "$NS" ip link set lo up
half=$(( (RTT_MS + 1) / 2 ))
$SUDO tc qdisc add dev veth-host root netem delay "${half}ms"
$SUDO ip netns exec "$NS" tc qdisc add dev veth-ns root netem delay "${half}ms"
echo "== netem link: ${RTT_MS}ms RTT, ${HOST_IP} <-> ${NS_IP} ==" >&2

if [ -z "$LEAF" ]; then
  [ -n "${CMP_CA_KEY:-}" ] || {
    echo "ERROR: set CMP_CA_KEY to mint a leaf, or NETEM_CERT and NETEM_KEY" >&2
    exit 1
  }
  work="$(mktemp -d)"
  ext="$work/leaf.ext"
  printf 'basicConstraints=CA:FALSE\nsubjectAltName=DNS:localhost,IP:%s\n' "$NS_IP" > "$ext"
  openssl x509 -inform der -in "$CMP_CA" -out "$work/ca.pem" 2>/dev/null
  openssl req -new -newkey rsa:2048 -nodes -keyout "$work/key.pem" -subj "/CN=leyline-netem" -out "$work/req.csr" 2>/dev/null
  openssl x509 -req -in "$work/req.csr" -CA "$work/ca.pem" -CAkey "$CMP_CA_KEY" -CAcreateserial \
    -out "$work/leaf.pem" -days 3 -extfile "$ext" 2>/dev/null
  openssl x509 -in "$work/leaf.pem" -outform der -out "$work/leaf.der"
  openssl pkcs8 -topk8 -nocrypt -in "$work/key.pem" -outform der -out "$work/key.der"
  LEAF="$work/leaf.der"
  KEY="$work/key.der"
fi

log="$(mktemp)"
extra_env=()
[ -n "${CMP_BODY:-}" ] && extra_env+=(CMP_BODY="$CMP_BODY")
$SUDO ip netns exec "$NS" env -i PATH=/usr/bin:/bin \
  CMP_CA="$CMP_CA" CMP_CERT="$LEAF" CMP_KEY="$KEY" \
  ${extra_env[@]+"${extra_env[@]}"} CMP_TLS="${CMP_TLS:-matched}" GOMAXPROCS=2 \
  /usr/bin/taskset -c "$SERVER_CPUS" "./bin/$ORIGIN" "$NS_IP:0" > "$log" 2>&1 &
ORIGIN_PID=$!

URL=""
for _ in $(seq 1 100); do
  kill -0 "$ORIGIN_PID" 2>/dev/null || { echo "ERROR: origin exited:" >&2; cat "$log" >&2; exit 1; }
  URL=$(grep -oE "https://${NS_IP//./\\.}:[0-9]+/" "$log" | head -1 || true)
  [ -n "$URL" ] && break
  sleep 0.1
done
[ -n "$URL" ] || { echo "ERROR: origin never listened" >&2; exit 1; }
echo "== origin at $URL ==" >&2

CMP_CA="$CMP_CA" TARGET_URL="$URL" ./paired.sh

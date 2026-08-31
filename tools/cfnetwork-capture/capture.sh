#!/usr/bin/env bash
# cfnetwork-capture: capture the CFNetwork/URLSession wire fingerprint
# (TLS ClientHello + H2 frame order) from iOS Simulator or macOS host.
#
# Outputs an evidence pack under out/<name>/:
#   raw.pcap            tcpdump loopback capture (handshake + h2 frames)
#   server.log          H2 frame order log (SETTINGS/WINDOW_UPDATE/HEADERS...)
#   clienthello.txt     tshark ClientHello field dump (order-preserving)
#   candidate.toml      skeleton profile fragment derived from the capture
#   meta.txt            platform/build/run metadata
#
# Requires: xcodebuild + simulator runtime (iOS), swift, go, tshark; sudo for
# tcpdump (loopback capture needs root on macOS).
#
# Usage:
#   capture.sh --platform ios18 [--runs 3] [--out NAME] [--device UDID]
#   capture.sh --platform macos [--runs 3] [--out NAME]
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLATFORM=""
RUNS=3
OUT=""
DEVICE=""           # UDID for ios; default: first iPhone of the chosen runtime
RUNTIME_VER="18.6"  # only used to pick a default device

while [ $# -gt 0 ]; do
    case "$1" in
        --platform) PLATFORM="$2"; shift 2 ;;
        --runs)     RUNS="$2"; shift 2 ;;
        --out)      OUT="$2"; shift 2 ;;
        --device)   DEVICE="$2"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

case "$PLATFORM" in
    ios18|ios26|macos) ;;
    *) echo "usage: capture.sh --platform ios18|ios26|macos [--runs N] [--out NAME] [--device UDID]" >&2; exit 2 ;;
esac

# tcpdump on loopback needs root. Prefer a cached ticket; otherwise sudo prompts.
if ! sudo -n true 2>/dev/null; then
    echo "[capture] tcpdump needs root; sudo will prompt." >&2
    sudo -v
fi

STAMP="$(date +%Y%m%d-%H%M%S)"
OSBUILD="$(sw_vers -buildVersion 2>/dev/null || echo unknown)"
OUT_DIR="$HERE/out/${OUT:-${PLATFORM}-${STAMP}-${OSBUILD}}"
mkdir -p "$OUT_DIR"
echo "[capture] platform=$PLATFORM runs=$RUNS"
echo "[capture] out=$OUT_DIR"

# ── build the probe ──────────────────────────────────────────────────────────
PROBE_BIN="$HERE/probe/.build/debug/cfnetwork-probe"
if [ "$PLATFORM" = "macos" ]; then
    (cd "$HERE/probe" && swift build -q)
else
    SDK="$(xcrun --sdk iphonesimulator --show-sdk-path)"
    (cd "$HERE/probe" && swift build -q --triple arm64-apple-ios-simulator --sdk "$SDK")
    PROBE_BIN="$HERE/probe/.build/arm64-apple-ios-simulator/debug/cfnetwork-probe"
fi
[ -x "$PROBE_BIN" ] || { echo "[capture] probe build failed" >&2; exit 1; }

# ── boot the simulator (ios only) ───────────────────────────────────────────
SIM_DEVICE="$DEVICE"
if [ "$PLATFORM" != "macos" ]; then
    if [ -z "$SIM_DEVICE" ]; then
        SIM_DEVICE="$(xcrun simctl list devices available \
            | grep -B1 -A4 "iOS $RUNTIME_VER" \
            | grep -oE '[0-9A-F-]{36}' | head -1 || true)"
    fi
    [ -n "$SIM_DEVICE" ] || { echo "[capture] no simulator device found for iOS $RUNTIME_VER" >&2; exit 1; }
    STATE="$(xcrun simctl list devices | grep "$SIM_DEVICE" | grep -oE '\((Booted|Shutdown)\)' || echo "(Shutdown)")"
    if [ "$STATE" = "(Shutdown)" ]; then
        echo "[capture] booting simulator $SIM_DEVICE"
        xcrun simctl boot "$SIM_DEVICE" || true
        sleep 5
    fi
    echo "[capture] device=$SIM_DEVICE"
fi

# ── start the server ─────────────────────────────────────────────────────────
(
    cd "$HERE/server"
    [ -x ./capture-server ] || go build -o capture-server .
) || { echo "[capture] server build failed" >&2; exit 1; }

CAPTURE_CERT_OUT="$OUT_DIR/server-cert" "$HERE/server/capture-server" 127.0.0.1:0 \
    > "$OUT_DIR/server.log" 2>&1 &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
    URL="$(grep -oE 'LISTENING .*' "$OUT_DIR/server.log" | head -1 || true)"
    [ -n "$URL" ] && break
    sleep 0.1
done
[ -n "$URL" ] || { echo "[capture] server did not start" >&2; exit 1; }
PORT="${URL##*:}"
TARGET="https://127.0.0.1:${PORT}/"
echo "[capture] target=$TARGET"

# ── capture ──────────────────────────────────────────────────────────────────
PCAP="$OUT_DIR/raw.pcap"
sudo tcpdump -i lo0 -w "$PCAP" "tcp port $PORT" >/dev/null 2>&1 &
TCPDUMP_PID=$!
trap 'kill $SERVER_PID $TCPDUMP_PID 2>/dev/null || true' EXIT
sleep 1

echo "[capture] running probe ($RUNS runs)"
for i in $(seq 1 "$RUNS"); do
    if [ "$PLATFORM" = "macos" ]; then
        "$PROBE_BIN" "$TARGET" 1 0.5
    else
        xcrun simctl spawn "$SIM_DEVICE" "$PROBE_BIN" "$TARGET" 1 0.5
    fi
    sleep 1
done

sleep 1
kill $TCPDUMP_PID 2>/dev/null || true
wait $TCPDUMP_PID 2>/dev/null || true
kill $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true
trap - EXIT

# ── parse ────────────────────────────────────────────────────────────────────
echo "[capture] parsing pcap ($(stat -f%z "$PCAP") bytes)"
# Wireshark 4.6 field names (underscores, not dots, on extension lists).
tshark -r "$PCAP" -Y "tls.handshake.type==1" -T fields \
    -e frame.number \
    -e tls.handshake.ciphersuite \
    -e tls.handshake.extensions_supported_group \
    -e tls.handshake.sig_hash_alg \
    -e tls.handshake.extensions_alpn_str \
    -e tls.handshake.extension.type \
    -E header=y -E separator='|' -E occurrence=a > "$OUT_DIR/clienthello.txt" \
    || echo "[capture] no ClientHello frames found in pcap" >&2
if ! grep -q '^[0-9]' "$OUT_DIR/clienthello.txt" 2>/dev/null; then
    echo "[capture] clienthello.txt has no hello rows" >&2
fi

echo "[capture] writing meta + candidate fragment"
{
    echo "platform=$PLATFORM"
    echo "captured_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "host_osbuild=$OSBUILD"
    if [ "$PLATFORM" = "macos" ]; then
        echo "captured_against=macOS-$(sw_vers -productVersion)-$OSBUILD"
    else
        RUNTIME="$(xcrun simctl list runtimes | grep -i ios | tr -s ' ')"
        echo "simulator_runtime=$RUNTIME"
        echo "captured_against=ios-$(echo "$RUNTIME" | grep -oE '[0-9]+\.[0-9]+' | head -1)"
    fi
    echo "runs=$RUNS"
    echo "target=$TARGET"
} > "$OUT_DIR/meta.txt"

echo ""
echo "[capture] DONE. Evidence pack:"
ls -la "$OUT_DIR"
echo ""
echo "[capture] Next: read clienthello.txt + server.log, author the profile"
echo "[capture] fragment under profiles/cfnetwork/"

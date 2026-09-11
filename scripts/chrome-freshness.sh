#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

CFT_URL='https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json'
LABEL='com.leyline.chrome-freshness'
PLIST_DST="${HOME}/Library/LaunchAgents/${LABEL}.plist"
MAX_LAG="${CHROME_FRESHNESS_MAX_LAG:-1}"

install_launchd() {
    mkdir -p "${HOME}/Library/LaunchAgents" "${HOME}/Library/Logs"
    local src="$repo_root/scripts/profile-oneshot.sh"
    cat >"$PLIST_DST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${LABEL}</string>
  <key>WorkingDirectory</key>
  <string>${repo_root}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>CHROME_FRESHNESS_ONESHOT</key>
    <string>1</string>
  </dict>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/bash</string>
    <string>${src}</string>
    <string>all</string>
  </array>
  <key>StartCalendarInterval</key>
  <dict>
    <key>Weekday</key>
    <integer>1</integer>
    <key>Hour</key>
    <integer>9</integer>
    <key>Minute</key>
    <integer>0</integer>
  </dict>
  <key>StandardOutPath</key>
  <string>${HOME}/Library/Logs/leyline-chrome-freshness.log</string>
  <key>StandardErrorPath</key>
  <string>${HOME}/Library/Logs/leyline-chrome-freshness.log</string>
</dict>
</plist>
EOF
    launchctl bootout "gui/${UID}/${LABEL}" >/dev/null 2>&1 || true
    launchctl bootstrap "gui/${UID}" "$PLIST_DST"
    echo "installed ${PLIST_DST} (Mondays 09:00 local)"
    echo "log: ${HOME}/Library/Logs/leyline-chrome-freshness.log"
}

uninstall_launchd() {
    launchctl bootout "gui/${UID}/${LABEL}" >/dev/null 2>&1 || true
    rm -f "$PLIST_DST"
    echo "removed ${LABEL}"
}

case "${1:-}" in
    --install-launchd) install_launchd; exit 0 ;;
    --uninstall-launchd) uninstall_launchd; exit 0 ;;
    -h|--help)
        sed -n '2,12p' "$0"
        exit 0
        ;;
esac

bundled="$(
    python3 - <<'PY'
from pathlib import Path
root = Path("crates/leyline/profiles/chrome")
majors = []
for p in root.glob("*.toml"):
    try:
        majors.append(int(p.stem))
    except ValueError:
        pass
if not majors:
    raise SystemExit("no chrome profile tomls")
print(max(majors))
PY
)"

stable_json="$(curl -fsSL --max-time 20 "$CFT_URL")"
stable="$(
    python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["channels"]["Stable"]["version"])' <<<"$stable_json"
)"
stable_major="${stable%%.*}"

lag=$((stable_major - bundled))
if (( lag < 0 )); then
    lag=0
fi

ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "${ts} bundled=${bundled} stable=${stable} lag=${lag} max_lag=${MAX_LAG}"

if (( lag <= MAX_LAG )); then
    echo "ok: chrome profile is within ${MAX_LAG} major of Chrome for Testing Stable"
    exit 0
fi

cat >&2 <<EOF
behind: bundled Chrome ${bundled}, Stable ${stable} (lag ${lag} > ${MAX_LAG})

Fill the gap (writes TOML and wires Browser/registry):
  scripts/profile-oneshot.sh

This script does not invent fingerprints.
EOF
if [[ "${CHROME_FRESHNESS_ONESHOT:-0}" == "1" ]]; then
    echo "running profile-oneshot.sh"
    "$repo_root/scripts/profile-oneshot.sh" || true
fi
exit 1

#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
exec env PYTHONPATH="$repo_root/scripts" python3 -m profile_capture "$@"

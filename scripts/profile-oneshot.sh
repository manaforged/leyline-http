#!/usr/bin/env bash
# Profile suite: catalog live versions, capture gaps, land TOML + wire Rust.
#
#   scripts/profile-oneshot.sh              # fill every fillable gap
#   scripts/profile-oneshot.sh status
#   scripts/profile-oneshot.sh --dry-run
#   scripts/profile-oneshot.sh chrome|firefox|safari|edge
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"
exec python3 "$repo_root/scripts/profile_suite.py" "$@"

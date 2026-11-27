#!/usr/bin/env bash
set -euo pipefail

app_name="bash2rust-hybrid"
trap 'echo "cleanup complete"' EXIT
eval 'status="dynamic_ok"'
eval 'second="also_unsupported"'
echo "${app_name}: done"

#!/usr/bin/env bash
set -euo pipefail

log_dir="/var/log"
msg="Path: ${log_dir}"
greeting="Hello $USER"
literal_only='Hello $USER'

echo "${msg}" "${greeting}" "${literal_only}"

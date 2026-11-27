#!/usr/bin/env bash
set -euo pipefail

today=$(date +%Y)
host_banner="host: $(hostname)"
upper_msg=$(printf "hello\n" | tr a-z A-Z)

echo "${today} ${host_banner} ${upper_msg}"

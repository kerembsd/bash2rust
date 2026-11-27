#!/usr/bin/env bash
set -euo pipefail

outer=$(printf "pre-%s-post" "$(printf "inner")")
printf "nested:%s\n" "$outer"

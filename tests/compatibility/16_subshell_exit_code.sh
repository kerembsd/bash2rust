#!/usr/bin/env bash
set -euo pipefail

set +e
(
    printf "sub_running\n"
    exit 7
)
rc=$?
printf "sub_rc:%s\n" "$rc"

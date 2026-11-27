#!/usr/bin/env bash
set -euo pipefail

status="ok"

if [ "${status}" = "ok" ]; then
    echo "System is healthy"
fi

enabled=false
if [ "${enabled}" = "true" ]; then
    echo "Feature enabled"
else
    echo "Feature disabled"
fi

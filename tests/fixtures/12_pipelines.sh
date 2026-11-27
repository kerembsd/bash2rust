#!/usr/bin/env bash
set -euo pipefail

printf "alpha\nbeta\ngamma\n" | grep beta | wc -l

if printf "ok\n" | grep -q ok; then
    echo "pipeline matched"
fi

#!/usr/bin/env bash
set -euo pipefail

for f in ./no_such_prefix_*.xyz; do
    printf "literal:%s\n" "$f"
done

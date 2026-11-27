#!/usr/bin/env bash
set -euo pipefail

count=0
while [ "${count}" -lt 3 ]; do
    echo "while count=${count}"
    count=$((count + 1))
done

limit=3
until [ "${limit}" -le 0 ]; do
    echo "until limit=${limit}"
    limit=$((limit - 1))
done

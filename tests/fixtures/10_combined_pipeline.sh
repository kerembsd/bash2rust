#!/usr/bin/env bash
set -euo pipefail

export MODE="batch"
readonly THRESHOLD=2
processed=0

for item in alpha beta gamma delta; do
    if [ "${processed}" -lt "${THRESHOLD}" ]; then
        echo "${MODE}: processing ${item}"
    else
        echo "${MODE}: skipping ${item}"
    fi
    processed=$((processed + 1))
done

printf "%s\n" "${processed}" | wc -l

#!/usr/bin/env bash
set -euo pipefail

IFS=":"
record="one:two::four"
for field in $record; do
    printf "<%s>\n" "$field"
done
unset IFS

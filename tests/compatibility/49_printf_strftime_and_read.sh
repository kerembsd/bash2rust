#!/usr/bin/env bash
# requires-bash: 4.2
set -euo pipefail

TZ=UTC printf '%(%Y-%m-%d %H:%M:%S)T\n' "1700000000"

printf -v formatted '%04d-%s' 7 "ok"
printf 'v=%s\n' "$formatted"
printf '|%*s|\n' 5 "hi"
printf '%b\n' 'a\tb\nc'

printf 'one,two,three,' > items.txt
out=()
while IFS= read -r -d ',' tok; do
    out+=("$tok")
done < items.txt || true
printf 'tokens=%s\n' "${out[*]}"
rm -f items.txt

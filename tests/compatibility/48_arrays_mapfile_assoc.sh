#!/usr/bin/env bash
# requires-bash: 4.0
set -euo pipefail

declare -A seen
for item in apple banana apple cherry banana; do
    seen["$item"]=1
done
printf 'uniq_count=%d\n' "${#seen[@]}"
for k in "${!seen[@]}"; do
    printf 'k=%s\n' "$k"
done | LC_ALL=C sort

arr=(one two three)
for idx in "${!arr[@]}"; do
    printf '%s:%s\n' "$idx" "${arr[idx]}"
done
printf 'repl=%s\n' "${arr[*]/o/0}"

printf 'a\nb\nc\nd\n' > lines.txt
mapfile -tn 2 head_lines < lines.txt
printf 'head=%s\n' "${head_lines[*]}"
mapfile -tn 0 all_lines < lines.txt
printf 'total=%d\n' "${#all_lines[@]}"
rm -f lines.txt

IFS=$'\n' read -d "" -ra parts <<< "${1:-x,y,z}" || true
IFS=$',' read -ra split_parts <<< "x,y,z"
printf 'split=%s\n' "${split_parts[*]}"

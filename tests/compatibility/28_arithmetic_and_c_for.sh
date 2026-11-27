#!/usr/bin/env bash
set -euo pipefail

arr=("one" "two" "three" "four")
n=${#arr[@]}
sum=0

for ((i = 0; i < n; i++)); do
    sum=$((sum + i * 2 + 1))
done

pct=50
width=80
scaled=$((pct * width / 100))

printf "n=%s sum=%s scaled=%s\n" "$n" "$sum" "$scaled"

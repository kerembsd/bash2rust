#!/usr/bin/env bash
# Compatibility fixture 38: dashboard array indexing in arithmetic & ${ARR[$idx]} + $(fn).
set -euo pipefail

WEIGHTS=(5 10 8 45 4 20 3)
TOTAL_WEIGHT=0
for ((i=0; i<${#WEIGHTS[@]}; i++)); do
  TOTAL_WEIGHT=$((TOTAL_WEIGHT + WEIGHTS[i]))
done

calc_total_pct() {
  local idx="$1" pct="$2"
  local done_w=0 i
  for ((i=0; i<idx && i<${#WEIGHTS[@]}; i++)); do
    done_w=$((done_w + WEIGHTS[i]))
  done
  local cur_w=0
  if [[ $idx -lt ${#WEIGHTS[@]} ]]; then
    cur_w=${WEIGHTS[$idx]}
  fi
  local total=$(( (done_w * 100 + cur_w * pct) / TOTAL_WEIGHT ))
  printf '%d' "$total"
}

t_pct=$(calc_total_pct 3 50)
echo "total_weight=$TOTAL_WEIGHT"
echo "total_pct=$t_pct"

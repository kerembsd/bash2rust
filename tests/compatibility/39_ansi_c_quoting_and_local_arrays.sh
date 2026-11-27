#!/usr/bin/env bash
# Compatibility fixture 39: ANSI-C $'...' quoting, local/declare -a arrays, and $? propagation.
set -euo pipefail

printf '%b\n' $'\033[0;32mOK\033[0m' | tr -dc '[:print:]\n'
printf '%s\n' $'hex=\x41\ttab\nline2'

check_tools() {
  local -a missing=()
  declare -a items=(one two)
  items+=(three)
  local t
  for t in sh __missing_tool_xyz__; do
    if ! command -v "$t" &>/dev/null; then
      missing+=("$t")
    fi
  done
  echo "missing=${missing[*]}"
  echo "items=${items[*]}"
}
check_tools

is_good_name() {
  local n="$1"
  [[ -n "$n" ]] && [[ "$n" =~ ^[a-z]+$ ]]
}

ok=1
is_good_name "abc" || ok=0
is_good_name "bad-1" && ok=0
echo "ok=$ok"

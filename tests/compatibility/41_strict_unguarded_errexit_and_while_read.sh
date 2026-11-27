#!/usr/bin/env bash
set -euo pipefail

# 1. Simple assignment after failed && resets $? to 0
flag="no"
[[ "a" == "b" ]] && flag="yes"
after_assign="ok"

# 2. RHS of || preserves $? from failed LHS
fail_rc=0
out=$(sh -c 'exit 3') || fail_rc=$?
printf 'fail_rc=%s after=%s flag=%s\n' "$fail_rc" "$after_assign" "$flag"

# 3. Dynamic errexit suppression inside unguarded function call trees
leaf_fail() {
    return 4
}
mid_call() {
    leaf_fail
}
call_rc=0
mid_call || call_rc=$?
printf 'call_rc=%s\n' "$call_rc"

# 4. while-read loop preserves stdin across body commands
emit_rows() {
    printf 'alpha\t10\nbeta\t20\n'
}
sum=0
while IFS=$'\t' read -r name val; do
    upper=$(printf '%s' "$name" | tr '[:lower:]' '[:upper:]')
    printf 'row=%s:%s\n' "$upper" "$val"
    sum=$((sum + val))
done < <(emit_rows)
printf 'sum=%s\n' "$sum"

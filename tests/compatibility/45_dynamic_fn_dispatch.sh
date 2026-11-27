#!/usr/bin/env bash
set -euo pipefail

greet_fn() {
    printf 'hello:%s\n' "$1"
}

check_even() {
    local n="$1"
    [[ $((n % 2)) -eq 0 ]]
}

fn_name="greet_fn"
"$fn_name" "world"

out=$("$fn_name" "subst")
printf 'out=%s\n' "$out"

pred="check_even"
if "$pred" 4; then
    printf 'even=yes\n'
fi
if ! "$pred" 3; then
    printf 'odd=yes\n'
fi

ext="printf"
"$ext" 'ext=%s\n' "ok"

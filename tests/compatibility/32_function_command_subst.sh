#!/usr/bin/env bash
set -euo pipefail

my_func() {
    local msg="$1"
    printf "hi:%s\n" "$msg"
}

inner() {
    printf "core"
}

outer() {
    local x="$1"
    printf "[%s]" "$x"
}

my_upper() {
    tr a-z A-Z
}

out=$(my_func "hello")
nested=$(outer "$(inner)")
p1=$(my_func "hello" | tr a-z A-Z)
p2=$(printf "world\n" | my_upper)

printf "out=%s\nnested=%s\np1=%s\np2=%s\n" "$out" "$nested" "$p1" "$p2"

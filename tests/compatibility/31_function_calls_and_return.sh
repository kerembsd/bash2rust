#!/usr/bin/env bash
set -euo pipefail

g_state="start"

check_positive() {
    local n="$1"
    if [ "$n" -gt 0 ]; then
        return 0
    fi
    return 1
}

preserve_status() {
    sh -c "exit 4"
    return
}

factorial() {
    local n="$1"
    if [ "$n" -le 1 ]; then
        printf "1"
        return 0
    fi
    local prev=$((n - 1))
    local sub
    sub=$(factorial "$prev")
    printf "%s" "$((n * sub))"
}

mutate_state() {
    g_state="mutated"
}

if check_positive 5; then
    printf "pos:ok\n"
fi

if ! check_positive 0; then
    printf "zero:rejected\n"
fi

set +e
preserve_status
printf "preserve:%s\n" "$?"
set -e

fact5=$(factorial 5)
printf "fact5:%s\n" "$fact5"

mutate_state
printf "state:%s\n" "$g_state"

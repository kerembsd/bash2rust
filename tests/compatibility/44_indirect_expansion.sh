#!/usr/bin/env bash
set -euo pipefail

target_a="alpha"
target_b="beta"
ptr="target_a"
printf 'a=%s\n' "${!ptr}"
ptr="target_b"
printf 'b=%s\n' "${!ptr}"

read_indirect() {
    local key="$1"
    printf 'fn=%s\n' "${!key}"
}
read_indirect "target_a"

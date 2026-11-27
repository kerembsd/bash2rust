#!/usr/bin/env bash
set -euo pipefail

greet_user() {
    local target="$1"
    local prefix="Hello"
    echo "${prefix}, ${target}!"
}

greet_user "world"

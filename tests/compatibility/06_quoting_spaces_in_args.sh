#!/usr/bin/env bash
set -euo pipefail

arg1="hello world"
arg2="two   spaces"

show_args() {
    printf "1=%s|2=%s|n=%s\n" "$1" "$2" "$#"
}

show_args "$arg1" "$arg2"

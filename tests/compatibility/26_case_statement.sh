#!/usr/bin/env bash
set -euo pipefail

classify() {
    local target="$1"
    local kind="unknown"
    case "$target" in
        alpha|beta)
            kind="greek"
            ;;
        *.rs)
            kind="rust"
            ;;
        err_*)
            kind="error"
            ;;
        *)
            kind="other"
            ;;
    esac
    printf "%s:%s\n" "$target" "$kind"
}

classify "alpha"
classify "beta"
classify "main.rs"
classify "err_timeout"
classify "notes.txt"

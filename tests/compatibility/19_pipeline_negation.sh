#!/usr/bin/env bash
set -euo pipefail

if ! printf "hello\n" | grep -q "missing"; then
    printf "negated_pipe_ok\n"
fi

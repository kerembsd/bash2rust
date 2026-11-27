#!/usr/bin/env bash
set -euo pipefail

if [ -d /tmp ]; then
    echo "/tmp is a directory"
fi

if [ ! -f /nonexistent/file.txt ] && [ -e /tmp ]; then
    echo "checks passed"
fi

if [[ -d /tmp && "abc" == "abc" ]]; then
    echo "extended test passed"
fi

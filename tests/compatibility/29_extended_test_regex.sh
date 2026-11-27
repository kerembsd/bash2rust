#!/usr/bin/env bash
set -euo pipefail

printf "hello" > ./sample.txt
msg="err_404_not_found"

if [[ "$msg" =~ ^err_[0-9]+ ]]; then
    printf "regex:matched\n"
fi

if [[ -n "$msg" && ! -d ./sample.txt ]]; then
    printf "nonempty_not_dir:ok\n"
fi

if [[ -f ./sample.txt && -e ./sample.txt ]]; then
    printf "file_exists:ok\n"
fi

#!/usr/bin/env bash
set -euo pipefail

mkdir -p ./sub_dir
(
    cd ./sub_dir
    printf "sub_cwd:%s\n" "$(basename "$PWD")"
)
printf "parent_exists:%s\n" "$(test -d ./sub_dir && printf yes || printf no)"

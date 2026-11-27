#!/usr/bin/env bash
set -euo pipefail

if false; then
    printf "branch_true\n"
else
    printf "branch_false_recovered\n"
fi

printf "after_if\n"

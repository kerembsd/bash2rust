#!/usr/bin/env bash
set -euo pipefail

outer="initial"
(
    outer="mutated_in_subshell"
    printf "inside:%s\n" "$outer"
)
printf "outside:%s\n" "$outer"

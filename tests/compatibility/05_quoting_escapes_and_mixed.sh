#!/usr/bin/env bash
set -euo pipefail

val="42"
printf "escaped: \$val=%s\n" "$val"
printf '%s\n' "pre"'mid'"post:${val}"

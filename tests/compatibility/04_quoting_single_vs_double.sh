#!/usr/bin/env bash
set -euo pipefail

who="world"
printf "double: hello %s\n" "$who"
printf 'single: hello $who\n'

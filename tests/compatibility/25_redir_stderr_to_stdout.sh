#!/usr/bin/env bash
set -euo pipefail

sh -c 'printf "out\n"; printf "err\n" >&2' > ./merged.txt 2>&1
cat ./merged.txt

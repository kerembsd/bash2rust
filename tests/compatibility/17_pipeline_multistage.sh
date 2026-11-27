#!/usr/bin/env bash
set -euo pipefail

printf "banana\napple\ncherry\n" | tr 'a-z' 'A-Z' | sort

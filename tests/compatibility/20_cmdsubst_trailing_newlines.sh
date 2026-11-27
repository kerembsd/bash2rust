#!/usr/bin/env bash
set -euo pipefail

val=$(printf "hello\n\n\n")
printf "[%s]\n" "$val"

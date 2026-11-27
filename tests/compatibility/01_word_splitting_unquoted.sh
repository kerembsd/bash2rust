#!/usr/bin/env bash
set -euo pipefail

items="alpha beta gamma"
for x in $items; do
    printf "[%s]\n" "$x"
done

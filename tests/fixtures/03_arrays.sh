#!/usr/bin/env bash
set -euo pipefail

fruits=("apple" "banana" "cherry")
fruits[1]="blueberry"
fruits+=("date" "elderberry")

echo "First: ${fruits[0]}"
echo "Count: ${#fruits[@]}"

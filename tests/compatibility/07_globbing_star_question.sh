#!/usr/bin/env bash
set -euo pipefail

printf "" > ./a1.txt
printf "" > ./a2.txt
printf "" > ./b10.txt

for f in ./a?.txt; do
    printf "q:%s\n" "$f"
done

for f in ./*.txt; do
    printf "s:%s\n" "$f"
done

#!/usr/bin/env bash
set -euo pipefail

printf "" > ./file1.dat
printf "" > ./file2.dat
printf "" > ./file9.dat
printf "" > ./.hidden.dat

for f in ./file[12].dat; do
    printf "bracket:%s\n" "$f"
done

for f in ./*.dat; do
    printf "visible:%s\n" "$f"
done

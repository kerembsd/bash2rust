#!/usr/bin/env bash
set -euo pipefail

printf "first\n" > ./io.txt
printf "second\n" >> ./io.txt
printf "first_overwrite\n" > ./io2.txt
cat < ./io.txt
cat < ./io2.txt

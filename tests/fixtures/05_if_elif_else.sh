#!/usr/bin/env bash
set -euo pipefail

score=85

if [ "${score}" -ge 90 ]; then
    grade="A"
elif [ "${score}" -ge 80 ]; then
    grade="B"
elif [ "${score}" -ge 70 ]; then
    grade="C"
else
    grade="F"
fi

echo "Grade: ${grade}"

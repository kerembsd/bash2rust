#!/usr/bin/env bash
set -euo pipefail

printf "before_fail\n"
false
printf "after_fail_should_not_run\n"

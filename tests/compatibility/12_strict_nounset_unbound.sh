#!/usr/bin/env bash
set -euo pipefail

printf "before_unbound\n"
printf "%s\n" "$DEFINITELY_UNBOUND_VAR_XYZ"
printf "after_unbound_should_not_run\n"

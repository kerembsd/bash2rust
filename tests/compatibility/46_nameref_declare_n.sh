#!/usr/bin/env bash
# requires-bash: 4.3
set -euo pipefail

orig="initial"
declare -n ref=orig
printf 'read=%s\n' "$ref"
ref="mutated"
printf 'orig=%s\n' "$orig"

update_via_ref() {
    local -n r="$1"
    r="from_fn"
}
update_via_ref orig
printf 'after_fn=%s\n' "$orig"

unset -n ref
printf 'still=%s\n' "$orig"

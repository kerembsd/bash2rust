#!/usr/bin/env bash
# requires-bash: 4.0
set -euo pipefail

if [[ "#1a2b3c" =~ ^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$ ]]; then
    printf 'full=%s r=%s g=%s b=%s\n' "${BASH_REMATCH[0]}" "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "${BASH_REMATCH[3]}"
fi

[[ -n "$BASH" ]] && echo "bash_set"
[[ -n "$BASH_VERSION" ]] && echo "ver_set"
[[ "${#BASH_VERSINFO[@]}" -ge 4 ]] && echo "versinfo_ok"
[[ -n "$HOSTTYPE" ]] && echo "hosttype_set"
[[ -n "$OSTYPE" ]] && echo "ostype_set"

inner_fn() {
    printf 'fn=%s|%s\n' "${FUNCNAME[0]}" "${FUNCNAME[1]}"
}
outer_fn() {
    inner_fn
}
outer_fn

shopt -s extdebug
shopt -u extdebug
declare -F | grep -E 'inner_fn|outer_fn' | LC_ALL=C sort

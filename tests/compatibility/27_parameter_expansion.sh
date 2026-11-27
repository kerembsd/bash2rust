#!/usr/bin/env bash
set -euo pipefail

empty=""
present="hello world"

printf "colon_unset:%s\n" "${DEFINITELY_UNSET_VAR_17:-fallback}"
printf "colon_empty:%s\n" "${empty:-fallback}"
printf "colon_present:%s\n" "${present:-fallback}"

printf "dash_unset:%s\n" "${DEFINITELY_UNSET_VAR_17-fallback}"
printf "dash_empty:[%s]\n" "${empty-fallback}"

printf "len_empty:%s\n" "${#empty}"
printf "len_present:%s\n" "${#present}"

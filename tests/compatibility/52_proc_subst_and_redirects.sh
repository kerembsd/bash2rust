#!/usr/bin/env bash
set -euo pipefail

>empty_a.txt
:>empty_b.txt
for f in empty_a.txt empty_b.txt; do
    if [[ -f "$f" && ! -s "$f" ]]; then
        printf '%s:empty\n' "$f"
    fi
done
rm -f empty_a.txt empty_b.txt

printf 'hello\nworld\n' > data.txt
content="$(<"data.txt")"
printf '[%s]\n' "$content"
rm -f data.txt

IFS= read -r line < <(printf 'from_proc_subst\n')
printf 'ps=%s\n' "$line"

trap 'printf "trap_exit\n"' EXIT
printf 'body\n'

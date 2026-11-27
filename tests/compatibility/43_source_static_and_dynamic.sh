#!/usr/bin/env bash
set -euo pipefail

cat > ./inc_helper.sh <<'EOF'
INC_VAR="from_inc:${1:-none}:${2:-none}"
inc_fn() {
    printf 'inc_fn:%s:%s\n' "$1" "$INC_VAR"
}
if [[ "${1:-}" == "early" ]]; then
    return 0
fi
INC_AFTER="reached"
EOF

dyn_path="./inc_helper.sh"
source "$dyn_path" "hello" "world"
printf 'var=%s after=%s\n' "$INC_VAR" "$INC_AFTER"
inc_fn "caller"

. "$dyn_path" "early" "skip"
printf 'early_var=%s\n' "$INC_VAR"

rm -f ./inc_helper.sh

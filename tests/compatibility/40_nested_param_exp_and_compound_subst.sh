#!/usr/bin/env bash
# Compatibility fixture 40: nested ${...} expansions, compound $(...), BraceGroup redirects, and "$@" fn dispatch.
set -euo pipefail

trim_ws() {
  local s="$1"
  s="${s#${s%%[![:space:]]*}}"
  s="${s%${s##*[![:space:]]}}"
  printf '%s' "$s"
}

echo "trimmed=[$(trim_ws '   hello world   ')]"

tag='"quoted"'
echo "unquoted=${tag//\"/}"

hdr="ABCDEFGHIJ"
echo "substr=${hdr:2:4}"

arr=(a b c d)
echo "slice=${arr[@]:1:2}"

fallback=$(__nonexistent_cmd_xyz__ 2>/dev/null || echo "fallback_ok")
echo "compound_subst=$fallback"

{
  printf 'line1=%s\n' "$tag"
  printf 'line2=%s\n' "$hdr"
} > ./group_out.txt
cat ./group_out.txt

helper_fn() {
  echo "helper:$1"
}
invoke_dyn() {
  "$@"
}
invoke_dyn helper_fn "dynamic_ok"

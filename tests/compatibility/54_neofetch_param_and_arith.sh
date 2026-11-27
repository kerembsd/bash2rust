#!/usr/bin/env bash

# 1. ${*//[[:space:]]/} inside trim-like function
trim_words() {
    set -f
    set -- $*
    printf '%s\n' "${*//[[:space:]]/}"
    set +f
}
trim_words "  hello   world  "

# 2. Array element arithmetic index + pattern op, and arr[idx]++
disk_info=(root 100G 40G 60G 40% /home)
echo "${disk_info[${#disk_info[@]} - 2]/\%}"
echo "${disk_info[${#disk_info[@]} - 1]}"

range=(0 3)
out=""
for ((range[0]; range[0]<=range[1]; range[0]++)); do
    out+="${range[0]},"
done
echo "$out"

# 3. $'...' inside ${var//...} (without trailing /)
colored=$'\e[32m\e[1mhello\e[0m'
stripped="${colored//$'\e['3[0-9]m}"
stripped="${stripped//$'\e['[0-9]m}"
echo "$stripped"

# 4. Parameter op inside ((...)) and $(...) inside $((...))
h="3 hours"
z="0 hours"
((${h/ *} == 0)) && echo "h-zero" || echo "h-nonzero"
((${z/ *} == 0)) && echo "z-zero" || echo "z-nonzero"
val="$(($(echo 2048) / 1024))"
echo "calc=$val"

# 5. 2>&1 inside $() on simple command and [[ -O ... ]]
err_cap="$(ls nonexistent_path_xyz_123 2>&1)" || true
[[ "$err_cap" == *"nonexistent_path_xyz_123"* ]] && echo "stderr-captured"

tmp_owned="owned_test_file_$$"
: > "$tmp_owned"
[[ -O "$tmp_owned" ]] && echo "owned-ok"
rm -f "$tmp_owned"

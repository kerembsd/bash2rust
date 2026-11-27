#!/usr/bin/env bash
set -euo pipefail

tmp_file="/tmp/bash2rust_fixture_17.txt"
tmp_out="/tmp/bash2rust_fixture_17_out.txt"

printf "alpha\n" > "${tmp_file}"
printf "beta\n" >> "${tmp_file}"
cat < "${tmp_file}" > "${tmp_out}" 2>&1
rm -f "${tmp_file}" "${tmp_out}"

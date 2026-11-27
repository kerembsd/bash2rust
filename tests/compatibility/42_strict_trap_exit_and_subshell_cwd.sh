#!/usr/bin/env bash
set -euo pipefail

orig_dir=$(pwd)
sub_out=$( cd / && pwd )
after_dir=$(pwd)
printf 'same_cwd=%s sub=%s\n' "$([[ "$orig_dir" == "$after_dir" ]] && echo yes || echo no)" "$sub_out"

on_exit() {
    printf 'trap_exit_ran\n'
}
trap on_exit EXIT

printf 'body_done\n'

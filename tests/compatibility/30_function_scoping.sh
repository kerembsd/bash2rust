#!/usr/bin/env bash
set -euo pipefail

global_count="0"
global_msg="init"

update_globals() {
    local flag="$1"
    local branch_val="unset"
    if [ "$flag" = "yes" ]; then
        branch_val="yes_branch"
        global_count="2"
        global_msg="updated_by_fn"
    else
        branch_val="no_branch"
        global_count="1"
    fi
    printf "fn:%s:%s:%s\n" "$branch_val" "$global_count" "$global_msg"
}

update_globals "yes"
printf "main:%s:%s\n" "$global_count" "$global_msg"

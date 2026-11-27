#!/usr/bin/env bash
set -euo pipefail

export APP_ENV="production"
readonly MAX_RETRIES=3

path_list="/usr/bin"
path_list+=":/usr/local/bin"

greet() {
    local user_name="alice"
    echo "User: ${user_name}, env: ${APP_ENV}, retries: ${MAX_RETRIES}"
}

greet
echo "${path_list}"

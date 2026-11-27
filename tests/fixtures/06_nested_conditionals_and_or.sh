#!/usr/bin/env bash
set -euo pipefail

env="staging"
debug=true

if [ "${env}" = "staging" ]; then
    if [ "${debug}" = "true" ]; then
        echo "Staging debug mode"
    else
        echo "Staging standard mode"
    fi
fi

[ -n "${env}" ] && echo "env is set" || echo "env is empty"

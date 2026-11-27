#!/usr/bin/env bash
set -euo pipefail

printf "start_pipe\n"
false | true
printf "after_pipe_should_not_run\n"

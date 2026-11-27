#!/usr/bin/env bash

false | true
printf "default_pipe_rc:%s\n" "$?"
true | false
printf "failing_last_rc:%s\n" "$?"

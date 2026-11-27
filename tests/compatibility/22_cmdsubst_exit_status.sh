#!/usr/bin/env bash

out=$(printf "captured"; exit 5)
rc=$?
printf "out=%s|rc=%s\n" "$out" "$rc"

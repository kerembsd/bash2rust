#!/usr/bin/env bash
set -euo pipefail

empty=""
spaces="   a   b   "

count_args() {
    printf "%s\n" "$#"
}

count_args $empty
count_args "$empty"
count_args $spaces
count_args "$spaces"

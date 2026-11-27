#!/usr/bin/env bash
# Compatibility fixture 34: shift/$# in functions & defined-function dispatch with redirects.
set -u

show_help() {
  echo "HELP:args=$#:$*"
}

parse_args() {
  while [ $# -gt 0 ]; do
    case "$1" in
      -h|--help)
        show_help
        shift
        ;;
      --err)
        show_help >&2
        shift
        ;;
      --pair)
        echo "pair=$2:$3"
        shift 2
        echo "after_shift2_count=$#"
        shift
        ;;
      *)
        echo "arg=$1"
        shift
        ;;
    esac
  done
  echo "done_count=$#"
}

main() {
  parse_args "$@"
  echo "main_count=$#"
}

main -h alpha --err --pair x y omega
show_help > ./help_out.txt
cat ./help_out.txt

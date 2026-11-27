#!/usr/bin/env bash
# requires-bash: 4.0
set -euo pipefail

s="HeLlO wOrLd"
printf '%s\n' "${s^}" "${s^^}" "${s,}" "${s,,}" "${s~}" "${s~~}"

trim_str() {
    : "${1#"${1%%[![:space:]]*}"}"
    : "${_%"${_##*[![:space:]]}"}"
    printf '[%s]\n' "$_"
}
trim_str "   hello   world   "
trim_str ""

strip_dyn() {
    printf '%s|%s|%s|%s\n' "${1//$2}" "${1/$2}" "${1##$2}" "${1%%$2}"
}
strip_dyn "foo_bar_foo" "foo"
strip_dyn "The Quick Brown Fox" "[aeiou]"

alt=""
printf 'alt_empty=%s\n' "${alt:+yes}"
alt="val"
printf 'alt_set=%s\n' "${alt:+yes}"
unset def_var || true
printf 'def=%s|after=%s\n' "${def_var:=fallback}" "$def_var"

LC_ALL=C
ch="A"
printf 'ord=%d\n' "'$ch"
printf 'slice=%s\n' "${s:0:5}"

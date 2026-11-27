#!/usr/bin/env bash
# Compatibility fixture 35: Bash special variables under set -euo pipefail.
set -euo pipefail

echo "euid=$EUID"
echo "uid=$UID"
echo "ppid=$PPID"
if [[ -n "$HOSTNAME" ]]; then echo "hostname_nonempty=yes"; else echo "hostname_nonempty=no"; fi
if [[ -n "$OSTYPE" ]]; then echo "ostype_nonempty=yes"; else echo "ostype_nonempty=no"; fi

r="$RANDOM"
if [[ "$r" -ge 0 && "$r" -lt 32768 ]]; then echo "random_range=ok"; else echo "random_range=bad"; fi

SECONDS=5
if [[ "$SECONDS" -ge 5 ]]; then echo "seconds_ok=ok"; else echo "seconds_ok=bad"; fi
if [[ -n "$LINENO" ]]; then echo "lineno_nonempty=yes"; else echo "lineno_nonempty=no"; fi

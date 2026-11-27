#!/usr/bin/env bash
# Compatibility fixture 33: umask & centralized shell builtins.
set -u

umask 027
echo "octal=$(umask)"
echo "reusable=$(umask -p)"
echo "symbolic=$(umask -S)"

umask u=rwx,g=rx,o=rx
echo "after_sym=$(umask)"

if umask 0999 2>/dev/null; then
  echo "invalid=unexpected_ok"
else
  echo "invalid=rejected:$?"
fi

: ignored_arg
echo "colon=$?"

true
echo "true=$?"

if false; then
  echo "false=unexpected_ok"
else
  echo "false=failed:$?"
fi

echo "cmd_v_umask=$(command -v umask)"
echo "cmd_V_true=$(command -V true)"
echo "type_t_umask=$(type -t umask)"

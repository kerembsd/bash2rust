#!/usr/bin/env bash
# requires-bash: 4.0
set -euo pipefail

echo {apples,oranges,pears}
echo pre_{a,b,c}_post
echo {0..5}
echo {00..04}
echo {a..d}
echo {D..A}
echo {1..9..2}
echo "{quoted,brace}"

hex="1a2b3c"
((r=16#${hex:0:2}, g=16#${hex:2:2}, b=16#${hex:4:2}))
printf 'rgb=%d,%d,%d\n' "$r" "$g" "$b"

x=5
y=10
((z=y>x?y:x))
printf 'max=%d\n' "$z"

for((i=1;i<=3;i++)){ printf 'c:%d\n' "$i"; }
for j in {1..3};{ printf 'b:%d\n' "$j"; }

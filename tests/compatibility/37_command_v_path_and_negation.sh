#!/usr/bin/env bash
# Compatibility fixture 37: command -v / type PATH resolution, &>/dev/null, and negation.
set -euo pipefail

mkdir -p ./bin
cat <<'EOF' > ./bin/mytool
#!/usr/bin/env sh
echo "hi"
EOF
chmod +x ./bin/mytool
ln -s ./mytool ./bin/mylink

if command -v sh &>/dev/null; then
  echo "sh_found=yes"
else
  echo "sh_found=no"
fi

if ! command -v __definitely_missing_binary_xyz__ &>/dev/null; then
  echo "missing_negated=yes"
else
  echo "missing_negated=no"
fi

PATH="./bin:$PATH"
echo "mytool=$(command -v mytool)"
echo "mylink=$(command -v mylink)"
echo "type_p=$(type -p mytool)"
echo "type_t=$(type -t mytool)"

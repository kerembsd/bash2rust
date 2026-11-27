#!/usr/bin/env bash
# Compatibility fixture 36: heredocs, here-strings, and process-substitution loop redirects.
set -euo pipefail

who="world"

cat <<'EOF'
literal=$who
EOF

cat <<EOF
expanded=$who
EOF

cat <<-EOF
	tab_stripped=$who
EOF

tr 'a-z' 'A-Z' <<< "hello $who"

count=0
while IFS= read -r line || [[ -n "$line" ]]; do
  count=$((count + 1))
  echo "hs:$count:$line"
done <<< "one"

msg='line1\n  line2  \nline3'
pcount=0
while IFS= read -r line || [[ -n "$line" ]]; do
  pcount=$((pcount + 1))
  echo "ps:$pcount:[$line]"
done < <(printf '%b\n' "$msg")

fn_heredoc() {
  cat <<'EOF'
fn_heredoc_ok
EOF
}
fn_heredoc >&2

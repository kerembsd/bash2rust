#!/usr/bin/env bash

# Compiled helper called from eval'd function
compiled_helper() {
    echo "helper:$1"
}

# Top-level function that can be overridden dynamically
mode_fn() {
    echo "default-mode"
}

mode_fn

flag="override"
if [[ "$flag" == "override" ]]; then
    mode_fn() {
        echo "overridden-mode"
    }
fi

mode_fn

# eval defining a function that calls a compiled function
read -r -d '' dynamic_block <<'EOF' || true
eval_entry() {
    compiled_helper "$1"
}
EOF

eval "$dynamic_block"
eval_entry "ok"

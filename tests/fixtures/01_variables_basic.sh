#!/usr/bin/env bash
set -euo pipefail

name="bash2rust"
greeting='Hello, world!'
count=42
message="${greeting} from ${name} (${count})"

echo "${message}"

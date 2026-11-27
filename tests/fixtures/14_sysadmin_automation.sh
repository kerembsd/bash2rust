#!/usr/bin/env bash
set -euo pipefail

log_dir="/var/log"
current_host=$(hostname)

if [ -d "${log_dir}" ]; then
    echo "Inspecting ${log_dir} on ${current_host}"
else
    echo "Missing ${log_dir}"
fi

for svc in sshd cron nginx; do
    if [ -n "${svc}" ]; then
        printf "%s\n" "${svc}" | tr a-z A-Z
    fi
done

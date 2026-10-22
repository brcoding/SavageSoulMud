#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")" && pwd)"
found=0
while read -r pid; do
    [[ -n "$pid" ]] || continue
    if [[ "$(readlink "/proc/$pid/cwd" 2>/dev/null || true)" == "$root" ]]; then
        kill -TERM "$pid"
        found=1
        echo "Requested graceful shutdown of Grim (PID $pid); allow 30 seconds."
    fi
done < <(pgrep -u "$(id -u)" -x savsoul || true)
if [[ "$found" == 0 ]]; then echo "This Grim instance is not running."; fi

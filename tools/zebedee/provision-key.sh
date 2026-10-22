#!/usr/bin/env bash
set -euo pipefail
umask 077
state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/savsoul-zebedee"
mkdir -p "$state_dir"
cp -- "$1" "$state_dir/relay.key"
chmod 600 "$state_dir/relay.key"
printf '%s\n' "$state_dir/relay.key"

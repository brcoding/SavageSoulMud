#!/usr/bin/env bash
set -euo pipefail
umask 077
mkdir -p /home/barry/.local/state/savsoul-zebedee
cp -- "$1" /home/barry/.local/state/savsoul-zebedee/relay.key
chmod 600 /home/barry/.local/state/savsoul-zebedee/relay.key

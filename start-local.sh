#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
export GRIM_BIND_HOST=127.0.0.1
if [[ -x /tmp/savsoul-grim-runtime/savsoul ]]; then
    exec /tmp/savsoul-grim-runtime/savsoul
fi
# Never execute a memory-mapped binary from the network-backed P: mount.
# /tmp is cleared across WSL restarts, so recreate the Linux runtime image.
if [[ -x bin/savsoul ]]; then
    runtime_dir=/tmp/savsoul-grim-runtime
    mkdir -p "$runtime_dir"
    runtime_image=$(mktemp "$runtime_dir/savsoul.XXXXXX")
    trap 'rm -f -- "$runtime_image"' EXIT
    cp bin/savsoul "$runtime_image"
    chmod 700 "$runtime_image"
    mv -f -- "$runtime_image" "$runtime_dir/savsoul"
    trap - EXIT
    exec "$runtime_dir/savsoul"
fi
export CARGO_HOME=/mnt/c/Users/barry/Documents/Codex/2026-10-12/i-w/work/rust/cargo
export RUSTUP_HOME=/mnt/c/Users/barry/Documents/Codex/2026-10-12/i-w/work/rust/rustup
export CARGO_TARGET_DIR=/tmp/savsoul-grim-target
export PATH="$CARGO_HOME/bin:$PATH"
exec cargo run --locked -p example-mud --bin savsoul

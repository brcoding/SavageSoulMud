#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
export GRIM_BIND_HOST="${GRIM_BIND_HOST:-127.0.0.1}"
# Never execute a memory-mapped binary from the network-backed P: mount.
# Always refresh from the trusted checkout, inside a private Linux directory.
if [[ -x bin/savsoul ]]; then
    runtime_dir="/tmp/savsoul-grim-runtime-$(id -u)"
    if ! (umask 077; mkdir "$runtime_dir") 2>/dev/null; then
        if [[ -L "$runtime_dir" || ! -d "$runtime_dir" || ! -O "$runtime_dir" || "$(stat -c %a "$runtime_dir")" != 700 ]]; then
            echo "Refusing unsafe runtime directory: $runtime_dir" >&2
            exit 1
        fi
    fi
    runtime_image=$(mktemp "$runtime_dir/savsoul.XXXXXX")
    trap 'rm -f -- "$runtime_image"' EXIT
    cp bin/savsoul "$runtime_image"
    chmod 700 "$runtime_image"
    mv -f -- "$runtime_image" "$runtime_dir/savsoul"
    trap - EXIT
    exec "$runtime_dir/savsoul"
fi
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/savsoul-grim/target}"
export PATH="$CARGO_HOME/bin:$PATH"
exec cargo run --locked -p example-mud --bin savsoul

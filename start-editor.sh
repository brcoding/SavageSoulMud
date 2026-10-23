#!/bin/bash
set -euo pipefail
base="${GRIM_ROOT:-/mnt/p/savsoulmud/grim}"
runtime="$HOME/.local/share/savsoul-editor"
export GRIM_EDITOR_STATE="$HOME/.local/state/savsoul-editor"
export GRIM_ROOT="$base"
export GRIM_VALIDATOR="/tmp/savsoul-grim-runtime/savsoul"
mkdir -p "$runtime/app" "$GRIM_EDITOR_STATE"
chmod 700 "$GRIM_EDITOR_STATE"
if [ ! -x "$runtime/venv/bin/python" ]; then
    python3 -m venv "$runtime/venv"
fi
cp -r "$base/tools/wizard-editor/." "$runtime/app/"
if ! cmp -s "$runtime/app/requirements.txt" "$runtime/installed-requirements.txt"; then
    "$runtime/venv/bin/pip" install --disable-pip-version-check -r "$runtime/app/requirements.txt"
    cp "$runtime/app/requirements.txt" "$runtime/installed-requirements.txt"
fi
exec "$runtime/venv/bin/python" "$runtime/app/server.py"

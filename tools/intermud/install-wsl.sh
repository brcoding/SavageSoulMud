#!/usr/bin/env bash
set -euo pipefail
# Local WSL installation; pinned upstream and isolated dependencies.
base=/home/barry/.local/share/savsoul-intermud
source_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$base" /home/barry/.config/systemd/user
if [ ! -d "$base/gateway/.git" ]; then
  git clone https://github.com/LuminariMUD/Intermud3.git "$base/gateway"
fi
git -C "$base/gateway" checkout 3317042aebb9ab463489896dbe50ce13cd123981
python3 "$source_dir/patch_gateway.py" "$base/gateway"
python3 -m venv "$base/venv"
"$base/venv/bin/pip" install -r "$source_dir/requirements.lock"
python3 "$source_dir/create_state.py"
cp "$source_dir/savsoul-intermud.service" /home/barry/.config/systemd/user/savsoul-intermud.service
systemctl --user daemon-reload
systemctl --user enable --now savsoul-intermud

#!/usr/bin/env bash
set -euo pipefail
umask 077
command -v python3 >/dev/null
command -v openssl >/dev/null
task_source=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
task_app="${XDG_DATA_HOME:-$HOME/.local/share}/savsoul-tls"
task_state="${XDG_STATE_HOME:-$HOME/.local/state}/savsoul-tls"
task_units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
mkdir -p "$task_app" "$task_state" "$task_units"
chmod 700 "$task_app" "$task_state"
cp "$task_source/proxy.py" "$task_app/proxy.py"
task_cert="${SAVSOUL_TLS_CERT:-$task_state/localhost.crt}"
task_key="${SAVSOUL_TLS_KEY:-$task_state/localhost.key}"
task_host="${SAVSOUL_TLS_HOST:-127.0.0.1}"
# User-supplied certificate paths must already exist. Never overwrite a key.
if [[ -z "${SAVSOUL_TLS_CERT:-}" && -z "${SAVSOUL_TLS_KEY:-}" && ! -e "$task_cert" && ! -e "$task_key" ]]; then
    openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days 365 \
        -keyout "$task_key" -out "$task_cert" -subj '/CN=localhost' \
        -addext 'subjectAltName=DNS:localhost,IP:127.0.0.1,IP:::1'
fi
test -r "$task_cert"
test -r "$task_key"
python3 - "$task_cert" "$task_key" <<'PY'
import ssl, sys
c = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
c.load_cert_chain(sys.argv[1], sys.argv[2])
PY
cat > "$task_units/savsoul-tls.service" <<EOF
[Unit]
Description=Savage Soul local Mudlet TLS ingress
After=network.target

[Service]
Type=simple
ExecStart=/usr/bin/python3 "$task_app/proxy.py" --host "$task_host" --port 4001 --backend-port 4003 --cert "$task_cert" --key "$task_key"
Restart=on-failure
RestartSec=3
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=read-only
UMask=0077

[Install]
WantedBy=default.target
EOF
systemctl --user daemon-reload
printf 'Installed savsoul-tls.service. Configure MUD GRIM_PROXY_PORT=4003, then run:\n  systemctl --user enable --now savsoul-tls.service\n'
printf 'Local certificate: %s\nSHA256 fingerprint:\n' "$task_cert"
openssl x509 -in "$task_cert" -noout -fingerprint -sha256

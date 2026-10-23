"""Loopback editor behind the fixed public Apache TLS virtual host."""
import sys
sys.path.insert(0, '/home/savsoul/grim/tools/wizard-editor')
from server import create_app, MAX_FILE
from waitress import serve
app = create_app('/home/savsoul/grim', '/home/savsoul/state/editor',
    '/home/savsoul/grim/bin/savsoul',
    ['/home/savsoul/runtime/venv/bin/python', '/home/savsoul/grim/tools/server-host/reload_remote.py'],
    port=4085, allowed_hosts=['savsoul.com:4081', 'www.savsoul.com:4081'])
app.config['SESSION_COOKIE_SECURE'] = True
# Waitress clears untrusted proxy headers before WSGI middleware runs.
# Trust only Apache's loopback peer and its final scheme/client-address hop.
# The application still checks its explicit Host allowlist and request Origin.
serve(app, host='127.0.0.1', port=4085, threads=4,
    max_request_body_size=MAX_FILE+8192,
    trusted_proxy='127.0.0.1', trusted_proxy_count=1,
    trusted_proxy_headers={'x-forwarded-for', 'x-forwarded-proto'})

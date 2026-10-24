"""Private bounded artifact receiver; no shell or Kubernetes credentials."""
import hmac
from http.server import BaseHTTPRequestHandler, HTTPServer
import ipaddress
import json
import os
from pathlib import Path
import re
import sys
import time

from release_transaction import Transaction

MAX_UPLOAD = 128 * 1024 * 1024


class LimitedBody:
    def __init__(self, stream, size):
        self.stream, self.remaining, self.deadline = stream, size, time.monotonic() + 120

    def read(self, size=-1):
        if time.monotonic() > self.deadline:
            raise TimeoutError('Release upload exceeded its deadline')
        limit = self.remaining if size < 0 else min(size, self.remaining)
        if not limit:
            return b''
        chunk = self.stream.read(limit)
        if not chunk:
            raise ValueError('Release upload ended early')
        self.remaining -= len(chunk)
        return chunk


class Handler(BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(30)

    def log_message(self, fmt, *args):
        # Do not log headers, credentials, request bodies or exception details.
        return

    def reply(self, status, data):
        content = json.dumps(data).encode()
        try:
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(content)))
            self.send_header('Connection', 'close')
            self.end_headers()
            self.wfile.write(content)
        except (BrokenPipeError, ConnectionResetError):
            # A client/probe leaving must not turn a committed release into failure.
            pass

    def do_GET(self):
        if self.path == '/healthz':
            self.reply(200, {'ready': True})
        elif self.path == '/ready':
            try:
                if ((self.server.transaction.state / 'active.json').exists() or
                        (self.server.transaction.runner / 'release-in-progress').exists() or
                        (self.server.transaction.runner / 'editor-save-block').exists() or
                        (self.server.transaction.runner / 'editor-copyover-pending').exists()):
                    raise RuntimeError('An unfinished release requires recovery')
                self.server.transaction.confirm_running()
                ports = {f[1].split(':')[-1] for f in
                         (line.split() for line in Path('/proc/net/tcp').read_text().splitlines()[1:])
                         if len(f) > 3 and f[3] == '0A'}
                if not all(format(port, '04X') in ports for port in (4085, 4086)):
                    raise RuntimeError('Supporting services are restarting')
                self.reply(200, {'ready': True, 'revision': self.server.transaction.baseline().get('revision')})
            except Exception:
                self.reply(503, {'ready': False})
        else:
            self.reply(404, {'error': 'Unknown endpoint'})

    def do_POST(self):
        if (not ipaddress.ip_address(self.client_address[0]).is_loopback or
                not hmac.compare_digest(self.headers.get('Authorization', '').encode(),
                                        ('Bearer ' + self.server.release_token).encode())):
            self.reply(403, {'error': 'Release authentication failed'})
            return
        match = re.fullmatch(r'/release/([0-9a-f]{40})', self.path)
        length = self.headers.get('Content-Length', '')
        if not match or not length.isdecimal() or self.headers.get('Transfer-Encoding'):
            self.reply(400, {'error': 'Expected revision and fixed-length artifact'})
            return
        if not 0 < int(length) <= MAX_UPLOAD:
            self.reply(413, {'error': 'Artifact exceeds upload bounds'})
            return
        try:
            result = self.server.transaction.release(LimitedBody(self.rfile, int(length)), match[1])
            self.reply(200, result)
        except Exception as error:
            print('Release failed: ' + type(error).__name__, file=sys.stderr, flush=True)
            self.reply(500, {'error': 'Release failed; inspect private receiver state before retrying'})


def main():
    os.umask(0o077)
    token = Path('/run/savsoul-release/token').read_text().strip()
    if len(token) < 32:
        raise RuntimeError('A private release key must be provisioned')
    transaction = Transaction()
    transaction.recover_startup()
    server = HTTPServer(('127.0.0.1', 4092), Handler)
    server.release_token, server.transaction = token, transaction
    server.serve_forever()


if __name__ == '__main__':
    main()

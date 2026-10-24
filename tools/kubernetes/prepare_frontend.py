"""Prepare public permissions; pipe --emit-secret directly to kubectl apply."""
import argparse
import base64
import json
from pathlib import Path

HOME = Path('/home/savsoul')


def prepare(home=HOME):
    public = home / 'public'
    public.chmod(0o750)
    for path in public.rglob('*'):
        if path.is_symlink():
            raise RuntimeError('Unexpected public asset symlink')
        path.chmod(0o750 if path.is_dir() else 0o640)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--emit-secret', action='store_true',
                        help='Sensitive stdout: pipe directly to kubectl apply; never log or save')
    args = parser.parse_args()
    prepare()
    if args.emit_secret:
        data = {key: base64.b64encode((HOME / 'state/tls' / name).read_bytes()).decode()
                for key, name in [('tls.crt', 'server.crt'), ('tls.key', 'server.key')]}
        print(json.dumps({'apiVersion': 'v1', 'kind': 'Secret', 'type': 'kubernetes.io/tls',
                          'metadata': {'name': 'savsoul-tls', 'namespace': 'savsoul'}, 'data': data}))
    else:
        print('Public assets prepared for the distinct frontend UID.')


if __name__ == '__main__':
    main()

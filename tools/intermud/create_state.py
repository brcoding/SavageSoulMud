"""Initialize or repair private I3 keys without printing credential values."""
from pathlib import Path
import fcntl
import os
import secrets
import tempfile


def atomic(path, text):
    fd, name = tempfile.mkstemp(dir=path.parent, prefix='.' + path.name)
    try:
        with os.fdopen(fd, 'w') as stream:
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def initialize(root):
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    root.chmod(0o700)
    with (root / '.initialize.lock').open('a') as lock:
        os.chmod(lock.name, 0o600)
        fcntl.flock(lock, fcntl.LOCK_EX)
        keypath, env = root / 'api.key', root / 'gateway.env'
        values = dict(line.split('=', 1) for line in env.read_text().splitlines()
                      if '=' in line) if env.exists() else {}
        key = keypath.read_text().strip() if keypath.exists() else values.get('SAVSOUL_I3_API_KEY', '')
        key = key or secrets.token_urlsafe(48)
        values['SAVSOUL_I3_API_KEY'] = key
        values['SAVSOUL_I3_GATEWAY_SECRET'] = values.get('SAVSOUL_I3_GATEWAY_SECRET') or secrets.token_urlsafe(48)
        # Recover from partial loss, or reconcile an explicitly rotated api.key.
        atomic(keypath, key + '\n')
        atomic(env, ''.join(f'{name}={value}\n' for name, value in values.items()))


if __name__ == '__main__':
    directory = Path(os.environ.get('XDG_STATE_HOME', Path.home() / '.local/state')) / 'savsoul-intermud'
    initialize(directory)
    print('Private Intermud credentials initialized; no credentials printed.')

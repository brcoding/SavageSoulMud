"""Rebase inherited I3 persistence paths without opening a network connection."""
import os
from pathlib import Path
import shutil
import tempfile
import yaml

HOME = Path('/home/savsoul')


def prepare(home=HOME):
    path = home / 'grim/tools/intermud/gateway.yaml'
    backup = home / 'state/kubernetes/original-network-config/gateway.yaml'
    backup.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    if not backup.exists():
        shutil.copy2(path, backup)
        backup.chmod(0o600)
    config = yaml.safe_load(path.read_text())
    private = home / '.local/state/savsoul-intermud'
    if not (private / 'router').is_dir() or not (private / 'gateway.env').is_file():
        raise RuntimeError('Approved private I3 state has not been imported')
    config['state']['directory'] = str(private / 'router')
    config.setdefault('logging', {})['file'] = str(private / 'gateway.log')
    if config['api']['host'] != '127.0.0.1' or config['gateway']['host'] != '127.0.0.1':
        raise RuntimeError('I3 APIs must remain loopback-only')
    fd, temporary = tempfile.mkstemp(prefix='.gateway-', dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as handle:
            yaml.safe_dump(config, handle, sort_keys=False)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)
    print('I3 persistence/log paths rebased to the retained private home; adapter remains inactive.')


if __name__ == '__main__':
    os.umask(0o077)
    prepare()

"""Stream only the live main stack; never stage a private archive on disk."""
from pathlib import Path
import stat
import sys
import tarfile

HOME = Path('/home/savsoul')
PREFIXES = ('grim', 'state', '.local/state', 'runtime/i3-gateway',
            'runtime/trainer', 'runtime/cacert.pem')


def skipped(name):
    """Leave logs, caches, Git metadata and old release backups on the source."""
    parts = Path(name).parts
    return (any(p in ('.git', '__pycache__', '.pytest_cache', 'node_modules') for p in parts)
            or name.startswith(('state/releases/backup-', 'state/releases/recovered-attempts',
                                'state/releases/public', 'state/releases/stage-'))
            or name.endswith(('.log', '.sock', '.pid', '.pyc')))


def add_tree(archive, source, destination):
    """Reject unexpected links/devices and emit regular entries with safe owners."""
    if skipped(destination):
        return
    mode = source.lstat().st_mode
    if stat.S_ISLNK(mode):
        # Only the known certificate aliases may resolve within their private tree.
        if destination not in ('state/tls/server.crt', 'state/tls/server.key'):
            raise ValueError('Unexpected source symlink: ' + destination)
        resolved = source.resolve(strict=True)
        if not resolved.is_relative_to(HOME / 'state/tls') or not resolved.is_file():
            raise ValueError('Unsafe TLS alias')
        source, mode = resolved, resolved.stat().st_mode
    if not stat.S_ISDIR(mode) and not stat.S_ISREG(mode):
        raise ValueError('Unexpected special source file: ' + destination)
    info = tarfile.TarInfo(destination)
    info.mode, info.uid, info.gid = mode & 0o777, 1009, 1011
    if source.is_dir():
        info.type = tarfile.DIRTYPE
        archive.addfile(info)
        for child in sorted(source.iterdir()):
            add_tree(archive, child, destination + '/' + child.name)
    else:
        info.size = source.stat().st_size
        with source.open('rb') as stream:
            archive.addfile(info, stream)


def main():
    with tarfile.open(fileobj=sys.stdout.buffer, mode='w|gz', compresslevel=1) as archive:
        for prefix in PREFIXES:
            add_tree(archive, HOME / prefix, prefix)
        add_tree(archive, HOME / 'state/releases/public', 'public')
        public_map = Path('/var/www/lpmud/mudlet/savsoul.xml')
        if public_map.exists():
            add_tree(archive, public_map, 'public/mudlet/savsoul.xml')


if __name__ == '__main__':
    main()

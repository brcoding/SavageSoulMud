"""Fresh-volume-only snapshot import, run as the game UID in a maintenance pod."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import sys
import tarfile

HOME = Path('/home/savsoul')
PREFIXES = {'grim', 'state', '.local', 'runtime', 'public'}


def import_snapshot(stream, root=HOME):
    """Reject path/link escapes and retain a hash manifest inside private state."""
    if any(p.name != 'lost+found' for p in root.iterdir()):
        raise ValueError('Volume is not fresh; refusing to overwrite candidate data')
    (root / '.seed-in-progress').touch(exist_ok=False, mode=0o600)
    hashes, total, count = {}, 0, 0
    with tarfile.open(fileobj=stream, mode='r|gz') as archive:
        for member in archive:
            parts = PurePosixPath(member.name).parts
            total += member.size
            count += 1
            if (not parts or parts[0] not in PREFIXES or member.name.startswith('/')
                    or '\\' in member.name or any(p in ('', '.', '..') for p in member.name.split('/'))
                    or not (member.isdir() or member.isfile()) or member.size < 0
                    or count > 20000 or total > 2 * 1024 ** 3):
                raise ValueError('Unsafe or oversized snapshot entry')
            target = root / member.name
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True, mode=0o700)
            else:
                target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                digest = hashlib.sha256()
                with archive.extractfile(member) as source, target.open('xb') as out:
                    while chunk := source.read(1024 * 1024):
                        digest.update(chunk)
                        out.write(chunk)
                target.chmod(member.mode & 0o777)
                hashes[member.name] = digest.hexdigest()
    state = root / 'state/kubernetes'
    state.mkdir(parents=True, exist_ok=True, mode=0o700)
    (state / 'warm-snapshot.json').write_text(json.dumps({'files': hashes, 'bytes': total}))
    (root / '.seed-in-progress').unlink()
    print('Imported warm snapshot:', len(hashes), 'files,', total, 'bytes; hashes retained privately.')


if __name__ == '__main__':
    os.umask(0o077)
    import_snapshot(sys.stdin.buffer)

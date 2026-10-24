"""Unprivileged release receiver. Provision a root-owned copy, not a mutable import."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import tempfile
import time

ROOT = Path('/home/savsoul/grim')
STATE = Path('/home/savsoul/state/releases')
PID = Path('/home/savsoul/state/runner/mud.pid')
PUBLIC_MAP = Path('/var/www/lpmud/mudlet/savsoul.xml')
MAX_BYTES = 256 * 1024 * 1024


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None


def requirements(path):
    return sorted(line.strip() for line in path.read_text().splitlines()
                  if line.strip() and not line.lstrip().startswith('#'))


def authored(name):
    return name.startswith(('assets/', 'data/help/', 'data/areas/', 'data/socials/',
                            'data/quests/', 'data/savage-soul/'))


def allowed(name):
    parts = PurePosixPath(name).parts
    return (bool(parts) and not name.startswith('/') and '\\' not in name
            and all(p not in ('.', '..', '') for p in name.split('/'))
            and (name == 'manifest.json' or name == 'bin/savsoul'
                 or name.startswith(('tools/', 'public/')) or authored(name)))


def unpack(stream, destination, revision):
    names, total = set(), 0
    with tarfile.open(fileobj=stream, mode='r|gz') as archive:
        for member in archive:
            total += member.size
            if (not member.isfile() or not allowed(member.name) or member.name in names
                    or len(names) >= 10000 or total > MAX_BYTES):
                raise ValueError('Unsafe or oversized release archive')
            names.add(member.name)
            path = destination / member.name
            path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            with archive.extractfile(member) as source, path.open('wb') as target:
                shutil.copyfileobj(source, target)
    manifest = json.loads((destination / 'manifest.json').read_text())
    if manifest.get('revision') != revision or set(manifest['files']) != names - {'manifest.json'}:
        raise ValueError('Release manifest mismatch')
    if 'bin/savsoul' not in manifest['files'] or not any(n.startswith('public/') for n in names):
        raise ValueError('Incomplete release')
    for name, checksum in manifest['files'].items():
        if digest(destination / name) != checksum:
            raise ValueError('Release digest mismatch: ' + name)
    return manifest


def safe_target(root, name):
    target = root / name
    for path in [target, *target.parents]:
        if path.is_symlink():
            raise ValueError('Symlink in deployment destination: ' + str(path))
        if path == root:
            break
    return target


def atomic_copy(source, target, mode=0o600):
    target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd, temp = tempfile.mkstemp(prefix='.release-', dir=target.parent)
    try:
        with os.fdopen(fd, 'wb') as out, source.open('rb') as inp:
            shutil.copyfileobj(inp, out)
            out.flush()
            os.fsync(out.fileno())
        os.chmod(temp, mode)
        os.replace(temp, target)
    finally:
        if os.path.exists(temp):
            os.unlink(temp)


def check_destinations(names, root=ROOT):
    """Check replacement permissions as the game UID before touching live files."""
    for name in sorted(names):
        if name.startswith('public/'):
            continue
        parent = safe_target(root, name).parent
        while not parent.exists():
            parent = parent.parent
        if not parent.is_dir() or not os.access(parent, os.W_OK | os.X_OK):
            raise PermissionError('Release destination directory is not writable: ' + str(parent))


def write_journal(path, entries):
    """Publish complete recovery metadata before mutating its associated file."""
    fd, temporary = tempfile.mkstemp(prefix='.journal-', dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(entries, stream)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def restore(backup, root=ROOT):
    journal = json.loads((backup / 'journal.json').read_text())
    for name in reversed(journal):
        target = safe_target(root, name)
        old = backup / 'files' / name
        if old.exists():
            if digest(old) != digest(target) or old.stat().st_mode & 0o777 != target.stat().st_mode & 0o777:
                atomic_copy(old, target, old.stat().st_mode & 0o777)
        elif target.exists():
            target.unlink()
    if (backup / 'public-map.xml').exists():
        atomic_copy(backup / 'public-map.xml', PUBLIC_MAP, 0o644)
    elif (backup / 'public-map-missing').exists() and PUBLIC_MAP.exists():
        PUBLIC_MAP.unlink()


def apply(stage, backup, manifest, baseline, root=ROOT):
    check_destinations(set(manifest['files']) | set(baseline.get('files', {})), root)
    journal, preserved = [], []
    backup.mkdir(mode=0o700, parents=True)
    (backup / 'previous.json').write_text(json.dumps(baseline))
    # Capture account/world/private JSON before copyover saves the next snapshot.
    with tarfile.open(backup / 'data.tar.gz', 'w:gz') as archive:
        if (root / 'data').exists():
            archive.add(root / 'data', arcname='data', recursive=True)
    previous = baseline.get('files', {})
    names = set(manifest['files']) - {n for n in manifest['files'] if n.startswith('public/')}
    # Code removed in a later release is removed from disk; authored data is retained.
    deleted = {n for n in previous if n.startswith('tools/') and n not in names}
    for name in sorted(names | deleted):
        target = safe_target(root, name)
        current = digest(target)
        if authored(name) and current is not None and current not in (previous.get(name), manifest['files'][name]):
            preserved.append(name)
            continue
        if name not in deleted and current == manifest['files'][name]:
            continue
        if target.exists():
            atomic_copy(target, backup / 'files' / name, target.stat().st_mode & 0o777)
        (backup / 'live-files-started').touch()
        journal.append(name)
        write_journal(backup / 'journal.json', journal)
        if name in deleted:
            target.unlink()
        else:
            atomic_copy(stage / name, target, 0o700 if name == 'bin/savsoul' or name.endswith('.sh') else 0o600)
    write_journal(backup / 'journal.json', journal)
    print('Preserved live edits:', ', '.join(preserved) or 'none', flush=True)


def game_pid():
    pid = int(PID.read_text())
    actual = os.readlink('/proc/{}/exe'.format(pid)).removesuffix(' (deleted)')
    if actual != str(ROOT / 'bin/savsoul') or Path('/proc/{}'.format(pid)).stat().st_uid != os.getuid():
        raise ValueError('Game process identity mismatch')
    return pid


def copyover(backup):
    old = game_pid()
    subprocess.run([sys.executable, str(ROOT / 'tools/mudlet/export_map.py'),
                    '--root', str(ROOT), '--output', '/var/www/lpmud/mudlet/savsoul.xml'], check=True)
    (backup / 'copyover-requested').touch()
    print('Requesting game copyover (SIGUSR2), PID {}.'.format(old), flush=True)
    os.kill(old, signal.SIGUSR2)
    for _ in range(90):
        time.sleep(1)
        try:
            new = game_pid()
            if new != old and os.path.samefile('/proc/{}/exe'.format(new), ROOT / 'bin/savsoul'):
                with socket.create_connection(('127.0.0.1', 4000), timeout=2) as conn:
                    conn.settimeout(2)
                    if conn.recv(4096):
                        print('Copyover ready: successor PID {} is running the released executable.'.format(new), flush=True)
                        return
        except (OSError, ValueError):
            pass
    raise RuntimeError('Successor game did not become healthy within 90 seconds')


def main():
    action, revision = sys.argv[1:]
    STATE.mkdir(mode=0o700, parents=True, exist_ok=True)
    backup = STATE / ('backup-' + revision)
    if action == 'preflight':
        if backup.exists():
            raise ValueError('Revision already attempted; use a new commit for retry')
        game_pid()
        baseline = json.loads((STATE / 'current.json').read_text())
        check_destinations(baseline.get('files', {}))
        print('Deployment revision and game identity verified.', flush=True)
        return
    if action == 'rollback':
        if not (backup / 'journal.json').exists():
            return
        restore(backup)
        old = backup / 'previous.json'
        atomic_copy(old, STATE / 'current.json')
        return
    if action == 'commit':
        atomic_copy(STATE / 'pending.json', STATE / 'current.json')
        backups = sorted(STATE.glob('backup-*'), key=lambda p: p.stat().st_mtime, reverse=True)
        for old in backups[3:]:
            shutil.rmtree(old)
        return
    if action == 'deploy' and backup.exists():
        raise ValueError('Revision already attempted; use a new commit for retry')
    with tempfile.TemporaryDirectory(prefix='stage-', dir=STATE) as temporary:
        stage = Path(temporary)
        manifest = unpack(sys.stdin.buffer, stage, revision)
        for name in ('tools/wizard-editor/requirements.txt', 'tools/web-terminal/requirements.txt',
                     'tools/intermud/requirements.lock'):
            if name in manifest['files'] and requirements(ROOT / name) != requirements(stage / name):
                raise ValueError('Provision updated Python dependencies before deploying: ' + name)
        baseline = json.loads((STATE / 'current.json').read_text())
        if action == 'validate':
            print('Release archive and dependency preflight validated; no changes applied.', flush=True)
            return
        try:
            apply(stage, backup, manifest, baseline)
            if PUBLIC_MAP.exists():
                atomic_copy(PUBLIC_MAP, backup / 'public-map.xml')
            else:
                (backup / 'public-map-missing').touch()
            copyover(backup)
            public = STATE / 'public'
            if public.exists():
                shutil.rmtree(public)
            shutil.copytree(stage / 'public', public)
            (STATE / 'pending.json').write_text(json.dumps(manifest))
        except BaseException:
            if (backup / 'journal.json').exists():
                restore(backup)
            raise


if __name__ == '__main__':
    main()

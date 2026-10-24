"""Unprivileged candidate release transaction; imported from an immutable ConfigMap."""
import fcntl
import json
import os
from pathlib import Path
import secrets
import signal
import shutil
import subprocess
import time

import receive_release as files

ROOT = Path('/home/savsoul/grim')
STATE = Path('/home/savsoul/state/releases')
PUBLIC = Path('/var/www/lpmud')
RUNNER = Path('/home/savsoul/state/runner')
INSTANCE = {'tools/intermud/gateway.yaml'}


def stop_service(root, relative, deadline=15):
    """Stop only a same-UID process with the exact service script and working root."""
    victims = []
    for item in Path('/proc').iterdir():
        if not item.name.isdecimal() or item.name == str(os.getpid()):
            continue
        try:
            args = (item / 'cmdline').read_bytes().split(b'\0')
            script = str(root / relative).encode()
            if (item.stat().st_uid == os.getuid() and
                    (script in args or relative.encode() in args) and
                    (item / 'cwd').resolve() == root):
                os.kill(int(item.name), signal.SIGTERM)
                victims.append(item)
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    end = time.monotonic() + deadline
    while any(item.exists() for item in victims):
        if time.monotonic() > end:
            raise RuntimeError('Service did not stop before the release deadline')
        time.sleep(.1)


def stop_editor(root=ROOT):
    stop_service(root, 'tools/server-host/remote_editor.py')


class Transaction:
    def __init__(self, root=ROOT, state=STATE, public=PUBLIC, runner=RUNNER):
        self.root, self.state, self.public, self.runner = root, state, public, runner

    def baseline(self):
        return json.loads((self.state / 'current.json').read_text())

    def recover_startup(self):
        active = self.state / 'active.json'
        if active.exists():
            # A crash can occur during the temporary unfreeze used to verify services.
            files.write_journal(self.runner / 'release-in-progress', json.loads(active.read_text()))
            files.write_journal(self.runner / 'editor-save-block', json.loads(active.read_text()))
            stop_editor(self.root)

    def validate(self, stage, manifest):
        for name in ('tools/wizard-editor/requirements.txt', 'tools/web-terminal/requirements.txt',
                     'tools/intermud/requirements.lock'):
            if name not in manifest['files'] or files.requirements(self.root / name) != files.requirements(stage / name):
                raise ValueError('Prepare updated runtime dependencies before releasing: ' + name)
        binary = stage / 'bin/savsoul'
        binary.chmod(0o700)
        result = subprocess.run([str(binary), '--version'], check=True, capture_output=True,
                                text=True, timeout=10)
        expected = '{} ({})'.format(manifest['version'], manifest['revision'][:12])
        if result.stdout.strip() != expected:
            raise ValueError('Executable build identity does not match the manifest')

    def publish(self, stage, backup, manifest):
        journal = []
        for name in sorted(n for n in manifest['files'] if n.startswith('public/')):
            relative = name.removeprefix('public/')
            target = files.safe_target(self.public, relative)
            old = backup / 'public' / relative
            if target.exists():
                files.atomic_copy(target, old, target.stat().st_mode & 0o777)
            journal.append(relative)
            files.write_journal(backup / 'public-journal.json', journal)
            target.parent.mkdir(parents=True, exist_ok=True, mode=0o750)
            for parent in [target.parent, *target.parent.parents]:
                if parent == self.public:
                    break
                parent.chmod(0o750)
            files.atomic_copy(stage / name, target, 0o640)

    def restore_public(self, backup):
        path = backup / 'public-journal.json'
        if not path.exists():
            return
        for name in reversed(json.loads(path.read_text())):
            target = files.safe_target(self.public, name)
            old = backup / 'public' / name
            if old.exists():
                files.atomic_copy(old, target, old.stat().st_mode & 0o777)
            else:
                target.unlink(missing_ok=True)

    def copyover(self, backup):
        files.copyover(backup)

    def confirm_running(self):
        pid = files.game_pid()
        if not os.path.samefile('/proc/{}/exe'.format(pid), self.root / 'bin/savsoul'):
            raise RuntimeError('Committed executable is not running')
        if not any(len(f) > 3 and f[1].endswith(':0FA0') and f[3] == '0A'
                   for f in (line.split() for line in Path('/proc/net/tcp').read_text().splitlines()[1:])):
            raise RuntimeError('Committed game listener is not ready')

    def wait_services(self, deadline=60):
        end = time.monotonic() + deadline
        while time.monotonic() < end:
            ports = {f[1].split(':')[-1] for f in
                     (line.split() for line in Path('/proc/net/tcp').read_text().splitlines()[1:])
                     if len(f) > 3 and f[3] == '0A'}
            if all(format(port, '04X') in ports for port in (4085, 4086)):
                return
            time.sleep(.5)
        raise RuntimeError('Supporting services did not recover before the deadline')

    def prune(self):
        backups = sorted(self.state.glob('backup-*'), key=lambda p: p.stat().st_mtime, reverse=True)
        for old in backups[3:]:
            if old.is_dir() and not old.is_symlink():
                shutil.rmtree(old)

    def execute(self, stage, manifest):
        baseline = self.baseline()
        self.validate(stage, manifest)
        active, marker = self.state / 'active.json', self.runner / 'release-in-progress'
        save_block = self.runner / 'editor-save-block'
        if (baseline.get('revision') == manifest['revision'] and
                files.digest(self.root / 'bin/savsoul') == manifest['files']['bin/savsoul']):
            if active.exists() and json.loads(active.read_text()).get('revision') != manifest['revision']:
                raise RuntimeError('A different unfinished release requires recovery')
            self.confirm_running()
            if active.exists():
                marker.unlink(missing_ok=True)
                self.wait_services()
                save_block.unlink(missing_ok=True)
                active.unlink()
            elif marker.exists() or save_block.exists():
                raise RuntimeError('An unexplained editor freeze requires operator recovery')
            return {'revision': manifest['revision'], 'version': manifest['version'], 'already_applied': True}
        if active.exists() or marker.exists() or save_block.exists():
            raise RuntimeError('An unfinished release requires operator recovery before retrying')
        backup = self.state / ('backup-' + manifest['revision'] + '-' + secrets.token_hex(4))
        filtered = {**manifest, 'files': {n: h for n, h in manifest['files'].items() if n not in INSTANCE}}
        previous = {**baseline, 'files': {n: h for n, h in baseline.get('files', {}).items() if n not in INSTANCE}}
        files.check_destinations(set(filtered['files']) | set(previous['files']), self.root)
        files.write_journal(active, {'revision': manifest['revision'], 'backup': backup.name})
        files.write_journal(save_block, {'revision': manifest['revision']})
        files.write_journal(marker, {'revision': manifest['revision']})
        restored = False
        try:
            stop_editor(self.root)
            files.apply(stage, backup, filtered, previous, self.root)
            self.publish(stage, backup, manifest)
            if files.PUBLIC_MAP.exists():
                files.atomic_copy(files.PUBLIC_MAP, backup / 'public-map.xml')
            else:
                (backup / 'public-map-missing').touch()
            self.copyover(backup)
            if any(n.startswith('tools/web-terminal/') and baseline.get('files', {}).get(n) != manifest['files'].get(n)
                   for n in set(baseline.get('files', {})) | set(manifest['files'])):
                stop_service(self.root, 'tools/web-terminal/bridge.py')
            marker.unlink()
            self.wait_services()
            files.write_journal(self.state / 'current.json', manifest)
            restored = True
            return {'revision': manifest['revision'], 'version': manifest['version'], 'already_applied': False}
        except BaseException:
            files.write_journal(marker, {'revision': manifest['revision']})
            stop_editor(self.root)
            if (backup / 'journal.json').exists():
                files.restore(backup, self.root)
            self.restore_public(backup)
            files.write_journal(self.state / 'current.json', baseline)
            if (backup / 'live-files-started').exists():
                self.copyover(backup)
            stop_service(self.root, 'tools/web-terminal/bridge.py')
            marker.unlink()
            try:
                self.wait_services()
            except BaseException:
                files.write_journal(marker, {'revision': manifest['revision']})
                raise
            restored = True
            raise
        finally:
            if restored:
                marker.unlink(missing_ok=True)
                save_block.unlink(missing_ok=True)
                active.unlink(missing_ok=True)

    def release(self, stream, revision):
        import tempfile
        self.state.mkdir(parents=True, exist_ok=True, mode=0o700)
        with (self.state / 'kubernetes.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with tempfile.TemporaryDirectory(prefix='stage-', dir=self.state) as folder:
                stage = Path(folder)
                manifest = files.unpack(stream, stage, revision)
                result = self.execute(stage, manifest)
                # Retention failure must not roll back a healthy committed release.
                try:
                    self.prune()
                except OSError:
                    pass
                return result

#!/usr/bin/python3
"""Root-owned SSH forced command. Compatible with the host's Python 3.4.

Never import modules from the game checkout. Parse artifacts and run game code
only after dropping all supplementary groups and switching to the game UID.
"""
import fcntl
import json
import os
import pwd
import re
import socket
import subprocess
import sys
import time

STATE = '/home/savsoul/state/releases'
PUBLIC = '/var/www/lpmud'
BACKUPS = '/var/lib/savsoul-release'
SERVICES = ['savsoul-editor', 'savsoul-web-terminal']
LOCK = '/var/lock/savsoul-release.lock'


def receiver(action, revision):
    user = pwd.getpwnam('savsoul')

    def demote():
        os.setgroups([])
        os.setgid(user.pw_gid)
        os.setuid(user.pw_uid)
        os.umask(0o077)

    subprocess.check_call([
        '/home/savsoul/runtime/python/bin/python3', '-I',
        '/usr/local/lib/savsoul-release/receive_release.py', action, revision],
        preexec_fn=demote, cwd='/home/savsoul/grim',
        env={'HOME': '/home/savsoul', 'PATH': '/usr/bin:/bin',
             'SSL_CERT_FILE': '/home/savsoul/runtime/cacert.pem'})


def regular_path(root, name):
    path = os.path.join(root, name)
    parent = path
    while parent != root:
        if os.path.islink(parent):
            raise ValueError('Symlink in public site path')
        parent = os.path.dirname(parent)
    return path


def website_name(name):
    vendor = {'vendor/addon-fit.js', 'vendor/addon-fit-LICENSE', 'vendor/VERSIONS.json',
              'vendor/xterm.css', 'vendor/xterm.js', 'vendor/xterm-LICENSE'}
    return name in vendor or bool(re.match(r'^[a-z][a-z0-9.-]*\.(?:html|css|js)$', name))


def public_content(path):
    # Even a raced parent symlink must not let this root helper read root files.
    user = pwd.getpwnam('savsoul')
    os.setgroups([])
    os.setegid(user.pw_gid)
    os.seteuid(user.pw_uid)
    try:
        return read_regular(path)
    finally:
        os.seteuid(0)
        os.setegid(0)


def read_regular(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as stream:
        import stat
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise ValueError('Expected regular static file')
        content = stream.read(4 * 1024 * 1024 + 1)
        if len(content) > 4 * 1024 * 1024:
            raise ValueError('Static file too large')
        return content


def write_atomic(path, content):
    import tempfile
    fd, temporary = tempfile.mkstemp(prefix='.release-', dir=os.path.dirname(path))
    try:
        with os.fdopen(fd, 'wb') as out:
            out.write(content)
            out.flush()
            os.fsync(out.fileno())
        os.chmod(temporary, 0o644)
        os.rename(temporary, path)
        directory = os.open(os.path.dirname(path), os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def publish(revision):
    source = STATE + '/public'
    backup = BACKUPS + '/' + revision
    os.makedirs(backup, mode=0o700)
    names, total = [], 0
    for directory, dirs, files in os.walk(source, followlinks=False):
        for name in files:
            relative = os.path.relpath(os.path.join(directory, name), source)
            if not website_name(relative):
                raise ValueError('Unsupported generated site path')
            content = public_content(regular_path(source, relative))
            total += len(content)
            if total > 8 * 1024 * 1024 or len(names) >= 100:
                raise ValueError('Generated website exceeds publication bounds')
            target = regular_path(PUBLIC, relative)
            saved = backup + '/' + relative
            os.makedirs(os.path.dirname(saved), mode=0o700, exist_ok=True)
            if os.path.exists(target):
                write_atomic(saved, read_regular(target))
            names.append(relative)
            write_atomic(backup + '/journal.json', json.dumps(names).encode('utf8'))
            write_atomic(target, content)


def restore_site(revision):
    backup = BACKUPS + '/' + revision
    if not os.path.exists(backup + '/journal.json'):
        return
    with open(backup + '/journal.json') as stream:
        names = json.load(stream)
    for name in names:
        target = regular_path(PUBLIC, name)
        saved = backup + '/' + name
        if os.path.exists(saved):
            write_atomic(target, read_regular(saved))
        elif os.path.exists(target):
            os.unlink(target)


def restart(service):
    child = subprocess.Popen(['/sbin/status', service], stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT)
    try:
        status = child.communicate(timeout=30)[0]
    except subprocess.TimeoutExpired:
        child.kill()
        child.communicate()
        raise
    if b'start/running' in status:
        if subprocess.call(['/sbin/restart', service], timeout=30) == 0:
            return
    subprocess.check_call(['/sbin/start', service], timeout=30)


def healthy():
    for port in (4000, 4085, 4086):
        for attempt in range(30):
            try:
                connection = socket.create_connection(('127.0.0.1', port), timeout=2)
                connection.close()
                break
            except OSError:
                if attempt == 29:
                    raise
                time.sleep(1)


def game_recovery_needed(revision):
    backup = STATE + '/backup-' + revision
    return (os.path.exists(backup + '/live-files-started')
            or os.path.exists(backup + '/copyover-requested'))


def main():
    if len(sys.argv) > 2:
        raise ValueError('Expected one release command')
    command = sys.argv[1] if len(sys.argv) == 2 else os.environ.get('SSH_ORIGINAL_COMMAND', '')
    if command == 'check':
        healthy()
        print('Restricted release command ready; production listeners healthy.')
        return
    match = re.match(r'^(deploy|validate) ([0-9a-f]{40})\Z', command)
    if not match:
        raise ValueError('Only check, validate <revision>, or deploy <revision> is allowed')
    action, revision = match.groups()
    with open(LOCK, 'w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if action == 'validate':
            receiver(action, revision)
            return
        receiver('preflight', revision)
        if os.path.exists(BACKUPS + '/' + revision):
            raise ValueError('Public backup for this revision already exists')
        applied, site_started = False, False
        try:
            subprocess.check_call(['/sbin/stop', 'savsoul-editor'], timeout=30)
            receiver('deploy', revision)
            applied = True
            for service in SERVICES:
                restart(service)
            site_started = True
            publish(revision)
            healthy()
            receiver('commit', revision)
            # Root-owned public backups are small; retain only the latest three.
            import shutil
            backups = sorted(os.listdir(BACKUPS), key=lambda n: os.stat(BACKUPS + '/' + n).st_mtime, reverse=True)
            for old in backups[3:]:
                shutil.rmtree(BACKUPS + '/' + old)
            print('Released ' + revision)
        except BaseException:
            # A killed receiver cannot restore itself. Preflight rejected any
            # existing attempt, so these markers belong to this invocation.
            # Rollback is idempotent if the receiver already restored its files.
            if applied or game_recovery_needed(revision):
                receiver('rollback', revision)
            if site_started:
                restore_site(revision)
            recovery = ['savsoul-grim'] if game_recovery_needed(revision) else []
            for service in recovery + SERVICES:
                restart(service)
            healthy()
            raise


if __name__ == '__main__':
    main()

import hashlib
import io
import json
import os
import signal
import subprocess
import sys
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import receive_release as release
import release_entry as entry
import deploy_ssh


def checksum(content):
    return hashlib.sha256(content).hexdigest()


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root, self.stage = self.base / 'live', self.base / 'stage'
        self.root.mkdir()
        self.stage.mkdir()
        self.backup = self.base / 'backup'

    def write(self, root, name, data):
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    def test_live_authored_edits_and_accounts_are_preserved(self):
        files = {'bin/savsoul': b'new-binary', 'data/areas/town.json': b'new-room',
                 'assets/motdlist.txt': b'new-motd'}
        for name, data in files.items():
            self.write(self.stage, name, data)
        self.write(self.root, 'bin/savsoul', b'old-binary')
        self.write(self.root, 'data/areas/town.json', b'live-editor-change')
        self.write(self.root, 'assets/motdlist.txt', b'old-motd')
        self.write(self.root, 'data/characters/rawr.json', b'private-player-progress')
        manifest = {'files': {n: checksum(data) for n, data in files.items()}}
        baseline = {'files': {'data/areas/town.json': checksum(b'old-room'),
                              'assets/motdlist.txt': checksum(b'old-motd')}}
        release.apply(self.stage, self.backup, manifest, baseline, self.root)
        self.assertEqual((self.root / 'bin/savsoul').read_bytes(), b'new-binary')
        self.assertEqual((self.root / 'data/areas/town.json').read_bytes(), b'live-editor-change')
        self.assertEqual((self.root / 'assets/motdlist.txt').read_bytes(), b'new-motd')
        self.assertEqual((self.root / 'data/characters/rawr.json').read_bytes(), b'private-player-progress')
        release.restore(self.backup, self.root)
        self.assertEqual((self.root / 'bin/savsoul').read_bytes(), b'old-binary')
        self.assertEqual((self.root / 'assets/motdlist.txt').read_bytes(), b'old-motd')
        self.assertEqual((self.root / 'data/characters/rawr.json').read_bytes(), b'private-player-progress')

    def test_removed_code_restored_and_new_seed_removed_on_rollback(self):
        self.write(self.root, 'tools/obsolete.py', b'old-code')
        self.write(self.stage, 'data/help/new.md', b'new-help')
        release.apply(self.stage, self.backup,
                      {'files': {'data/help/new.md': checksum(b'new-help')}},
                      {'files': {'tools/obsolete.py': checksum(b'old-code')}}, self.root)
        self.assertFalse((self.root / 'tools/obsolete.py').exists())
        release.restore(self.backup, self.root)
        self.assertEqual((self.root / 'tools/obsolete.py').read_bytes(), b'old-code')
        self.assertFalse((self.root / 'data/help/new.md').exists())

    def test_interrupted_journal_update_preserves_previous_recovery_entries(self):
        """A failed second journal publication must retain the first file's undo record."""
        for name in ['bin/savsoul', 'tools/helper.py']:
            self.write(self.root, name, b'old')
            self.write(self.stage, name, b'new')
        replace = os.replace
        publications = 0

        def interrupt(source, target):
            nonlocal publications
            if Path(target).name == 'journal.json':
                publications += 1
                if publications == 2:
                    raise OSError('interrupted journal publication')
            return replace(source, target)

        with patch.object(release.os, 'replace', side_effect=interrupt):
            with self.assertRaisesRegex(OSError, 'interrupted journal'):
                release.apply(self.stage, self.backup,
                              {'files': {n: checksum(b'new') for n in ['bin/savsoul', 'tools/helper.py']}},
                              {'files': {}}, self.root)
        self.assertEqual(json.loads((self.backup / 'journal.json').read_text()), ['bin/savsoul'])
        self.assertEqual((self.root / 'bin/savsoul').read_bytes(), b'new')
        self.assertEqual((self.root / 'tools/helper.py').read_bytes(), b'old')
        release.restore(self.backup, self.root)
        self.assertEqual((self.root / 'bin/savsoul').read_bytes(), b'old')

    def test_interrupted_public_journal_still_restores_published_pages(self):
        """Static-site rollback retains its prior journal if the next update fails."""
        source, public, backups = self.base / 'public', self.base / 'website', self.base / 'sites'
        public.mkdir()
        backups.mkdir()
        names = ['index.html', 'news.html']
        for name in names:
            self.write(source, name, b'new')
            self.write(public, name, b'old')
        rename = os.rename
        publications = 0

        def interrupt(temporary, target):
            nonlocal publications
            if Path(target).name == 'journal.json':
                publications += 1
                if publications == 2:
                    raise OSError('interrupted public journal')
            return rename(temporary, target)

        with patch.object(entry, 'STATE', str(self.base)), \
                patch.object(entry, 'PUBLIC', str(public)), \
                patch.object(entry, 'BACKUPS', str(backups)), \
                patch.object(entry.os, 'walk', return_value=[(str(source), [], names)]), \
                patch.object(entry, 'public_content', side_effect=lambda path: Path(path).read_bytes()), \
                patch.object(entry.os, 'rename', side_effect=interrupt):
            with self.assertRaisesRegex(OSError, 'interrupted public journal'):
                entry.publish('a' * 40)
            journal = backups / ('a' * 40) / 'journal.json'
            self.assertEqual(json.loads(journal.read_text()), ['index.html'])
            self.assertEqual((public / 'index.html').read_bytes(), b'new')
            self.assertEqual((public / 'news.html').read_bytes(), b'old')
            entry.restore_site('a' * 40)
        for name in names:
            self.assertEqual((public / name).read_bytes(), b'old')

    def test_rejects_private_and_escaping_paths(self):
        for name in ('../evil', '/bin/savsoul', 'tools/../evil', 'data/characters/rawr.json',
                     'data/intermud.json', 'data/mudlet-map-ids.json', 'tools\\evil', 'tools//evil'):
            with self.subTest(name=name):
                self.assertFalse(release.allowed(name))

    def test_destination_symlink_rejected(self):
        (self.root / 'tools').symlink_to(self.stage, target_is_directory=True)
        with self.assertRaises(ValueError):
            release.safe_target(self.root, 'tools/file.py')

    def archive(self, files, revision='a' * 40, extra=None):
        buffer = io.BytesIO()
        manifest = {'revision': revision, 'files': {n: checksum(d) for n, d in files.items()}}
        with tarfile.open(fileobj=buffer, mode='w:gz') as archive:
            for name, data in {**files, 'manifest.json': json.dumps(manifest).encode()}.items():
                entry = tarfile.TarInfo(name)
                entry.size = len(data)
                archive.addfile(entry, io.BytesIO(data))
            if extra:
                archive.addfile(extra)
        buffer.seek(0)
        return buffer

    def test_valid_archive(self):
        stream = self.archive({'bin/savsoul': b'binary', 'public/index.html': b'page'})
        manifest = release.unpack(stream, self.stage, 'a' * 40)
        self.assertEqual(len(manifest['files']), 2)

    def test_revision_mismatch(self):
        with self.assertRaises(ValueError):
            release.unpack(self.archive({'bin/savsoul': b'b', 'public/index.html': b'p'}),
                           self.stage, 'b' * 40)

    def test_archive_symlink_rejected(self):
        link = tarfile.TarInfo('tools/link')
        link.type, link.linkname = tarfile.SYMTYPE, '/etc/passwd'
        with self.assertRaises(ValueError):
            release.unpack(self.archive({'bin/savsoul': b'b', 'public/index.html': b'p'}, extra=link),
                           self.stage, 'a' * 40)

    def test_duplicate_member_rejected(self):
        extra = tarfile.TarInfo('bin/savsoul')
        with self.assertRaises(ValueError):
            release.unpack(self.archive({'bin/savsoul': b'b', 'public/index.html': b'p'}, extra=extra),
                           self.stage, 'a' * 40)

    def test_duplicate_preflight_cannot_run_rollback_or_stop_services(self):
        with patch.dict('os.environ', {'SSH_ORIGINAL_COMMAND': 'deploy ' + 'a' * 40}), \
                patch.object(entry.sys, 'argv', ['release_entry.py']), \
                patch.object(entry, 'receiver', side_effect=ValueError('already attempted')) as receiver, \
                patch.object(entry, 'BACKUPS', str(self.base)), \
                patch('builtins.open', return_value=io.StringIO()), \
                patch.object(entry.fcntl, 'flock'), \
                patch.object(entry.subprocess, 'check_call') as command:
            with self.assertRaises(ValueError):
                entry.main()
            receiver.assert_called_once_with('preflight', 'a' * 40)
            command.assert_not_called()

    def test_privileged_site_names_are_static_only(self):
        for name in ('vendor/shadow', 'vendor/.htaccess', 'config.php', '../index.html'):
            self.assertFalse(entry.website_name(name))
        self.assertTrue(entry.website_name('vendor/xterm.js'))
        self.assertTrue(entry.website_name('index.au.html'))

    def test_fifo_source_rejected_without_blocking(self):
        fifo = self.base / 'pipe'
        os.mkfifo(fifo)
        with self.assertRaises(ValueError):
            entry.read_regular(str(fifo))

    def test_rollback_restores_public_map(self):
        self.backup.mkdir()
        (self.backup / 'journal.json').write_text('[]')
        (self.backup / 'public-map.xml').write_bytes(b'previous-map')
        public = self.base / 'map.xml'
        public.write_bytes(b'failed-map')
        with patch.object(release, 'PUBLIC_MAP', public):
            release.restore(self.backup, self.root)
        self.assertEqual(public.read_bytes(), b'previous-map')

    def test_requirements_ignore_line_endings_but_detect_changed_pins(self):
        old, new = self.base / 'old.txt', self.base / 'new.txt'
        old.write_bytes(b'# installed\r\naiohttp==3.14.5\r\r\ntelnetlib3==5.0.1\r\n')
        new.write_bytes(b'telnetlib3==5.0.1\naiohttp==3.14.5\n')
        self.assertEqual(release.requirements(old), release.requirements(new))
        new.write_bytes(b'telnetlib3==5.0.2\naiohttp==3.14.5\n')
        self.assertNotEqual(release.requirements(old), release.requirements(new))

    def exercise_entry(self, failure=None, marker=None):
        revision = 'a' * 40
        backups = self.base / 'site-backups'
        backups.mkdir()

        def receive(action, _revision):
            if action == 'deploy':
                if marker:
                    backup = self.base / ('backup-' + revision)
                    backup.mkdir()
                    (backup / marker).touch()
                if failure:
                    raise failure

        with patch.dict('os.environ', {'SSH_ORIGINAL_COMMAND': 'deploy ' + revision}), \
                patch.object(entry.sys, 'argv', ['release_entry.py']), \
                patch.object(entry, 'STATE', str(self.base)), \
                patch.object(entry, 'BACKUPS', str(backups)), \
                patch.object(entry, 'LOCK', str(self.base / 'release.lock')), \
                patch.object(entry, 'receiver', side_effect=receive), \
                patch.object(entry.subprocess, 'check_call'), \
                patch.object(entry, 'publish'), patch.object(entry, 'healthy'), \
                patch.object(entry, 'restart') as restart:
            if failure:
                with self.assertRaisesRegex(RuntimeError, str(failure)):
                    entry.main()
            else:
                entry.main()
            return [call.args[0] for call in restart.call_args_list]

    def test_successful_release_never_restarts_game_service(self):
        self.assertEqual(self.exercise_entry(), entry.SERVICES)

    def test_rejected_archive_does_not_restart_game(self):
        self.assertEqual(self.exercise_entry(RuntimeError('invalid archive')), entry.SERVICES)

    def test_failed_live_file_changes_restart_game_for_recovery(self):
        self.assertEqual(self.exercise_entry(RuntimeError('apply failed'), 'live-files-started'),
                         ['savsoul-grim'] + entry.SERVICES)

    def test_failed_copyover_without_file_changes_still_recovers_game(self):
        self.assertEqual(self.exercise_entry(RuntimeError('copyover failed'), 'copyover-requested'),
                         ['savsoul-grim'] + entry.SERVICES)

    def test_killed_receiver_restores_files_before_restarting_services(self):
        """SIGKILL after a real replacement must not expose an uncommitted binary."""
        revision = 'a' * 40
        backup = self.base / ('backup-' + revision)
        sites = self.base / 'site-backups'
        sites.mkdir()
        self.write(self.root, 'bin/savsoul', b'old-binary')
        self.write(self.stage, 'bin/savsoul', b'new-binary')
        baseline = {'revision': 'b' * 40, 'files': {'bin/savsoul': checksum(b'old-binary')}}
        current = self.base / 'current.json'
        current.write_text(json.dumps(baseline))
        actions = []
        child = '''import json, os, signal, sys
from pathlib import Path
import receive_release as r
stage, backup, root, baseline = map(Path, sys.argv[1:])
copy = r.atomic_copy
def interrupt(source, target, mode=0o600):
    copy(source, target, mode)
    if target == root / 'bin/savsoul':
        os.kill(os.getpid(), signal.SIGKILL)
r.atomic_copy = interrupt
r.apply(stage, backup, {'files': {'bin/savsoul': r.digest(stage / 'bin/savsoul')}},
        json.loads(baseline.read_text()), root)
'''

        def receive(action, _revision):
            actions.append(action)
            if action == 'deploy':
                subprocess.run([sys.executable, '-c', child, str(self.stage),
                                str(backup), str(self.root), str(current)],
                               cwd=Path(__file__).resolve().parent, check=True)
            elif action == 'rollback':
                self.assertEqual((self.root / 'bin/savsoul').read_bytes(), b'new-binary')
                release.restore(backup, self.root)
                release.atomic_copy(backup / 'previous.json', current)

        def restart(service):
            actions.append(service)
            self.assertEqual((self.root / 'bin/savsoul').read_bytes(), b'old-binary')
            self.assertEqual(json.loads(current.read_text()), baseline)

        with patch.object(entry.sys, 'argv', ['release_entry.py', 'deploy ' + revision]), \
                patch.object(entry, 'STATE', str(self.base)), \
                patch.object(entry, 'BACKUPS', str(sites)), \
                patch.object(entry, 'LOCK', str(self.base / 'release.lock')), \
                patch.object(entry, 'receiver', side_effect=receive), \
                patch.object(entry.subprocess, 'check_call'), \
                patch.object(entry, 'healthy'), patch.object(entry, 'publish') as publish, \
                patch.object(entry, 'restart', side_effect=restart):
            with self.assertRaises(subprocess.CalledProcessError) as failure:
                entry.main()
        self.assertEqual(failure.exception.returncode, -signal.SIGKILL)
        self.assertEqual(actions, ['preflight', 'deploy', 'rollback', 'savsoul-grim'] + entry.SERVICES)
        publish.assert_not_called()

    def test_copyover_signals_old_pid_and_waits_for_new_executable(self):
        self.backup.mkdir()
        with patch.object(release, 'game_pid', side_effect=[101, 202]), \
                patch.object(release.subprocess, 'run') as export, \
                patch.object(release.os, 'kill') as kill, \
                patch.object(release.os.path, 'samefile', return_value=True), \
                patch.object(release.time, 'sleep'), \
                patch.object(release.socket, 'create_connection') as connect:
            connect.return_value.__enter__.return_value.recv.return_value = b'login banner'
            release.copyover(self.backup)
        kill.assert_called_once_with(101, signal.SIGUSR2)
        export.assert_called_once()
        self.assertTrue((self.backup / 'copyover-requested').exists())

    def test_copyover_timeout_is_a_failure(self):
        self.backup.mkdir()
        with patch.object(release, 'game_pid', return_value=101), \
                patch.object(release.subprocess, 'run'), patch.object(release.os, 'kill'), \
                patch.object(release.time, 'sleep'):
            with self.assertRaisesRegex(RuntimeError, 'Successor game'):
                release.copyover(self.backup)

    def test_deploy_account_only_invokes_fixed_sudo_wrapper(self):
        for command in ('check', 'validate ' + 'a' * 40, 'deploy ' + 'b' * 40):
            with self.subTest(command=command), \
                    patch.dict('os.environ', {'SSH_ORIGINAL_COMMAND': command}), \
                    patch.object(deploy_ssh.subprocess, 'call', return_value=0) as run:
                self.assertEqual(deploy_ssh.main(), 0)
                run.assert_called_once_with(['/usr/bin/sudo', '-n', '/usr/local/sbin/savsoul-deploy', command])

    def test_deploy_account_rejects_shell_sftp_and_extra_commands(self):
        for command in ('', 'sh', 'id', 'sudo -n sh', 'internal-sftp', 'scp -t /tmp/test',
                        'check; id', 'deploy ' + 'a' * 40 + '\nid', 'deploy ' + 'a' * 40 + '\n'):
            with self.subTest(command=command), \
                    patch.dict('os.environ', {'SSH_ORIGINAL_COMMAND': command}), \
                    patch.object(deploy_ssh.subprocess, 'call') as run:
                with self.assertRaises(ValueError):
                    deploy_ssh.main()
                run.assert_not_called()

    def test_root_dispatcher_accepts_only_one_explicit_command(self):
        with patch.object(entry.sys, 'argv', ['release_entry.py', 'check']), \
                patch.object(entry, 'healthy') as healthy:
            entry.main()
            healthy.assert_called_once()
        with patch.object(entry.sys, 'argv', ['release_entry.py', 'check', 'id']):
            with self.assertRaises(ValueError):
                entry.main()


if __name__ == '__main__':
    unittest.main()

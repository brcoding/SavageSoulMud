"""Exercise preservation, service-failure rollback and interrupted-release guards."""
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server-host'))
import receive_release as files
import release_transaction as release
from release_server import LimitedBody


class CandidateReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        base = Path(self.temp.name)
        self.root, self.state, self.public, self.runner, self.stage = [base / n for n in
            ('root', 'state', 'public', 'runner', 'stage')]
        for path in (self.root, self.state, self.public, self.runner, self.stage):
            path.mkdir()
        self.write(self.root, 'bin/savsoul', 'old binary')
        self.write(self.root, 'assets/room.json', 'live wizard edits')
        self.write(self.root, 'data/accounts.json', 'private players')
        self.write(self.root, 'tools/intermud/gateway.yaml', 'instance gateway')
        self.write(self.public, 'index.html', 'old website')
        self.old = {'revision': 'a' * 40, 'version': '1.0.0', 'files': {
            'bin/savsoul': files.digest(self.root / 'bin/savsoul'),
            'assets/room.json': 'previous authored hash'}}
        files.write_journal(self.state / 'current.json', self.old)
        for name, value in {'bin/savsoul': 'new binary', 'assets/room.json': 'repository room',
                            'tools/intermud/gateway.yaml': 'repository default',
                            'public/index.html': 'new website', 'public/new/page.html': 'new page'}.items():
            self.write(self.stage, name, value)
        self.new = {'revision': 'b' * 40, 'version': '1.0.1', 'files': {
            str(p.relative_to(self.stage)): files.digest(p) for p in self.stage.rglob('*') if p.is_file()}}
        self.tx = release.Transaction(self.root, self.state, self.public, self.runner)
        for target in ('validate', 'confirm_running', 'copyover', 'wait_services'):
            mock = patch.object(self.tx, target).start()
            setattr(self, target, mock)
        self.stop = patch.object(release, 'stop_service').start()
        patch.object(release, 'stop_editor').start()
        patch.object(files, 'PUBLIC_MAP', self.public / 'map.xml').start()
        self.addCleanup(patch.stopall)

    @staticmethod
    def write(root, name, text):
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def test_preserves_private_and_live_authored_state_and_is_idempotent(self):
        self.tx.execute(self.stage, self.new)
        self.assertEqual((self.root / 'data/accounts.json').read_text(), 'private players')
        self.assertEqual((self.root / 'assets/room.json').read_text(), 'live wizard edits')
        self.assertEqual((self.root / 'tools/intermud/gateway.yaml').read_text(), 'instance gateway')
        self.assertEqual((self.public / 'new/page.html').stat().st_mode & 0o777, 0o640)
        self.assertEqual((self.public / 'new').stat().st_mode & 0o777, 0o750)
        self.assertTrue(self.tx.execute(self.stage, self.new)['already_applied'])
        self.copyover.assert_called_once()
        self.assertFalse((self.state / 'active.json').exists())

    def test_failed_editor_recovery_rolls_back_binary_website_and_receipt(self):
        self.wait_services.side_effect = [RuntimeError('bad editor'), None]
        with self.assertRaisesRegex(RuntimeError, 'bad editor'):
            self.tx.execute(self.stage, self.new)
        self.assertEqual((self.root / 'bin/savsoul').read_text(), 'old binary')
        self.assertEqual((self.public / 'index.html').read_text(), 'old website')
        self.assertFalse((self.public / 'new/page.html').exists())
        self.assertEqual(self.tx.baseline(), self.old)
        self.assertEqual(self.copyover.call_count, 2)
        self.assertFalse((self.runner / 'release-in-progress').exists())

    def test_failed_rollback_retains_freeze_and_blocks_next_release(self):
        self.copyover.side_effect = RuntimeError('game unavailable')
        with self.assertRaisesRegex(RuntimeError, 'game unavailable'):
            self.tx.execute(self.stage, self.new)
        self.assertTrue((self.state / 'active.json').exists())
        self.assertTrue((self.runner / 'release-in-progress').exists())
        with self.assertRaisesRegex(RuntimeError, 'unfinished release'):
            self.tx.execute(self.stage, self.new)

    def test_removed_bridge_module_requires_restart(self):
        self.write(self.root, 'tools/web-terminal/obsolete.py', 'old module')
        self.old['files']['tools/web-terminal/obsolete.py'] = files.digest(
            self.root / 'tools/web-terminal/obsolete.py')
        files.write_journal(self.state / 'current.json', self.old)
        self.tx.execute(self.stage, self.new)
        self.stop.assert_called_with(self.root, 'tools/web-terminal/bridge.py')
        self.assertFalse((self.root / 'tools/web-terminal/obsolete.py').exists())

    def test_body_is_bounded_and_rejects_early_disconnect(self):
        body = LimitedBody(io.BytesIO(b'abcdef'), 3)
        self.assertEqual(body.read(100), b'abc')
        self.assertEqual(body.read(), b'')
        with self.assertRaisesRegex(ValueError, 'ended early'):
            LimitedBody(io.BytesIO(), 2).read()

    def test_restart_refreezes_crash_during_support_readiness(self):
        files.write_journal(self.state / 'active.json', {'revision': self.new['revision']})
        self.assertFalse((self.runner / 'release-in-progress').exists())
        self.tx.recover_startup()
        self.assertTrue((self.runner / 'release-in-progress').exists())
        with self.assertRaisesRegex(RuntimeError, 'unfinished release'):
            self.tx.execute(self.stage, self.new)


if __name__ == '__main__':
    unittest.main()

"""Guard release identity across actual merge, squash and rebase histories."""
from pathlib import Path
import subprocess
import json
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import build_version as version
import package_release


class VersionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git('init', '-b', 'main')
        self.git('config', 'user.name', 'Version test')
        self.git('config', 'user.email', 'test@example.invalid')
        self.base = self.commit('Legacy baseline')
        self.baseline = patch.object(version, 'BASE_REVISION', self.base)
        self.baseline.start()
        self.addCleanup(self.baseline.stop)

    def git(self, *args):
        return version.git(*args, root=self.root)

    def commit(self, message):
        self.git('commit', '--allow-empty', '-m', message)
        return self.git('rev-parse', 'HEAD')

    def test_semantic_headers_and_breaking_markers(self):
        for text, level in [('fix: repair', 0), ('docs: clarify', 0),
                            ('feat(login): version', 1), ('refactor!: remove API', 2),
                            ('fix: change\n\nBREAKING CHANGE: incompatible', 2),
                            ('fix: change\n\nBREAKING-CHANGE: incompatible', 2),
                            ('fix: quote\n\n> BREAKING CHANGE: example', 0)]:
            with self.subTest(text=text):
                self.assertEqual(version.message_level(text), level)
        with self.assertRaisesRegex(ValueError, 'Conventional Commit'):
            version.message_level('Added a feature')

    def test_merge_highest_bump_is_counted_once_and_reruns_are_stable(self):
        self.git('checkout', '-b', 'feature')
        self.commit('fix!: incompatible save')
        self.commit('feat: new menu')
        self.git('checkout', 'main')
        self.git('merge', '--no-ff', 'feature', '-m',
                 'Merge pull request #1 from feature\n\nfeat: menu')
        merged = self.git('rev-parse', 'HEAD')
        self.assertEqual(version.main_version(merged, self.root), (1, 0, 0))
        self.assertEqual(version.main_version(merged, self.root), (1, 0, 0))
        head = self.commit('fix: follow-up')
        self.assertEqual(version.main_version(head, self.root), (1, 0, 1))

    def test_squash_and_rebased_changes_increment_in_order(self):
        self.commit('feat: squash feature (#4)')
        self.commit('fix: rebased fix')
        head = self.commit('chore: maintenance')
        self.assertEqual(version.main_version(head, self.root), (0, 2, 2))

    def test_nested_merge_retains_breaking_title_and_invalid_merge_fails(self):
        self.git('checkout', '-b', 'outer')
        self.git('checkout', '-b', 'child')
        self.commit('fix: update schema')
        self.git('checkout', 'outer')
        self.git('merge', '--no-ff', 'child', '-m',
                 'Merge pull request #2 from child\n\nfeat!: replace save schema')
        outer = self.git('rev-parse', 'HEAD')
        with self.assertRaisesRegex(ValueError, 'highest commit bump'):
            version.preview_version(self.base, outer, 'fix: combine work', '3', self.root)
        self.git('checkout', 'main')
        self.git('merge', '--no-ff', 'outer', '-m', 'fix: combine work')
        self.assertEqual(version.main_version('HEAD', self.root), (1, 0, 0))
        self.git('checkout', '-b', 'next')
        self.commit('fix: follow-up')
        self.git('checkout', 'main')
        self.git('merge', '--no-ff', 'next', '-m', 'Unclassified authored merge')
        with self.assertRaisesRegex(ValueError, 'Conventional Commit'):
            version.main_version('HEAD', self.root)

    def test_behind_main_preview_checks_real_head_and_title(self):
        self.git('checkout', '-b', 'feature')
        head = self.commit('feat: menu')
        self.git('checkout', 'main')
        base = self.commit('fix: another change')
        label = version.preview_version(base, head, 'feat: menu', '4', self.root)
        self.assertEqual(label, '0.2.0-pr.4+' + head[:12])
        with self.assertRaises(ValueError):
            version.preview_version(base, head, 'not semantic', '4', self.root)
        with self.assertRaisesRegex(ValueError, 'highest commit bump'):
            version.preview_version(base, head, 'fix: misleading title', '4', self.root)

    def test_invalid_new_commit_and_missing_baseline_fail_closed(self):
        head = self.commit('unclassified change')
        with self.assertRaises(ValueError):
            version.preview_version(self.base, head, 'fix: classified title', '4', self.root)
        with self.assertRaises(subprocess.CalledProcessError):
            version.main_version(head, self.root, baseline='f' * 40)

    def test_package_retains_exact_semantic_identity(self):
        binary = self.root / 'savsoul'
        binary.write_bytes(b'executable')
        output = self.root / 'release.tar.gz'
        label = '0.2.0-pr.4+' + self.base[:12]
        package_release.package(self.root, binary, output, self.base, label)
        with tarfile.open(output) as archive:
            manifest = json.load(archive.extractfile('manifest.json'))
        self.assertEqual((manifest['version'], manifest['revision']), (label, self.base))
        for invalid in ('01.2.3', '1.2.3-.', '1.2.3-01', '1.2.3+bad..id'):
            with self.subTest(version=invalid), self.assertRaises(ValueError):
                package_release.package(self.root, binary, output, self.base, invalid)


if __name__ == '__main__':
    unittest.main()

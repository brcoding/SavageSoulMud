"""Fresh-volume imports must not overwrite state or follow archive links."""
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

from import_snapshot import import_snapshot


class SnapshotTests(unittest.TestCase):
    def archive(self, name, kind=tarfile.REGTYPE):
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode='w:gz') as archive:
            entry = tarfile.TarInfo(name)
            entry.type, entry.linkname = kind, '../../outside'
            entry.size, entry.mode = (4 if kind == tarfile.REGTYPE else 0), 0o600
            archive.addfile(entry, io.BytesIO(b'data') if kind == tarfile.REGTYPE else None)
        buffer.seek(0)
        return buffer

    def test_valid_snapshot_retains_hash_and_refuses_second_import(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            import_snapshot(self.archive('grim/data/example.json'), root)
            self.assertEqual((root / 'grim/data/example.json').read_bytes(), b'data')
            self.assertTrue((root / 'state/kubernetes/warm-snapshot.json').exists())
            with self.assertRaisesRegex(ValueError, 'not fresh'):
                import_snapshot(self.archive('grim/another'), root)

    def test_links_and_path_escapes_are_rejected(self):
        for name, kind in [('grim/../../outside', tarfile.REGTYPE),
                           ('/grim/absolute', tarfile.REGTYPE),
                           ('grim/link', tarfile.SYMTYPE), ('grim/link', tarfile.LNKTYPE),
                           ('elsewhere/file', tarfile.REGTYPE)]:
            with self.subTest(name=name, kind=kind), tempfile.TemporaryDirectory() as folder:
                with self.assertRaises(ValueError):
                    import_snapshot(self.archive(name, kind), Path(folder))


if __name__ == '__main__':
    unittest.main()

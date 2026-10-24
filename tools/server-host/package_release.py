"""Build a release from tracked public sources, never a working data directory."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import tarfile

NUMBER = r'(?:0|[1-9][0-9]*)'
PRE_ID = r'(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)'
BUILD_ID = r'[0-9A-Za-z-]+'
SEMVER = (rf'{NUMBER}\.{NUMBER}\.{NUMBER}(?:-{PRE_ID}(?:\.{PRE_ID})*)?'
          rf'(?:\+{BUILD_ID}(?:\.{BUILD_ID})*)?')


def authored(name):
    return name.startswith(('assets/', 'data/help/', 'data/areas/', 'data/socials/',
                            'data/quests/', 'data/savage-soul/'))


def package(root, binary, output, revision, version='0.1.0-dev'):
    if not re.fullmatch(r'[0-9a-f]{40}', revision):
        raise ValueError('Expected full Git revision')
    tracked = subprocess.check_output(['git', 'ls-files', '-z'], cwd=root).decode().split('\0')
    files = {'bin/savsoul': binary}
    for name in tracked:
        if name and (name.startswith('tools/') or authored(name)):
            path = root / name
            if path.is_symlink() or not path.is_file():
                raise ValueError('Release sources must be regular files: ' + name)
            files[name] = path
    for path in (root / 'website/dist').rglob('*'):
        if path.is_file():
            if path.is_symlink():
                raise ValueError('Symlink in website build')
            files['public/' + path.relative_to(root / 'website/dist').as_posix()] = path
    if not re.fullmatch(SEMVER, version):
        raise ValueError('Expected a semantic game version')
    manifest = {'revision': revision, 'version': version, 'files': {}}
    with tarfile.open(output, 'w:gz') as archive:
        for name, path in sorted(files.items()):
            data = path.read_bytes()
            manifest['files'][name] = hashlib.sha256(data).hexdigest()
            entry = tarfile.TarInfo(name)
            entry.size = len(data)
            entry.mode = 0o700 if name == 'bin/savsoul' else 0o600
            archive.addfile(entry, io.BytesIO(data))
        data = json.dumps(manifest, sort_keys=True).encode()
        entry = tarfile.TarInfo('manifest.json')
        entry.size = len(data)
        entry.mode = 0o600
        archive.addfile(entry, io.BytesIO(data))
    print('Packaged', len(files), 'files for', revision)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--version', required=True)
    args = parser.parse_args()
    package(args.root, args.binary, args.output, args.revision, args.version)

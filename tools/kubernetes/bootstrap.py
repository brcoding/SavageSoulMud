"""Rebuild the candidate runtime after a successful private snapshot import."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

HOME = Path('/home/savsoul')
STATE = HOME / 'state/kubernetes'


def main():
    os.umask(0o077)
    if not (STATE / 'warm-snapshot.json').is_file():
        raise RuntimeError('Import a verified warm snapshot before bootstrapping')
    if (STATE / 'bootstrap-complete').exists():
        raise RuntimeError('Runtime already prepared; do not overwrite candidate state')
    original = STATE / 'original-network-config'
    original.mkdir(mode=0o700, exist_ok=True)
    for name in ('intermud', 'grapevine', 'zebedee'):
        path = HOME / ('grim/data/' + name + '.json')
        if path.exists():
            if not (original / path.name).exists():
                shutil.copy2(path, original / path.name)
            config = json.loads(path.read_text())
            config['enabled'] = False
            path.write_text(json.dumps(config, indent=2) + '\n')
            path.chmod(0o600)
    environment = HOME / 'runtime/venv'
    subprocess.run([sys.executable, '-m', 'venv', str(environment)], check=True, timeout=300)
    requirements = ['tools/wizard-editor/requirements.txt', 'tools/web-terminal/requirements.txt',
                    'tools/intermud/requirements.lock']
    args = [str(environment / 'bin/python'), '-m', 'pip', 'install',
            '--disable-pip-version-check', '--no-cache-dir', '--only-binary=:all:']
    for path in requirements:
        args.extend(['-r', str(HOME / 'grim' / path)])
    subprocess.run(args, check=True, timeout=600)
    subprocess.run([str(environment / 'bin/python'),
                    str(Path(__file__).with_name('prepare_gateway.py'))], check=True)
    # Existing editor copyover commands refer to this stable Python path.
    pointer = HOME / 'runtime/python/bin'
    pointer.mkdir(parents=True, mode=0o700, exist_ok=True)
    target = pointer / 'python3'
    if target.is_symlink():
        if target.resolve() != Path(sys.executable).resolve():
            raise RuntimeError('Unexpected existing Python pointer')
    elif target.exists():
        raise RuntimeError('Refusing to replace existing runtime executable')
    else:
        target.symlink_to(sys.executable)
    (HOME / 'public/mudlet').mkdir(mode=0o700, exist_ok=True)
    subprocess.run([sys.executable, str(Path(__file__).with_name('prepare_frontend.py'))], check=True)
    (STATE / 'bootstrap-complete').touch(mode=0o600)
    print('Candidate runtime prepared. External network adapters remain disabled.')


if __name__ == '__main__':
    main()

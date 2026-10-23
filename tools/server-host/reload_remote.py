"""Refresh the public mapper export, then request copyover of the verified MUD."""
from pathlib import Path
import os
import signal
import subprocess
import sys


def main():
    root = Path(__file__).resolve().parents[2]
    pid = int(Path('/home/savsoul/state/runner/mud.pid').read_text())
    executable = os.readlink(f'/proc/{pid}/exe').removesuffix(' (deleted)')
    if executable != str(root / 'bin/savsoul'):
        raise RuntimeError('Game process identity mismatch')
    subprocess.run([
        sys.executable, str(root / 'tools/mudlet/export_map.py'),
        '--root', str(root), '--output', '/var/www/lpmud/mudlet/savsoul.xml',
    ], check=True)
    os.kill(pid, signal.SIGUSR2)
    print('Copyover requested.')


if __name__ == '__main__':
    main()

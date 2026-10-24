#!/usr/bin/python3
"""Forced SSH command for the dedicated deployment account (Python 3.4)."""
import os
import re
import subprocess
import sys


def main():
    command = os.environ.get('SSH_ORIGINAL_COMMAND', '')
    if command != 'check' and not re.match(r'^(deploy|validate) [0-9a-f]{40}\Z', command):
        raise ValueError('Only check, validate <revision>, or deploy <revision> is allowed')
    # No shell interpolation or preserved caller environment. Sudo permits only
    # this root-owned wrapper, whose dispatcher independently checks the command.
    return subprocess.call(['/usr/bin/sudo', '-n', '/usr/local/sbin/savsoul-deploy', command])


if __name__ == '__main__':
    sys.exit(main())

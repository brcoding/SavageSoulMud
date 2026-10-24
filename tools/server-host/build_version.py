"""Deterministic game versions and Conventional Commit checks; no Git writes."""
import argparse
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[2]
# Immutable starting point: history before versioning was introduced is legacy.
BASE_REVISION = 'd7dc3ffddb37b49a09ef2d49961c3b946e17c2ec'
BASE_VERSION = (0, 1, 0)
HEADER = re.compile(r'^(feat|fix|build|chore|ci|docs|style|refactor|perf|test|revert)'
                    r'(\([^()\r\n]+\))?(!)?: [^\s].*$')


def git(*args, root=ROOT):
    """Run Git with separate arguments, failing closed on missing history."""
    return subprocess.check_output(['git', *args], cwd=root, text=True).strip()


def message_level(message):
    """Return patch=0, minor=1, major=2, rejecting non-conventional headers."""
    header = message.splitlines()[0] if message else ''
    match = HEADER.fullmatch(header)
    if not match:
        raise ValueError('Use a Conventional Commit, e.g. feat(login): show version; got: ' + header)
    if match[3] or re.search(r'^BREAKING[ -]CHANGE: \S', message, re.MULTILINE):
        return 2
    return 1 if match[1] == 'feat' else 0


def bump(version, level):
    """Apply one SemVer bump, clearing subordinate components."""
    major, minor, patch = version
    return ((major + 1, 0, 0) if level == 2 else
            (major, minor + 1, 0) if level == 1 else (major, minor, patch + 1))


def introduced_level(parent, revision, root=ROOT):
    """Include nested merge title intent, excluding already reachable commits."""
    commits = git('rev-list', parent + '..' + revision, root=root).splitlines()
    return max((commit_intent(sha, root) for sha in commits), default=0)


def commit_intent(revision, root=ROOT):
    """Read semantic intent; recognize only specific Git-generated wrappers."""
    parents = git('show', '-s', '--format=%P', revision, root=root).split()
    message = git('show', '-s', '--format=%B', revision, root=root)
    if len(parents) >= 2:
        # GitHub's wrapper is generated, but the following PR title is authored.
        if re.fullmatch(r'Merge pull request #[1-9][0-9]* from \S+', message.splitlines()[0]):
            return message_level(message.partition('\n\n')[2])
        if re.fullmatch(r"Merge (?:remote-tracking )?(?:branch|tag) '[^'\r\n]+'(?: into [^\r\n]+)?",
                        message.splitlines()[0]):
            return 2 if re.search(r'^BREAKING[ -]CHANGE: \S', message, re.MULTILINE) else 0
    return message_level(message)


def commit_level(revision, root=ROOT):
    """Use merge title and introduced changes, so a title cannot hide a break."""
    parents = git('show', '-s', '--format=%P', revision, root=root).split()
    level = commit_intent(revision, root)
    return max(level, introduced_level(parents[0], revision, root)) if len(parents) >= 2 else level


def main_version(revision, root=ROOT, baseline=None):
    """Replay first-parent changes: repeat/queued builds need no shared counter."""
    baseline = baseline or BASE_REVISION
    git('merge-base', '--is-ancestor', baseline, revision, root=root)
    commits = git('rev-list', '--reverse', '--first-parent',
                  baseline + '..' + revision, root=root).splitlines()
    version = BASE_VERSION
    for sha in commits:
        version = bump(version, commit_level(sha, root))
    return version


def preview_version(base, head, title, number, root=ROOT):
    """Require semantic PR title/commits and predict its highest production bump."""
    if not re.fullmatch(r'[1-9][0-9]*', number):
        raise ValueError('Expected a positive PR number')
    git('merge-base', '--is-ancestor', BASE_REVISION, head, root=root)
    level = message_level(title)
    if level < introduced_level(base, head, root):
        raise ValueError('PR title must carry the highest commit bump: feat for minor, ! for major')
    version = '.'.join(map(str, bump(main_version(base, root), level)))
    revision = git('rev-parse', '--verify', head + '^{commit}', root=root)
    return version + '-pr.' + number + '+' + revision[:12]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--head', default='HEAD')
    parser.add_argument('--base')
    parser.add_argument('--pr')
    parser.add_argument('--title', default=os.environ.get('PR_TITLE', ''))
    args = parser.parse_args()
    revision = git('rev-parse', '--verify', args.head + '^{commit}')
    if args.pr:
        if not args.base:
            parser.error('--pr requires --base')
        version = preview_version(args.base, revision, args.title, args.pr)
    else:
        version = '.'.join(map(str, main_version(revision)))
    print('Game version:', version, 'revision:', revision)
    if os.environ.get('GITHUB_OUTPUT'):
        with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
            output.write('version=' + version + '\nrevision=' + revision + '\n')


if __name__ == '__main__':
    main()

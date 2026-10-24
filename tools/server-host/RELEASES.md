# Production releases

Merging to `main` starts **CI and production release**. Build, lint, workspace tests,
release preservation tests, website build and static musl packaging must all pass
before the `production` deployment job runs. Existing failing quality checks block
releases; they are not bypassed. PR jobs never receive production secrets.

After checks pass on merge, the release **attempts copyover first** by sending
`SIGUSR2` to the verified game PID. Successful releases do not restart the game
service, so supported live player connections can survive the release. The job
log records the request and the successor PID.

The deployment environment permits only `main`. Releases are serialized without
canceling an active deployment; the job rejects a revision superseded by a newer
`main`. Actions also provides a manual workflow run on `main` for a revision that
has not already been attempted. Deployments are recorded in GitHub's production
environment history. This does not create public GitHub Release tags.

## Server setup

Game versions follow the [game version policy](#game-version-policy) below.

The fixed entry point is `/usr/local/lib/savsoul-release/release_entry.py`, run
by the host's Python 3.4. Its root-owned companion `receive_release.py` runs using
the existing private Python 3.12 only after dropping to the `savsoul` UID/GID.
They are provisioned separately over administrator SSH and cannot update themselves
from a release payload. Changes to these two files require an administrator to
re-provision them. Copies under the game's `tools/` are reference sources only.
The same root ownership applies to `deploy_ssh.py` and the wrapper installed as
`/usr/local/sbin/savsoul-deploy`. The templates `deploy-sshd.conf` and
`deploy-sudoers.conf` record the account restrictions; validate SSH changes with
`sshd -t` and sudo changes with `visudo -cf` before activating them.

SSH uses the dedicated `savsoul-deploy` account, with a locked password, no
administrative groups and a root-owned home. Its root-owned public key file is
`/etc/ssh/savsoul-deploy.keys`. The key and an sshd `Match User` block force
`deploy_ssh.py`, disable forwarding/PTY access and permit public-key sign-in only.
Shell commands, SFTP/SCP, arbitrary test scripts and reading private game files
through SSH are unavailable. CI runs unit/integration tests on GitHub runners;
server-side testing is limited to the release `check` and `validate` commands.

Production currently uses OpenSSH 6.6.1, which predates Unix-domain socket
forwarding. It rejects `AllowStreamLocalForwarding` as an unknown directive.
When upgrading SSH to 6.7 or newer, enable `AllowStreamLocalForwarding no`
in the account's Match block before reloading, validate with `sshd -t`, and
check the effective account policy with `sshd -T -C
user=savsoul-deploy,host=localhost,addr=127.0.0.1`. See the
[OpenSSH 6.7 release notes](https://www.openssh.org/txt/release-6.7).

Its only sudo permission is the root-owned `/usr/local/sbin/savsoul-deploy`
wrapper. That invokes the fixed dispatcher, which independently validates one
`check`, `validate FULL_GIT_SHA`, or `deploy FULL_GIT_SHA` command. The account
cannot use general root commands or change these helpers. The dispatcher has
only the existing MUD-stack operations described below, including publishing its
generated website and managing the fixed game/editor/bridge services. Payload
parsing and application code still run as the separate `savsoul` game UID.

The old root-authorized CI key is removed. Neither the root password nor an
unrestricted administrator key is shared with GitHub. The encrypted production
environment secrets are:

- `SAVSOUL_DEPLOY_KEY`: that dedicated private key.
- `SAVSOUL_KNOWN_HOSTS`: the verified server public host key for `173.255.210.234`.

Provision `/home/savsoul/state/releases/current.json` as a private savsoul-owned
manifest containing `revision` and `files` (repository-relative paths mapped to
SHA256 hashes). For the initial baseline use the reviewed repository's authored
assets/data hashes: differing live content is conservatively preserved. Do not
take a new live-content baseline after editor changes; doing so would make those
changes eligible to be overwritten. Subsequent releases maintain this manifest.
Also create root-owned `/var/lib/savsoul-release` mode 700. No service is restarted
merely by installing this configuration or running the `check`/`validate` commands.

## What changes

The artifact includes the static `bin/savsoul`, tracked `tools/`, authored
assets/data and generated static website. Runtime accounts, characters, bans,
network configurations, mapper IDs, signing keys, TLS keys, model weights and
external gateway/runtime installations are excluded. Production Python requirement
files must already match the release; dependency mismatches and malformed archives
are rejected during receiver validation in `deploy`, before live files change.
Such rejection resumes supporting services without restarting the game. Dependency
changes require separate wheel provisioning for this old host.

Authored assets, help, rooms, rules and quests update only if their live hash still
matches the previous authored baseline (or already matches the new release).
Changed live files are preserved and listed in the job log. New authored files
are seeded; deleted authored files are retained. Code is replaced, including removal
of previously managed tools deleted in a newer revision. This preserves existing
world edits but does not merge JSON fields: reconcile a preserved file in the
wizard editor when a release also changes it.

The editor pauses while applying files. The receiver validates regular-file paths,
revision and digests, backs up existing files plus private data, atomically replaces
files, refreshes the public map and requests PID-verified copyover. It waits for a
different game PID running the new executable and a responding listener. The
editor and browser terminal bridge restart to load Python changes. TLS, I3 gateway
and trainer model services remain running. Generated HTML/CSS/JS and the explicit
terminal-vendor files publish without replacing the legacy website archive or
certificate validation files. Final checks cover game, editor and web bridge ports.

## Failure and recovery

Failure restores changed code/assets and the generated site, then resumes the
editor/bridge through Upstart and checks listeners again. The game restarts only
if live file changes began or copyover was requested. That recovery fallback can
disconnect players. It restores only files changed by that deployment, keeping
player progress saved during the attempt. Full private data snapshots are retained
for administrator recovery and are never automatically restored over newer saves.

The parent dispatcher also rolls back a receiver that exits abruptly after a live
change, using that attempt's phase markers rather than requiring a successful
receiver return. If rollback fails, recovery stops before restarting services.
Recovery journals for both game files and public pages are written to a temporary
file, flushed and synced, then atomically replaced and their directory synced
before the associated live mutation. An interrupted journal update retains the
previous complete journal, so previously applied files remain recoverable.
This requires the parent dispatcher to remain running: host power loss or killing
the dispatcher itself requires administrator reconciliation of journals and the
previous manifest before services or deployments resume. There is no automatic
power-loss recovery guarantee.

Backups: `/home/savsoul/state/releases/backup-SHA/` and root-owned
`/var/lib/savsoul-release/SHA/`. Keep the last three completed attempts; artifact
downloads expire after seven days. An already attempted revision is rejected
before services stop. To retry, fix the cause and push a new commit. To revert a
successful release, merge a Git revert PR; it follows the same checked pipeline.
If automatic recovery fails, inspect `/var/log/upstart/savsoul-*.log`, the failed
Actions log and these backup directories over administrator SSH before restarting.

## Destination permissions and failed attempts

The `savsoul` UID must be able to create and replace files in managed destination
directories, including `tools/web-terminal`. File ownership alone is insufficient:
atomic replacement creates a temporary sibling in the parent directory. The
receiver checks these permissions before live mutation; the installed baseline
is also checked before stopping the editor. Do not grant the deploy account
additional or general sudo permissions, or make the separately provisioned
`/usr/local/lib/savsoul-release` helpers writable.
Reprovision reviewed helper changes there as root; payload copies do not update them.

A service restart after an exception is recovery, not a successful release.
Check `current.json` and the running executable. Preserve failed journals and
backups. Normally retry with a new merged revision. If an operator must retry the
same revision, first complete its rollback and verify recovery, then archive the
entire failed game backup outside the `backup-*` namespace. If publication began,
also verify public-site rollback and archive `/var/lib/savsoul-release/SHA/`
outside `/var/lib/savsoul-release/`; that directory independently blocks a retry.
Preserve both directories' recovery metadata so the retry creates fresh journals;
never reuse or erase the failed attempt's metadata.

## Game version policy

Use Conventional Commits for every new commit and PR title:

| Message | Bump | Example from 1.2.3 |
|---|---|---|
| `fix: repair the gate` | patch | 1.2.4 |
| `feat: add crafting` | minor | 1.3.0 |
| `feat!: replace save format` or a `BREAKING CHANGE:` footer | major | 2.0.0 |
| `docs:`, `ci:`, `chore:`, `build:`, `test:`, `perf:`, `refactor:`, `style:`, `revert:` | patch | 1.2.4 |

The `version` CI job requires both a semantic PR title and semantic new commit
messages. Git-generated merge wrappers are exempt. The PR title must express at
least the highest bump in its commits, preserving the intent when squash merging.
Edit the title if necessary; title edits rerun validation. Keep the semantic title
when merging or squashing. A nonsemantic main commit blocks packaging/deployment.

`tools/server-host/build_version.py` starts at **0.1.0** at the immutable baseline
`d7dc3ffddb37b49a09ef2d49961c3b946e17c2ec` (PR #3). Earlier history is legacy.
Each first-parent change since that baseline applies one bump. For a merge, use
the highest intent from its title and newly introduced commits, including nested
PR titles; already merged changes are excluded. Rebased commits each get their own bump. Other types
receiving a patch is a project policy so every production revision is distinct.
Do not move the baseline or rewrite main history after adopting this policy.

Builds are deterministic: retries reuse the version, and queued main revisions
calculate distinct numbers without tag races or bot version commits. PR builds
preview the standard merge/squash bump as `0.2.0-pr.4+COMMIT`; rebase merging may
produce multiple bumps. Unstamped local builds show `0.1.0-dev (local)`.
These are game versions; workspace Cargo crate versions are independent.

CI embeds `SAVSOUL_VERSION`/`SAVSOUL_REVISION` in the executable, verifies its
`--version` output, and records the same version/revision in `manifest.json` and
the deployed `current.json`. Login shows a textual, colored version line; all
terminal clients receive it. Copyover loads the successor binary's identity.
The scheme needs complete Git history but no additional GitHub write token or
server privilege. Required branch checks should include `version`.

# Production releases

Merging to `main` starts **CI and production release**. Build, lint, coverage,
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

Backups: `/home/savsoul/state/releases/backup-SHA/` and root-owned
`/var/lib/savsoul-release/SHA/`. Keep the last three completed attempts; artifact
downloads expire after seven days. An already attempted revision is rejected
before services stop. To retry, fix the cause and push a new commit. To revert a
successful release, merge a Git revert PR; it follows the same checked pipeline.
If automatic recovery fails, inspect `/var/log/upstart/savsoul-*.log`, the failed
Actions log and these backup directories over administrator SSH before restarting.

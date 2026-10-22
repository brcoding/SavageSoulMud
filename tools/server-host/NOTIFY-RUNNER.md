# Copyover under Upstart

Upstart tracks one long-lived runner process. `notify_runner.py` launches the MUD as its own session/process group, acts as a Linux child subreaper and follows authenticated `sd_notify MAINPID` messages when the predecessor hands its sockets to a successor. It keeps the editor's private `mud.pid` updated. It does not restart the MUD itself; Upstart's respawn policy handles a crash. Normal game exit returns0; crashes retain a nonzero code. TERM/INT go to the current game PID; SIGUSR2 forwards copyover.

Run the wrapper as the dedicated unprivileged game account, with a private canonical local state directory. Example Upstart job fragment (adjust paths to installed portable Python/runtime):

```text
setuid savsoul
setgid savsoul
chdir /home/savsoul/app/grim
respawn
respawn limit 5 60
normal exit 0
kill timeout 50
exec /home/savsoul/runtime/python/bin/python3 tools/server-host/notify_runner.py --state /home/savsoul/state/runner -- /home/savsoul/app/grim/bin/savsoul
```

Point the editor/copyover helper at `/home/savsoul/state/runner/mud.pid`, the actual game PID. The wrapper's PID belongs to Upstart. Do not use `expect fork` or `expect daemon`: the wrapper remains in the foreground. Adjust stop timeout/grace consistently with the game's normal SIGTERM save/shutdown duration; Upstart kill timeout must exceed the runner's45second deadline plus cleanup.

Requires Linux3.4+ for `PR_SET_CHILD_SUBREAPER`; the host's modern kernel supports it despite old Ubuntu userspace. Python3.12 portable runtime is sufficient; no pip modules, modern systemd, shells, or changes to existing services are needed. Uses libc `prctl`, POSIX `waitid(WNOWAIT)` and credential-bearing Unix datagrams already supported by the platform. Existing supervisor libraries do not follow this application's systemd-style MAINPID socket handoff; this adapter bridges that narrow gap rather than implementing a general service manager.

Security/lifecycle: private0700 directory,0600 socket and atomic0600 PID file; exclusive file lock prevents duplicate wrappers. Socket credentials must match UID and a descendant process; claimed main PID must be a supervised descendant running the expected executable. Datagrams are capped at4096bytes and64per iteration. The runner ignores unknown fields and never logs raw notification payloads. It drains final notifications while exited children are still unreaped, preventing PID-reuse/handoff races. Failed successors are reaped; remaining service-process-group members are terminated on final exit. A copyover executable replacement may change the binary inode, so validation uses the expected canonical executable path, including a predecessor's `(deleted)` inode path.

Focused fixture checks (no actual network/game): `python3 -m unittest discover -s tools/server-host -p test_notify_runner.py`. They exercise successor adoption after predecessor exit0, same-UID unrelated spoof/oversize notification rejection, updated PID file, stop forwarding and crash exit status.

Primary references: [Linux subreaper](https://man7.org/linux/man-pages/man2/PR_SET_CHILD_SUBREAPER.2const.html), [Unix socket credentials](https://man7.org/linux/man-pages/man7/unix.7.html), [sd_notify](https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html).

# Savage Soul remote server

Automatic releases after merge to `main`, preservation rules, GitHub environment
secrets and recovery instructions: [RELEASES.md](RELEASES.md).

Set `GRIM_BIND_HOST=0.0.0.0` in the production game service environment; local
launches default to `127.0.0.1`. Startup logs report the actual listener address.

Host: `savsoul.com` (`173.255.210.234`). Game TCP `4000`, TLS TCP `4001`, Zebedee UDP `4002`.
The game data, including the saved Rawr character, lives in `/home/savsoul/grim`.
The game executable is `/home/savsoul/grim/bin/savsoul`; Cargo's default runnable target is `savsoul`.
The account `savsoul` is locked and has a non-login shell; services run unprivileged.
Existing server applications and OS packages were not changed.

## Wizard editor

The editor binds only `127.0.0.1:4085` on the server. Use an SSH tunnel:

```sh
ssh -N -L 4085:127.0.0.1:4085 YOUR_SSH_USER@173.255.210.234
```

Visit `http://127.0.0.1:4085/` and use the existing MUD wizard login.
The local development editor remains on port4080. Cookies are separate when
using 127.0.0.1 for the server and localhost for the local editor.

## Services

This host uses Upstart. New jobs are `/etc/init/savsoul-{grim,tls,editor,intermud}.conf`.
Inspect with `status savsoul-grim`; use `start`, `stop` or `restart` as administrator.
Logs: `/var/log/upstart/savsoul-*.log`.

Runtime: statically linked x86_64-musl game binary, private CPython3.12 runtime,
and private wheel-only Python environment in `/home/savsoul/runtime`.
Portable Python and supporting libraries avoid upgrading the Ubuntu14 host.

The stable `notify_runner.py` follows the game's real copyover process. Its
private PID file is `/home/savsoul/state/runner/mud.pid`. The editor invokes
the tracked `tools/server-host/reload_remote.py` as savsoul to validate and signal that PID.
Deploy that file with the other server-host tools; both `/api/apply` and the
administrator command below invoke the same helper. It refreshes the public
Mudlet map before signalling the verified game executable. Do not signal
the Upstart runner PID with kill9 or start a duplicate game manually.

For an administrator-requested copyover:

```sh
su -s /bin/sh -c '/home/savsoul/runtime/venv/bin/python /home/savsoul/grim/tools/server-host/reload_remote.py' savsoul
```

TLS uses a publicly trusted PositiveSSL certificate covering `savsoul.com` and
`www.savsoul.com`, installed with its CA chain under `/home/savsoul/state/tls`.
Connect by hostname. The certificate expires May5,2027; reissue through Namecheap
before expiry and install the updated full chain with its matching private key.
The CSR and private issuance key are in the private `positivessl` subdirectory;
previous certificate copies are retained in a private backup directory.
The trusted backend
is loopback4003. Editor4085, I3 APIs4081/4082, trainer4091 stay loopback only.

## Networks

The I3 identity is `The Savage Soul Dev`; its existing private router/API state
was migrated over SSH. The local development I3 gateway is stopped and disabled at startup to prevent
duplicate identities. Do not run both gateways with these credentials.
Private state: `/home/savsoul/.local/state/savsoul-intermud`.

Zebedee runs directly on UDP4002; it no longer needs the Windows relay.
All I3, Zebedee and Grapevine commands remain wizard-only.

Grapevine code is installed but awaits game registration. In an interactive
administrator SSH terminal, save credentials without exposing the secret:

```sh
su -s /bin/sh -c 'HOME=/home/savsoul /home/savsoul/runtime/python/bin/python3 /home/savsoul/grim/tools/grapevine/configure.py' savsoul
```

Then in the MUD: `grapevine reload`, `gv status`, `gv listen gossip`.
Do not put keys in authored JSON, chat, shell arguments or version control.

Optional Mudlet audio URLs are unconfigured remotely because the wizard editor
is private. Configure a public HTTPS media URL once a domain/media host exists.

The private CPU-only Tully translator runs as `savsoul-trainer` on loopback4091,
using Qwen3-0.6B Q8_0 and the pinned llama.cpp b11723 source. It uses two threads
at reduced priority. The statically linked runtime enables AVX2, which was
verified on this server's AMD EPYC7713 CPU. No GPU is used. Its model and binary
live in `/home/savsoul/runtime/trainer`. Built-in answers and XP rules remain
authoritative; the model only selects a structured topic.

The server now holds an independent data copy. Local development edits and
character progress do not automatically synchronize. Preserve server data and
private state when deploying future source/binary updates. Back up both before
changes; do not replace data with a stale local snapshot.

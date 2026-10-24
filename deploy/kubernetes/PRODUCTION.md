# Production cutover — 2026-10-24

The owner explicitly requested the immediate IP switch instead of waiting for
the former 24-hour DNS cache lifetime. Both savsoul.com and www A records now
point to 69.164.205.149 with TTL 300. All five authoritative servers were queried
directly with recursion disabled. Linode nameservers, Google MX, wildcard and
other hosts were retained. The complete original zone was downloaded privately.

Kubernetes character progress was explicitly chosen for the only differing
character save. Final comparison after stopping source writers found matching
accounts/authored content and no additional character conflicts. No candidate
character was overwritten. Pre-cutover backups remain privately on the source
and retained volume. Final I3 router files were streamed over SSH/TLS and hash
verified; no private archive or credentials were committed or staged locally.

The old savsoul-grim/editor/intermud/trainer/tls/web-terminal jobs are stopped
with Upstart `manual` overrides. I3 was activated on Kubernetes only after the
old gateway stopped. Gateway readiness returns 200; the game adapter is enabled.
The retired VM deployment account's dedicated SSH authorization file is empty,
with its original public authorization retained privately for rollback. Effective
SSHD configuration disables password login and uses that dedicated file. No
deployment was in flight when authorization was revoked; operator access is
unchanged. This prevents an older CI run from restarting the retired world.
Optional Grapevine/Zebedee remain disabled. The additive production I3 policy
allows only 204.209.44.3 TCP 8080; ordinary candidate/DNS isolation remains intact.

## Cached DNS clients

The old VM runs the bounded `retired_relay.py` under the savsoul account:

- Public 4000/4001 pass plain/TLS bytes to the new public MUD endpoints.
- Loopback 4085 forwards the existing Apache wizard backend over verified TLS
  to the new editor 4081, preserving its Host/Origin/CSRF checks.
- Loopback 4086 forwards the existing browser-terminal backend over verified TLS
  to 443, restoring the websocket path stripped by the old Apache proxy.

It creates no local world, account state or I3 identity. During forwarding,
Kubernetes sees the old VM's address for those cached clients. After DNS caches
converge and old endpoint use is no longer needed, stop/disable the relay and
remove its job. Keep the old game jobs disabled. Other sites/databases on the
VM are unchanged; follow LEGACY-MIGRATION.md separately.

Verified through both the new and old IP: plain/TLS login greetings, trusted TLS
certificate, websocket 101, same-origin wizard authentication guard and rejection
of a foreign Origin. The application builds match 0.3.1 at cutover. Kubernetes is
the only writable world. Future main releases target Kubernetes only.

## Rollback

Keep the source backup and original DNS zone. New Kubernetes player/editor
writes make a DNS-only rollback unsafe: freeze the new writers, transfer their
current authoritative state back under an explicit merge policy, stop the new
I3 gateway, then resume the old world and restore DNS. Stop the relay before
binding old game/backend ports. Never start two I3 gateways with this identity.

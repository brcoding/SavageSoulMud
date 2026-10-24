# Staging record — 2026-10-24

The candidate is on LKE in namespace `savsoul`, StatefulSet `savsoul`, retained
PVC `savsoul-home` (20Gi). Its current standard NodeBalancer IP is
`69.164.205.149`. Production DNS and the Ubuntu VM remain unchanged.

The approved private snapshot was streamed directly over SSH/TLS with no local
private archive: 828 files, 770338359 uncompressed bytes. The private volume keeps
per-file SHA256 hashes. The compressed stream was 664788539 bytes, SHA256
`1307927e8ee693c44bcc321b75e1ba11762a93962b806823df09b4cc447d0b73`.
This was a live warm snapshot; final paused-writer sync remains required.

Verified on the candidate:

- All six containers running with nonroot identities on modern images:
  Python services use Debian13 and UID1009/GID1011; Alpine Nginx uses UID1012.
- Website, wizard landing page and public Mudlet map return200 over validated
  savsoul.com TLS using an IP override; DNS was not changed.
- Plain MUD4000 and TLS MUD4001 present the Savage Soul login greeting.
- Browser websocket endpoint upgrades with101. Foreign wizard request Origin
  receives403; same-origin request with cookie/CSRF reaches authentication401
  for empty credentials. No user password was needed for these checks.
- Direct public NodePort access is denied. Live Internet egress test times out;
  all three game adapters are disabled, activation marker absent, no I3 gateway
  process running.
- CPU trainer health and a bounded inference request return200. Inference used
  767MiB at the initial768MiB cap; the final manifest increases its cap to896MiB
  and request to768MiB. Sum of container memory limits is1536MiB.
- Editor copyover helper regenerated944 public rooms in12 areas. A disposable
  invisible tester authenticated, entered the world, survived copyover on the
  same socket and received a fresh room response. Its temporary account and
  character records were removed. Pre-authentication sockets are intentionally
  closed by the existing copyover flow. No production connection was touched.
- Fresh-volume importer tests reject traversal/link archives and occupied roots.
- Offline I3 configuration loads with the preserved private credentials and
  router password. Inherited state/log paths have been rebased from the previous
  local username to the retained private home, with the original YAML backed up.

The initial TCP readiness and NodeBalancer probes opened game sessions and
triggered wiznet reconnect-flood warnings. Readiness now inspects the verified
process and listening socket without connecting. Nginx terminates private
NodeBalancer health traffic itself instead of forwarding it into the game.
The final live check found zero probe connections/flood warnings over60 seconds.

CodeRabbit's TLS-isolation finding was fixed with a distinct frontend UID and a
namespace TLS Secret mounted only into Nginx. The approved key was streamed from
the retained volume directly into the Kubernetes API; no local key file or
private manifest was saved. A negative check from the editor could not open the
key through either Nginx process's `/proc` root. Nginx UID1012 can read its own
key and passes its configuration check. The public terminal returns the intended
nosniff/referrer/CSP/cache headers, and public TLS remains valid.

Not yet cut over: final live-state synchronization, authenticated wizard editing
with a user's login, actual game login using existing player credentials, new
Intermud cache/identity verification after old adapter shutdown, Namecheap DNS
records, scoped Kubernetes CI release identity, off-cluster backup scheduling.
Do not use cluster-admin credentials in CI.

UDP4002 is not exposed: the provider's UDP support is a Premium NodeBalancer
limited beta, so the standard candidate exposes TCP only. The owner confirmed
I3 is sufficient; Zebedee stays disabled and does not block cutover.
The design review caught this limitation before provisioning; no premium service
was purchased. The design review also required removing the seed pod before
StatefulSet scheduling; the completed maintenance pod has been removed.

See `README.md` for the coordinated cutover/rollback sequence and
`LEGACY-MIGRATION.md` for the deferred websites and databases.

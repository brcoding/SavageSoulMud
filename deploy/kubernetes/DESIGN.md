# Main stack on LKE, before DNS cutover

User authorization: deploy main Savage Soul stack and savsoul.com to supplied
LKE cluster; leave domain cutover for after verification. Plan other old sites
and databases for later. Use existing Downloads/MudCluster-kubeconfig.yaml;
never copy its credentials to the repository. Auto-review rejected a workspace
credential file; none was written. CLI uses the user-supplied file directly.

Cluster: Kubernetes 1.36.5; two ready amd64 Debian13 nodes, 1CPU/~1.8Gi allocatable
each; no user workloads or ingress; Linode CSI retain storage class. Source UID
1009/GID1011. Current main stack about800Mi disk including Qwen model. Main MUD
data28Mi, private state110Mi, model/runtime629Mi. Existing main stays live.

Constraints checked: repo ARCHITECTURE binary owns application composition;
CONTEXT Connection transport state; AGENTS least privilege, preserve private state,
copyover, no duplicate Intermud identity, authoritative live authored data. Existing
SERVER.md/NOTIFY-RUNNER.md: Linux runner follows copyover PID and verified executable;
reload helper writes /var/www/lpmud map. No engine architecture/ADR change. Hosting
manifests are infrastructure; game rules and scene routing unchanged.

Decision: dedicated savsoul namespace and retained 20Gi RWO PVC. First stage is a
seed/maintenance pod using official Python3.12 Debian13 image; stream a warm source
snapshot from administrator SSH directly to kubectl stdin, with safe tar extraction
and no private archive or kubeconfig persisted locally. Seed only fresh claims;
never overwrite an existing migration's state without explicit reconciliation.
Preserve game data/private state and static web content. Rebuild isolated Python
environment from pinned requirements, retain gateway source/model/binary. Snapshot
is for candidate testing; final writer-paused synchronization is required at cutover.

Run one StatefulSet Pod (OnDelete) with common UID/GID, shared process namespace and
retained home/PVC: existing notify runner/game, wizard editor, web bridge, private
I3 gateway, CPU model and Nginx front end. All APIs stay loopback in pod. Readonly
root filesystems, dropped capabilities, no Kubernetes token mounted, bounded resource
requests/limits. Use official images, shared writable PVC for runtime/app state;
versioned images/config manifests contain no private state. Mount website directory
at /var/www/lpmud so existing map refresh path works.

A single LKE NodeBalancer Service publishes site80/443, game4000/4001, editor4080/4081
and Zebedee UDP4002. Provider official CCM docs support mixedUDP and per-port config.
Use PROXYv1 on TCP ports; Nginx accepts it, restores source IP from trusted LB CIDR,
and forwards game bytes via the existing loopback trusted ingress4003 with PROXY.
Nginx terminates MUD/site/editor TLS using the existing PositiveSSL chain/key.
HTTP web/editor sanitize forwarded headers and preserve strict Host/Origin rules.
UDP receives no PROXY framing. Application readiness is independent of LB checks.

Candidate NetworkPolicy denies Internet egress (including old I3 router), allowing
only DNS; private loopback sidecars work. No duplicate network identity may activate.
Enable external Intermud only at coordinated cutover after stopping old adapters.
Ingress permits only the public frontend ports and health checks from provider
LB/private node paths. No API/editor backend ports are exposed.

Tasks: namespace/PVC/seed; transfer and verify data hashes/ownership; install runtime;
generate secret-free manifest/config; deploy and verify internal/public site/game/TLS/
editor/websocket/trainer/map and copyover; document LB IP and exact DNS changes without
changing DNS. Plan final freeze/sync, I3 activation and scoped Kubernetes CI deployment
identity; keep current VM release path until cutover. Write separate legacy-site plan.
If UDP provider feature is unavailable, report concrete controller error before any
DNS move and provide a supported UDP exposure option. No quiet protocol regression.

Implementation decisions after review: Akamai's current primary docs restrict
UDP to Premium NodeBalancers with account-specific beta access. The renderer now
explicitly uses a common TCP-only NodeBalancer. The owner confirmed I3 suffices;
Zebedee stays disabled and is not required for this cutover. No Premium resource
was purchased. Remove the completed seed pod before scheduling the StatefulSet to
avoid RWO attachment conflicts. Measured trainer inference reached767MiB, so its
limit is896MiB/request768MiB; total container limits1536MiB fit one1.8Gi node.

Review verdict: adversarial reviewer found the UDP restriction and RWO scheduling
sequence; both addressed. Live verification proves public TLS/site/map/game/
websocket, Origin guard, blocked direct NodePort, denied Internet egress, local
DNS/model health/inference and authenticated hidden-player copyover. No game-rule
or architecture changes. No private data or credentials in manifests or images.
Production remains authoritative; final sync and single-identity I3 activation
are cutover operations, not candidate startup operations.

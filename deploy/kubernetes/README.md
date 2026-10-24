# Savage Soul on Kubernetes

This deployment stages the main stack before DNS cutover. The existing VM remains
authoritative until all writers are paused and a final synchronized snapshot is
verified. A warm snapshot is suitable for testing, not for replacing live saves.

The stack uses a retained 20Gi volume, one StatefulSet pod, official digest-pinned
Python 3.12/Debian 13 and Nginx images, and a Linode NodeBalancer. The pod includes
the game/notify runner, wizard editor, browser terminal, I3 gateway, CPU trainer,
and TLS/static frontend. It fits a single node; replica count must remain one.
`OnDelete` prevents an ordinary configuration apply from disconnecting players.
All private APIs use pod loopback. Shared process visibility allows the editor's
existing executable-verified copyover helper to signal the game.
Nginx runs as UID 1012, distinct from the game/editor UID 1009. Its TLS Secret is
mounted only there, so shared process visibility does not give the other services
access to its key through `/proc/<pid>/root`. Public assets are group-readable;
private game state keeps its existing owner-only permissions.

## Initial staging

Use the operator's existing kubeconfig directly; never commit credentials or a
private snapshot. Apply `seed.json` once to a fresh namespace/volume. Stream
`tools/kubernetes/snapshot_source.py` from the source VM over authenticated SSH
directly into `import_snapshot.py` running through `kubectl exec -i`. The importer
rejects occupied volumes, traversal, links, devices, and oversized archives. It
retains a private file/hash manifest. A failed import requires inspecting the
in-progress marker and reconciling the candidate volume before retrying.

Run `bootstrap.py` in the seed pod after successful import. It rebuilds pinned
Python dependencies and disables all three external game adapters. The I3
launcher does not start the gateway until a separate cutover activation marker
exists. Candidate NetworkPolicy additionally denies external egress except DNS.
Copy the public `tools/kubernetes` directory into `/tmp/kubernetes` in the seed
pod and run `/tmp/kubernetes/bootstrap.py`, so its gateway preparation helper is
available beside it. The helper rebases inherited I3 state/log paths to the
retained private home, preserving the original config and router password.
Before starting the stack, stream the approved certificate/key into the
namespace's TLS Secret without a local file or logged payload:

```sh
kubectl -n savsoul exec savsoul-seed -- python /tmp/kubernetes/prepare_frontend.py --emit-secret | kubectl apply --server-side --field-manager=savsoul-tls -f -
```

Use the same private stream on future certificate rotation, then validate and
reload Nginx. Never run `--emit-secret` into a terminal, log or tracked file.

Run `python tools/kubernetes/render.py` to regenerate `stack.json` after changing
the deployment inputs. Remove the completed seed pod before starting the
StatefulSet so the RWO volume can attach on either node. Validate with server-side
dry run, then apply it. Do not
commit private data in ConfigMaps. The existing game binary and private state are
on the retained volume, not in public images. The public Nginx container mounts
only the managed website and the current certificate/key Secret.

The NodeBalancer uses PROXY v1 for TCP 80/443/4000/4001/4080/4081; Nginx restores
the client address and forwards the game through trusted loopback ingress 4003.
NodeBalancer health traffic is terminated by Nginx without opening a game session;
game readiness checks its process/listener without connecting as a player.
UDP 4002 is not exposed in this candidate: Linode requires Premium NodeBalancers
and account-specific beta access for UDP. The owner elected to proceed with I3
and leave Zebedee disabled; it does not block the main-stack cutover. Any future UDP frontend
must use no PROXY framing. NetworkPolicy accepts frontend traffic only from
the NodeBalancer backend range 192.168.255.0/24, preventing public NodePort clients
from supplying forged PROXY headers. Verify this range and actual policy
enforcement before publishing DNS. Backend ports 4085/4086/4091 are never Services.

## Verification and cutover

Before changing DNS, verify the public LB using hostname overrides, keeping TLS
certificate validation enabled. Check website, TLS MUD greeting, editor Host and
Origin checks, websocket terminal, map download, CPU model health and copyover.
Confirm Internet egress is actually denied and no candidate I3 gateway is running.
Inspect pod memory under a trainer request; requests must fit one node.

For cutover, first back up the full current DNS records. Pause old game, editor,
gateway/history and any other state writers; perform a final verified sync with
an explicit policy for candidate-only test changes. Do not use the fresh-only
importer to overwrite an occupied volume. Retain both pre-cutover snapshots.
Start the final candidate with external adapters still disabled, verify save
counts/hashes, then stop all old external adapters before enabling the new ones.
Only then update the savsoul.com/www addresses to the verified NodeBalancer IP.
Do not alter MX/TXT or unrelated records. Keep the old VM available for rollback.

An active game writes new saves after cutover: rollback requires synchronizing
those writes back before restarting the old stack. DNS reversal alone is unsafe.
Replace the VM-specific CI release receiver with a narrowly scoped Kubernetes
release identity before changing the production deploy workflow; never use the
cluster-admin kubeconfig in GitHub Actions. Do not enable two release targets.

The maintenance pod has a two-hour deadline and is removed after bootstrap;
do not delete the PVC or its retained PV. The main stack is intentionally isolated
from the old server's unrelated websites and databases; see `LEGACY-MIGRATION.md`.

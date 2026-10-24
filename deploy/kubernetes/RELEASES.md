# Automatic production releases

Each successful main build releases the checked artifact to
`savsoul/savsoul-0`, the production stack behind savsoul.com. The VM deploy job
has been removed. The main-only credential environment and concurrency group
retain their older `kubernetes-candidate` names so credentials are not copied
and releases remain serialized across the promotion. Queued releases check that
their revision is still main before deploying. DNS and external network
activation are operator-managed; normal releases preserve them.

## Deployment identity

An operator applies `release-rbac.json` and provisions two independent secrets:

- `SAVSOUL_KUBE_TOKEN`: token for namespace ServiceAccount `savsoul-release`.
  RBAC permits GET of named pod `savsoul-0` and GET/CREATE of its port-forward
  subresource. It grants no exec, Secret reads, workload mutation, other-pod or
  other-namespace access.
- `SAVSOUL_KUBE_RELEASE_TOKEN`: random bearer key matching namespace Secret
  `savsoul-release-auth`, field `token` (at least 32 characters).

Environment variables `SAVSOUL_KUBE_SERVER` and `SAVSOUL_KUBE_CA` carry the HTTPS
endpoint and base64 CA certificate. Never upload the administrator kubeconfig.
The provisioned long-lived ServiceAccount token lives in namespace Secret
`savsoul-release-ci`; delete/recreate that Secret and replace the GitHub encrypted
token to rotate it. Delete the RoleBinding to revoke cluster access immediately.
Rotate the independent release key in both Secret and GitHub, then restart only
the release container to load it. Never display either value in logs or commit it.

Port-forward RBAC cannot restrict destination ports: this identity can reach
other loopback APIs within this application pod. Those APIs retain their own
authentication. The release receiver binds only loopback 4092, authenticates
before reading uploads, and has no Kubernetes token or TLS-key mount. Nginx uses
a different UID so the release process cannot read its key via shared `/proc`.

The receiver and file primitives run from the operator-managed ConfigMap, not
the mutable release checkout. Apply the rendered stack and restart the candidate
once when initially installing this sidecar; normal code releases use copyover.
Configuration/image/runtime dependency changes still require operator preparation.
Regenerate `stack.json` with `python tools/kubernetes/render.py` for such changes.

## Transaction and recovery

The receiver verifies archive paths, digests, revision, executable build identity
and installed dependency locks. It durably freezes editor startup, stops the
editor, journals replacements on the retained volume, publishes group-readable
website assets and signals the verified game process for copyover. It checks the
successor executable and waits for editor/browser-terminal listeners before
committing `state/releases/current.json`. A separate durable `editor-save-block`
prevents saves while the editor is restarted for verification; it clears only
after successful commit or complete rollback. An identical healthy retry is a no-op.
Editor saves and world reloads share a filesystem lock with the freeze transition.
Admitted operations finish before the release freezes the editor. A world reload
keeps that lock until its successor executable responds; a durable
`state/runner/editor-copyover-pending` marker blocks further writes/releases if
completion cannot be verified. Inspect its predecessor PID and verify successor
identity and login readiness before an operator clears that marker.
Three recent backup directories are retained after successful release processing.

Player/account JSON and private network state are excluded from artifacts. Live
authored room/help/content edits are preserved when they differ from the previous
release. Instance-specific `tools/intermud/gateway.yaml` is always preserved.
Candidate I3/Grapevine/Zebedee settings remain disabled until cutover.

Failures restore journaled files, public assets and the previous manifest, then
copy over to the restored executable. An incomplete rollback retains
`state/releases/active.json`, `state/runner/editor-save-block` and
`state/runner/release-in-progress`; editor startup
stays frozen and subsequent releases fail closed. Do not simply delete markers.
An operator must inspect the named private backup's `journal.json`,
`public-journal.json`, `previous.json`, data snapshot and copyover status, restore
files with the immutable receiver helpers, and verify the game and supporting
services before clearing recovery state. A committed receipt with an interrupted
response is safe to retry for that same revision after health verification.

Backups contain private saves and remain on the retained volume. Do not upload
them as public CI artifacts. CI uses a temporary owner-only scoped kubeconfig and
removes it when the tunnel exits.

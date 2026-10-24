# Later migration of the remaining VM services

The first migration covers only Savage Soul's main stack and savsoul.com. Keep
other legacy applications on the existing VM until each has an owner, a tested
replacement, a verified backup, and a separate cutover window.

## Inventory and recovery

Inventory savagesoul.com/net/org, beachsunfun.com, shelter7.com and ffxivbattle.com:
record Apache virtual hosts, document roots, redirects, TLS, scheduled tasks,
upload paths, mail dependencies, DNS records and database ownership. Determine
which sites still need dynamic functionality. Preserve files before discarding
old videos, experiments or duplicate trees; disk size alone is not proof that
content is unused. Export configuration without publishing secrets.

The old environment includes MySQL5.5 (~38MB logical data), PHP5.5 applications,
CouchDB5984, Elasticsearch9200/9300, a Ruby service on8000, a Mono service on1111,
mail/FTP/DNS services and legacy MUD ports6660/6665/6666/6667/6672/8001. Confirm
actual listeners and consumers before deciding what to keep. Do not expose these
backends publicly simply because they were reachable on the old machine.

## Application-by-application migration

1. Move static sites first. Copy only their required assets into immutable images
   or private retained storage, retain redirects, and test with hostname overrides.
   Give each application its own namespace/service account and ingress policy.
2. Take logical MySQL exports plus a restorable source backup. Test import into a
   supported database in isolation, checking charset/collation, SQL modes, stored
   routines and application queries. Do not mount MySQL5.5 files into a modern
   database. Upgrade PHP applications and dependencies against the restored copy.
3. Export CouchDB data and Elasticsearch mappings/documents using supported
   logical formats. Decide whether each is still required. Reindex/rebuild on
   supported versions rather than attaching old data directories directly.
4. Rebuild Ruby/Mono services on supported runtimes; identify libraries, secrets,
   external integrations, persistence and authentication before enabling traffic.
5. Evaluate legacy MUDs individually. Preserve world/player state and unique
   network identities; keep old Intermud adapters off while testing copies.
6. Migrate email and authoritative DNS only as separate projects. Audit delivery,
   SPF/DKIM/DMARC, reverse DNS, queues and dependent accounts. Replace FTP with a
   scoped modern transfer mechanism where practical.

## Cutover and retirement

For each dynamic site: test restores and browser flows, lower DNS TTL ahead of
time, pause all writers, perform a final logical export/upload sync, verify row
counts and content, then change only its records. Keep a written rollback plan
that accounts for new writes. Add scoped CI deployment, probes, retention,
off-cluster backups and restore checks before calling the migration complete.

Retire the Ubuntu14.04 VM only after all retained services have moved, mail/DNS
dependencies are removed, backups have been restored successfully elsewhere and
an observation period shows no remaining legitimate traffic. Keep a private final
image/archive according to the owner's retention policy; revoke its credentials
and network access when decommissioning it.

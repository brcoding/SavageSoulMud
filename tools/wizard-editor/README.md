# Savage Soul wizard editor

Open http://localhost:4080 on the Windows machine hosting the MUD. Log in
with your wizard character name (for example Rawr) or its account email,
and the same password used in the MUD. Browser login does not take over
your telnet connection. The current `admin` role is the wizard permission.

## Editing

- Browse World to select an area, then search rooms by name, slug or NPC.
- Fields edits room descriptions, exits, doors, NPCs, shop stock, objects,
  fixtures, scripts and optional settings. Expand nested sections to edit them.
- Add/duplicate/delete rooms; IDs are generated for new rooms. Existing IDs
  are read-only in the forms because exits reference them. Deleting a linked
  room requires removing or redirecting its incoming exits first.
- Add field adds optional keys such as npcs, objects, scenery, spawn, shop,
  combat_behavior or doors. Source edits the complete JSON document directly.
  Advanced fields and templates use the existing blueprint names and formats.
- Help, rules, quests, socials, migration overrides and text assets use the
  same forms/source editor. New file creates an unsaved draft. A new help
  topic also needs an entry in data/help/index.json to appear in MUD help.
- Validate checks before saving. Areas and NPC profiles pass through the
  MUD's native Rust parser; area IDs, slugs and exits are checked globally.
  Other known configurations use JSON Schema; arbitrary JSON receives syntax
  validation. This does not prove Lua scripts or quest logic are correct.
- Save makes an atomic replacement and backs up the prior file. A changed
  revision returns a conflict and leaves your draft intact. History can
  restore an earlier version, saving another backup of the current content.

Help and MOTD files take effect on their next use. World/rules/social files
need **Apply world changes**, which validates saved content and requests
copyover. Connections survive; active fights, ground items and NPC positions
reset. The button reports that reload was requested, rather than claiming
that every world behavior has been tested. Embedded login-banner/login-help
assets require rebuilding the MUD after editing; the editor labels this.
Migration overrides are used by conversion tools, not world startup.

Files are edited in the running installation, normally P:/savsoulmud/grim.
The development checkout does not automatically mirror these changes. Before
re-importing Savsoul, update its reconstruction overrides for any edits you
want that import to retain. The local deployment helper now detects edits
made after its last deployment and skips conflicting authored files.

## Service and boundaries

`start-editor.sh` installs pinned dependencies in the WSL user's persistent
`~/.local/share/savsoul-editor/venv`, copies the app into Linux storage, and
runs Waitress on 127.0.0.1:4080. Flask/Waitress follow the official Flask
production deployment guidance: https://flask.palletsprojects.com/en/stable/deploying/waitress/

The user systemd service is savsoul-editor; restart with:

    systemctl --user restart savsoul-editor
    journalctl --user -u savsoul-editor -n 30

Backups and audit logs are in `~/.local/state/savsoul-editor`. Preserve this
folder alongside regular MUD backups. No retention deletion is automatic.
Each authenticated request rechecks account ownership, wizard role, bans and
password fingerprint. Sessions expire after 30 minutes of inactivity and
are invalidated when the editor restarts. Mutation requests require same-origin
CSRF tokens; arbitrary Hosts, path traversal and symbolic links are denied.
Account/character saves, bans, credentials, server code and executables are
not exposed as editable content. No arbitrary shell or systemctl command API.
The service is local only; remote hosting needs a separate HTTPS deployment.

## Files

- server.py: auth, content API, native/schema validation, revisions, backups,
  history, fixed copyover action. Generic editor state is separate from MUD data.
- static/index.html, style.css, app.js: accessible browser editor; no CDN.
- requirements.txt: pinned dependencies; savsoul-editor.service: user service.
- test_editor.py: focused isolated authentication/save/validation checks.
- ../../crates/example-mud/src/seed.rs: native validate_area_json.
- ../../crates/example-mud/src/main.rs: --validate-area parse-only CLI.

Run checks without touching live content:

    GRIM_TEST_VALIDATOR=/tmp/savsoul-grim-runtime/savsoul \
      ~/.local/share/savsoul-editor/venv/bin/python tools/wizard-editor/test_editor.py

File catalogs use directory-entry metadata and a five-second cache to avoid per-file round trips to P:. Saves invalidate the catalog immediately; external file additions/removals appear within five seconds. Authorization is rechecked on every request, and actual file reads/writes still perform full path/symlink validation. File sizes are not enumerated for the navigation list.

## Maps and exit editing

Use **World atlas** for an illustrated region index, or open an area and choose
**Map**. Select Fantasy atlas, Hex grid or Exit blueprint. Region,
Town and Room presets provide overview, settlement and detail scales. Wheel
zooms at the pointer, drag pans, Fit frames the current view. Arrow keys pan;
plus/minus zoom; F fits. SVG rooms, region buttons and diagnostic entries are
keyboard accessible.

The map sits beside a full room editor. Select a room to edit its descriptions,
exits, doors, NPCs, objects, scenery and nested settings without leaving Map.
Both Fields and Map edit the same area draft; either Save button validates and
saves the entire area. Draft edits refresh map labels and diagnostics without
replacing the focused input. Drag the divider or use **Room inspector width**
to resize the panes; the divider supports arrow keys, Home and End. Width is
remembered in local browser storage. Narrow layouts stack the panes vertically.

Navigation updates the URL with the file, stable room ID, editor tab, map scope,
style and floor. Bookmark or copy the address to reopen the same location after
wizard login. Browser Back/Forward follows these locations; switching files still
checks for unsaved changes. Links carry navigation only, never login credentials
or draft content. Recipients need access to this editor server and wizard login.

World regions are arranged as an index, not geographical coastline/distance
claims. Clicking a region opens actual blueprint topology. Area and two-room
neighborhood scopes use exits to infer integer x/y/z. Up/down establishes
relative floors within each connected area component; disconnected components
and all-floor panels are packed separately. Floor choices include selected
room floor, one explicit level, or all floors. Room detail adds illustrative
furniture and stairs; it does not establish exact room dimensions, object
locations or TTRPG movement distances. Terrain comes from names/descriptions.
No visual-layout metadata is persisted and rendering never changes exits.

Diagnostics flag missing targets, unknown directions, duplicate IDs, self-loops,
wrong reciprocal returns, differing door states, collocated rooms and directional
loops with contradictory inferred positions. Potential line crossings and
one-way routes are informational (toggle notes to include them). Geometry
warnings can represent legitimate variable-length paths or portals: inspect
before changing them. Every room is retained when inferred coordinates collide.
Doors show open/closed state; S marks secret doors; arrows mark floor changes.
The map reflects the current unsaved area draft. Clicking warnings selects the
associated room. Navigating to another area still asks before discarding a draft.

Room Fields and the map inspector use **named exit destinations**. Add/remove
an exit explicitly, or add a missing reciprocal return within the same file.
Existing returns are never overwritten automatically. Cross-area returns must
be edited in their own area, avoiding partial multi-file transactions. Existing
door definitions are preserved when retargeting; deleting an exit with a door
asks before removing both definitions. Save and Apply still use the normal
validated/backup/copyover workflow.

`GET /api/world` is wizard-only, bounded by the editor's file limit, and cached
for five seconds with save/restore invalidation. It reads area files once each,
not per-room metadata, and only returns presentation/topology fields. No live
entity IDs, character data or NPC SoulData are exposed by this endpoint.

Map files: `static/map-model.js` (pure deterministic graph/layout analysis),
`static/map-view.js` (native SVG renderer/navigation), and `static/app.js`
(editor draft/exit integration). Native SVG avoids a graph/render framework
and supports resolution-independent pan/zoom without a CDN or raster service.
Run the focused graph scenarios with `node tools/wizard-editor/test_map.cjs`.

## Remembered browser login

Successful wizard login sets a persistent HttpOnly, SameSite=Strict cookie with a rolling 30-day lifetime. Each editor request refreshes its expiration. The signing key is stored as `session-signing.key` in the private editor state directory (`/home/barry/.local/state/savsoul-editor` locally), so editor restarts do not log everyone out. Preserve this state directory when upgrading; do not commit or expose its key. Logging out clears the browser cookie. Wizard ownership, role, bans and password fingerprint are still checked on each authenticated request; remembering the login does not bypass those checks.

Species templates: edit `data/savage-soul/races.json` under Rules. Saves run the
MUD's `--validate-races` validator (supported traits, costs, conflicts and 600-XP
affordability). Copyover reloads the choices for new characters. Existing
characters retain the complete template/version/cost snapshot from creation.

## Public server access

The deployed editor is at https://savsoul.com:4081/; HTTP port 4080 redirects
there. Apache binds these ports only on the server public IP, keeping the
existing loopback-only gateway on 4081 separate. It proxies to 127.0.0.1:4085
using the trusted savsoul.com certificate. The editor remains loopback-bound.
The server wrapper allows only savsoul.com:4081 and www.savsoul.com:4081, trusts
one Apache hop for scheme/client IP, and sets Secure cookies. Wizard credentials,
role rechecks, CSRF and same-origin validation remain required. Local create_app
instances retain their loopback host allowlist unless explicitly configured.

The remote Waitress listener explicitly trusts proxy headers only from
127.0.0.1, with one trusted hop and only X-Forwarded-For/X-Forwarded-Proto.
This must be configured in Waitress itself: its default strips untrusted
forwarded headers before WSGI middleware, causing HTTPS Origin checks to fail.

# Mudlet sign-in and mapper

Connect to savsoul.com:4001 with TLS and GMCP enabled. Mudlet4.18+ can send the
character name/password saved in its connection profile via Char.Login v1.
Email-only opens the account roster; email:character selects an owned character.
The game still handles password checks, bans, MOTD and accessibility. Empty
credentials use the normal text prompts. No OAuth or reconnect tokens are minted.

Open Mudlet's mapper after login. Client.Map offers the public MMP XML and
Client.GUI offers SavsoulMapper, a tiny package that follows Room.Info. Allow
server package downloads if disabled. The package owns only its handlers;
reloads replace them rather than accumulating duplicates. It doesn't speedwalk
or read/store credentials. Use Mapper settings to re-download an updated map.

Run `python tools/mudlet/export_map.py --root . --output /public/path/savsoul.xml`
before reloading the world. On the server, the editor reload helper does this
before copyover. Preserve data/mudlet-map-ids.json: IDs are compact and stable,
with removed IDs retained to avoid reuse. Never copy a stale registry over it.
Map publication only includes canonical rooms reachable by visible exits from
savsoul/start. Names, area, coordinates and exits are public; NPCs, descriptions,
secrets and player data aren't exported. Geometry is inferred; loops/portals may
be displaced while actual directed connections remain intact. Closed gates
retain topology links; normal puzzle/movement checks still enforce passage.

Public artifacts: https://savsoul.com/mudlet/savsoul.xml and
https://savsoul.com/mudlet/SavsoulMapper.mpackage. Settings are map_url, gui_url
and gui_version in data/mudlet.json. Bump package version for script updates.

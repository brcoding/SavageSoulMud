# Savsoul local MUD

Installed at `P:\savsoulmud\grim`, based on gvanderest/grim.

Run `powershell -ExecutionPolicy Bypass -File P:\savsoulmud\grim\start-local.ps1`.
Connect your MUD client to `localhost`, port `4000`. Create an account and character
through the login prompts. The initial room remains Haven's **The Rusted Anvil**.

Type `open west`, then `west` to enter **Royal Oak Wall Gate**. From the gate,
`north` reaches South Royal Oak Main Street; `west`, then `west` reaches the medical
room. Use `read sign` (or `look sign`), `look table`, `look cabinets`, and other scenery keywords.
Return to the gate and go `east` to reach Haven. The door's open/closed state is
shared between both sides; `open east` opens it from Royal Oak.

The server binds to loopback only. A compiled Linux binary is saved in `bin/savsoul`;
the launcher uses it directly. Ubuntu-24.04 WSL and workspace-local Rust 1.98.1
are used because upstream's hot-restart code requires Unix. No remote hosting has
been configured. Stop this instance with
`powershell -ExecutionPolicy Bypass -File P:\savsoulmud\grim\stop-local.ps1`;
allow 30 seconds for graceful shutdown before restarting.
Account/character saves live in `data/accounts` and `data/characters`; back these up.

## Conversion review

- Converted all 20 public Savsoul rooms and their internal exits into
  `data/areas/savsoul.json`. Stable room IDs and original-to-new names are in
  `migration/room-map.json`.
- Preserved scenery aliases and descriptions as inspectable, immovable fixtures.
  Preserved both medical signs; both `read sign` and `look sign` display their text.
  The medical table is immovable. Its screen message is legacy flavor: corpse
  regeneration is not implemented in this conversion (the source table itself
  had no regeneration callback).
- Grim currently supports north/east/south/west/up/down. Main Street's northeast
  trail exit and its southwest return use east/west here. The full trail remains
  connected. Original descriptions repeatedly reuse Main Street text in shops,
  bar rooms, and trail rooms; retained as authored rather than inventing replacements.
- Daytime descriptions are used for the initial basic world. Night descriptions
  and climate metadata are retained in the migration map; dynamic time/weather,
  lighting, combat/weapon stats, and LPC callbacks have not been ported.
- The medical room's down exit referenced the external campus, which was not in
  this domain. It is omitted; the Haven door is the new cross-world connection.
- `death`, `freezer`, `furnace`, `pod`, `void`, and `wiz_hall` are Dead Souls support
  rooms, with driver-specific behavior or exits into external domains. Their
  original files are archived, rather than added as broken public rooms.
- `weap/brush.c` is archived as an unplaced item; no source room spawned it.
  All original room/object/text/weapon files are in `migration/savsoul-lpc`.
  The running Dead Souls installation was only read, never edited.

Edit JSON world files and restart to change content. Engine Rust changes require
rebuilding and copying the rebuilt executable to `bin/savsoul`. Upstream regression scenarios use unchanged vanilla area fixtures;
the Savsoul scenario exercises the actual converted world and door.

Build, formatting, lint (warnings denied), all 46 selected gameplay/seed tests,
world graph validation, and Windows localhost login connectivity passed.

## Current population update

The October 12 backup conversion adds 500 rooms and 134 NPC instances. From the Royal Oak Med Room, go `down` to Campus reception, `look jennybot`, then `west` to the Travel Concourse. Follow `north` and use marked west archways to visit other areas. See `migration/POPULATION.md`.

The PowerShell launcher now starts the `savsoul-grim` WSL user service, which continues after the launcher exits. Status: `wsl -d Ubuntu-24.04 -- systemctl --user status savsoul-grim`. Log: `wsl -d Ubuntu-24.04 -- journalctl --user -u savsoul-grim -n 30`. Use the existing stop script for graceful shutdown.

## Savsoul start correction

New characters now start in the Royal Oak Med Room, matching `lib/secure/include/rooms.h` (`ROOM_START /domains/savsoul/room/start`). Existing characters retain their saved room. Haven remains accessible through the Savsoul gate. Soulboy is pending identification: no matching filename or text was found in the current backup.

## Trader Sam shop

Starting credits: 100 (including existing characters without a credits field). Check your balance anywhere with `credits`. At Trader Sam's checkout use `list`, `buy shovel`, `buy ration`, and `sell shovel`. `buy 2*shovel`, `sell 2.shovel`, and `sell all food` work. Stock is unlimited; buy/sell prices are fixed. Use `drink water` / `drink bottle` to consume water and leave an empty bottle; use `eat ration` / `eat food` to consume a ration pack. These inventory changes save immediately. Credits and inventory save immediately after each successful trade.

## Savage Soul character and melee core

New characters skip the legacy race/class picker, begin in the medical room with 600 XP and 1,000 credits, and build using `score`, `train`, `pecs`, and `learn`. Existing credit balances remain unchanged. Rawr remains a wizard. Use `wield`, `wear`, and `attack <creature>` for equipment and melee; wizard tools include `awardxp` and `soulheal`. See `migration/SAVAGE-SOUL-IMPLEMENTATION.md` for the implemented core and remaining rulebook coverage.

Consumables: `drink water`, `drink bottle`, `drink 2.bottle`, `eat ration`, and `eat food` consume carried supplies. Drinking replaces a full water bottle with an empty bottle; empty bottles cannot be drunk or sold as full bottles. Eating removes a ration pack. Each action saves inventory and its consumption timestamp before applying the change. Water/food do not grant healing or mana.

## Runtime reliability

Startup copies bin/savsoul into /tmp/savsoul-grim-runtime when the Linux runtime copy is absent. Never execute the binary directly from network-backed P:. WSL restart clears /tmp. The service now uses Restart=on-failure with a three-second delay; copyover retains the existing notify-based process handoff.

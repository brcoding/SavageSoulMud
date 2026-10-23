# Development backup recovery

Source: `C:\Users\barry\OneDrive\Documents\37devbackup\ds3.7a7-dev\lib`.
The backup is read-only. The merge base was the live `P:\savsoulmud\grim`
content, with its original files snapshotted in `work/dev-import-backup`.

## Imported world

- 99 authored rooms: Morlock hill trails and caves, Tully Lake shores and water,
  the three Arena Road rooms, Walts ATM and four Southfield rooms.
- 45 native wasteland rooms connecting the source coordinates `(4,0)` at the
  end of Royal Oak South Road to `(2,-14)` outside Southfield.
- 15 new NPC blueprints in new rooms, plus Ron at the charging station and Tully
  at the shack. Source population counts and replacement delays are configured
  per spawn point. Authored wandering is limited to two rooms within the area.
- Source descriptions, item aliases, static sign text (including LPC heredocs),
  fixtures and placed objects. The existing inn, gate, medical, wraith, shop and
  character fixes are preserved. NPC level is provenance; new creature combat
  uses the current Savage Soul defaults rather than translating LPC HP or level
  into invented Savage Soul statistics.

The resulting world has 945 rooms and 160 NPC blueprints before any later
content edits. Population caps may create multiple live creatures per blueprint.
Every imported room is reachable from the current starting room and every native
exit references a loaded room identity. Existing IDs were retained.

## Routes

- Morlock hills: west from `rosouthroad07`; follow the hill trail west. The
  branches south reach Tully Lake; the trail's western end descends into caves.
- Southfield: south from `rosouthroad17` into wasteland `(4,0)`, continue south
  fourteen rooms, west twice to `(2,-14)`, then west into East Southfield and west
  to the courtyard. The metalshop is south; the restaurant is north.
- Ron Metalswiller: `rochargingstation02`.
- ATM: west from Trader Sam's checkout.

## Recovered Morlock machine

The game-local `morlock.rs` adapter implements the source four-button optical
puzzle. `look lenses` shows the current compass bearings; `press red button`
(and blue, green, yellow) rotates the original combinations. The map-room
technical diagram preserves the source clue. Aligning the four lenses north,
east, south and west opens the panel and awards **100 XP and 10 quest points**,
once per character. The complete character snapshot is saved before rewards
are published. `get suit from panel` retrieves the source white NBC suit, saved
with the complete pack. The panel starts closing after 100 seconds and closes
and resets at 120 seconds. World state resets on copyover; earned quests and
carried loot remain saved.

The source suit is preserved as a collectible. Its old radiation/disease/gas
protection numbers are not treated as Savage Soul armor values without a rules
adapter.

## Explicit recovery boundaries

The source wasteland generator defines almost 5,000 mostly identical rooms.
This recovery materializes only the 45-cell corridor required to reach the
new authored Southfield shops, with original eight-way directions and prose.
Exits beyond that projection are omitted. The source omitted a reciprocal west
exit at `(2,-14)` despite Southfield pointing east there; the recovery repairs
that connection. The bounded corridor's eastern/southern edges are recovery
boundaries, not newly authored wilderness.

The PvP arena beyond Arena Road is not activated. The ATM and its historical
readable sign are preserved, with an explicit notice that banking services are
not active. No legacy currency conversion is applied to integer credits.

LPC ambient audio/random searches, swimming and environmental admission checks,
automatic gun-turret targeting, Tully's scripted interactions, and source
containers' nested inventories are not runtime ports. Source containers that
are fixed scenery have their initial contents placed alongside them, except
the machine panel, which has its specific timed retrieval adapter. All relevant
source files are archived rather than evaluated or silently discarded.

## Maintenance

`migration/import_development_rooms.py` prepares repository output, never writes
the live installation, never executes LPC and never changes accounts/characters.
It defaults to the current live installation as its merge base. **Do not run it
over later authored changes without reviewing the merge**: it is a recovery tool,
not a live synchronizer. Its report is `development-room-import.json`; archived
inputs and their SHA-256 hashes are in `development-source/` and that report.

The source-to-native identity map is in the report's `added_rooms`. New room IDs
are stable hashes of source paths; existing aliases (including the medical hub)
resolve to the existing room rather than duplicating it. The importer asserts
unique IDs, complete exit references and reachability before reporting success.
Crafting recipes, supplies and merchant additions are maintained separately in
`data/savage-soul/crafting.json` and the native area JSON.

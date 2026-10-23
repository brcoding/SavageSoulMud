# Development crafting recovery

Source: `C:/Users/barry/OneDrive/Documents/37devbackup/ds3.7a7-dev/lib` (read-only).

`data/savage-soul/crafting.json` is the editable runtime catalog. It contains 110 real object templates, eight authored recipes, thirteen nonempty reductions, seven edible food templates, and source paths/hashes. `items` values use the same StoredObject representation as ordinary carried objects. `craft:<id>` keywords provide stable material/tool identity across saves; generic names such as wood or meat are not sufficient identity. Recipe ingredients and products use item IDs and integer quantities. All listed tools are required and may be carried or fixed in the room.

The import script emits a separate `work/dev-crafting-merchants.json` artifact for merging into existing NPC stock and room scenery; it does not replace area files. It starts equipment updates from the supplied live catalog, preserving existing named entries. Run intentionally when reimporting (it rewrites the generated catalog):

```powershell
python migration/import_development_crafting.py --source 'C:/Users/barry/OneDrive/Documents/37devbackup/ds3.7a7-dev/lib' --equipment-base 'P:/savsoulmud/grim/data/savage-soul/items.json'
```

## Playable source behavior

`craft <recipe>` needs the authored materials and all tools. Recipes were globally available in the active LPC command: its learning, output-quality, skill progression and XP checks were TODO. No new learning requirement or crafting XP is invented. `recipes` and `reductions` expose the catalog; their old versions were NPC-local menus. Crafting products remain ordinary inventory, suitable for saving, eating, equipping and shop transactions.

Rusty receives the four cooking resources, a ham sandwich and a cheap knife. Metalhead receives his five authored metal resources plus portable crafting tools, a piece of wood and wood handles. Ron receives his fourteen authored weapon variants; Trader Sam receives the two authored chainmail variants and GH3 rounds. These are additions to current stock, not replacements. Existing shops use unlimited supply; legacy storeroom reset counts are not stock limits. Source `SetValue` or credit `SetBaseCost` sets integer buy prices; resale is half, rounded down. An absent price uses 10 credits as an explicit fallback.

The stove belongs in Royal Oak Bar N Grill's kitchen. The bullet press, die forge, grinding wheel and radial rivet machine belong in Southfield Metalshop as nonportable scenery, matching their bolted-down source definitions.

## Explicit adaptations and unfinished source

- Wood handle requires a bandsaw in the source recipe, but no bandsaw definition or placement exists in this backup. That recipe stays visibly unavailable. Selling authored wood handles through Metalhead restores the claw-hammer ingredient chain without inventing a bandsaw.
- `crafting/reductions/claw_hammer.c` is empty, so no hammer salvage recipe is fabricated.
- All thirteen nonempty reductions retain their exact product quantities and source skill metadata. The first Grim adapter uses deterministic recovery after validation. The old destructive luck/skill formula belongs to incompatible LPC stats and is not transplanted into Savage Soul attributes; this port grants no reduction XP.
- Seven corpse types, including rat and morlock variants, resolve from current corpse names. Existing corpses need not already have a `craft:` keyword. The short-sword reduction is retained for an existing carried short sword, but no missing short-sword object is fabricated. The actual adjustable-wrench source template is included.
- The 77 weapon and four armor definitions are catalog objects; the source has **no weapon or armor forging recipes**. Preserve quality q0–q3 in distinct item names and descriptions. This is craftsmanship, not wear: every newly created item starts at condition 100, including q0.
- Equipment maps by weapon type and rough size to existing Savage Soul Combat/Pocket Knife, Light/Heavy Sword, Light/Heavy Club, or Medium Armor entries. These are provisional base-type rules; numeric LPC Class, protection, mass and DamagePoints are not treated as Savage Soul modifiers. Craftsmanship tiers currently preserve authored identity, appearance and value, without inventing mechanical quality bonuses. The chosen adapter is recorded per item in `source_items`.
- GH3 shells and rounds can be crafted/reduced as authored ammunition objects. This import does not add a firearm/reload combat subsystem.
- The source's stove is restored even though none of the eight active recipes requires it. No invented cooking recipes are added.

Every `source_items` entry records the originating file's SHA-256 and chosen credit price. Custom edits belong in the runtime JSON; rerunning the importer is an explicit regeneration action, not a normal server startup operation.

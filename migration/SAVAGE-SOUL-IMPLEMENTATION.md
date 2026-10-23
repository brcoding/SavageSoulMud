# Savage Soul in the local MUD

The character and melee-combat core uses The Savage Soul 2026 rules, with Soulgen used to cross-check formulas and equipment data. Faith follows the rulebook and cannot be trained directly, as the user requested.

## Player commands

- `score` / `stats`: attributes, XP, credits, injury condition, defenses, mana, carried equipment load, and equipment.
- `train`: list trainable attributes. `train body`, `train melee attack`, etc. buy the next level. Ordinary attribute levels cost 50 × the new level; Body costs twice as much. No single attribute may consume more than half total XP.
- `pecs`: list implemented Peculiarities. `learn brainy`, `learn brutish`, `learn burly`, `learn genteel`, `learn unarmed fighter`, `learn quickness`. The first four are mutually exclusive one-level Attribute Modifiers; the last two have three implemented levels. XP-earned Combat points are consumed before buying extra points for 100 XP each.
- `credits`, `list`, `buy pocket knife`, `buy light armor`, `sell <item>`: existing shops. Sam now also carries nine Average-quality rulebook equipment entries.
- `wield pocket knife`, `wear light armor`, `equipment`, `remove weapon`, `remove armor`: equipment must be carried, and size restrictions apply. Dropping/selling equipment removes its combat effect.
- `task athletics 10`: resolve an ordinary d10 task against the stated difficulty.
- `attack <creature>` / `kill <creature>`: start an automatic melee duel in the room. `stop` ends it. `defend auto` / `defend dodge` enables automatic Dodge; `defend none` saves your Ready token.
- `recover`: recover from stun outside combat; during combat the next activation automatically recovers from stun.
- `rest`: outside combat, spend a five-second round then attempt Will Power at DF 10 to recover from Winded. This does not heal wounds.
- `exchange <XP>`: before purchasing attributes/Peculiarities, trade unspent starting XP for credits at 100 credits per XP.

## Wizard commands

- `awardxp Rawr 100`: award and save XP for a loaded character.
- `soulheal self` / `soulheal Rawr`: administrative restoration of conditions and mana. This is a wizard tool, not a medical procedure or spell.

## Migration and persistence

New characters choose name and gender, then enter Savsoul with 600 XP, average-human base attributes, and 1,000 credits. Default Grim race/class selection is bypassed. Existing character identities, accounts, roles, inventory, explicit credit balances, and old race/class labels are preserved. Characters without a Savage Soul record gain an unspent 600-XP record; their old Grim level does not affect combat. Soul state is a namespaced, backward-compatible `game_state.savage_soul` record in the existing character JSON. Training, equipment changes, and each combat round save the complete character inventory/credits/state atomically before applying live changes. Inventory/equipment ownership is rechecked after transfers and drops.

## Combat mapping

The initial MUD adapter is automatic, one player against one creature, with one activation per side in each five-second round. Both are their own force leaders; a competing Leadership attempt determines first activation. There is no initiative roll. Active Defense discards a Ready token if present, so defending can prevent retaliation. The current automated reaction is Dodge; tactical Block, Preemptive Attack, formations, and group leadership are not enabled.

Attack 1s miss; defense 10s succeed. Optional attack/task natural-10 rerolls default to keeping the 10. Ordinary natural 1s use the rulebook d5 correction and possible critical failure. Damage uses DC + SM (capped at 5), armor coverage, normal/sharp DR, cumulative wound conditions, injury penalties, and Will Power resistance to stun/unconsciousness. The game uses no HP pool. Stun consumes the next activation to recover. Critical injury resistance checks use persisted real-time deadlines at 2 minutes, 10 minutes, 1 hour, 4 hours, and 16 hours. Winded checks occur after 12 duel rounds. Natural healing, bleeding treatment, waking another character, and medical spells are not implemented yet; use wizard restoration for administration.

## Remaining rulebook work

This is the character/melee core, not all 452 pages. The wider Peculiarity catalog, disadvantages/non-combat point budgets, species templates, spellcasting/rituals, ranged attacks/ammunition, tactical range/facing, group combat, loot, chemistry, and natural healing still need implementations. Unimplemented Peculiarities are refused rather than charging XP without their effects. Imported LPC creatures currently use average base Savage Soul scores (0 purchased XP) unless their NPC blueprint supplies an explicit `soul` profile. Their LPC levels/races are descriptive metadata, not automatic Savage Soul stats.

Known equipment has rulebook weights; legacy objects without equipment definitions currently contribute no known weight. Sell prices remain the local 50% policy, not a claimed rulebook requirement. See data/savage-soul/equipment-notes.md for documented workbook/PDF equipment differences.

## Sources and checks

Core sources: rulebook chapters 2, 3, 5, 7, 9; Soulgen Char./Ads/Tables/Buyers Guide. Eight focused checks cover core formulas, wound merging, Faith exclusion, load/prone penalties, creation, training, equipment, save/restart, combat/wizard recovery, and shops.

The launcher prefers a Linux runtime executable under `/tmp/savsoul-grim-runtime`, with the installed P: binary as fallback. Linux-side replacement avoids the Windows-mounted executable lock for future copyovers. The systemd notify service tracks successor PIDs.

Consumables: `drink water`, `drink bottle`, `drink 2.bottle`, `eat ration`, and `eat food` consume carried supplies. Drinking replaces a full water bottle with an empty bottle; empty bottles cannot be drunk or sold as full bottles. Eating removes a ration pack. Each action saves inventory and its consumption timestamp before applying the change. Water/food do not grant healing or mana.

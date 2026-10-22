# `example-mud`
> The vanilla GRIM game: the one binary that composes the plugins and seeds the world.

**Role:** vertical — example game (the workspace's only binary)
**Depends on:** `grim`

## Components
| Component | File | Purpose |
|---|---|---|
| `Merchant` | `src/shop.rs` | Authored shop stock and pricing. |
| `Contained` | `src/medical.rs` | Medical-table containment. |
| `Candidate` | `src/population.rs` | Managed NPC spawn membership. |
| `Conversation` | `src/trainer.rs` | Trainer follow-up context. |

## Systems
| System | Schedule | File | Purpose |
|---|---|---|---|
| `seed::seed_world` | `Startup` | `src/seed.rs` | Read area blueprints (`data/areas/*.json`) from the filesystem, spawn every `canonical` area's rooms/exits/NPCs, and set `StartingRoom`. Panics if no starting room resolves. |
| `shop::handle_shop` | `Update` | `src/shop.rs` | Atomic credit and inventory transactions. |
| `help::answer_help` | `Update` | `src/help.rs` | Editable topic and command help. |
| `trainer_ai::tick` | `Update` | `src/trainer_ai.rs` | Validate and deliver optional translation results. |

## Commands
Player-facing verbs and where to find their handlers.
| Command | Handler | Summary |
|---|---|---|
| `grapevine` / `gv` | `src/grapevine.rs` | Wizard-only Grapevine status, subscriptions, channel chat, tells, games/players, history and credential reload. |
| `score`, `train`, `pecs`, `learn`, `wield`, `wear`, `attack`, `kill`, `stop`, `defend`, `rest`, `recover`, `task`, `exchange` | `savage_soul::run_command` (`src/savage_soul.rs`) | Savage Soul character and combat actions. |
| `creator`, `rebuild`, `respec`, `races`, `race`, `buyoff` | `creator::command` (`src/creator.rs`) | Preview and commit med-bay character changes. |
| `list`, `credits`, `buy`, `sell` | `shop::handle_shop` (`src/shop.rs`) | Shop transactions. |
| `help` | `help::answer_help` (`src/help.rs`) | General and command help. |
| `drink`, `eat` | `consumables::consume` (`src/consumables.rs`) | Consume carried supplies. |
| `ask`, `talk` | `royal_oak::ask` (`src/royal_oak.rs`) | Trainer conversation. |
| `put`, `place`, `sit`, `lie`, `lay`, `stand` | `medical::place` / `rest` / `stand` (`src/medical.rs`) | Table, body and posture interactions. |
| `hack` | `hack::command` (`src/hack.rs`) | Terminal puzzles. |
| `craft`, `reduce`, `recipes`, `reductions`, `resources` | `crafting::command` (`src/crafting.rs`) | Material and crafting actions. |
| `screenreader`, `combatlog`, `prompt` | `preferences::command` (`src/preferences.rs`) | Saved presentation preferences. |
| `intermud`, `i3`, `imud` / `zebedee`, `i2` / `protocols` | `src/intermud.rs`, `src/zebedee.rs`, `src/client_protocols.rs` | Network adapters and client protocols. |

Game command vocabulary is installed by `src/game_commands.rs` from
`SavageSoulPlugin`; the shared scene parser remains game-independent.
The plugin replaces look/read/get/open/close factories without changing their
engine prefix priorities. Equipment reconciliation is throttled to once per second.

## Resources & Events
| Name | Kind (Resource/Message) | File |
|---|---|---|
| `AreaBlueprintDir` | Resource | `src/seed.rs` |
| `trainer_ai::Bridge` | Resource | `src/trainer_ai.rs` (optional worker queues) |

## Binaries
| Binary | File | Purpose |
|---|---|---|
| `savsoul` (default-run) | `src/main.rs` | The MUD server: `MinimalPlugins` (frame loop capped at 60 Hz via `ScheduleRunnerPlugin::run_loop` — bare `MinimalPlugins` spins idle at 99% CPU, issue #121) + `LogPlugin` + `GrimDefaultPlugins { telnet_port: 4000, ..Default::default() }`, seeded with `seed::seed_world`. |
| `copyover_fixture` | `src/bin/copyover_fixture.rs` | Lightest real server for the copyover integration test; same 60 Hz cap; port / data dir / areas dir come from env (`GRIM_TEST_PORT`, `GRIM_TEST_DATA`, `GRIM_AREAS_DIR`) so the test isolates state and re-execs itself on `SIGUSR2`. |

## Tests
| Test | File | Purpose |
|---|---|---|
| dependency-direction guard | `tests/dep_direction.rs` | Asserts set-equality of the intra-workspace crate dependency graph against the allow-list from `docs/ARCHITECTURE.md`. |
| end-to-end scenarios | `tests/scenarios.rs` (+ `tests/harness/mod.rs`) | Boots the real world seed through the headless harness (`GrimHeadlessPlugins`, no transport) and drives it as a telnet user would. |
| copyover | `tests/copyover.rs` | Spawns the real `copyover_fixture`, triggers a copyover, asserts the same TCP connection survives and resumes in place. Unix-only. |

## Notes
- The world seed is exposed as a **library** (`src/lib.rs` → `seed`) so the test harness boots the *same* world the binary ships.
- Area/room content lives in `data/areas/*.json` (repo root), not in code — edit and restart, no recompile. References use stable `GrimId`s, not slugs; only `canonical` areas load at startup (see `docs/adr/0001-area-identity-and-instancing.md`).
- Savsoul adds 20 Royal Oak rooms, reached through the tavern's west door. Room blueprints may include `scenery` with object-shaped descriptions/keywords; the seed omits the `Object` marker so these fixtures are inspectable but immovable. See `SAVSOUL-LOCAL.md` for migration details and startup.
- The current LPC backup adds 500 static rooms and 134 placed NPCs. Go down from the Royal Oak medical room to Campus, then west to the Travel Concourse. NPC blueprints accept optional `race`, `level` (default 1), and `gender` (male/female/neutral); scripts require separate ports. See `migration/POPULATION.md` and `population-report.json` for destinations and conversion gaps.
- Vanilla scenarios use `tests/fixtures/areas`; `tests/savsoul.rs` runs against the deployed area files and validates the new door and room routes.
- `main.rs` composes plugins + seeds; `seed.rs` reads blueprints; `src/bin/copyover_fixture.rs` is the copyover test fixture.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

## Shops and credits

Hacking: `hack <terminal>`, `hack try WORD`, `hack status`, `hack abort` are
handled by `src/hack.rs`. Scenery blueprints accept a `Hackable` component
with stable ID, attempts, lockout and access/open-exit/Royal Oak effects.
Puzzles and one-time quest rewards persist atomically with character state.
See `migration/HACKING.md` for configuration and `data/help/hack.md` for play.

`ShopPlugin` is game-local and reuses ordinary `Object`/`CarriedBy` inventory. NPC blueprints can have `shop.stock`, with `item` (a StoredObject), integer `buy_price`, and integer `sell_price`. Trader Sam stocks a shovel (20/10), ration pack (5/2), and water bottle (3/1). Catalog stock is unlimited; shops only buy exact matching catalog instances.

Commands: `list`, `credits`, `buy <item>`, `sell <item>`. Item selectors work: `2.item`, `2*item`, `all food`. A buy quantity repeats the best matching catalog item (maximum 100); buying all purchases one of every matching catalog row. Sales select carried instances. Any refused item or insufficient funds cancels the entire transaction.

Character `credits` is one nonnegative `u64` integer, default 1,000 for new Savage Soul and pre-credit-field legacy characters; an explicit zero remains zero. Transactions check arithmetic, write credits and the entire inventory to one temporary character JSON file, sync it, and rename it before changing live state. Save failure leaves the trade unapplied. Credits also persist through ordinary movement/disconnect saves. This first version has no merchant purse, stock depletion, bargaining, or healing effects.

## Savage Soul

`ask <person> <question>` and `talk to <person> <question>` route through
`royal_oak::ask`. Tully answers free-form training questions using `trainer.rs`,
including personal costs, affordability, spending caps, progression and Peculiarity
previews from the actual rules. Attribute follow-ups such as "can I afford it?"
remember the prior attribute for that character and NPC. Advice never spends XP.
An optional local model translates natural wording to allowlisted intents in
`trainer_ai.rs`; `trainer.rs` builds every answer from fresh character data.
The worker never blocks Bevy and cannot spend XP or change characters. Scripted
answers remain the fallback when the model is disabled, busy, unsure or offline.
See `tools/trainer-ai/README.md` for one-time setup and service controls. `help ask`
lists examples. Tully's recovered LP training-point prose is updated for the XP system.

The live example runs `CharacterBuildMode::SavageSoul` and `SavageSoulPlugin`: 600-XP character builds, rulebook attribute/derived stats, six supported Peculiarities, carried equipment definitions, d10 tasks, and automatic five-second melee duels. Generic `Character.game_state` persists the game record. Source rules are game-local in `savage_rules.rs`; orchestration lives in `savage_soul.rs`. NPC blueprints accept an optional `soul` profile; without it NPCs receive base scores and 0 purchased XP. See `migration/SAVAGE-SOUL-IMPLEMENTATION.md` for commands, source decisions, tests, and remaining mechanics.

Consumables: `drink water`, `drink bottle`, `drink 2.bottle`, `eat ration`, and `eat food` consume carried supplies. Drinking replaces a full water bottle with an empty bottle; empty bottles cannot be drunk or sold as full bottles. Eating removes a ration pack. Each action saves inventory and its consumption timestamp before applying the change. Water/food do not grant healing or mana.

## Editable help

`src/preferences.rs` handles `screenreader on|off`, `combatlog brief|normal|detailed`,
and `prompt` presets/custom templates. ConfigRegistry also exposes the settings
through `config`. Complete inventory and character preferences save atomically.
Screen-reader mode restores previous display settings when disabled; while enabled
it forces plain text, no ASCII maps, brief combat and a spoken (or disabled) prompt.
`main.rs` enables the optional `AccessibilityOnboarding` question once per character.
Combat rounds emit one message and the telnet transport coalesces prompts per frame.
NPC blueprints may set `combat_behavior.xp_reward`; ordinary rats award 5 XP once per
spawn's defeat. XP is saved before announcing the result. Quest rewards stay separate.

`HelpPlugin` (`src/help.rs`) handles `help`, `help <topic>`, `help topics`, `help commands`, and `help search <words>`. `HelpDir` selects the content directory, default `data/help`. The JSON index and UTF-8 topic files are reread for every request; editing help needs no build or restart. Wizard topics are filtered by the current character role. See `data/help/README.md` for authoring.

## Royal Oak desk

`src/desk.rs` handles `disarm`, object open/close, desk inspection, `get <item> from desk`, and reading the dagger note. The Savage Soul plugin dispatches interactions and ticks injected venom. Character.game_state.royal_oak_desk and complete inventory are saved atomically. The two quest items are granted once per character, independently of other players. Source disarming always succeeds. Venom uses rulebook9.28.1; DF10 is an adaptation of unspecified old poison potency. Mist dagger uses the Combat Knife equipment profile. The wraith/escape quest remains pending.

## South city gate

`gate::guard` observes the existing AttemptWalk event for Royal Oak Wall Gate south only. Opening the visible door cannot bypass the source quest-plus-carried-Soulboy check. `Character.game_state.quests` contains canonical quest names. Northbound return is unrestricted. Gate controls and seventeen south-road rooms plus the charging station and shack are recovered by migration/restore_gate_from_backup.py. The terminal/wraith quest-completion scripts remain pending; no player is granted a quest by this repair.

## Savior of Royal Oak

`royal_oak.rs` intercepts the source Archwraith encounter before generic melee. Wielded mist dagger is consumed; the exact gate key, canonical quest, 10 quest points and source 2000 XP reward commit with complete inventory before the wraith is despawned. Completion is per character and cannot pay twice. The encounter respawns after sixty seconds or world reseeding. No generic combat kill can bypass it. Nurse Sue issues a free Soulboy on entry/login in the medical hub when one is absent; the replacement and pack save atomically. `activate soulboy` displays score and quest information. The old permanent-stat-drain AI is not enabled; ordinary attacks are refused with a clue. Source 60% NPC placement becomes deterministic so quest progression is reliable.

## NPC combat profiles and morale

Ordinary rats use authored profiles from data/savage-soul/npc-profiles.json, applied by restore_marsh_from_backup.py. Small-rodent Size/Mass0 follows the rulebook; Body1 keeps the animal alive, and low attack/leadership/Will Power are game-authored values for the source level1, ten-HP rat. No direct HP conversion is claimed. Defense mode none means the rat does not act like a trained human duelist. `NpcCombatBehavior` controls optional flee_when_injured/flee_after_rounds on blueprints; absent fields leave other NPCs unchanged. Rats try to retreat when hurt or after three rounds, recovering from stun first, and only use existing unblocked exits. This is explicit encounter design, not a tabletop species template. Active Defense remains legal after Ready tokens run out, per rulebook7.2. Player round timing and attributes are unchanged.

## Rat encounter correction

The forced rat retreat configuration has been removed. Known grounded rats advertise `can_be_trampled`; the duel selects Trample when the standing player is more than two Sizes larger. This is an interface policy and room-scale movement abstraction, not a change to Size0 Passive Defense. Trample uses rulebook5.5.8.1 Mass-based Blunt damage, SM0 and Knockdown; the rat chooses the allowed Desperation Dive reaction at Perception DF12. Normal Dodge is not used. Individual cells, preemptive attacks and alternative sidestep AI remain outside this encounter model. `trample`/`stomp` explicitly select it. `attack` stops at incapacitation; `kill` continues until NPC death while skipping unconscious NPC actions. Standard attacks use the optional natural10 continuation when otherwise unable to hit.

Trample retaliation: a ready target now spends its activation on Attack a Moving Character before the trample (rulebook 5.5.8.1/5.8.4). It cannot also make its normal attack that round. Active defense against that reaction does not cancel declared movement; stun and incapacitation still interrupt it. Desperation Dive remains available as a free reaction. Previously always diving after losing initiative could suppress all retaliation.

NPC remains: savage_soul::leave_npc_remains converts dead NPCs into ordinary Object entities with corpse names/keywords/descriptions, removing Creature, Actor, SoulData, merchant and script components. Carried objects fall into the room for normal pickup. Destroyed NPCs leave remains; obliterated NPCs leave no body. Carried corpses use the existing inventory persistence; ground remains follow the existing world reset behavior. Corpse contents remain outside containers; timed decay is described below.

Get-from dispatch lives in savage_soul::get_from: only named desk/drawer targets enter the desk quest handler. Visible ground or carried corpse objects report empty (NPC belongings drop beside them); missing sources and non-containers get ordinary target errors. Corpse selection uses existing target selectors and works after inventory restoration.

Brief combat now emits a single line every round containing attack outcomes and recovery. Misses and non-wounding hits stay visible, preventing a log-detail change from making the running duel appear paused. Normal uses one line per attack; detailed adds initiative and rule calculations.

## Item condition and corpse decay

`src/conditions.rs` owns corpse lifecycle policy. `data/savage-soul/world.json`
sets `corpse_decay_seconds` (default60, range1..86400; loaded at startup).
New settings apply to new corpses; existing saved deadlines do not reset.
The one-second sweep changes relative condition and room descriptions, then
removes expired corpses. Absolute start/expiry seconds survive full inventory
snapshots; carried expiry saves the complete pack before despawning. Legacy
carried corpse initialization is also saved immediately. Save failures retain
the object for retry. Existing corpse names/keywords and pickup targets stay stable.

Generic items can specify `"condition": {"quality": 45}` in object blueprints or
shop StoredObject data. Internal quality is0..100; look shows excellent, good,
worn, badly worn, or ruined. Missing values are excellent. Corpse condition uses
fresh, starting to decay, decaying and badly decayed. Automatic equipment wear,
repair and condition-based prices are not implemented.

## NPC population and movement

`src/population.rs` owns Candidate discovery, Member spawn ownership, managed
Point budgets, replacement cooldowns and movement objectives. Seed captures each
complete NpcBlueprint at its original area/room/ordinal; spawn_npc is reused for
replacement so scripts, merchants and combat profiles are retained. Per-blueprint
spawn settings override profiles in data/savage-soul/population.json. Unconfigured
NPCs remain unmanaged. The Archwraith uses its old timer unless explicitly managed;
quest completion never queues both timers.

Rats: one living member per original point, two in their shared pool,60-second
replacement after death,15-second movement interval, radius2 and home area only.
Population accounting follows members across rooms and excludes corpse objects.
Shared slot cooldowns prevent another point bypassing a death's respawn delay.
Objectives stay/wander/guard_room/guard_npc use normal Move commands; doors,
transition vetoes, notices, combat locks and incapacity apply. Guard objectives
control positioning without inventing automatic hostility. `spawns [filter]`
dispatches from Savage Soul to population::status and is wizard-only. Public
PopulationEnabled lets command tests freeze their fixed encounters; focused
manual-clock tests exercise actual dynamic populations. World reloads reseed
NPCs and reset these runtime timers/positions. See migration/NPC-POPULATIONS.md.

## Medical table regeneration

`src/medical.rs` implements two separate treatments in savsoul:start:
`put/place <corpse> on table` regenerates that NPC, preserving donor condition;
`sit/lie/lay on table` restores the living patient's wounds, fatigue, mana and
needle venom without consuming inventory. Both save complete character/pack
state before publishing changes. XP, credits and carried equipment are unchanged.

Regenerated creatures remain in a containment field for three seconds; the room
then receives a threat announcement and vaporization. They have no scripts,
merchant, XP behavior or spawn membership. begin_battle rejects contained targets
before special quest interception, preventing Archwraith reward duplication.
Vaporization directly despawns them, creating neither rewards nor corpses.

Current generated corpses are NPC-only. Their names select a prototype cached
from seeded candidates; unidentified/legacy names use a basic contained creature.
Same-name variants use the first cached prototype, so this is species reconstruction
rather than persisted exact-instance restoration. Player corpse resurrection is
not implemented. Cache/population discovery is separate from containment and
cannot manage regenerated creatures. Sit/lie treatment is instant rather than
a general persistent-posture system. Commands reject combat, incapacity, expired
corpses and a busy table. The restored source advertises regeneration but has no
effect handler; user-directed behavior defines this adaptation. See help medical-table.

Medical posture uses Character.game_state posture, posture_room (stable room ID) and posture_display. Sitting/lying and stand save SoulData and full inventory atomically. Walking is vetoed while table posture is active in that room; attacks require standing. Teleport cleanup removes stale table posture. Room presence shows seated/lying characters, preserving AFK/linkdead suffixes. stand/stand up clears posture and prone without healing; it requires no active combat.

## Wizard web editor

Game-local Flask/Waitress sidecar: `tools/wizard-editor/server.py`, launched by `start-editor.sh` as the `savsoul-editor` user service. Open http://localhost:4080 with a wizard MUD login to edit authored content. `seed::validate_area_json` and the binary `--validate-area <path>` mode parse proposed blueprints without starting Bevy or running scripts. The editor provides forms/source, optimistic revisions, backups/history and an explicit copyover action. See `tools/wizard-editor/README.md` for supported data, activation and service details.

The wizard editor includes a native SVG world atlas, inferred multi-floor room maps, TTRPG/blueprint styles, topology diagnostics and named exit editing. Its `/api/world` read model is game-local; the GRIM runtime topology/ASCII map is unchanged. See `tools/wizard-editor/README.md#maps-and-exit-editing`.

## Recovered crafting

`src/crafting.rs` handles `craft`, `reduce`, `recipes`, `reductions`, and `resources`. Editable `data/savage-soul/crafting.json` supplies ordinary StoredObject products, ingredients and nearby/carried tools. Transactions save the complete pack before removing exact ingredient instances or spawning output. Source crafting is deterministic; fixed reduction yields are a documented adapter for the old luck/skill system. `src/consumables.rs` consumes catalogued meal products through `eat`/`drink`.

Recovered Morlock machine: `press <color> button` and `get suit from panel` are handled in `src/morlock.rs`; one-time rewards and loot use complete character snapshots. Source audit and route notes: `migration/DEVELOPMENT-ROOMS.md`.

Combat avoidance narration in `src/savage_soul.rs` distinguishes active Dodge, passive armed guard and wild swings; detail settings retain the same resolution and activation economy.

`src/intermud.rs` provides `intermud` / `i3` / `imud`: an optional authenticated gateway plugin for remote channels, tells, MUD/channel discovery and who. Game settings live in `data/intermud.json`; gateway installation and operation are documented in `tools/intermud/README.md`. Network I/O never runs on the ECS update thread.

The Intermud adapter also handles `i3 activity` and `i3 recent [channel]`, with bounded public-channel history stored locally across copyovers; private tells are excluded.

Mudlet payloads and settings live in client_protocols.rs and data/mudlet.json. No account credentials or hidden exits are exported. GMCP uses stable Grim room IDs as strings; mapper scripts must map these IDs to their own numeric rooms. Optional sound test uses the fixed assets/media/protocol-ready.wav served by the editor; links and audio start disabled. See help protocols. Incoming I3 who/finger requests generate wizard-only local audit notices through the pinned gateway patch.

Combat narration lives in combat_prose.rs. Normal logs vary weapon/contact descriptions from resolved attack results; brief remains compact and detailed retains numeric checks. Narrative selection does not roll dice or change combat state.

Grapevine is game-local (grapevine.rs), registered through SavageSoulPlugin, with wizard-only grapevine/gv commands. It uses a bounded WebSocket worker and certificate-verified TLS independently of I3/Zebedee. Authored configuration is data/grapevine.json; Client ID/Secret live only in private Linux state. Run configure-grapevine.ps1, then gv reload. See tools/grapevine/README.md.

Headless fixtures insert DisableGrapevine to prohibit external connections even when using production area blueprints. Authors may insert the same resource to omit this optional network adapter.

### Med-bay character rebuilding

`creator` (`rebuild` / `respec`) stores a per-character draft and applies it only
on `creator confirm` in a configured med bay, out of combat and while capable
of acting. Shared species/training rules calculate the target at the player's
current earned XP. Training is refunded into the draft; previous race-change
fees (`SoulData.rebuild_xp`) remain spent and never count as bought C.

`data/savage-soul/creator.json` configures the 10,000-credit procedure, 400-XP
race change (800 for Android/Replicant), med-bay addresses and crafting-ID body
materials. The default synthetic body uses six steel sheets, four rods and twelve
rivets. Same-race retraining needs neither race-change fee nor replacement parts.
Quotes are persisted; changed economics/settings invalidate the draft. Inventory
materials are rechecked at confirmation. The complete target character and
remaining pack are saved atomically before live credits/build/items change. A
private backup goes outside the character scan directory into rebuild-backups.
Injuries, mana already spent, quests, roles, location and preferences survive.

`--validate-creator <path>` validates creator settings for the wizard editor.

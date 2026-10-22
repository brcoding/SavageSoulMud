# `grim-actor`
> The "beings" of the world (characters, players, placement) and the being-reading command verbs.

**Role:** vertical — actors (beings + their verbs)
**Depends on:** `grim-core`, `grim-world`, `grim-networking`, `grim-color`, `grim-target`

An **actor** is any entity that can act in the world and be placed in a room —
player characters and creatures (mobs). Every being carries the shared `Actor`
base (race/level/gender); a PC additionally carries a `Character` (account
state) and, *while connected*, a `Player`; a mob carries a `Creature` marker.
This crate owns those being components, the flat `StoredCharacter` disk DTO, and
the command handlers that read them. It sits strictly **above** the being-free
`grim-world`: it depends on `grim-world` (room topology + address lookups +
shutdown machinery + `RoomLocation`) and never the reverse.

Replacement seam: this crate is the default D&D being bundle. An author driving
something other than a character (a ship, a party) brings their own being
components where `Character` sits and reuses or ignores `Actor`/`InRoom`.

Entity composition: online PC = `Name + Actor + Character + Player + InRoom`;
linkdead PC = `Name + Actor + Character + Linkdead + InRoom` (no `Player`);
creature = `Name + Actor + Creature + InRoom`. The display **name** lives in the
`Name` component (`grim_core::components::Name`), never on `Character`.

## Components

| Component | File | Purpose |
|---|---|---|
| `Actor` | `src/actor.rs` | Shared "alive thing" base carried by every being (PC + creature): `race`, `level`, `gender`. Movement/perception/WHO read build data here. |
| `Creature` | `src/actor.rs` | Marks a being as a non-player mob (replaces the former `grim_world::Npc`). |
| `Character` | `src/character.rs` | PC-only being belonging to an account; carries `id`, `account_id`, `credits` (a nonnegative whole-credit balance), `created_at`, roles, `class`, `title`, `restrings`, `config` (per-character setting choices, resolved via `grim-config`), `last_room`. No `name`/`race`/`level`/`gender` (→ `Name`/`Actor`). |
| `Player` | `src/player.rs` | Present **only while connected**; links to the live `Connection`. Absence (with `Character`) = linkdead. |
| `OutputHistory` | `src/player.rs` | Bounded ring buffer of recent output lines, for reconnect. |
| `Linkdead` | `src/player.rs` | Character is in-world but its player disconnected (no `Player`). |
| `InRoom` | `src/placement.rs` | Which room an actor currently stands in. |

## Systems

| System | Schedule | File | Purpose |
|---|---|---|---|
| `look::handle_look` | `Update` | `src/commands/look.rs` | Reads `Command::Look`; emits `LookRoom` (no target) or `LookEntity`, else a "not here" `InfoMessage`. Targets resolve via `grim-target` (`BEING` spec: best rank, or Nth with `2.goblin`). |
| `desc::handle_desc` | `Update` | `src/commands/desc.rs` | Reads `Command::Desc`; views/edits the actor's own `Description` paragraphs (`Show` reuses `LookEntity` on self; `Edit` emits `OpenEditor` preloaded). |
| `desc::handle_editor_done` | `Update` | `src/commands/desc.rs` | Reads `EditorDone` for `EditorKind::Description`; replaces the actor's paragraphs on `@save`, confirms the discard on `@exit`. |
| `movement::handle_move` | `Update` | `src/commands/movement.rs` | Reads `Command::Move`; validates the exit, then runs the phased pipeline (a closed visible door refuses as `The … is closed.`; a closed *hidden* door refuses as `You can't go that way.`, identical to no exit) in a queued closure: sync vetoable `AttemptWalk`/`AttemptLeave`/`AttemptEnter` triggers, then placement, `MoveEvent`, auto-look. `Leave`/`Enter` facts queue into `PendingFacts` and fire next tick via `fire_pending_facts` (chained). Refreshes `last_room`. Hidden exits are also stripped from the `map` snapshot (secrets never draw). |
| `movement::handle_goto` | `Update` | `src/commands/movement.rs` | Admin teleport to a room by address (entity/grim id/slug, `area:room`); fires attempt triggers deferred (denial ignored — admin override) and queues facts the same way. Skipped when source == destination. |
| `recall::handle_recall` | `Update` | `src/commands/recall.rs` | Player teleport to the Town Square (`haven:square`); like `goto` but ungated, skipping silently with no `InRoom`. Emits `RecallEvent` for the room echoes plus `LookRoom`; no `MoveEvent`. |
| `quit::handle_quit` | `Update` | `src/commands/quit.rs` | Reads `Command::Quit`; emits `DisconnectRequest` for the player's connection. |
| `title::handle_title` | `Update` | `src/commands/title.rs` | Reads `Command::Title`; sets/clears the actor's title (≤60 chars). |
| `config::handle_config` | `Update` | `src/commands/config.rs` | Reads `Command::Config`; lists registered settings, cycles one, or validates + stores one on `Character.config`. Replies via `InfoMessage`; ignores non-characters. |
| `shutdown::handle_shutdown_command` | `Update` (`grim_world::ShutdownSet::Command`) | `src/commands/shutdown.rs` | Reads `Command::Shutdown`/`Reboot`/`Copyover`; admin-gates the warned countdown (halt / cold restart / hot restart at expiry; state/tick stay in `grim-world`). |

## Commands
Player-facing verbs and where to find their handlers.

| Command | Handler | Summary |
|---|---|---|
| `look [target]` | `src/commands/look.rs` | Describe the current room, or a named entity within it (`self` = you; exact name beats prefix, shortest prefix name wins; `2.goblin` takes the second, `"two words"` needs every word). |
| `map` | `src/commands/map.rs` | Render the area around your room as ASCII (`@` you, `o` rooms, `--`/`\|` exits, `,`/`'` up/down); answers only you. |
| `config [key] [value]` | `src/commands/config.rs` | List settings with values + valid options; bare key cycles to the next value (naming the previous); key + value sets it (validated against `grim-config`, stored on `Character.config`). |
| `desc [clear\|+\|-\|edit]` | `src/commands/desc.rs` | View or edit your description paragraphs (`clear` empties, `+ <line>` appends, `-` drops the last, `edit` opens the line editor). |
| `move` — `n`/`e`/`s`/`w`/`u`/`d` (+ `north`…) | `src/commands/movement.rs` | Walk through an exit; emits `MoveEvent`. Direction aliases parsed in `grim-scene`. Refused at a closed door. |
| `open <direction>` / `close <direction>` | `src/commands/doors.rs` | Flip the exit door (both sides); emits `DoorEvent`. |
| `recall` | `src/commands/recall.rs` | Return to the Town Square (`haven:square`); emits `RecallEvent` (attempt + disappearance left, recall-marked arrival right). No-op with a reply when already there. Available to every player. |
| `goto <address>` | `src/commands/movement.rs` | Admin teleport to a room by address. |
| `quit` | `src/commands/quit.rs` | Request a clean disconnect (save + despawn happen in `grim-scene`). |
| `title [text]` | `src/commands/title.rs` | Set (or, bare, clear) the actor's title; rejected over 60 chars. |
| `shutdown [seconds]` | `src/commands/shutdown.rs` | Admin-only graceful server-shutdown countdown (halt at expiry; default 30s). |
| `reboot [seconds]` | `src/commands/shutdown.rs` | Admin-only warned countdown ending in a non-zero exit, so the service manager restarts the server cold (default 30s). |
| `copyover [seconds]` | `src/commands/shutdown.rs` | Admin-only warned countdown ending in a `CopyoverDue` handoff to a successor process (default 30s). |

## Resources & Events

| Name | Kind (Resource/Message) | File |
|---|---|---|
| `EngineCommand` | Message (input, from `grim-core`) | each `src/commands/*.rs` |
| `InfoMessage` | Message (output, from `grim-core`) | `look`/`map`/`config`/`desc`/`movement`/`recall`/`title`/`shutdown` |
| `LookRoom` / `LookEntity` / `MoveEvent` / `RecallEvent` | Message (world-happening events, **registered by `grim_world::WorldPlugin`**) | emitted by `look`/`movement`/`recall`/`desc` (`Show`) |
| `AttemptLeave` / `AttemptEnter` / `Leave` / `Enter` | Trigger events (**no registration**) | `src/transition.rs`; phased movement vocabulary (Pre attempts, Post facts). Attempts carry a denial latch; consumed by `grim-script` observers. |
| `DisconnectRequest` | Message (from `grim-networking`) | `src/commands/quit.rs` |
| `OpenEditor` / `EditorDone` | Message (output/input, from `grim-core`) | `desc` (`Edit` opens preloaded; `handle_editor_done` applies `@save`) |
| `ServerBroadcast` | Message (**registered by `grim_world::ShutdownPlugin`**) | `src/commands/shutdown.rs` |

## Types
Non-component types this crate defines.

| Type | Kind | File | Purpose |
|---|---|---|---|
| `Role` | serde enum (`Admin`) | `src/character.rs` | A privilege a character holds; stored in `Character.roles` and gates admin verbs. Not an ECS component. |
| `Beings` | query alias | `src/occupants.rs` | Canonical beings shape (identity + `InRoom` + `Name` + addressability markers). New systems declare this instead of inventing a tuple. |
| `beings_in` / `in_room` / `is_addressable` | free functions | `src/occupants.rs` | Shared room-membership predicates: addressable beings in a room (creature, or character with `Player`/`Linkdead`), the generic `(Entity, room)` core for richer tuples, and the addressability rule. `except` covers beings-minus-self; `self` resolves at the parse layer. |

## Notes
- **One plugin:** `ActorPlugin` (`src/plugin.rs`) calls each command's
  `pub(crate) fn register(app)` — the per-command convention: one file per
  command under `src/commands/`, each owning its systems + the messages it
  registers. `commands.rs` and `lib.rs` are declarations + re-exports only.
- **Room membership queries** live in `src/occupants.rs` (see Types above):
  `look` resolves targets via `in_room`, and downstream crates (`grim-object`
  `give`/`steal`, `grim-social` roommates, `grim-channel` audience,
  `grim-scene` broadcasts, `grim-script` watcher) funnel through the same
  helpers instead of hand-rolled `ir.room == room` filters.
- The world-happening events (`LookRoom`/`LookEntity`/`MoveEvent`/`RecallEvent`) are owned by
  `grim_world::WorldPlugin`; the shutdown countdown/signal machinery by
  `grim_world::ShutdownPlugin`. A full stack composes those alongside
  `ActorPlugin` (see `GrimHeadlessPlugins`).
- **`Player` is present only while connected.** On disconnect, `save_on_disconnect`
  (grim-persistence) removes `Player` and inserts `Linkdead`; on reconnect the
  scene inserts `Player` and removes `Linkdead`. So online ⇔ has `Player`,
  linkdead ⇔ has `Character` and no `Player`. Takeover keeps the *live* `Player`
  (whose `connection` differs from the closing one) untouched.
- `Gender` stays in `grim-core` (the session `ClientState` references it);
  `Actor`/`StoredCharacter` point *up* at it. `RoomLocation` now lives in
  `grim-world` (`Character.last_room` and `StoredCharacter.last_room` point up at
  it). See ARCHITECTURE.md §4 and ADR-0001.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

Credit balances persist in `StoredCharacter` as a single `u64` integer. New characters start with 1,000 credits; old files without `credits` default to 1,000. An explicit zero balance stays zero. The Savsoul shop commands are composed by `example-mud`.

`Character.game_state` and `StoredCharacter.game_state` provide a namespaced map of JSON values for game plugins. The engine preserves all entries through login, normal saves, and copyover without depending on game-specific types. Older saves default to an empty map. Savage Soul uses the `savage_soul` key; explicit existing credit balances remain unchanged.

`StoredObject.condition` optionally stores the shared ItemCondition value. Missing fields in existing inventory JSON remain compatible. Condition follows each item instance rather than its name or blueprint.

look falls back from room targets to items carried by the requesting actor, using core::CarriedBy and existing target selectors. It never searches another actor's pack.

ASCII maps and room minimaps share the Savage Soul palette: green `o` rooms, red `@` current location, neutral grey paths, and yellow up/down markers. Topology and secret-exit filtering are unchanged.

`DarkVision` and the `visibility::Visibility` SystemParam provide a shared
room-visibility check for room/entity looks and maps. A placed light or a light
carried by anyone in the room illuminates it. Games reconstruct transient
vision/light markers from their persisted authoring data.

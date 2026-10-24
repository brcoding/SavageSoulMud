# `grim-core`
> Shared primitive types — components, game events, and value types the engine's plugins agree on.

**Role:** horizontal (primitives)
**Depends on:** `grim-color` (re-exported), plus `bevy`, `serde`, `nanoid`, `chrono`.

## Components
Only the primitives this crate still owns. The being components
(`Character`/`Actor`/`Creature`/`Player`/`InRoom`/`OutputHistory`/`Linkdead`) live
in **`grim-actor`**, and the world-topology components (`Area`/`Room`/`Exits`) plus
`RoomLocation` live in **`grim-world`** — see those crates' READMEs.

| Component | File | Purpose |
|---|---|---|
| `Client` | `src/components.rs` | Session state machine, one per connection (holds `ClientState`, account/character links, input queue, cooldown, `last_active`/`idle_warned`/`afk` liveness). |
| `Account` | `src/components.rs` | Persisted account: identifier, password hash, owned character IDs. |
| `Name` | `src/components.rs` | Display name for any visible entity (a being's name lives here, not on `Character`). |
| `Description` | `src/components.rs` | Long description shown by `look <target>`: paragraphs with no embedded newlines, newline-joined at render. |
| `Keywords` | `src/components.rs` | Extra `look <keyword>` words for creatures (prefix-matched, case-insensitive). |
| `RoomDescription` | `src/components.rs` | Room-listing line for a being/object (the creature long line); PCs render from position instead. |

## Systems
None. This crate is type definitions only — no `Plugin`, no `add_systems`, no observers.

## Commands
This crate *defines* the `Command` enum (the closed set of player verbs, `src/events/command.rs`) — the vocabulary only; it registers no handlers. The live `grim::CommandRegistry<Command>` **resource is owned by `grim-scene`** (`parser::command_registry()`, inserted in its `ScenePlugin`), which parses input into `EngineCommand`. Handlers live in the owning gameplay crates — see their READMEs for the per-command file:

| Command(s) | Handler crate → file |
|---|---|
| `look` / `desc` / `move` / `goto` / `quit` / `title` / `shutdown` / `reboot` / `copyover` | `grim-actor` → `src/commands/<name>.rs` (`shutdown.rs` handles all three shutdown verbs) |
| `say` / `yell` / `ooc` / `tell` / `reply` / `gecho` | `grim-channel` → `src/channel.rs` |
| `finger` / `who` / `afk` / `wizlist` / `where` / `sockets` / `commands` / `areas` | `grim-scene` → `src/command.rs` (session-local) |
| `ban` (`BanOp::List` / `Add` / `Remove` over `BanKind::Ip` / `Account` / `Character`) | `grim-scene` → `src/ban.rs` (engine queue; persists + kicks) |

The closed `Command` enum + last-registered-wins registry are documented as defects slated for per-plugin typed dispatch (ARCHITECTURE.md §5.2, §8).

## Resources & Events
This crate owns no `Resource`s — `StartingRoom` moved to `grim-world` in Placement Phase 2a. The events are all `Message` types in `src/events.rs`:

| Name | Kind | File |
|---|---|---|
| `EngineCommand` | Message | `src/events.rs` |
| `LookRoom` | Message | `src/events.rs` |
| `LookEntity` | Message | `src/events.rs` |
| `SayEvent` | Message | `src/events.rs` |
| `YellEvent` | Message | `src/events.rs` |
| `OocEvent` | Message | `src/events.rs` |
| `GlobalEcho` | Message | `src/events.rs` |
| `MoveEvent` | Message | `src/events.rs` |
| `RecallEvent` | Message | `src/events.rs` |
| `InfoMessage` | Message | `src/events.rs` |
| `LoginAnnounce` | Message | `src/events.rs` |
| `LogoutAnnounce` | Message | `src/events.rs` |
| `LinkdeadAnnounce` | Message | `src/events.rs` |
| `ServerBroadcast` | Message | `src/events.rs` |
| `CopyoverDue` | Message (an expired in-game `copyover` countdown asks the telnet transport to hand off) | `src/events.rs` |

- Also defines non-ECS value types: `Command` (`src/events/command.rs`), the `ban` vocabulary `BanKind`/`BanOp` (`src/events.rs`), and `ClientState` (`src/components.rs`), `Gender` (`src/character.rs`), `Cardinal` (`src/cardinal.rs` — variant order is the display order, north/east/south/west/up/down followed by northeast/northwest/southeast/southwest, via derived `Ord`), and `GrimId` (base62 ×12 id, `src/id.rs`). (`RoomLocation` moved to `grim-world` in Placement Phase 2a step 3.) A `prelude` re-exports the common set; `src/color.rs` re-exports `grim-color` so `grim::color::*` keeps resolving.
- The events named `SayEvent`/`MoveEvent`/etc. are **facts only** — there is no attempt/fact split yet, so nothing can veto them (ARCHITECTURE.md §6, §8).
- Called a "god-types crate" in ARCHITECTURE.md §8: colour, `tr`, the command registry, and wire types have already been split out; game events + components are what remain. Expect this crate to shrink over time — improve over time.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

`Command::Help { topic }` carries an editable game-help request; content is owned by the game plugin.

`components::ItemCondition` is optional per-instance quality data with relative labels. `ItemDecay` carries absolute start/expiry seconds; game plugins own decay policy and scheduling. Neither type persists runtime entity IDs.

CarriedBy is the single shared placement type in components.rs, reexported by grim-object. This transitional ownership lets actor look inspect its own pack without an actor-to-object dependency.

## Staff testing visibility

`Account.testing` defaults false for existing saves and is staff-managed. `WizInvis` and `TestAccount` are session-admin command intents.

See [staff testing](../../docs/STAFF-TESTING.md) for permissions and shared-world limitations.

The command vocabulary lives in `src/events/command.rs` and is reexported as
`events::Command`; event messages remain in `src/events.rs`.

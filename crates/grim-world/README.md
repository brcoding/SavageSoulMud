# `grim-world`
> World topology (areas, rooms, exits) + room-address lookups, and the graceful server-shutdown signal/countdown machinery.

**Role:** vertical — world topology (being-free)
**Depends on:** `grim-core`

This crate is **being-free**: it knows the rooms, not who stands in them. The
beings (`Character`, `Player`, `InRoom`, …) and the verbs that read them
(`look`, `move`, `goto`, `quit`, `title`, and the admin `shutdown` gate) live in
`grim-actor`, which depends on this crate — never the reverse. As of Placement
Phase 2a step 2 the `grim-networking` and `grim-color` dependencies were dropped:
their only users (the `quit` handler and the `goto`/`title` escapes) moved into
`grim-actor`.

## Components

| Component | File | Purpose |
|---|---|---|
| `Area` | `src/world/topology.rs` | A named region grouping rooms (yell scope). |
| `Room` | `src/world/topology.rs` | A single location; carries its owning `area`. |
| `Exits` | `src/world/topology.rs` | Directional links from a room to adjacent rooms. |
| `RoomDarkness` | `src/lighting.rs` | Marks a room as unlit unless a light source or viewer ability permits sight. |
| `LightSource` | `src/lighting.rs` | Marks an object as a source of light, independently of its placement. |
| `Door` / `Doors` | `src/world/topology.rs` | A door on one exit side (name/keywords/open/hidden) + the per-room direction map; closed blocks `move`. `hidden` is per-side (never flipped by `open`/`close`), so a one-way secret hides one side only. |

(The `Npc` marker moved to `grim_actor::Creature` — creatures are beings, so the
marker belongs in the actor layer, not the being-free world.)

## Systems

| System | Schedule | File | Purpose |
|---|---|---|---|
| `install_signal_handler` | `Startup` | `src/shutdown.rs` | Installs the SIGTERM handler feeding graceful shutdown. |
| `poll_shutdown_signal` | `Update` (`ShutdownSet::Poll`) | `src/shutdown.rs` | If SIGTERM fired, starts the countdown (unless one is running). |
| `tick_shutdown` | `Update` (`ShutdownSet::Tick`) | `src/shutdown.rs` | Advances `ActiveShutdown`, broadcasts warnings; at zero halts (`AppExit::Success`), exits non-zero for a `reboot`, or emits `CopyoverDue` for a `copyover` (clearing the countdown, no exit). |

`WorldPlugin` (`src/world/plugin.rs`) has no systems; it registers the
world-happening event vocabulary
(`LookRoom`/`LookEntity`/`MoveEvent`/`RecallEvent`) the actor
verbs emit.

## Commands
Player-facing verbs and where to find their handlers.

This crate registers **no command handlers** — they moved to `grim-actor`
(Placement Phase 2a step 2). The `shutdown`/`reboot`/`copyover` verbs share one
handler, `grim_actor::commands::shutdown`; it slots into this crate's
`ShutdownSet::Command`, while the countdown state, ticking, and SIGTERM signal
stay here.

## Resources & Events

| Name | Kind (Resource/Message) | File |
|---|---|---|
| `StartingRoom(Entity)` | Resource | `src/world/topology.rs` |
| `RaceRegistry` / `ClassRegistry` | Resource (creation content) | `src/registry.rs` |
| `ActiveShutdown(ShutdownCountdown)` | Resource | `src/shutdown.rs` |
| `RebootShutdown` | Resource (marker; cold-restart exit at expiry) | `src/shutdown.rs` |
| `CopyoverShutdown` | Resource (marker; handoff at expiry, no exit) | `src/shutdown.rs` |
| `CopyoverDue` | Message (from `grim-core`; expiry asks the telnet transport to hand off) | `src/shutdown.rs` |
| `ShutdownSet` | `SystemSet` (Poll → Command → Tick ordering seam) | `src/shutdown.rs` |
| `ShutdownSignal` | Resource (private; SIGTERM flag) | `src/shutdown.rs` |
| `ServerBroadcast` | Message (shutdown warnings) | `src/shutdown.rs` |
| `LookRoom` / `LookEntity` / `MoveEvent` / `DoorEvent` / `RecallEvent` | Message (registered by `WorldPlugin`; emitted by `grim-actor`) | `src/world/plugin.rs` |

## Types
Non-component types this crate defines.

| Type | Kind | File | Purpose |
|---|---|---|---|
| `RoomLocation` | serde struct (`area`, `room` `friendly_id`s) | `src/world/location.rs` | The stable, entity-independent storage location of a room — survives a reseed. Persisted on `grim_actor::Character.last_room` / `StoredCharacter.last_room`. Relocated here from `grim-core` (Placement Phase 2a step 3) so it sits below its consumers with no cycle. |
| `RoomLookup` | enum (`Found`/`NotFound`/`Ambiguous`) | `src/world/area.rs` | Outcome of resolving a room address. |
| `MapConfig` / `render_map` | pure fn (`center`, topology snapshot, config → canvas rows) | `src/world/map.rs` | ASCII area-map rendering shared by the `map` verb (`grim-actor`) and the look minimap (`grim-scene`): BFS in fixed `Cardinal` order (deterministic, first-claim wins), rooms → connectors (including diagonal `/` and `\`) → up/down markers, clipped to canvas. Glyphs carry `{`-family colour (`@` `{R`, `#` `{w`, links `{8`, up `,` `{Y`, down `'` `{y`, each `{x`-reset; `@` escaped as `@@`). |

## Notes
- Two plugins: `WorldPlugin` (world-event vocabulary) and `ShutdownPlugin`
  (signal + countdown machinery). The admin `shutdown`/`reboot`/`copyover`
  command handler lives in `grim-actor` and slots into `ShutdownSet::Command`;
  chaining the sets means a SIGTERM and a same-tick admin command still
  schedule exactly one countdown.
- Topology types (`Area`/`Room`/`Exits`/`StartingRoom`), `RoomLocation`, and the
  room-address lookups (`resolve_room_address`, `room_location`, `RoomLookup`) are
  hoisted to the crate root — consumers use
  `grim_world::{Area, Room, Exits, RoomLocation, resolve_room_address, …}`.
- Also owns the race/class registry content (`RaceRegistry`, `ClassRegistry`)
  read by `grim-scene`'s creation flow.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

ASCII maps and room minimaps share the Savage Soul palette: green `o` rooms, red `@` current location, white paths, and yellow up/down markers. Topology and secret-exit filtering are unchanged.

Map rendering separates deterministic BFS room placement (`place_rooms`) from
canvas drawing (`render_map`), preserving first-claim collision and exit rules.

`RoomDarkness` opts a room into unlit visibility. `LightSource` marks a lit
object independently of its later placement/ownership. Both are being-free
markers; perception and ownership resolution remain in `grim-actor`.

The visibility markers are defined in `src/lighting.rs` and reexported at the
crate root; `lib.rs` remains a module shell.

# `grim-channel`
> Player-speech channels: `say`, `yell`, `ooc`, `tell`/`whisper`, `reply`, and admin `gecho`.

**Role:** vertical — player speech / communication channels
**Depends on:** `grim-core`, `grim-world`, `grim-actor`, `grim-text`, `grim-target`

## Components
| Component | File | Purpose |
|---|---|---|
| `LastWhisperFrom(Entity)` | `src/whisper.rs` | Records the last player who whispered this entity, so `reply` can target them. |

## Systems
| System | Schedule | File | Purpose |
|---|---|---|---|
| `handle_say` | `Update` | `src/commands/say.rs` | Reads `Command::Say`, emits `SayEvent` for the room plus a first-party `InfoMessage` echo. |
| `handle_yell` | `Update` | `src/commands/yell.rs` | Reads `Command::Yell`, emits `YellEvent` scoped to the actor's `Area`. |
| `handle_ooc` | `Update` | `src/commands/ooc.rs` | Reads `Command::Ooc`, emits `OocEvent` (out-of-character global). |
| `handle_gecho` | `Update` | `src/commands/gecho.rs` | Reads `Command::Gecho`, emits `GlobalEcho`. Re-checks admin (defense in depth). |
| `handle_tell` | `Update` | `src/commands/tell.rs` | Reads `Command::Tell`, rank-matches the target player (`grim-target`: exact beats prefix), delivers a private whisper. |
| `handle_reply` | `Update` | `src/commands/reply.rs` | Reads `Command::Reply`, whispers the entity's `LastWhisperFrom`. |
| `handle_channel` | `Update` | `src/handler.rs` | **Data-driven** unified channel handler - reads `EngineCommand`, dispatches to `ChannelMessage` based on channel config. |

## Commands
Player-facing verbs and where to find their handlers.
| Command | Handler | Summary |
|---|---|---|
| `say <text>` | `handle_say` (`src/commands/say.rs`) | Broadcast to the current room; echoes "You say, '…'" to the speaker. |
| `yell <text>` | `handle_yell` (`src/commands/yell.rs`) | Broadcast to every room in the actor's area. |
| `ooc <text>` | `handle_ooc` (`src/commands/ooc.rs`) | Out-of-character global chat. |
| `tell <target> <text>` | `handle_tell` (`src/commands/tell.rs`) | Private message to one player (exact name beats prefix, shortest wins; `self` targets sender). |
| `whisper <target> <text>` | `handle_tell` (`src/commands/tell.rs`) | Alias for `tell` (parsed in `grim-scene`). |
| `reply <text>` | `handle_reply` (`src/commands/reply.rs`) | Whisper the last player who whispered you (`LastWhisperFrom`). |
| `gecho <text>` | `handle_gecho` (`src/commands/gecho.rs`) | Admin-only world-wide echo. Gated at dispatch in `grim-scene`; re-checked here. |

## Resources & Events
| Name | Kind (Resource/Message) | File |
|---|---|---|
| `ChannelRegistry` | Resource | `src/registry.rs` | Stores registered channel configurations (name, scope, eligibility). |
| `ChannelMessage` | Message | `src/message.rs` | **Data-driven** unified channel event - one shared event with data-driven scope/resolution. |
| `GlobalEcho` | Message (consumed) | `src/commands/gecho.rs` |
| `EngineCommand` | Message (input) | `src/handler.rs`, `src/commands/*.rs` |
| `InfoMessage` | Message (output echo) | `src/handler.rs`, `src/commands/*.rs` |
## Functions
| Name | File | Purpose |
|---|---|---|
| `resolve_audience_mapped` | `src/registry.rs` | Scope → audience from pre-mapped `(Entity, room)` pairs: room arm via `grim-actor::in_room`, area arm via a `same_area` closure (fail-closed on missing `Room`), global arm is everyone (`grim-scene` delegates to it). |

## Configuration
Channels are **data**, not code. Register channels via `ChannelRegistry`:

```rust
use grim_channel::{Channel, Scope, Identify, SpeakEligibility, ListenEligibility, ChannelRegistry};

app.add_plugins(ChannelPlugin);
// ChannelPlugin initializes ChannelRegistry with say/yell/ooc by default

// Add a custom channel
app.world_mut().resource_mut::<ChannelRegistry>().add_channel(Channel {
    name: "gossip".to_string(),
    scope: Scope::Global,
    identify: Identify::Always,
    toggleable: true,
    speak: SpeakEligibility::Authenticated,
    listen: ListenEligibility::Authenticated,
    key: "channel.gossip".to_string(),
});
```

**Axes:**
| Axis | Values | Note |
|------|--------|------|
| `scope` | `Room`, `Area`, `Global` | Earshot only — see ARCHITECTURE.md §7 |
| `identify` | `Perceived` / `Always` | `Perceived` uses in-world sound; `Always` is OOC |
| `toggleable` | bool | Per-player subscription state, owned by `grim-persistence` |
| `speak` | predicate | Who may send |
| `listen` | predicate | Who may receive — separate from `speak` |
| `key` | Catalog prefix | Resolves `.actor` / `.observer` |

## Notes
- One file per command under `src/commands/` (mirrors `grim-actor`'s convention); `src/plugin.rs` wires each command's `register(app)` into `ChannelPlugin`, and `src/whisper.rs` holds the `LastWhisperFrom` component plus the `deliver_whisper` helper shared by `tell`/`reply`.
- Command *parsing* (including the `whisper` alias) lives in `grim-scene`'s parser; this crate only handles resolved `Command` variants delivered via `EngineCommand`.
- Rendering / per-recipient attribution is the renderer's job (`grim-scene`'s `format_output`), not this crate's — handlers emit intent events only.
- **Data-driven channels (ARCHITECTURE.md §7):** The `ChannelMessage` event replaces the distinct `Say/Yell/Ooc` events. Channel behavior (scope, audience, formatting) is determined by the channel configuration stored in `ChannelRegistry`.
- `gecho` admin gating is enforced at dispatch (`grim-scene`) and re-verified in `handle_gecho` — fails closed (silent) for non-admins.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

## Staff testing visibility

Tell/reply resolve only mutually visible participants before updating reply history. `ChannelMessage.origin` / `hidden_origin` retain causal privacy for script-triggered NPC speech; ordinary channel messages leave them empty/false.

See [staff testing](../../docs/STAFF-TESTING.md) for permissions and shared-world limitations.

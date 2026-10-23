# `grim-object`
> Things beings can pick up, carry, and drop.

**Role:** vertical — things
**Depends on:** `grim-core`, `grim-actor`, `grim-text`, `grim-target`

An object is `Object + Name + Keywords + RoomDescription (+ InRoom while on the
ground, + CarriedBy while carried — never both)`. `Name` is the short name
(inventory rows, pickup lines); `RoomDescription` is the long line under the
room description; `Keywords` feeds target matching via `grim-target` (`look`'s ranking + selectors).

## Components
| Component | File | Purpose |
|---|---|---|
| `Object` | `src/object.rs` | Marker for a pickable thing. |
| `CarriedBy { carrier }` | `src/object.rs` | Carrier link; present instead of `InRoom`, never alongside it. |

## Types
| Type | Kind | File | Purpose |
|---|---|---|---|
| `Ground` | query alias | `src/ground.rs` | Canonical ground shape (`Object + InRoom + Name + Keywords`, `Without<CarriedBy>`). New systems declare this instead of inventing a tuple. |
| `ground_in` | free function | `src/ground.rs` | Every ground object in a room — the one ground-membership definition (`get` routes through it). |

## Systems
| System | Schedule | File | Purpose |
|---|---|---|---|
| `handle_get` | `Update` | `src/commands/get.rs` | Reads `Command::Get`; swaps `InRoom`→`CarriedBy`, emits `ItemEvent::Pickup`, else a "not here" `InfoMessage`. |
| `handle_drop` | `Update` | `src/commands/get.rs` | Reads `Command::Drop`; swaps `CarriedBy`→`InRoom`, emits `ItemEvent::Drop`, else a "not carrying" `InfoMessage`. |
| `handle_inventory` | `Update` | `src/commands/inventory.rs` | Reads `Command::Inventory`; emits an `InfoMessage` listing carried shorts (sorted) or the empty line. |
| `handle_give` | `Update` | `src/commands/give.rs` | Reads `Command::Give`; moves the match into a PC's pack, emits `TransferEvent::Give`. Creatures refuse; misses answer directly. |
| `handle_steal` | `Update` | `src/commands/steal.rs` | Reads `Command::Steal`; moves the victim's match into the thief's pack, emits `TransferEvent::Steal`. Existence checks only. |

## Commands
Player-facing verbs and where to find their handlers.
| `get <target>` | `get::handle_get` | Pick up matches from the ground (`2.coin` 2nd, `3*coin` three, `all [words]` all; one `ItemEvent` each). |
| `drop <target>` | `get::handle_drop` | Drop matches from the pack into the room (same selectors; one `ItemEvent` each). |
| `inventory` / `inv` | `inventory::handle_inventory` | List carried short names. |
| `give <item> <who>` | `give::handle_give` | Hand matches to a PC here (creatures refuse; one `TransferEvent` each). Item selectors: `2.coin` (2nd), `3*coin` (three), `all [words]` (all); quoted phrases stay together. |
| `steal <item> <who>` | `steal::handle_steal` | Take matches from a being here (same selectors on the item side). |

## Resources & Events
| Name | Kind (Resource/Message) | File |
|---|---|---|
| `ItemEvent` | Message (emitted; `grim-core`) | `src/commands/get.rs` |
| `TransferEvent` | Message (emitted; `grim-core`) | `src/commands/give.rs`, `src/commands/steal.rs` |
| `EngineCommand` | Message (consumed) | `src/commands/` |
| `InfoMessage` | Message (emitted) | `src/commands/` |

- Room membership: `get` resolves matches via `ground_in` (`src/ground.rs`);
  being targets resolve via `grim-actor::beings_in` (the canonical `Beings`
  shape). No hand-rolled `ir.room == room` filters.
- Rendering is per-recipient in `grim-scene` (`format_item_events`, `format_transfer_events`): the mover sees first-party, the other party second-party, the room third-party — every line names both parties and the item.
- Room listings show ground objects' `RoomDescription` under the creatures (`grim-scene`); `look <being>` appends their pack below the description (`format_look_pack`).
- Packs persist whole per instance (`persist`, into the character file's `inventory`); ground objects regenerate from blueprints, so post-reboot duplicates are correct state.


Pack persistence includes optional ItemCondition. Held/Carried queries snapshot it; restore_bundle preserves quality and absolute decay timestamps (default condition for legacy items). Picking up, dropping or transferring swaps placement only and never resets condition.

CarriedBy is reexported from grim-core::components; existing grim_object::CarriedBy callers use the same component type.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

# `grim`
> The engine facade: re-exports every subsystem and offers the default plugin group so a MUD author depends on one crate.

**Role:** horizontal (infrastructure)
**Depends on:** `grim-core`, `grim-text`, `grim-command`, `grim-target`, `grim-command-events`, `grim-networking`, `grim-networking-telnet`, `grim-scene`, `grim-auth`, `grim-world`, `grim-actor`, `grim-channel`, `grim-object`, `grim-persistence`, `grim-script`

## Components

| Component | File | Purpose |
|---|---|---|
| None | — | Composition-only facade; defines no components. Re-exports subsystem types (see `src/lib.rs`, `src/prelude.rs`). |

## Systems

| System | Schedule | File | Purpose |
|---|---|---|---|
| None | — | — | Registers no systems of its own; the plugin groups compose subsystem plugins. |

## Commands
Player-facing verbs and where to find their handlers.

| Command | Handler | Summary |
|---|---|---|
| None | — | Commands live in the subsystem crates, not the facade. |

## Resources & Events

| Name | Kind (Resource/Message) | File |
|---|---|---|
| None | — | Re-exported only. Resources/events are defined in the subsystem crates. |

## Plugin groups

| Group | File | Purpose |
|---|---|---|
| `GrimHeadlessPlugins` | `src/plugin_groups.rs` | Whole engine **except a transport** (networking wiring + world + actor + channel + persistence + scene + auth). What a headless test harness composes. |
| `GrimDefaultPlugins` | `src/plugin_groups.rs` | `GrimHeadlessPlugins` plus the telnet transport (`telnet_port`, default 4000). The full stack a MUD author gets for free. |

## Notes
- `LoginVersion` re-exports the auth resource for a binary to supply its login build identity.
- Facade / composition-only. It depends on the subsystem crates, re-exports their public surface, and bundles them into plugin groups — nothing here is privileged.
- `prelude` (`src/prelude.rs`) gives downstream authors a single `use grim::prelude::*;` surface; `src/plugins/mod.rs` re-exports each plugin so `grim::plugins::WorldPlugin` etc. resolve.
- For what a subsystem actually does, read that crate's own README (e.g. `grim-world`, `grim-scene`, `grim-persistence`).
- Placement Phase 1 relocated single-owner domain types into their owning crates; the facade re-exports them at the crate root so `grim::X` stays stable downstream.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

`AttemptWalk` is re-exported from grim-actor so game plugins can guard recovered callback exits using the existing synchronous observer seam.

`SceneSystems` is re-exported so game plugins can publish public presence after
staff visibility has been reconciled and staff commands have run.

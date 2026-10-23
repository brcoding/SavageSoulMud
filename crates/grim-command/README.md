# `grim-command`
> Maps the first word of a line to the command that registered it, resolving abbreviations by prefix.

**Role:** horizontal (infrastructure)
**Depends on:** `bevy` (no workspace crates)

## Components
None.

## Systems
None. This crate is a passive resource + registry; it schedules no systems and installs no plugin.

## Commands
Registers no game commands itself. It owns the `CommandRegistry` — the infrastructure where a caller registers commands via `CommandRegistry::register(name, factory)`. It is generic over the produced command type `C`, so it carries no game vocabulary.

| Command | Handler | Summary |
|---|---|---|
| None | — | Game verbs live in the callers that populate the registry. |

## Resources & Events
| Name | Kind (Resource/Message) | File |
|---|---|---|
| `CommandRegistry<C>` | Resource | `src/registry.rs` |
| `Contest` | plain struct (report type, not a Resource/Message) | `src/registry/contest.rs` |

## Notes
- Resolution order: exact match wins outright; otherwise the highest-priority prefix match. Case-insensitive.
- Priority is an explicit ordering, not a number. `register` inserts at the **front** (highest), so the last command to claim a prefix wins by default. `prioritize` / `deprioritize` move a name to front / back.
- `replace(name, factory)` changes an existing factory while preserving its priority and listing section; returns false if absent.
- Factories are `fn(&str) -> Option<C>`; returning `None` rejects the input (e.g. `say` with no text).
- `contested_prefixes()` returns every prefix more than one command answers to, each with its `winner` and the `shadowed` commands, so ambiguity can be logged at startup rather than discovered by a confused player. A prefix that is itself an exact command name is not reported.
- Generic over `C: Send + Sync + 'static` so it drags in no game types — only Bevy, to be usable as a resource.
- See `docs/ARCHITECTURE.md` §5.2 (Command — a line of input becomes an intent).

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

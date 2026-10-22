# `grim-networking`
> Transport-agnostic networking primitives: the `Connection` component and the wire messages every transport speaks in terms of.

**Role:** horizontal (infrastructure)
**Depends on:** `bevy`, `serde` (no workspace crates)

## Components
| Component | File | Purpose |
|---|---|---|
| `Connection` | `src/connection.rs` | A live transport-level link. Carries the transport-side `id`, peer `addr`, and `echo_hidden` (whether the server has hidden input, e.g. for a password). Spawned by a transport; knows nothing about the game. |

## Systems
| System | Schedule | File | Purpose |
|---|---|---|---|
| — | — | `src/plugin.rs` | `GrimNetworkingPlugin` registers the wire message types only. It schedules no systems — a transport crate (or a headless test harness) drives I/O on top of these messages. |

## Commands
None. This crate carries no game vocabulary.

| Command | Handler | Summary |
|---|---|---|
| None | — | — |

## Resources & Events
| Name | Kind (Resource/Message) | File |
|---|---|---|
| `ConnectionEstablished` | Message (transport → session) | `src/messages.rs` |
| `ConnectionInput` | Message (transport → session) | `src/messages.rs` |
| `ConnectionClosed` | Message (transport → session) | `src/messages.rs` |
| `ConnectionOutput` | Message (session → transport) | `src/messages.rs` |
| `DisconnectRequest` | Message (session → transport) | `src/messages.rs` |
| `WiznetAlert` | Message (operational alert for admins; produced anywhere, broadcast by `grim-scene`) | `src/wiznet.rs` |
| `ConnectionResumed` | Message (transport → session, copyover) | `src/copyover.rs` |
| `HandoverManifest` | serde payload struct (not a Bevy type) | `src/copyover.rs` |
| `HandoverEntry` | serde payload struct (not a Bevy type) | `src/copyover.rs` |
| `WiznetCategory` | Value (`Security`, `Logins`) | `src/wiznet.rs` |
## Notes
- `GrimNetworkingPlugin` only registers the six message types. A headless test harness registers this plugin and injects `ConnectionInput` / drains `ConnectionOutput` directly, with no transport at all.
- `WiznetAlert` is deliberately *not* registered here: each producer/consumer (`TelnetPlugin`, `AuthPlugin`, `ScenePlugin`) registers what it touches so every combination stands alone. Emit via `admin_log!(writer, category, fmt, …)`.
- `ConnectionOutput` carries optional `echo` (toggle terminal echo before the text, for password masking) and `prepend_newline` (avoid landing unsolicited events on the user's prompt line). Use `ConnectionOutput::new(entity, text)` for the common case.
- Copyover (hot restart): `ConnectionResumed` is raised for a re-adopted socket so the session layer places `character` straight back into the world, skipping login. `HandoverManifest` is the serde payload sent alongside the live socket fds; entry index *i* pairs with the *i*-th fd, so no transport-side id needs to survive the restart. See `docs/DEPLOY.md`.
- Per `docs/ARCHITECTURE.md` §5.1, this crate is intended to own the tokio bridge once a second transport exists; for now the bridge still lives in `grim-networking-telnet`.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

TerminalSize stores negotiated columns/rows independently of a client protocol. HandoverEntry optionally carries the original peer address for trusted proxy connections across copyover; older manifests remain compatible.

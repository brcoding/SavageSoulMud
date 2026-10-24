# `grim-networking-telnet`
> Telnet transport for GRIM: TCP accept loop on a tokio thread, bridged to Bevy, with IAC negotiation, ANSI rendering, and copyover fd handoff.

**Role:** horizontal (infrastructure) — transport
**Depends on:** `grim-networking`, `grim-color`, `grim-core`, `grim-actor`, `bevy`, `tokio`, `serde_json`, `signal-hook`, `sendfd`, `sd-notify`, `rustix`

Connection setup, socket reading and bounded line assembly live in
`src/bridge/connection.rs`; proxy listener parsing lives in `src/server/proxy.rs`.
Protocol negotiation and prompt token substitution have dedicated helpers so
their existing framing, limits and escaping remain independently readable.

## Components
Reuses `grim_networking::Connection`. `TelnetCapabilities` (`src/protocols.rs`)
stores negotiated options, terminal dimensions/types, whitelisted environment
properties and GMCP package declarations. NAWS also updates the transport-neutral
`grim_networking::TerminalSize` component for room wrapping.

## Systems
| System | Schedule | File | Purpose |
|---|---|---|---|
| `install_copyover_signal` | Startup | `src/copyover.rs` | Install the `SIGUSR2` handler that flips the copyover flag. |
| `start_telnet_server` | Startup | `src/server.rs` | Spawn the detached tokio thread: adopt inherited fds from a copyover predecessor or bind fresh, signal systemd readiness, run the accept/command `select!` loop. |
| `drain_creations` / `drain_dependents` | Update (chained) | `src/drain.rs` | Two-phase drain: creations spawn `Connection` entities first (applied before the next system), then input/close/shed resolve against them — a same-tick connect+input never drops. |
| `send_network_commands` | Update (chained) | `src/bridge.rs` | Read `ConnectionOutput` / `DisconnectRequest` and route them back to the network thread (render + echo toggle + disconnect). |
| `send_protocol_commands` | Update (chained) | `src/bridge.rs` | Deliver negotiated `TelnetOutput` subnegotiations, or explicit trusted server markup. |
| `poll_copyover_signal` | Update (chained) | `src/copyover.rs` | On a raised `SIGUSR2` flag, snapshot in-game sessions into a `HandoverManifest` and start the handoff to the successor. |
| `trigger_copyover_on_due` | Update (chained, before poll) | `src/copyover.rs` | On an in-game `copyover` countdown's expiry (`CopyoverDue`), raise the same latched flag `SIGUSR2` raises. |
| `finish_copyover` | Update (chained) | `src/copyover.rs` | Once the successor acks the handoff, emit `AppExit` to exit the predecessor cleanly. |

## Commands
None. This is a transport; it produces `ConnectionInput` messages, not game commands.

| Command | Handler | Summary |
|---|---|---|
| None | — | — |

## Resources & Events
| Name | Kind (Resource/Message) | File |
|---|---|---|
| `TelnetSystems::Send` | SystemSet (protocol frames flush before text/disconnect output) | `src/plugin.rs` |
| `TelnetPort` | Resource | `src/bridge.rs` |
| `TelnetLimits` | Resource (line/buffer/rate/connect/shed caps, seconds for time; author-overridable) | `src/limits.rs` (`TelnetPlugin::with_limits`) |
| `NetworkBridge` | Resource (crate-internal) | `src/bridge.rs` |
| `TelnetInput` / `TelnetOutput` | Message (`connection`, `option`, `payload`) | `src/protocols.rs` |
| `CopyoverSignal` | Resource (crate-internal) | `src/copyover.rs` |
| `CopyoverDone` | Resource (crate-internal) | `src/copyover.rs` |

Wire messages (`ConnectionEstablished`, `ConnectionInput`, `ConnectionClosed`, `ConnectionOutput`, `ConnectionResumed`, `DisconnectRequest`) are defined in `grim-networking`; `TelnetPlugin` re-registers them so the transport can be used standalone.

## Notes
The local Savsoul installation binds to `127.0.0.1` by default. Set `GRIM_BIND_HOST` explicitly when configuring a different listen address.


- `TelnetPlugin::new(port)` inserts `TelnetPort` plus default `TelnetLimits`, inits `CopyoverSignal` / `CopyoverDone`, and schedules the Startup + Update systems. `with_limits` overrides the caps. The six Update systems are `.chain()`-ed in order: drain → send protocol commands → send → bridge copyover-due → poll copyover → finish copyover.
- Input guards (`src/guard.rs`, enforced in the read task before any byte reaches Bevy): lines past `max_line_len` truncate-and-deliver with the remainder discarded to the newline; a line past `max_buffer` without a newline, or more than `max_lines` per `rate_window_secs`, drops the connection with a `GuardTrip` reason logged alongside the peer address.
- Accept shed (`src/shed.rs`): past `max_connects_total` accepts per `total_window_secs` the listener sheds (accept-and-close, no handshake) for `shed_secs`; transitions cross to Bevy as `Shed` events (entry, ≤1/min heartbeats, lazy exit). Copyover re-adoptions bypass the gate.
- Drain alerts (`src/drain.rs`): connects/closes/resumes report `Logins` with the peer address; guard trips and shed transitions report `Security`. The read task stays Bevy-free — trips surface as `Disconnected { reason }`.
- Bevy's schedule is synchronous and tokio owns its threads, so the two are joined by a channel seam (`NetworkBridge`), not by running async TCP on Bevy's executor. See `docs/ARCHITECTURE.md` §5.1.
- IAC (`src/iac.rs`): a streaming decoder survives arbitrary TCP fragmentation,
  doubles/undoubles literal IAC bytes and keeps subnegotiation payloads out of
  line input. Malformed or over-8-KiB subnegotiations close the connection. A
  separate 128-frame/second negotiation quota prevents an out-of-band flood
  from bypassing ordinary input limits. Password ECHO remains controlled by
  `ConnectionOutput.echo`; optional negotiation never changes it.
- Negotiation (`src/protocols.rs`): SGA, EOR, CHARSET (UTF-8), TTYPE/MTTS, NAWS,
  NEW-ENVIRON/MNES, GMCP, MSDP, MSSP, MSP, MXP and ATCP. Duplicate acknowledgements
  and refusals do not loop. TTYPE requests stop after three distinct responses
  or an early repeated response. Environment requests whitelist terminal/client
  settings; USER, PASSWORD and client-supplied IPADDRESS are never retained.
- `TelnetInput` uses an empty payload when GMCP/MSDP/MSSP/MSP/MXP/ATCP is first
  negotiated, allowing the author plugin to send initial truthful game data.
  Subsequent GMCP/MSDP/ATCP/MSP/MXP payloads are delivered separately from game
  input. GMCP Core.Hello and Core.Supports update capabilities; game packages
  remain author-owned. Payload size and enabled-option checks also apply on
  output; the transport owns IAC escaping. EOR uses the actual end-of-record
  command after in-game prompts rather than a subnegotiation.
- Ordinary output escapes MXP metacharacters when MXP is active and neutralizes
  MSP media tags when MSP is active. `TelnetOutput { option: RAW_MARKUP, .. }`
  is a trusted author-only wire API for authored MXP/MSP/OSC-8 content: it rejects
  IAC and oversized payloads. Never pass player/remote text directly to it.
- Copyover sockets reset and renegotiate optional protocols; no compression
  stream is enabled. Parser fragments and partial input lines are not persisted
  through handoff, so copyover should occur between commands. Fully consumed
  protocol frames cannot leak into game input, and password echo is preserved.
- Rendering (`src/render.rs`): prepend a newline for unsolicited events, append the in-game `> ` prompt (`<AFK> ` while the session's `Client.afk` is set), convert colour codes to ANSI (via `grim-color`), translate `\n` → `\r\n`.
- Copyover / hot restart (`src/copyover.rs`, `src/server.rs`): `SIGUSR2` — or an admin's in-game `copyover [seconds]` countdown expiring into `CopyoverDue` — hands the live listener + in-game client sockets to a freshly-spawned successor over a unix socket (`SCM_RIGHTS`, via `sendfd`), waits for the ack, then exits. The `GRIM_COPYOVER_SOCK` env var tells a successor to adopt fds instead of binding fresh. See `docs/DEPLOY.md`.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

Server-initiated disconnects stop reads immediately and drain queued output before closing; a two-second timeout aborts blocked writers. This preserves login-failure explanations and still guarantees socket closure across copyovers.


Per-character output preferences are persisted in `Character.config`: `screenreader=on` forces plain output and a spoken `Ready.` prompt (or silence with `prompt=off`); `colors=off` independently removes ANSI. Room rendering omits minimaps and debug IDs in screen reader mode, and `map` lists exits instead of drawing a diagram. Descriptions, doors and room occupants remain available.

Prompt presets are `classic`, `compact` (default `[OK] > `; injury abbreviations BI/I/SI/CI and U/S/W/F condition flags), `status`, `combat`, `accessible`, `off`, and `custom`. Custom templates live in `game_state.prompt_template` and accept `{name}`, `{condition}`, `{xp}`, `{credits}`, `{mana}`, `{maxmana}`, `{opponent}`. The game supplies `savage_soul`, `prompt_maxmana`, and `combat_opponent` values; the transport does no rules calculations. Inserted values are escaped. In-game messages for one connection in a Bevy frame are combined before one prompt, reducing repeated prompt chatter. Echo negotiation and final disconnect messages remain delivered.

Grim deliberately does not negotiate SUPPRESS-GO-AHEAD (SGA): it accepts complete command lines. Combining SGA with temporary password ECHO caused Mudlet to report character-at-a-time mode. ECHO still masks password prompts, and EOR provides negotiated prompt boundaries. After applying this change, reconnect Mudlet once to clear its earlier SGA detection state.

TelnetSystems::Send orders authored protocol frames before ordinary output and
disconnect, allowing Char.Login.Result to reach clients before a failed sign-in
closes the socket. Authentication remains owned by grim-auth.

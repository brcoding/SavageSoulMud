# `grim-scene`
> The session subsystem: in-game input parsing/routing, output formatting + per-recipient broadcast, admin-gated dispatch, and copyover resume.

**Role:** horizontal (infrastructure) — session / networking↔command bridge
**Depends on:** `grim-core`, `grim-networking`, `grim-command`, `grim-text`, `grim-color`, `grim-persistence`, `grim-world`, `grim-actor`

The pre-game phase (login / account-creation / character-select / MOTD) lives in
`grim-auth`, which layers on this crate (auth → scene). `grim-scene` routes only
lines whose session stack tops at `InGameScene`; stackless sessions are handled
by `grim-auth`.

## Components

| Component | File | Purpose |
|---|---|---|
| `ConnectedAt(DateTime<Utc>)` | `src/session.rs` | When this session connected (used by the WHO list ordering). |
| `SceneStack(Vec<Entity>)` | `src/scene_stack.rs` | Ordered scene stack on the session; input is interpreted by the top. Thin slice: only the in-game scene exists — pushed once at world entry (auth transition guard, copyover resume), read by the router, cascaded on despawn. |
| `InGameScene` | `src/scene_stack.rs` | Marker on the scene entity topping an in-world session. Missing/empty stack reads as not-in-game (fail closed). |
| `Client` | (`grim-core::components`) | Per-connection session state; consumed here, defined upstream. |
| `ClientState` | (`grim-core::components`) | Login/creation/in-game state machine; the in-game arm drives this crate, the pre-game arms drive `grim-auth`. Still the pre-game driver — the stack mirrors only world entry until ADR-0003 lands fully. |
| `EditorSession` | `src/editor.rs` (state on `Client::editor`, `grim-core`) | Modal line-editor state (character, kind, buffer). A plain field — not a component — so opening/closing is visible to later lines in the same tick. |

## Systems

| System | Schedule | File | Purpose |
| `touch_sessions_on_input` | `Update` (`SceneSystems::TouchInput`, before both dispatchers) | `src/idle.rs` | Stamps `Client::last_active`, resets the idle-warn latch, and silently clears `afk` on every input line (pre-game, in-game, editor). |
| `check_idle` | `Update` (after `SceneSystems::InGameInput`) | `src/idle.rs` | Auto-flags quiet in-game sessions AFK (they stay connected unless `IdleConfig::disconnect_ingame_idle` opts in); warns then severs quiet pre-game sessions via `DisconnectRequest` → the normal linkdead path. |
| `handle_connection_resumed` | `Update` | `src/resume.rs` | Re-attaches a session after copyover / reconnect (skips login). Refuses banned IPs/characters/accounts first (`refuse_banned`), before spawning or attaching anything. |
| `handle_ban_command` | `Update` | `src/ban.rs` | Admin `ban list`/`add`/`remove` off the engine queue (defense-in-depth admin re-check): lists, persists to `bans.json`, and kicks every matching live session on `add`. |
| `handle_wiznet` | `Update` | `src/wiznet.rs` | Admin `wiznet [on\|off\|security\|logins]` off the engine queue (defense-in-depth admin re-check): lists and flips the persisted wiznet prefs. |
| `broadcast_wiznet` | `Update` | `src/wiznet.rs` | Fans `WiznetAlert` plus login/logout/linkdead notices to online admins whose master + category prefs are on, with best-effort socket details. |
| `format_output` | `Update` | `src/output.rs` | Renders domain events per-recipient into `ConnectionOutput`. |
| `format_recall` | `Update` | `src/recall_output.rs` | Renders `RecallEvent` per-recipient (attempt + disappearance left, recall-marked arrival right). Separate system: `format_output` is at Bevy's parameter ceiling. |
| `format_server_broadcast` | `Update` | `src/output.rs` | Renders `ServerBroadcast` (e.g. shutdown warnings) to all sessions. |
| `capture_output` | `Update` | `src/output.rs` | Collects output for flushing to connections. |
| `open_editor` | `Update` | `src/editor.rs` | Attaches `EditorSession` on `OpenEditor` and shows the numbered entry view. |

## Commands
Parsed by `grim-scene`'s registry (`src/parser.rs`); these verbs are handled **session-locally** in `src/command.rs` (they never reach the engine queue).

| Command | Handler | Summary |
| `read <target>` | parser → look handler (`src/parser.rs`, `grim-actor/src/commands/look.rs`) | Read a sign or other named fixture using its description. |
| `desc …` | parser → engine queue (`src/parser.rs`, `grim-actor/src/commands/desc.rs`) | View/edit your description paragraphs (`clear`, `+ <line>`, `-` drops last, `edit` opens the line editor). |
| `who` | `handle_ingame` → `format_who` (`src/who.rs`) | List online characters (admins first, then level/connect/name); linkdead characters are excluded (no live session — still visible in rooms via `look`); AFK sessions carry an `(AFK)` marker. |
| `afk` | `handle_ingame` → `flag_afk` (`src/command.rs`) | Flag yourself AFK (auto-set after `IdleConfig::afk_after_secs` idle; any input clears; prompt becomes `<AFK> `). |
| `wizlist` | `handle_ingame` → `format_wizlist` (`src/wizlist.rs`) | List every admin: online rows plus `(offline)` rows from the `WizlistAdmins` startup snapshot (`load_wizlist_admins`). |
| `desc …` | parser → engine queue (`src/parser.rs`, `grim-actor/src/commands/desc.rs`) | View/edit your description paragraphs (`clear`, `+ <line>`, `-` drops last). |
| `sockets` | `handle_ingame` → `format_sockets` (`src/sockets.rs`) | List live connections as an aligned table (ID/IP/State/Name/Account/Idle seconds; admin-only; masked as unknown for others). |
| `ban list [type]` / `ban add <type> <pattern>` / `ban remove <type> <pattern>` | parser → engine queue (`src/parser.rs`, `src/ban.rs`) | Blocklist admin verbs (types `ip`/`account`/`character`; IP patterns `*`-wildcarded per octet). Admin-only + masked; an `add` persists and kicks every matching session. |
| `wiznet [on\|off\|security\|logins]` | parser → engine queue (`src/parser.rs`, `src/wiznet.rs`) | List and toggle the admin-alert categories (persisted `grim-config` settings `wiznet`, `wiznet.security`, `wiznet.logins`). Admin-only + masked. |
| `where` | `handle_ingame` → `format_where` (`src/who.rs`) | List beings in your area by room (`In your area (<name>):`, yourself included); only `Character`/`Creature` carriers — objects excluded. |
| `inventory` | parser → engine queue (`src/parser.rs`, `grim-object/src/commands/inventory.rs`) | List carried objects' short names (sorted), or the empty line. |
| `equipment` | `handle_ingame` → `tr!("equipment.empty")` (`src/command.rs`) | Uses `game_state["equipment_display"]` when available; otherwise the empty-equipment response. |
| `get <target>` | parser → engine queue (`src/parser.rs`, `grim-object/src/commands/get.rs`) | Pick up matches from the ground (`grim-target` selectors: `2.coin`, `3*coin`, `all [words]`); room sees "<name> picks up <short>" per item. |
| `drop <target>` | parser → engine queue (`src/parser.rs`, `grim-object/src/commands/get.rs`) | Drop matches from the pack (same selectors); room sees "<name> drops <short>" per item. |
| `give <item> <who>` | parser → engine queue (`src/parser.rs`, `grim-object/src/commands/give.rs`) | Hand matches to a PC here (creatures refuse); quote-aware split (`give "brass lantern" bob`; `all`-headed items run to the last word). Mover/other/room each see a named line (`format_transfer_events`). |
| `steal <item> <who>` | parser → engine queue (`src/parser.rs`, `grim-object/src/commands/steal.rs`) | Take matches from a being here (same split/selectors); same three-way echo. Existence checks only. |
| `open <direction>` / `close <direction>` | parser → engine queue (`src/doors.rs`, `grim-actor/src/commands/doors.rs`) | Flip the exit door, both sides; needs a direction (bare is unknown). |
| `areas` | `handle_ingame` → `format_areas` (`src/who.rs`) | List known areas. |
| `commands` | `handle_ingame` → `format_commands` (`src/formatter.rs`) | Show the command list. |
| `help` | `handle_ingame` → `format_commands` (`src/command.rs`) | Parses `Command::Help`; the game help handler answers the requested topic. |

Other verbs (`look`, `map`, `move`, `say`, `shutdown`, …) are parsed here then routed: most enqueue via `process_command_queue`; engine-queued admin verbs (`shutdown`/`reboot`/`copyover`/`goto`/`gecho`/`ban`/`wiznet`) go through `dispatch_admin_gated` (masked as unknown for non-admins). `sockets` is also admin-gated + masked, but answered session-locally from a per-tick `ClientSnapshot` (a second `Client` query would conflict with the dispatcher's `&mut` borrow).

## Resources & Events

| Name | Kind (Resource/Message) | File |
|---|---|---|
| `JustEnteredWorld` | Resource (routing-split guard; pub) | `src/session.rs` |
| `SceneSystems` | `SystemSet` (pub; `TouchInput` stamps activity before both dispatchers, pre-game runs before in-game input) | `src/plugin.rs` |
| `EngineCommand` | Message (emitted to engine) | `src/command.rs` |
| `BanList` | Resource (consumed for `ban` + resume refusal; owned by `grim-persistence`, `bans.json`-backed) | `src/ban.rs`, `src/resume.rs` |
| `IdleConfig` | Resource (idle thresholds in seconds: AFK / disconnect / warn lead, plus the `disconnect_ingame_idle` opt-in, default off) | `src/idle.rs` |
| `WizlistAdmins` | Resource (startup disk snapshot of admin characters for the offline half of `wizlist`) | `src/wizlist.rs` (`load_wizlist_admins`) |
| `ConnectionOutput` | Message (emitted; from `grim-networking`) | `src/output.rs` |
| `ItemEvent` / `TransferEvent` | Message (consumed → rendered per-recipient) | `src/item_output.rs` (`format_item_events`, `format_transfer_events`, `format_look_pack`) |
| `OpenEditor` / `EditorDone` | Message (consumed/emitted; the editor callback) | `src/editor.rs` (`open_editor`, `handle_editor_line`) |
| `LookRoom` / `LookEntity` / `MoveEvent` / `DoorEvent` / `RecallEvent` | Message (consumed → rendered) | `src/output.rs` (`LookRoom`/`LookEntity` in `src/look_output.rs`, `RecallEvent` in `src/recall_output.rs`) |
| `SayEvent` / `YellEvent` / `OocEvent` / `GlobalEcho` | Message (consumed → rendered) | `src/output.rs` |
| `LoginAnnounce` / `LogoutAnnounce` / `LinkdeadAnnounce` | Message (session announces) | `src/output.rs`, `src/command.rs` |
| `ServerBroadcast` | Message (consumed → rendered) | `src/output.rs` |
| `WiznetAlert` | Message (consumed → broadcast to opted-in admins; from `grim-networking`) | `src/wiznet.rs` (`broadcast_wiznet`) |

The shared render helpers in `src/formatter.rs` (`format_motd`, `format_selection_menu`, `parse_menu_choice`, `MenuItem`) are `pub` because the `grim-auth` pre-game flow reads them; the module is re-exported at `grim_scene::formatter`.

## Notes
- Single plugin: `ScenePlugin`. It is the **bridge** between `grim-networking` and `grim-command` (ARCHITECTURE.md §5.2/§5.3): reads raw in-game input, parses to `Command`, dispatches, and renders every domain event back per-recipient.
- **Input routing split (Phase 2b + scene-stack thin slice).** One `handle_client_input` became two systems: `handle_ingame_input` here (stack top is `InGameScene`) and `handle_pregame_input` in `grim-auth` (every pre-game `ClientState`). The pre-game system runs first (`.before(SceneSystems::InGameInput)`); a line that advances a session into the world also pushes its in-game scene there and records the connection in `JustEnteredWorld`, so this crate's system skips that line the same tick and routes by stack from the next tick on.
- Parser (`src/parser.rs`) owns direction aliases (`n`/`north` … and `ne`/`nw`/`se`/`sw` with full diagonal names), the `whisper`→`tell` alias, `help`→`Command::Help`, and prefix resolution (last-registered wins on ties; e.g. `n` → north over social).
- `!` repeats the last input; blank lines re-trigger the prompt.
- Admin-gated commands are byte-identical to "unknown command" for non-admins (no information leak) — see `dispatch_admin_gated`.
- Scene stack (§5.3): thin slice landed — `SceneStack` + `InGameScene`, pushed at world entry, routing by top. Pre-game scenes, pop, and output policy are still deferred (ADR-0003).
- Minimap: `emit_look_room` (`src/output.rs`) renders the looker's room through `grim_world::render_map` on the 9x7 canvas and staples it left of the room text (`formatter::staple_minimap`: 9-wide gutter + two spaces, no trailing blanks) whenever the resolved `minimap` setting (`grim-config`, per-character, default on) is on.
- Look exit lines: `emit_look_room` (`src/look_output.rs`, `partition_exits`) splits the room's exits into `Exits: …` (plain exits plus open doors, hidden-open included) and `Doors: …` (closed doors), space-separated on one line, each `none` when empty. Admins additionally see closed secrets appended on the same line (`Secret: …`, itself `none` when the room hides nothing); non-admins never see the heading. Direction names use Savage Soul green (`{g`); item room lines are magenta (`{m`) and NPC names bright green (`{G`), each terminated with the `{x` reset. Authored NPC descriptions keep neutral prose where the name can be identified. Screen-reader and colors-off settings still strip all styling. NPC name highlights include leading articles and adjectives (for example, “An artrell”). The ASCII map uses green room markers and a red current-location marker; room titles render `{B…{x` (blue), with admin debug ids after the reset so the colour never bleeds into them.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

`help [topic]` parses to `Command::Help`; the game-local `example-mud::help::HelpPlugin` supplies editable documentation. `commands` continues to list registered verbs.

Royal Oak container intents preserve directional open/close. Non-directional open/close dispatch game object interactions; `get ... from ...`, desk inspection and note reading route through SavageSoul for the game-local desk handler. Other desk/note looks fall back to the ordinary Look event.

`activate <target>` routes to the game-local Soulboy status handler through SavageSoul intent.

`ask <person> <question>` and `talk to <person> <question>` route game-local
NPC advice through SavageSoul intent; `royal_oak.rs` supplies Tully's dialogue.

Object examination in look_output::emit_look_entity appends one relative Condition line, including for carried objects. It reads ItemCondition through the description query; room/being/fixture looks do not receive object condition labels.

Room presence in look_output reads a character's optional posture_display only when posture_room matches the current room's stable GrimId. Seated/lying descriptions preserve AFK/linkdead indicators; characters without active posture retain normal standing text.

MOTD selection (`grim-scene/src/formatter.rs`): each login independently chooses a nonblank line from runtime `assets/motdlist.txt`. One line is one complete message; blank lines are ignored, CRLF and UTF-8 BOM are supported. Edit the list without rebuilding or restarting. Missing/empty lists fall back to runtime `assets/motd.txt`, then its embedded copy. Normal entry waits for Enter; linkdead reconnects display a fresh message and resume immediately. Random selection can repeat messages.

The game-specific `intermud`, `i3` and `imud` aliases are wizard-gated in `src/command.rs` before dispatch and hidden from mortal command listings. The game adapter independently checks current wizard roles for commands and inbound traffic.

Room minimaps reserve an eleven-column gutter; `formatter::staple_minimap` wraps room text in the remaining 69 visible columns before combining rows, preserving long descriptions and colour tokens within the standard 80-column layout.

Room/minimap text wrapping uses the connection's negotiated terminal width when available, defaulting to 80 columns.

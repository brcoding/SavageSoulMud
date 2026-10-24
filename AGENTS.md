# GRIM Engine — Agent Guide

AI agent instructions for this repo. For project docs, see [README.md](./README.md) (what GRIM is + the crate map with per-crate README links), [ARCHITECTURE.md](./docs/ARCHITECTURE.md) (target architecture + roadmap), and [CONTEXT.md](./CONTEXT.md) (glossary).

## Architecture

**[docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md) is authoritative for the target
architecture, and [CONTEXT.md](./CONTEXT.md) for vocabulary.** Read both before
designing anything. This file describes how to work in the repo and what the code
looks like *today*.

Current shape — event-passing subsystems behind a facade. The dependency
direction now matches the target: `grim` depends on the subsystems and re-exports
them (`GrimDefaultPlugins`); nothing depends back on the facade.

⚠️ "Client" is retired in the vocabulary; the session crate is `grim-scene`. But
it still uses the `ClientState` enum internally — the scene-stack entity model is
deferred (ARCHITECTURE.md §8). See CONTEXT.md.

## Crate Map (Current)

| Crate | Purpose |
|---|---|
| `grim-color` | Colour markup, ANSI rendering, palette, `escape_codes`. No Bevy, no serde |
| `grim-text` | Text catalog: `tr`/`tr!`, inlined defaults. Depends only on `grim-color`. No Bevy |
| `grim-command` | `CommandRegistry<C>` — generic, resource-ready. Exact-then-prefix resolution, `prioritize`/`deprioritize`, `contested_prefixes`. Bevy-only |
| `grim-command-events` | Semantic intent events (`MoveIntent`, `LookIntent`, … — one `Message` per player intent). Plain library, no plugin |
| `grim-networking` | `Connection` component + wire events (`ConnectionInput`/`Output`, `ConnectionEstablished`/`Closed`, `DisconnectRequest`). Bevy-only |
| `grim-networking-telnet` | `TelnetPlugin`: TCP server, IAC negotiation, tokio↔Bevy bridge, ANSI render |
| `grim-core` | Game events, components, validation; re-exports `grim-color` |
| `grim-scene` | `ScenePlugin`: in-game input dispatch (`ClientState::InGame`), output formatting + per-recipient broadcast, copyover resume. Owns the `CommandRegistry` resource + the shared `formatter` (MOTD/menus, read by `grim-auth`) |
| `grim-auth` | `AuthPlugin`: the pre-game flow — login, account/character creation, character-select, MOTD. State-handler-per-file (login/creation/character_select/finalize/world_entry/greeter). Owns `ReservedNamePrefixes` + credential/name validation. Layered on `grim-scene` (auth→scene; scene never depends on auth) |
| `grim-world` | Being-free world topology (rooms/areas/exits + room-address lookups + `RoomLocation`); `WorldPlugin` (world-event vocabulary) + `ShutdownPlugin` (SIGTERM signal + countdown machinery). Also owns race/class registries |
| `grim-actor` | The "beings": `Actor` base (race/level/gender, on every being) + PC-only `Character` (account/roles/class/title/restrings/last_room) + `Creature` mob marker + `Player`/`InRoom`/`Linkdead`/`OutputHistory`/`Role`, plus the `StoredCharacter` flat disk DTO. Names live in the `Name` component. Being-reading verbs (`look`/`move`/`goto`/`quit`/`title` + admin `shutdown` gate). `ActorPlugin`. Depends on `grim-world`, never the reverse |
| `grim-channel` | `ChannelPlugin`: say/yell/ooc handlers |
| `grim-object` | Things: `Object` marker + `CarriedBy` carrier link (ground = `InRoom`, carried = `CarriedBy`, never both) + verbs (`get`/`drop`/`inventory`/`give`/`steal`, `ItemEvent`/`TransferEvent` facts rendered per-recipient by `grim-scene`, pack block on being-look). Packs snapshot whole per instance into the character file (`persist`); ground regrows from seed. `ObjectPlugin`. Depends on `grim-actor`, never the reverse |
| `grim-persistence` | `PersistencePlugin`: account/character load + save-on-disconnect |
| `grim` | Facade: depends on and re-exports every subsystem; `GrimDefaultPlugins` group. No code of its own |
| `example-mud` | Binary (`crates/example-mud`): `GrimDefaultPlugins` + world seed |

## Crate Map (Target)

See [ARCHITECTURE.md §4](./docs/ARCHITECTURE.md). Naming rules that matter when adding
a crate:

- **`grim-<system>`** for a subsystem, **`grim-<system>-<extension>`** only once a
  second real implementation exists.
- **There is no `grim-core-*`.** The `grim-` prefix already marks a crate as ours.
- Not every crate is a plugin. A crate with no `App` state (pure functions) stays a
  plain library — do not add an empty `impl Plugin`.

## Key Conventions for Agents

- When converting legacy content, preserve player command wording such as `read sign`; expose a compatible command alias rather than requiring users to discover a replacement.

These describe **current** code. Several are slated to change — ARCHITECTURE.md §8
lists every gap. Do not treat them as targets: notably "last registered wins" command
resolution and the closed `Command` enum are documented as defects, not conventions to
extend.

- **`\n` everywhere**: Code uses `\n` for newlines. The protocol layer (`send_network_commands`) converts `\n` → `\r\n` before writing to TCP.
- **Text catalog**: Use `grim::tr!("key", var = value)` (or `grim::tr(key, args)`) for all author-facing strings — this is `grim-text`. It converts `{X` colour codes to `@xRGB`, then substitutes `%{var}` placeholders, escaping each value so it cannot inject colour. Defaults are inlined in `grim-text`. `rust-i18n`/`t!` and `locales/en.json` are gone. Colour rendering to ANSI (`ansi`, `convert_16color`, `escape_codes`) is `grim-color`, re-exported at `grim::color::*`.
- **User input filter**: Protocol layer strips all non-printable ASCII (32-126) from user input before creating `ClientInput`, preventing ANSI/control code injection.
- **`Exit` vs `Exits`**: The component is `Exits { exits: HashMap<Cardinal, Entity> }`. On a Room entity.
- **`Name` vs `GrimName`**: Import aliased from `grim` as `GrimName` in binary/client to avoid collisions with Bevy's `Name`.
- **Single-letter directions** (`n`/`e`/`s`/`w`/`u`/`d`) work via prefix matching against the `CommandRegistry`. Directions are registered last in `parser.rs::build_registry()`, and `register` puts each new command at the front of the priority ordering, so directions win single-character input.
- **Command resolution**: `grim::CommandRegistry<Command>` (from `grim-command`). Commands are registered by name + `fn(&str) -> Option<Command>` factory. Resolution is case-insensitive: exact name first, then highest-priority prefix. Priority is explicit and reorderable via `prioritize`/`deprioritize` — not `max(entry_idx)`. Single-character/punctuation shorthands are not registered names (`l` reaches `look`, `n` reaches `north` via prefix; the `'` say-alias is gone). It is a live Bevy **resource** (`grim-scene` inserts `parser::command_registry()`, which logs contested prefixes), threaded into `handle_client_input` via the `SessionRes` `SystemParam`. The `OnceLock` is gone.
- **Player presence = online.** `Player` is attached only while a connection is driving the character (`Player { connection: Entity }`, non-optional). **Online ⇔ the character has a `Player`; linkdead ⇔ it has a `Character` and no `Player`** (marked `Linkdead`). There is no "linkdead `Player`".
- **Character takeover**: If a character is already online (has a `Player`) and another session selects it, the old session's connection receives "Someone else has logged into this character." and is disconnected; the new session inserts its own `Player` (with the new connection) and proceeds. The old connection's later `ConnectionClosed` must NOT re-mark the character linkdead — see the guard below.
- **Online indicator**: The character select menu shows "(online)" for characters that have a `Player` and "(linkdead)" for linkdead characters (`Linkdead`, no `Player`).
- **Multiple characters per account**: Allowed simultaneously. No check prevents multiple characters from the same account being `InGame` at the same time.
- **save_on_disconnect guard**: On connection close, `save_on_disconnect` **removes** the `Player` and **inserts** `Linkdead` (the disk save routes through `StoredCharacter`; `OutputHistory` transfers to the character entity). It skips this only if the character was taken over — its live `Player.connection` differs from the closing connection — so a stale `ConnectionClosed` from a takeover never disturbs the new session's live `Player`.
- **Ownership/visibility checks must fail closed.** When a lookup that *gates* what a session may see can itself fail — most commonly a `commands.spawn` entity queried in the same tick before it is flushed — a failed lookup must show **nothing**, never fall through to unfiltered data. This bit `show_character_menu`: the account-ownership filter sat inside `if let Ok(account) = accounts.get(..)`, so a just-created account (unflushed entity) skipped the filter and listed every character in the world. Resolve the gating entity once, up front; if it is unavailable, return the empty/denied result. Mirror the pattern already used on the selection path (`let Ok(..) = accounts.get(..) else { continue };`).

## Architecture Decisions

**Prefer existing crates over rolling your own.** Before writing non-trivial
infrastructure (protocol handling, OS/syscall glue, serialization, async
primitives, anything with a well-known name), research crates.io first: is there
a mature, maintained crate that does this? Check download count and last-updated
date. Default to gluing a proven crate over hand-rolling — hand-rolled infra is a
maintenance and correctness liability. Only reimplement when no crate fits, the
fit is poor, or the dependency cost is clearly not worth it — and say why. Do this
research at the *start* of a feature, before designing the bespoke version.

When developing new features, start by asking:

1. **Is there a crate for this?** → Research crates.io before building infra (see above)
2. **Does this already fit into the architecture?** → Implement as a plugin
3. **Does this require a change to the architecture?** → Document in `docs/ARCHITECTURE.md`
4. **Does this add new functionality needing API-level discussion?** → Open an issue

**Reference:** `docs/ARCHITECTURE.md` for current architecture decisions.

## Workflow

0. **Design** — feature/design work starts with the `grim-design` skill (`.agents/skills/grim-design/SKILL.md`): arch review, ADR conflicts, local iteration in `.planning/tmp/`, publish on request.
1. **Branch pre-check** — check current branch; if wrong, branch from `main`
2. **Branch** from `main`
3. **Commit** incrementally
4. **Push** the branch
5. **Create a PR** — `gh pr create --fill --base main`. Pushing without a PR is unfinished.
6. **CI** checks: build, lint, test
7. **Human review**
8. **Squash merge** into `main`

## Editing Discipline

1. **Re-ground after every edit.** Fresh snapshot tag + renumber from edit response or fresh `read`.
1a. **Edits eat neighbors.** A range touching a boundary line can silently delete the adjacent statement (an insert `PUT <N:`/`PUT >N:` never can — prefer it near load-bearing lines like plugin registrations). Signals: auto-repair warnings on the edit response, or NEW `dead_code`/`unused` compiler warnings after your change. On either signal, `git diff` the file immediately and restore what the range ate (`editor::register` was lost this way, un-registering its messages and failing every downstream system validation).
2. **Verify structure before chaining.** `read` the affected area before the next edit.
3. **Prefer rewrite over surgical patch when code is young.** Files <5 edits old → `write` from scratch.
4. **Read the whole function.** Elided ranges = unseen — expand range.
5. **Test immediately after the last edit.**
6. **Explicit character sets, not ASCII ranges.** `'k'..='w'` ≠ `krgybmcw`. Use `'k' | 'r' | 'g' | 'y' | 'b' | 'm' | 'c' | 'w'`.
6a. **Never byte-slice untrusted input.** `s[..n]` panics on multibyte input (`give épée bob` crashed `is_all_head`). Use `s.get(..n)` and treat `None` as no-match — player input must miss, never crash.
6b. **MUD colour resets take no closing brace.** The reset token is `{x` (brace + letter, exactly like `{R`/`{m`), and in a Rust format string it is spelled `{{x` — never `{{x}}`. A trailing `}` either fails to compile (`unmatched }`) or, worse, renders literally in player output (`{Rnorth{x}` on the look line). The same holds for every `{X` code: open brace + one letter, nothing after. When an edit touches a line containing `{x`, re-read the line and count the braces before moving on.
7. **Update README.md** when architecture, roadmap, or conventions change — not this file.
7a. **Every crate has a `README.md` following [`docs/README.template.md`](./docs/README.template.md).** It's the map to that crate — Components, Systems, and (GRIM-specifically) **Commands with the file each handler lives in**, plus Resources/Events. Keep it current *in the same change*: whenever you add, move, or remove a Component / System / Resource / Event / Command, update that crate's README so the pointers stay true — the Commands→handler table especially (a stale handler pointer is worse than none). Findability is the point; treat the README as part of the code, not an afterthought.
8. **NEVER use `--no-verify` on commits.** Pre-commit hooks (lint, fmt, coverage) are mandatory. If they block, fix what they catch.
   After every PR commit and push, review CodeRabbit feedback and the CI results
   for that exact commit. Local tests or older green runs are not completion.
   Read failed job logs, address actionable comments, and report remaining
   failures explicitly; do not hand off while reviews or checks are pending.
9. **Codify every root cause.** When the user asks why something broke, when there's confusion or frustration, or when an unstated assumption surfaces — add a rule to AGENTS.md. These signals mean something wasn't obvious. Write it down so the next agent doesn't repeat it.
10. **100% coverage target, 90% floor.** All code should be tested. The goal is 100% line coverage. The 90% threshold (in the Makefile) exists only as a safety net for genuinely uncoverable lines (unreachable defensive branches, language limitations). Any gap below 90% is a bug — fix it. If a file can't reach 90%, ratchet down in 5% decrements, re-evaluating at each step.
11. **Tests that own the clock skip `TimePlugin`.** `MinimalPlugins` includes the
  automatic time update, which overwrites manual `advance_by` (idle sweeps saw
  `elapsed == 0` forever). Clock-driven tests use a bare `App` plus
  `init_resource::<Time>()` and advance it manually — see `shutdown.rs` and `idle.rs`.
`n- When recovered source has placeholders, label reconstruction as provisional. Prefer a supplied live-session transcript over older placeholders; do not invent missing puzzle behavior or silently present inferred rooms as faithful conversions.

- WSL must execute the MUD from Linux storage, never the network-backed P: executable. Recreate the runtime image when /tmp is cleared; verify /proc/<pid>/exe after starting. Use Restart=on-failure for the local service.

- Server-initiated disconnects must deliver queued explanation text before EOF. Drain the writer with a bounded timeout; never abort it immediately after enqueuing an authentication failure.

- Converting a readable fixture requires resolving SetRead and its static file-backed callback, not just SetItems. A description telling players to read an object is incomplete without its written content.

- Do not use forced NPC fleeing as a workaround for poor hit probability. Verify the requested combat outcome end-to-end; preserve size rules and implement appropriate rule-supported actions rather than hiding an unfinished fight.

- Trample encounters must exercise the defender's opportunity to Attack a Moving Character before knockdown. Spending every NPC activation on Desperation Dive can silently remove all retaliation; verify both sides can attack and that reactions do not grant extra activations.

- NPC death must end creature/script/merchant behavior and create actual remains, not just rewrite a living entity's room description. Verify corpse look/get/drop and transfer held possessions without duplication.

- Route generic get-from commands by their named source before invoking quest handlers. Non-desk targets must never receive the trapped-desk error; test empty corpses and missing containers outside the quest room.

- Brief combat must remain observable each round: summarize hits, misses and recovery on one line. Hiding all non-wounding attacks makes active combat look stopped, particularly after a live log-detail change.

- Corpse expiry is absolute per-instance state: preserve it through snapshots, restores, pickup/drop and copyover. Save legacy carried corpse initialization and expired-pack removal atomically. Match removals by runtime entity, never shared corpse names; verify visible condition progression and actual disappearance.

- Managed NPC populations count living spawn members across all rooms, not occupants of home rooms. Death releases exactly one slot with its delay; corpses and wandering must not duplicate replacements. Keep custom quest respawning exclusive with managed respawning. AI movement must use normal movement vetoes, stay within configured bounds and pause in combat.

- The medical table regenerates the body placed on it; donating a creature corpse must never heal the donor. Regenerated creatures are contained, then identified as threats and vaporized after three seconds without rewards or new corpses. Sitting/lying heals the living patient separately. Never infer an unrelated corpse-as-healing-fuel mechanic from a room description.

- Sitting/lying output must correspond to stored posture and have a working stand command. Save posture and table-owned prone together; match the stored room before rendering/blocking movement, and clear stale posture after teleport. Standing must not heal the character.

- Browser content catalogs must avoid per-file stat/resolve/ancestor checks on the network-backed P: drive. Enumerate with scandir without following links, validate fixed roots once, cache only non-sensitive listing metadata briefly, and invalidate on save. Keep fresh authorization and full path validation for actual reads/writes; measure cold and warm endpoint times on the deployed filesystem.

- Editor maps must preserve every room when inferred coordinates collide, separate floors/components before display packing, distinguish intentional one-way exits from broken targets, and label inferred geometry/artwork as schematic. Use current unsaved drafts for diagnostics; map navigation must not discard a draft or rewrite exits automatically.

- Recovered crafting uses stable material identities across whole-pack snapshots. Do not confuse source craftsmanship tiers with wear, transplant LPC combat numbers into Savage Soul, or implement TODO recipes as established rules. Merge against live authored content and report missing tool/source dependencies explicitly.

- Combat prose must follow the resolved outcome: Dodge for active avoidance, weapon parries/blocks only with an armed, capable defender, and no movement by incapacitated targets. Narrative variation must not consume combat dice or grant additional actions.

- Intermud adapters must use bounded off-loop I/O, authenticated loopback APIs, fresh recipient authorization, and remote-text escaping. Keep router credentials outside authored content. After fresh startup, verify both MUD and channel caches; a ready TCP/router connection alone is insufficient. Do not replay chat/tells on reconnect or send public test messages.

- Every Intermud alias and subcommand is wizard-only, including local history/activity and discovery. Enforce current roles at both session dispatch and delivery; a configuration flag must not grant mortal access. Bulk subscriptions must be paced and must not claim access to restricted network channels.

- Remembered editor sessions need both an explicit persistent-cookie lifetime and a stable signing key stored in private local state. Do not generate a new key at every service restart; continue checking current wizard ownership, bans and credential fingerprints on each request.

- Wrap room prose to the available text-column width before combining it with the ASCII minimap. Terminal/client wrapping starts at column zero and breaks the map gutter. Measure rendered colour-aware width, preserve explicit line breaks, and never truncate long descriptions or split colour tokens.

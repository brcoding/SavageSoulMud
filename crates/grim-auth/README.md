# `grim-auth`
> The pre-game phase: login, account/character creation, character selection, and the MOTD — everything a session does before entering the world.

**Role:** vertical — pre-game authentication & character onboarding
**Depends on:** `grim-core`, `grim-scene`, `grim-actor`, `grim-world`, `grim-persistence`, `grim-networking`, `grim-text`, `grim-color`

Layered on the session core: `grim-auth → grim-scene` (it reuses the scene's
shared render helpers and `ConnectedAt`/`JustEnteredWorld`/`SceneSystems`). The
reverse edge is forbidden — `grim-scene` has zero references to `grim-auth`.
Still driven by `grim_core::ClientState`; the typed scene-stack model remains
deferred (ARCHITECTURE.md §5.3/§8).

Replacement seam: this crate is the default D&D pre-game bundle, not the floor. An
author replacing the whole login→world flow omits `AuthPlugin` and pushes their
own scenes behind `grim-scene`; overriding MOTD/login wording alone is a Catalog
key override, no code.

## Components
`grim-auth` defines no components of its own. It reads/writes the beings
(`Account` on `grim-core`; `Character`/`Player`/`InRoom`/`Linkdead`/`OutputHistory`
on `grim-actor`) and stamps `grim_scene::ConnectedAt` when a session enters the
world.

Structured credentials are handled in `handle_pregame_input`, ordered by
`AuthSystems::Greet` and `AuthSystems::Input`.

## Systems

| System | Schedule | File | Purpose |
|---|---|---|---|
| `handle_connection_established` | `Update` | `src/greeter.rs` | Spawns a `Client`, prints the login banner + first prompt — the entry to the flow. Refuses banned IPs first (ban message + sever, no session), then per-IP reconnect floods (throttle message + sever + security alert). |
| `validate_registries` | `Startup` | `src/plugin.rs` | Panics on a mis-seeded world (empty race registry / no tier-1 class in Legacy mode) so creation can't trap a player. SavageSoul mode requires no tier-1 class. |
| `handle_pregame_input` | `Update` (`.after` the greeter, `.before(SceneSystems::InGameInput)`) | `src/input.rs` | Routes a line by pre-game `ClientState`; skips `InGame` (scene's job); records connections it advances into the world in `JustEnteredWorld`. |

## State handlers
The pre-game flow is a state machine, one handler file per concern (not the
one-file-per-command layout, which is for in-game command handlers).

| `ClientState` | Handler | File |
|---|---|---|
| `LoginPrompt` | `login_prompt` | `src/login.rs` |
| `NewAccountPrompt` | `new_account_prompt` | `src/new_account.rs` |
| `PasswordPrompt` (failed attempt → `Disconnecting`, terminal) | `password_prompt` (→ `create_account` in `src/account.rs` / `authenticate`) | `src/password.rs` |
| `CharacterSelect` | `character_select` (+ `show_character_menu`, `account_character_list`) | `src/character_select.rs` |
| `CreateCharacter` | `create_character` | `src/creation.rs` |
| `SelectGender` / `SelectRace` / `SelectClass` | `select_gender` / `select_race` / `select_class` | `src/creation.rs` |
| `MotdPrompt` | `motd_prompt` (advances to `InGame`, auto-looks the room) | `src/character_select.rs` |
| `AccessibilityPrompt` (opt-in) | `ask_if_needed` / `answer` (save preference, then enter) | `src/accessibility.rs` |

Entering the world (reconnect linkdead / take over resident / spawn from disk)
is `enter_world_by_name` in `src/world_entry.rs`, shared by the login-by-name,
character-select, and legacy-backfill (class-pick) paths. Completing a new build
is `finalize_character` / `backfill_and_enter` in `src/finalize.rs`.

Ban enforcement is one gate function per identity (`BanList::is_ip_banned` /
`is_account_banned` / `is_character_banned`), called at three points: IP at the
greeter (no session is spawned), account in `authenticate` after a correct
password (message + sever, never the menu), character in `character_select`
(message + menu re-show) and in `enter_world_by_name` (covers login-by-name;
refuse back to the login prompt). A mid-play `ban add` kicks live sessions from
the scene side (`grim-scene/src/ban.rs`); copyover resume re-checks all three
before placing (`refuse_banned` in `grim-scene/src/resume.rs`).

## Resources & Events

| Name | Kind (Resource/Message) | File |
|---|---|---|
| `ReservedNamePrefixes` | Resource (character-name prefix blocklist; author-overridable) | `src/validation.rs` |
| `CharacterBuildMode` | Resource (legacy class flow or Savage Soul XP build) | `src/build_mode.rs` |
| `CharacterCreationDefaults` / `StartingCredits` | Resources (author-provided initial state and new-character credit balance; absent balance defaults to 100) | `src/creation_defaults.rs` |
| `LoginCredentials` / `LoginResult` | Messages (structured credentials reuse password/ban checks and report success) | `src/client_login.rs`, `src/input.rs` |
| `AuthSystems` | SystemSet (Greet and Input order structured sign-in against normal input) | `src/client_login.rs` |
| `AccessibilityOnboarding` | Resource (default false; one-time screen-reader question on normal login) | `src/accessibility.rs` |
| `LoginVersion` | Optional resource (binary-supplied build identity printed before the login prompt; escaped and bounded to 128 characters) | `src/greeter.rs` |
| `RaceRegistry` / `ClassRegistry` | Resource (read for the creation menus; `init_resource`, from `grim-world`) | `src/plugin.rs` |
| `PersistenceConfig` | Resource (account/character JSON dir; `init_resource`, from `grim-persistence`) | `src/plugin.rs` |
| `BanList` | Resource (read at every login gate; owned by `grim-persistence`) | `src/greeter.rs`, `src/password.rs`, `src/character_select.rs`, `src/world_entry.rs` |
| `ReconnectLimits` | Resource (per-IP throttle knobs, seconds; `init_resource`, author-overridable) | `src/throttle.rs` (`AuthPlugin`) |
| `ReconnectThrottle` | Resource (per-IP attempt timestamps + reject windows; pruned on access) | `src/throttle.rs` (read in `src/greeter.rs`) |
| `LoginAnnounce` / `LinkdeadAnnounce` | Message (emitted on world entry / linkdead reconnect) | `src/character_select.rs`, `src/world_entry.rs` |
| `LookRoom` | Message (emitted at MOTD to auto-look) | `src/character_select.rs` |
| `ConnectionOutput` / `DisconnectRequest` | Message (from `grim-networking`) | throughout |

Input validation lives entirely in `src/validation.rs`: `hash_password` /
`verify_password`, `validate_identifier` (email), `validate_character_name` /
`normalize_character_name`, `validate_password`, `is_name_reserved`, plus the
`ReservedNamePrefixes` resource and `ValidationError`.

## Notes
- Insert `AccessibilityOnboarding(true)` to ask each character whether they use
  a screen reader before the first room description. A valid `screenreader`
  config value (`on` / `off`) records that the question has been answered. Yes
  also selects `minimap=off`, `colors=off`, `combatlog=brief`, and
  `prompt=accessible`; no preserves other preferences. The complete character
  and inventory save atomically before entry. Copyover skips the login flow.
- `CharacterBuildMode` (`src/build_mode.rs`) selects the creation flow. It defaults
  to `Legacy` (gender, race, class). Insert `CharacterBuildMode::SavageSoul` to
  ask for name, gender and race, then finish with the `savage-soul` class tag;
  the game handles attributes and Peculiarities after login. Existing characters
  keep their stored race and class. A nonempty race registry is required in both
  modes; only Legacy requires creatable classes.
- Single plugin: `AuthPlugin` — one plugin registering the greeter, the pre-game
  input system, the reserved-name / race / class / persistence / reconnect-throttle
  resources, and the mis-seed guard.
- **Routing coordination.** The pre-game system runs before the scene in-game
  system (`SceneSystems::InGameInput`). Because a single line can transition a
  session to `InGame`, the pre-game system records that connection in
  `grim_scene::JustEnteredWorld` (cleared each tick) so the scene system does not
  re-dispatch the triggering line — verified by the `example-mud` login E2E
  scenarios.
- `grim-auth` defines its own `SystemParam` bundles (`src/params.rs`) over the
  underlying components/resources rather than reusing the scene's, so the two
  crates stay decoupled; only genuinely shared render helpers
  (`grim_scene::formatter`) are called across the boundary.


Login branding comes from `assets/login-banner.txt`, embedded at build time and rendered by `src/greeter.rs`. The Savage Soul uses its recovered green radiation symbol and red/yellow title. Character screen-reader preferences and the onboarding question apply after authentication.

MOTD selection (`grim-scene/src/formatter.rs`): each login independently chooses a nonblank line from runtime `assets/motdlist.txt`. One line is one complete message; blank lines are ignored, CRLF and UTF-8 BOM are supported. Edit the list without rebuilding or restarting. Missing/empty lists fall back to runtime `assets/motd.txt`, then its embedded copy. Normal entry waits for Enter; linkdead reconnects display a fresh message and resume immediately. Random selection can repeat messages.

- `CharacterCreationDefaults` supplies game-owned initial `game_state` values
  keyed by the chosen race. They are stored in the first character save, before
  the MOTD. Existing characters and legacy backfills never get these defaults.

Structured client sign-in uses LoginCredentials/LoginResult messages owned by
auth, ordered with AuthSystems::Greet/Input. Credentials are redacted for Debug
and never routed through text input or history. Existing password auth enforces
normal ownership/bans/takeover. Manual input wins a mixed same-tick batch.

---
*Format: [`docs/README.template.md`](../../docs/README.template.md). Improve over time.*

## Staff testing visibility

The pre-game dispatcher runs before `SceneSystems::Visibility`, so account testing status is applied before public output and in-game input. New accounts always begin with testing disabled.

See [staff testing](../../docs/STAFF-TESTING.md) for permissions and shared-world limitations.

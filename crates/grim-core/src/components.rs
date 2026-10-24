use std::collections::VecDeque;
use std::time::Duration;

use bevy::prelude::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::events::{Command, EditorKind};
use crate::id::GrimId;

// Re-export `Gender` so `components::*` (and, through it, the crate prelude)
// surfaces it alongside the session types. `Gender` is a creation-time choice
// referenced by `ClientState` below; the `Character` that stores it lives in
// `grim-actor`, which points *up* at this type.
pub use crate::character::Gender;

/// Shared placement link, reexported by grim-object for API compatibility.
#[derive(Component, Debug, Clone, Copy)]
pub struct CarriedBy {
    pub carrier: Entity,
}

/// Per-instance physical condition. Optional absolute decay times survive saves.
#[derive(Component, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ItemCondition {
    #[serde(default = "full_quality")]
    pub quality: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decay: Option<ItemDecay>,
}
fn full_quality() -> u8 {
    100
}
impl Default for ItemCondition {
    fn default() -> Self {
        Self {
            quality: 100,
            decay: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ItemDecay {
    pub started_at: u64,
    pub expires_at: u64,
}
impl ItemCondition {
    pub fn label(&self) -> &'static str {
        match (self.decay.is_some(), self.quality.min(100)) {
            (true, 80..=100) => "fresh",
            (true, 60..=79) => "starting to decay",
            (true, 30..=59) => "decaying",
            (true, 1..=29) => "badly decayed",
            (true, 0) => "rotted away",
            (false, 90..=100) => "excellent",
            (false, 70..=89) => "good",
            (false, 40..=69) => "worn",
            (false, 1..=39) => "badly worn",
            (false, 0) => "ruined",
            _ => unreachable!(),
        }
    }
}

// ─── Session ────────────────────────────────────────────────────────

/// The client session state machine — one per connection, on a separate
/// entity from `Connection` so the engine never touches socket types.
#[derive(Component, Debug)]
pub struct Client {
    /// The `Connection` entity this client is bound to.
    pub connection: Entity,
    pub state: ClientState,
    pub account: Option<Entity>,
    pub character: Option<Entity>,
    /// Parsed commands waiting to be dispatched (cooldown-gated).
    pub input_queue: VecDeque<Command>,
    pub command_cooldown: Timer,
    /// The last raw input text (for "!" repeat support), excluding the "!" itself.
    pub last_input: Option<String>,
    /// Bevy-`Time` elapsed at the last input line (any state, incl. editor).
    /// `None` until the first input or sweep sighting; the idle sweep stamps it.
    pub last_active: Option<Duration>,
    /// Warn-once latch for the idle-disconnect warning; reset on any input.
    pub idle_warned: bool,
    /// Whether this session is flagged AFK. Set by the `afk` command or the
    /// idle sweep; cleared by any input line. Dies with the session, so stale
    /// AFK is structurally impossible — a plain field (not a component) so
    /// every dispatcher that already holds `&mut Client` can read/write it
    /// without a second query (which would conflict) or `Commands` threading.
    pub afk: bool,
    /// Line-editor modal state. `Some` exactly while the session is inside the
    /// editor: input routes to the buffer instead of the command parser. A
    /// plain field (not a component) so opening/closing is visible to later
    /// lines in the same tick — deferred `Commands` could never do that.
    pub editor: Option<EditorSession>,
}

/// Modal line-editor state: what is edited and the lines so far. Lives on
/// [`Client::editor`]; the open/close events carry the rest.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorSession {
    pub character: Entity,
    pub kind: EditorKind,
    pub buffer: Vec<String>,
}

impl Client {
    pub fn new(connection: Entity) -> Self {
        Self {
            connection,
            state: ClientState::LoginPrompt,
            account: None,
            character: None,
            input_queue: VecDeque::new(),
            command_cooldown: Timer::from_seconds(0.5, TimerMode::Once),
            last_input: None,
            last_active: None,
            idle_warned: false,
            afk: false,
            editor: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientState {
    /// Waiting for the user to type a username or email.
    LoginPrompt,
    /// Waiting for a password. `is_new` distinguishes "create new" vs "login existing".
    /// `character` is the character NAME to auto-select after a correct password
    /// (set when the user logged in by character name); `None` shows the account menu.
    PasswordPrompt {
        identifier: String,
        is_new: bool,
        character: Option<String>,
    },
    /// "No account found. Create one? (y/n)"
    ConfirmCreate { identifier: String },
    /// Waiting for an email at the new-account prompt (reached via `new`).
    NewAccountPrompt,
    /// A failed password attempt asked the transport to sever the socket.
    /// Terminal: the pre-game dispatcher ignores input in this state so a
    /// pipelined burst cannot test further passwords while the disconnect
    /// is in flight (`DisconnectRequest` → `ConnectionClosed`).
    Disconnecting,
    /// Showing the character selection menu.
    CharacterSelect,
    /// Waiting for the user to type a new character name.
    CreateCharacter,
    /// Picking a gender for the new character. Carries the accepted `name`.
    SelectGender { name: String },
    /// Picking a race. Carries the `name` and chosen `gender`.
    SelectRace { name: String, gender: Gender },
    /// Picking a (tier-1) class. Carries the `name`, `gender`, and race `slug`.
    /// On a valid pick the character is persisted and the session advances to
    /// the MOTD.
    SelectClass {
        name: String,
        gender: Gender,
        race: String,
    },
    /// MOTD prompt after character select — hit Enter to enter the world.
    MotdPrompt,
    /// Optional one-time, per-character accessibility preference question.
    AccessibilityPrompt,
    /// In-game — input is parsed and queued as commands.
    InGame,
}

// ─── Account ────────────────────────────────────────────────────────

/// An account — persists across sessions. Saved to `data/accounts/<id>.json`.
#[derive(Component, Serialize, Deserialize, Clone, Debug)]
pub struct Account {
    /// Staff-managed test identity. Never grants administrator privileges.
    #[serde(default)]
    pub testing: bool,
    pub id: GrimId,
    /// The user-facing identifier (username or email, normalized lowercase).
    pub identifier: String,
    pub password_hash: String,
    /// Grim IDs of characters owned by this account.
    pub characters: Vec<GrimId>,
    pub created_at: DateTime<Utc>,
}

// `RoomLocation` used to live here. Placement Phase 2a step 3 relocated it to
// `grim-world` (the being-free world layer) once `Character` split, so the type
// now sits below all its consumers with no reverse edge. See
// `grim_world::RoomLocation`.

// ─── Descriptive (shared by characters, creatures, items) ────────────

/// Display name for any visible entity.
#[derive(Component, Debug, Clone)]
pub struct Name(pub String);

/// Paragraphs shown by `look <target>`, one line each: entries carry no
/// newlines and the renderer joins them with `\n`. One entry per paragraph.
#[derive(Component, Debug, Clone)]
pub struct Description(pub Vec<String>);

/// Extra lookup words for `look <keyword>` (e.g. `["grimmok", "smith"]` for
/// "Grimmok Ironhand"). Characters match by name and carry none of these;
/// creatures match by name or by any keyword prefix (case-insensitive).
#[derive(Component, Debug, Clone, Default)]
pub struct Keywords(pub Vec<String>);

/// Room-listing line for a being or object: the sentence shown under the room
/// description (e.g. `"Grimmok Ironhand stands here, hammering metal."`).
/// Distinct from [`Description`], which is the detail shown by `look <target>`.
/// Player characters do not carry this — their line is rendered from their
/// position (`"<name> is standing here."` until positions exist).
#[derive(Component, Debug, Clone)]
pub struct RoomDescription(pub String);

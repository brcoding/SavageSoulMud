use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cardinal::Cardinal;

// â”€â”€â”€ Client â†’ Engine â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

#[derive(Message, Debug)]
pub struct EngineCommand {
    pub client: Entity,
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Editable game help topic.
    Help { topic: String },
    /// Game-specific Savage Soul character, equipment, and combat commands.
    SavageSoul { verb: String, arg: String },
    ListShop,
    Buy { target: String },
    Sell { target: String },
    Credits,
    /// `look` or `look <target>` â€” the target is a `grim-target` being spec
    /// (`2.goblin` looks at the second; `"two words"` needs every word).
    Look { target: Option<String> },
    /// `config` â€” list registered settings, show one (`config minimap`), or
    /// set one (`config minimap off`). Handled in `grim-actor`.
    Config {
        key: Option<String>,
        value: Option<String>,
    },
    /// `map` â€” render the area around the actor's room as ASCII (`@` self,
    /// `o` rooms, `--`/`|` exits, `,`/`'` up/down markers).
    Map,
    /// `say <text>` â€” room-scoped
    Say { text: String },
    /// `yell <text>` â€” area-scoped
    Yell { text: String },
    /// `ooc <text>` â€” global
    Ooc { text: String },
    /// `tell <target> <text>` â€” private message to one player (fuzzy-matched by
    /// name; `self` targets the sender).
    Tell { target: String, text: String },
    /// `reply <text>` â€” private message to the last player who whispered you.
    Reply { text: String },
    /// `<social> [target]` â€” a data-driven social (`grin`, `smile`, â€¦,
    /// registered at startup from `grim-social`). `target` is the first word
    /// of the rest of the line, when present; the handler resolves it against
    /// the actor's roommates (`self` targets the sender).
    Social {
        name: String,
        target: Option<String>,
    },
    /// `socials` â€” list the data-driven socials (registered at startup from
    /// `grim-social`). Answered from the command registry, like `commands`.
    SocialList,
    /// `channel <name> <text>` â€” unified channel command (data-driven).
    Channel { channel: String, text: String },
    /// `title <text>` sets the actor's WHO title (max 60 chars); a bare `title`
    /// (empty text) clears it.
    Title { text: String },
    /// `desc` views your description paragraphs; `desc clear` empties them,
    /// `desc + <line>` appends one, `desc -` drops the last, `desc edit`
    /// opens the line editor.
    Desc { op: DescOp },
    /// Movement via cardinal direction
    Move { direction: Cardinal },
    /// `open <direction>` â€” open the door guarding that exit.
    /// Bare `open` is rejected as unknown (like `get`/`drop`).
    Open { direction: Cardinal },
    /// `close <direction>` â€” close the door guarding that exit.
    Close { direction: Cardinal },
    /// `recall` â€” return to the Town Square (`haven:square`). No-op with a
    /// reply when already there. Available to every player.
    Recall,
    /// `quit` â€” clean disconnect
    Quit,
    /// `who` â€” list connected players
    Who,
    /// `wizlist` â€” list admin-flagged characters online
    Wizlist,
    /// `where` â€” show who's in your area and their room
    Where,
    /// `finger <name>` â€” show a character's description, online or off.
    /// Online characters answer from their live description; offline ones
    /// load from disk (custom descriptions do not persist yet, so offline
    /// output is the default description for now).
    Finger { target: String },
    /// `sockets` â€” admin-only, session-local. List every live connection
    /// (id, address, session state, character, account, idle seconds).
    /// Masked as unknown for non-admins, like the other admin verbs.
    Sockets,
    /// `wiznet` / `wiznet on|off|security|logins` â€” admin-only. List and
    /// toggle the admin-alert (wiznet) categories. Masked as unknown for
    /// non-admins, like the other admin verbs.
    Wiznet { arg: Option<String> },
    /// `afk` â€” flag yourself as away. Auto-set after `IdleConfig::afk_after_secs`
    /// idle; any input line clears it. Shows on `who`; prompt becomes `<AFK>`.
    Afk,
    /// `inventory` â€” list the short names of carried objects.
    Inventory,
    /// `equipment` â€” dummy: always reports empty (no item system yet).
    Equipment,
    /// `get <target>` â€” pick up matches from the room. A `grim-target` item
    /// spec: one match by default, `2.coin` the second, `3*coin` up to three,
    /// `all [words]` every match.
    Get { target: String },
    /// `drop <target>` â€” drop matches from the pack (same selectors as `get`).
    Drop { target: String },
    /// `give <item> <target>` â€” hand matches to a being here. Item-side
    /// selectors as in `get` (`give all sword bob`); the being is one
    /// recipient (`2.bob` for the second).
    Give { item: String, target: String },
    /// `steal <item> <target>` â€” take matches from a being's pack here.
    /// Selectors as in `give`. Existence checks only; no skill checks
    /// (example workflow).
    Steal { item: String, target: String },
    /// `commands` â€” list all registered commands
    Commands,
    /// `areas` â€” list every area in the world by its slug.
    Areas,
    /// `goto <address>` â€” admin-only. Teleport to a room resolved from an
    /// address (an entity id, `<area>:<room>`, or a bare room slug/grim id).
    Goto { target: String },
    /// `gecho <text>` â€” admin-only. Echo a message to every player in the world,
    /// including the sender. Other admins see it attributed (`Name> text`);
    /// everyone else sees the raw text.
    Gecho { text: String },
    /// `shutdown [seconds]` â€” admin-only. Schedules a graceful server shutdown
    /// after a countdown, broadcasting warnings to all connected players.
    Shutdown { seconds: u64 },
    /// `reboot [seconds]` â€” admin-only. Like `shutdown`, but the process exits
    /// non-zero at expiry so the service manager restarts it (cold restart:
    /// connections drop, the world reloads from disk).
    Reboot { seconds: u64 },
    /// `copyover [seconds]` â€” admin-only. Warns like `shutdown`, then hands the
    /// live listener + player sockets to a successor process (hot restart:
    /// players stay connected). The telnet transport performs the handoff.
    Copyover { seconds: u64 },
    /// `ban list [type]` â€” admin-only. List bans, optionally filtered by
    /// `ip` / `account` / `character`.
    /// `ban add <type> <pattern>` â€” admin-only. Block an IP (exact or
    /// `*`-wildcard prefix like `127.0.*`), an account (identifier or id), or
    /// a character (name; case-insensitive). Kicks every matching session.
    /// `ban remove <type> <pattern>` â€” admin-only. Lift a ban.
    Ban { op: BanOp },
}

/// Which identity a ban blocks: an IP, an account, or a character. Account and
/// character patterns match exactly and case-insensitively; IP patterns match
/// per-octet where `*` skips one octet (`127.*` bans `127.0.0.0/8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BanKind {
    Ip,
    Account,
    Character,
}

impl BanKind {
    /// Parse `ip` / `account` / `character` (case-insensitive). Anything else
    /// is `None`, so the command parser reports the line unknown.
    pub fn parse(s: &str) -> Option<Self> {
        if s.eq_ignore_ascii_case("ip") {
            Some(Self::Ip)
        } else if s.eq_ignore_ascii_case("account") {
            Some(Self::Account)
        } else if s.eq_ignore_ascii_case("character") {
            Some(Self::Character)
        } else {
            None
        }
    }

    /// Canonical lowercase name, as stored and displayed.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ip => "ip",
            Self::Account => "account",
            Self::Character => "character",
        }
    }
}

impl std::fmt::Display for BanKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `ban` sub-operation: which blocklist edit or query to apply.
#[derive(Debug, Clone, PartialEq)]
pub enum BanOp {
    /// `ban list [type]` â€” every ban, or only one kind.
    List { filter: Option<BanKind> },
    /// `ban add <type> <pattern>` â€” block and kick matches.
    Add { kind: BanKind, pattern: String },
    /// `ban remove <type> <pattern>` â€” lift.
    Remove { kind: BanKind, pattern: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ban_kind_parses_case_insensitively() {
        assert_eq!(BanKind::parse("ip"), Some(BanKind::Ip));
        assert_eq!(BanKind::parse("ACCOUNT"), Some(BanKind::Account));
        assert_eq!(BanKind::parse("Character"), Some(BanKind::Character));
        assert_eq!(BanKind::parse("email"), None);
        assert_eq!(BanKind::parse(""), None);
    }

    #[test]
    fn ban_kind_round_trips_display_and_serde() {
        for kind in [BanKind::Ip, BanKind::Account, BanKind::Character] {
            assert_eq!(BanKind::parse(&kind.to_string()), Some(kind));
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
            assert_eq!(serde_json::from_str::<BanKind>(&json).unwrap(), kind);
        }
    }
}
/// An object changed hands: picked up from a room or dropped into one.
/// Rendered per-recipient â€” the actor sees the first-party line ("You pick
/// up â€¦"), everyone else in the room the third-party line ("<name> picks
/// up â€¦"). `actor_name` and `short` are precomputed so renderers need no
/// lookups.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct ItemEvent {
    pub actor: Entity,
    pub room: Entity,
    pub actor_name: String,
    pub short: String,
    pub kind: ItemKind,
}

/// Which way an [`ItemEvent`] moved the object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Pickup,
    Drop,
}

/// An object moved between two packs in a room: handed over (`give`) or taken
/// (`steal`). Rendered per-recipient with all three wordings â€” the mover sees
/// the first-party line ("You give â€¦"), the other party the second-party line
/// ("â€¦ gives you â€¦" / "â€¦ steals your â€¦"), and the rest of the room the
/// third-party line ("â€¦ gives â€¦ to â€¦"). Names and the short are precomputed
/// so renderers need no lookups.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct TransferEvent {
    pub mover: Entity,
    pub mover_name: String,
    pub other: Entity,
    pub other_name: String,
    pub room: Entity,
    pub short: String,
    pub kind: TransferKind,
}

/// Which way a [`TransferEvent`] moved the object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    Give,
    Steal,
}

/// The `desc` sub-operation: which self-description edit to apply.
#[derive(Debug, Clone, PartialEq)]
pub enum DescOp {
    /// Bare `desc` â€” show your own paragraphs.
    Show,
    /// `desc clear` â€” drop every paragraph.
    Clear,
    /// `desc + <line>` â€” append one paragraph.
    Add(String),
    /// `desc -` â€” drop the last paragraph.
    Remove,
    /// `desc edit` â€” open the line editor on your paragraphs.
    Edit,
}

/// What an editor session edits. The callback routing: completion carries
/// this back, and each consumer handles its own kind. New consumers add a
/// variant, never a new event type.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorKind {
    /// The actor's own `Description` paragraphs (`desc edit`).
    Description,
}

/// Engine â†’ scene: open the line editor for `character`, preloaded with
/// `initial`. The scene attaches the modal session and shows the entry view.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct OpenEditor {
    pub character: Entity,
    pub kind: EditorKind,
    pub initial: Vec<String>,
}

/// Scene â†’ engine: the editor closed. `lines` is `Some` on `@save` (apply
/// them) and `None` on `@exit` (discard). The consumer confirms to its actor.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct EditorDone {
    pub character: Entity,
    pub kind: EditorKind,
    pub lines: Option<Vec<String>>,
}

// â”€â”€â”€ Engine â†’ Client (semantic events for formatting) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// "Show this room description to this client."
#[derive(Message, Debug)]
pub struct LookRoom {
    pub target: Entity,
    pub room: Entity,
}

/// "Show this entity's description to this client."
#[derive(Message, Debug)]
pub struct LookEntity {
    pub target: Entity,
    pub subject: Entity,
}

/// A character said something in a room. Broadcast to room occupants except the actor.
#[derive(Message, Debug)]
pub struct SayEvent {
    pub room: Entity,
    pub actor: Entity,
    pub text: String,
}

/// A character yelled something. Broadcast to all characters in the same area (rooms sharing an Area).
#[derive(Message, Debug)]
pub struct YellEvent {
    pub area: Entity,
    pub actor: Entity,
    pub text: String,
}

/// A character said something OOC (out of character). Broadcast globally.
#[derive(Message, Debug)]
pub struct OocEvent {
    pub actor: Entity,
    pub text: String,
}

/// An admin `gecho`. Broadcast to every player in the world, including the
/// sender. Rendering is per-recipient (see `format_output`): another admin sees
/// it attributed as `Name> text`; the sender and non-admins see the raw text.
#[derive(Message, Debug)]
pub struct GlobalEcho {
    pub actor: Entity,
    pub text: String,
}

/// A character moved from one room to another. Used for "X leaves north" / "X arrives" broadcasts.
#[derive(Message, Debug)]
pub struct MoveEvent {
    pub actor: Entity,
    pub from: Entity,
    pub to: Entity,
    pub direction: Cardinal,
}

/// A character opened or closed a door on an exit. Rendered per-recipient:
/// the actor gets a first-party line, same-room witnesses an attributed
/// line, and the far room hears the door move on its own side.
#[derive(Message, Debug)]
pub struct DoorEvent {
    pub actor: Entity,
    pub from: Entity,
    pub to: Entity,
    pub direction: Cardinal,
    pub opened: bool,
    pub name: String,
}

/// A character recalled to the Town Square. Rendered per-recipient in both
/// rooms: the room left sees the attempt then the disappearance; the room
/// entered sees a recall-marked arrival (never a plain walk-in `MoveEvent`).
#[derive(Message, Debug)]
pub struct RecallEvent {
    pub actor: Entity,
    pub from: Entity,
    pub to: Entity,
}

/// Direct text message to a specific client (via their character entity).
#[derive(Message, Debug)]
pub struct InfoMessage {
    pub target: Entity,
    pub text: String,
}

/// A character has entered the world. Shown to players in the same room.
/// Carries the subject entity so renderers resolve the room by identity,
/// never by name (NPCs share the name query and could collide).
#[derive(Message, Debug)]
pub struct LoginAnnounce {
    pub subject: Entity,
    pub name: String,
}

/// A character quit the game cleanly. Carries the room they stood in:
/// quit despawns the character entity first, so renderers cannot look it
/// up afterwards.
#[derive(Message, Debug)]
pub struct LogoutAnnounce {
    pub name: String,
    pub room: Entity,
}

/// A character went linkdead or reconnected. Shown to players in the same room.
/// Carries the subject entity (see `LoginAnnounce`).
#[derive(Message, Debug)]
pub struct LinkdeadAnnounce {
    pub subject: Entity,
    pub name: String,
    pub reconnecting: bool, // true = reconnecting, false = going linkdead
}

/// An out-of-band server message shown to every connected player, regardless of
/// room or scene. Used for shutdown-countdown warnings. `text` may contain
/// colour markup and should end with `\n`.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct ServerBroadcast {
    pub text: String,
}

/// A scheduled `copyover` countdown has expired: the telnet transport should
/// begin the fd handoff to a successor process now. Written by the
/// being-free shutdown tick (`grim-world`), read by `grim-networking-telnet`.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyoverDue;

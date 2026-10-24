use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cardinal::Cardinal;

// ─── Client → Engine ─────────────────────────────────────────────────

#[derive(Message, Debug)]
pub struct EngineCommand {
    pub client: Entity,
    pub command: Command,
}

mod command;
pub use command::Command;

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
    /// `ban list [type]` — every ban, or only one kind.
    List { filter: Option<BanKind> },
    /// `ban add <type> <pattern>` — block and kick matches.
    Add { kind: BanKind, pattern: String },
    /// `ban remove <type> <pattern>` — lift.
    Remove { kind: BanKind, pattern: String },
}

/// An object changed hands: picked up from a room or dropped into one.
/// Rendered per-recipient — the actor sees the first-party line ("You pick
/// up …"), everyone else in the room the third-party line ("<name> picks
/// up …"). `actor_name` and `short` are precomputed so renderers need no
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
/// (`steal`). Rendered per-recipient with all three wordings — the mover sees
/// the first-party line ("You give …"), the other party the second-party line
/// ("… gives you …" / "… steals your …"), and the rest of the room the
/// third-party line ("… gives … to …"). Names and the short are precomputed
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
    /// Bare `desc` — show your own paragraphs.
    Show,
    /// `desc clear` — drop every paragraph.
    Clear,
    /// `desc + <line>` — append one paragraph.
    Add(String),
    /// `desc -` — drop the last paragraph.
    Remove,
    /// `desc edit` — open the line editor on your paragraphs.
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

/// Engine → scene: open the line editor for `character`, preloaded with
/// `initial`. The scene attaches the modal session and shows the entry view.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct OpenEditor {
    pub character: Entity,
    pub kind: EditorKind,
    pub initial: Vec<String>,
}

/// Scene → engine: the editor closed. `lines` is `Some` on `@save` (apply
/// them) and `None` on `@exit` (discard). The consumer confirms to its actor.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct EditorDone {
    pub character: Entity,
    pub kind: EditorKind,
    pub lines: Option<Vec<String>>,
}

// ─── Engine → Client (semantic events for formatting) ───────────────

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

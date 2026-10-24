//! The player-character being ([`Character`]) and the [`Role`] privileges it can
//! hold.
//!
//! A `Character` is the **PC-only** half of a being: it belongs to an account and
//! carries the persistent player state that a creature has no use for (account,
//! roles, class, title, restrings, last room). The shared "alive thing" fields
//! (race, level, gender) live on [`Actor`](crate::Actor), and the display name
//! lives on the `Name` component (`grim_core::components::Name`) â€” a
//! `Character` no longer stores a `name`.
//!
//! `Character` is not itself serialized: its on-disk form is the flat
//! [`StoredCharacter`](crate::StoredCharacter) DTO, which is the only disk
//! surface and preserves the pre-split JSON layout.

use std::collections::HashMap;

use bevy::prelude::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use grim_core::id::GrimId;
use grim_world::RoomLocation;

/// A privilege a character holds. Serialized lowercase (`"admin"`) so the
/// character JSON stays human-editable â€” granting admin is a manual JSON edit
/// today (see `docs/DEPLOY.md`). Serde stays on `Role` because the
/// [`StoredCharacter`](crate::StoredCharacter) DTO carries it to disk.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Tester,
}

/// A player character â€” the PC-only being. Belongs to an account, can be
/// in-world. The name lives in the `Name` component and the shared race/level/
/// gender in [`Actor`](crate::Actor); this struct holds only the PC-specific
/// persistent state. Its disk form is [`StoredCharacter`](crate::StoredCharacter).
#[derive(Component, Clone, Debug)]
pub struct Character {
    pub id: GrimId,
    pub account_id: GrimId,
    /// Available money, measured in whole credits.
    pub credits: u64,
    pub created_at: DateTime<Utc>,
    /// Last known room, as a stable [`RoomLocation`] (area + room `friendly_id`s).
    pub last_room: Option<RoomLocation>,
    /// Privileges. Empty for normal players.
    pub roles: Vec<Role>,
    /// Class slug (e.g. `"warrior"`), keyed into `grim_world::ClassRegistry`.
    /// Holds a single tier-1 slug for now; a future reroll swaps it to the
    /// tier-2 `evolves_to` (see `docs/adr/0002-character-class-tiers.md`).
    pub class: String,
    /// An optional self-set descriptor shown after the name in the WHO list.
    /// Capped at 60 chars by the `title` command.
    pub title: Option<String>,
    /// Persisted per-character display overrides, keyed by a recognized name.
    /// The WHO renderer honours `who_level`, `who_gender`, `who_race`,
    /// `who_class`, `who_guild` (each overrides one stat column) and `who`
    /// (replaces the whole stat block). No in-game setter yet â€” edited on disk.
    pub restrings: HashMap<String, String>,
    /// Per-character config choices, keyed by registered option (`grim-config`).
    /// Only registered + valid values apply at read (via
    /// `ConfigRegistry::resolve`); anything else falls back to the default.
    /// Empty for older characters â€” which therefore run all defaults.
    pub config: HashMap<String, String>,
    /// Namespaced, game-defined persistent state. Engine systems preserve it unchanged.
    pub game_state: HashMap<String, serde_json::Value>,
}

impl Character {
    pub fn is_tester(&self) -> bool {
        self.roles.contains(&Role::Tester)
    }

    pub fn is_hidden(&self) -> bool {
        self.is_tester()
            || (self.is_admin()
                && self.game_state.get("wizinvis").and_then(|v| v.as_bool()) == Some(true))
    }

    /// Self and administrators can supervise hidden actors; ordinary users cannot.
    pub fn visible_to(&self, viewer: Option<&Character>, same: bool) -> bool {
        !self.is_hidden() || same || viewer.is_some_and(Character::is_admin)
    }
    /// Whether this character holds the admin role.
    pub fn is_admin(&self) -> bool {
        self.roles.contains(&Role::Admin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn character(roles: Vec<Role>) -> Character {
        Character {
            credits: 100,
            id: GrimId::new(),
            account_id: GrimId::new(),
            created_at: chrono::Utc::now(),
            last_room: None,
            roles,
            class: String::new(),
            title: None,
            restrings: HashMap::new(),
            config: HashMap::new(),
            game_state: Default::default(),
        }
    }

    #[test]
    fn is_admin_reflects_roles() {
        assert!(!character(vec![]).is_admin());
        assert!(character(vec![Role::Admin]).is_admin());
    }

    #[test]
    fn role_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Role::Admin).unwrap(), "\"admin\"");
        assert_eq!(
            serde_json::from_str::<Role>("\"admin\"").unwrap(),
            Role::Admin
        );
    }
}

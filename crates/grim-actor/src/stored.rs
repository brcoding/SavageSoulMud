//! [`StoredCharacter`]: the flat on-disk DTO for a player character.
//!
//! After the being split a PC is three components â€” `Name + Actor + Character`.
//! Persistence and copyover need a single serializable record, so
//! `StoredCharacter` is the **only** serde surface for a character. Its field set
//! is byte-for-byte the pre-split `Character` JSON layout â€” every optional field
//! keeps its `#[serde(default)]` â€” so **old `data/characters/<name>.json` files
//! still load unchanged**. That compatibility is the contract; a round-trip test
//! against a pre-split blob guards it.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use grim_core::character::Gender;
use grim_core::components::Name;
use grim_core::id::GrimId;
use grim_world::RoomLocation;
use serde::{Deserialize, Serialize};

use crate::actor::Actor;
use crate::character::{Character, Role};

/// The level every new (or pre-level-field) character starts at.
fn default_level() -> u32 {
    1
}

/// The starting credit balance for new and pre-credit-field characters.
fn default_credits() -> u64 {
    1000
}

/// The flat, serializable form of a player character â€” the only disk surface.
/// Splits into `Name + Actor + Character` via [`into_components`], and is built
/// from those three via [`from_components`]. Field order and `#[serde(default)]`
/// placement mirror the pre-split `Character` exactly to keep the on-disk format
/// stable.
///
/// [`into_components`]: StoredCharacter::into_components
/// [`from_components`]: StoredCharacter::from_components
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StoredCharacter {
    pub id: GrimId,
    pub name: String,
    pub account_id: GrimId,
    /// Whole-credit balance. Older saves receive the starting balance once.
    #[serde(default = "default_credits")]
    pub credits: u64,
    pub created_at: DateTime<Utc>,
    /// Last known room. `#[serde(default)]` keeps old JSON loading cleanly.
    #[serde(default)]
    pub last_room: Option<RoomLocation>,
    /// Privileges. `#[serde(default)]` keeps pre-roles JSON loading cleanly.
    #[serde(default)]
    pub roles: Vec<Role>,
    /// `#[serde(default)]` â†’ [`Gender::Neutral`] for pre-gender JSON.
    #[serde(default)]
    pub gender: Gender,
    /// Race slug. Empty on old JSON (and never picked); resolve leniently.
    #[serde(default)]
    pub race: String,
    /// Class slug. Empty on old JSON.
    #[serde(default)]
    pub class: String,
    /// Character level. `#[serde(default = ...)]` gives old JSON level 1.
    #[serde(default = "default_level")]
    pub level: u32,
    /// Optional WHO title. `#[serde(default)]` keeps pre-title JSON loading.
    #[serde(default)]
    pub title: Option<String>,
    /// Per-character display overrides. `#[serde(default)]` keeps old JSON loading.
    #[serde(default)]
    pub restrings: HashMap<String, String>,
    /// Per-character config choices. `#[serde(default)]` keeps pre-config JSON
    /// loading with an empty map (all registry defaults).
    #[serde(default)]
    pub config: HashMap<String, String>,
    /// Namespaced game/plugin data, preserved across saves and copyovers.
    #[serde(default)]
    pub game_state: HashMap<String, serde_json::Value>,
    /// Full snapshot of every carried object instance, taken at save and
    /// re-spawned on login. `#[serde(default)]` keeps pre-inventory JSON
    /// loading with an empty pack. Ground objects are never stored here â€”
    /// they respawn from area blueprints â€” so a reboot restores the pack
    /// while the room listing regenerates from seed.
    #[serde(default)]
    pub inventory: Vec<StoredObject>,
}

/// The full on-disk snapshot of one carried object instance. Every instance
/// is stored whole (never by reference): a reboot re-spawns the seed copy
/// from its blueprint *and* restores this one into the pack, so duplicates
/// after pickup-then-reboot are correct state, not corruption.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StoredObject {
    /// Optional per-instance condition, including a corpse's absolute expiry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<grim_core::components::ItemCondition>,
    /// Short name: inventory rows, pickup lines, get/drop/give matching.
    #[serde(default)]
    pub name: String,
    /// Look paragraphs.
    #[serde(default)]
    pub description: Vec<String>,
    /// Get/drop/give keywords.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Long room-listing line.
    #[serde(default)]
    pub room_description: String,
}
impl StoredCharacter {
    /// Split the DTO into the three components a live PC carries.
    pub fn into_components(self) -> (Name, Actor, Character) {
        let name = Name(self.name);
        let actor = Actor {
            race: self.race,
            level: self.level,
            gender: self.gender,
        };
        let character = Character {
            credits: self.credits,
            id: self.id,
            account_id: self.account_id,
            created_at: self.created_at,
            last_room: self.last_room,
            roles: self.roles,
            class: self.class,
            title: self.title,
            restrings: self.restrings,
            config: self.config,
            game_state: self.game_state,
        };
        (name, actor, character)
    }

    /// Build the DTO from a live PC's components, ready to serialize to disk.
    pub fn from_components(name: &Name, actor: &Actor, character: &Character) -> Self {
        Self {
            credits: character.credits,
            id: character.id,
            name: name.0.clone(),
            account_id: character.account_id,
            created_at: character.created_at,
            last_room: character.last_room.clone(),
            roles: character.roles.clone(),
            gender: actor.gender,
            race: actor.race.clone(),
            class: character.class.clone(),
            level: actor.level,
            title: character.title.clone(),
            restrings: character.restrings.clone(),
            config: character.config.clone(),
            game_state: character.game_state.clone(),
            // Built from live components, which never carry a pack â€” saves
            // attach the snapshot afterwards (see `grim-object::persist`).
            inventory: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_json_without_new_fields_loads_with_defaults() {
        // Pre-split character file: only the fields that always existed. Ids are
        // Grim IDs (base62 x12). Every added field must default cleanly.
        let json = r#"{"id":"aaaaaaaaaaaa","name":"Old","account_id":"bbbbbbbbbbbb","created_at":"2020-01-01T00:00:00Z"}"#;
        let stored: StoredCharacter = serde_json::from_str(json).unwrap();
        assert_eq!(stored.name, "Old");
        assert!(stored.last_room.is_none());
        assert!(stored.roles.is_empty());
        assert_eq!(stored.gender, Gender::Neutral);
        assert!(stored.race.is_empty());
        assert!(stored.class.is_empty());
        assert_eq!(stored.level, 1);
        assert_eq!(stored.credits, 1000);
        assert!(stored.title.is_none());
        assert!(stored.restrings.is_empty());
        assert!(stored.config.is_empty());
        assert!(stored.game_state.is_empty());
    }

    #[test]
    fn round_trips_through_components() {
        let mut stored: StoredCharacter = serde_json::from_str(
            r#"{"id":"aaaaaaaaaaaa","name":"Hero","account_id":"bbbbbbbbbbbb","created_at":"2020-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        stored.gender = Gender::Female;
        stored.race = "dwarf".into();
        stored.class = "mage".into();
        stored.level = 7;
        stored.credits = 42;
        stored.title = Some("the Bold".into());
        stored.roles = vec![Role::Admin];
        stored.restrings.insert("who_class".into(), "God".into());
        stored.config.insert("minimap".into(), "off".into());
        stored.game_state.insert(
            "savage_soul".into(),
            serde_json::json!({"version": 1, "attributes": {"strength": 3}, "peculiarities": ["survivor"]}),
        );
        stored.game_state.insert("other_plugin".into(), serde_json::json!([true, null, 42]));
        stored.last_room = Some(RoomLocation {
            area: "haven".into(),
            room: "square".into(),
        });

        let original = stored.clone();
        let (name, actor, character) = stored.into_components();
        // Fields land on the right component.
        assert_eq!(name.0, "Hero");
        assert_eq!(actor.race, "dwarf");
        assert_eq!(actor.level, 7);
        assert_eq!(actor.gender, Gender::Female);
        assert_eq!(character.class, "mage");
        assert_eq!(character.credits, 42);
        assert!(character.is_admin());
        assert_eq!(character.game_state, original.game_state);
        assert_eq!(character.title.as_deref(), Some("the Bold"));
        assert_eq!(character.config.get("minimap"), Some(&"off".to_string()));

        let rebuilt = StoredCharacter::from_components(&name, &actor, &character);
        // Re-serializing yields the same JSON as the original DTO.
        assert_eq!(
            serde_json::to_string(&rebuilt).unwrap(),
            serde_json::to_string(&original).unwrap()
        );
    }

    #[test]
    fn explicit_zero_credits_is_not_reset() {
        let stored: StoredCharacter = serde_json::from_str(
            r#"{"id":"aaaaaaaaaaaa","name":"Broke","account_id":"bbbbbbbbbbbb","created_at":"2020-01-01T00:00:00Z","credits":0}"#,
        )
        .unwrap();
        let (name, actor, character) = stored.into_components();
        assert_eq!(character.credits, 0);
        assert_eq!(StoredCharacter::from_components(&name, &actor, &character).credits, 0);
    }

    #[test]
    fn gender_serializes_lowercase() {
        let stored: StoredCharacter = serde_json::from_str(
            r#"{"id":"aaaaaaaaaaaa","name":"H","account_id":"bbbbbbbbbbbb","created_at":"2020-01-01T00:00:00Z","gender":"female"}"#,
        )
        .unwrap();
        assert_eq!(stored.gender, Gender::Female);
        let json = serde_json::to_string(&stored).unwrap();
        assert!(json.contains("\"gender\":\"female\""));
    }
}

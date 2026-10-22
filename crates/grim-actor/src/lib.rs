//! `grim-actor`: the "beings" of the world and the command verbs that read them.
//!
//! An actor is any entity that can *act* in the world and be placed in a room —
//! player characters and creatures (mobs). Every being carries the shared
//! [`Actor`] base (race/level/gender); a PC additionally carries a [`Character`]
//! (account/roles/class/…) and, while connected, a [`Player`]; a mob carries a
//! [`Creature`] marker. This crate owns those being components ([`Actor`],
//! [`Character`], [`Creature`], [`Player`], [`InRoom`], [`Linkdead`],
//! [`OutputHistory`], [`Role`]), the flat [`StoredCharacter`] disk DTO, and the
//! being-reading command handlers (`look`, `move`, `goto`, `quit`, `title`,
//! `shutdown`).
//!
//! Entity composition: online PC = `Name + Actor + Character + Player + InRoom`;
//! linkdead PC = `Name + Actor + Character + Linkdead + InRoom` (no `Player`);
//! creature = `Name + Actor + Creature + InRoom`.
//!
//! It depends on `grim-world` (rooms/areas/exits + address lookups) and never
//! the reverse: the world's static topology knows nothing about who stands in
//! it, so `grim-world` is being-free and the actor layer sits above it.

pub mod actor;
pub mod character;
pub mod commands;
pub mod occupants;
pub mod placement;
pub mod player;
pub mod plugin;
pub mod stored;
pub mod transition;

pub use actor::{Actor, Creature};
pub use character::{Character, Role};
pub use occupants::{beings_in, in_room, is_addressable, Beings};
pub use placement::InRoom;
pub use player::{Linkdead, OutputHistory, Player};
pub use plugin::ActorPlugin;
pub use stored::{StoredCharacter, StoredObject};
pub use transition::{AttemptEnter, AttemptLeave, AttemptWalk, Enter, Leave};

pub mod visibility;
pub use visibility::DarkVision;

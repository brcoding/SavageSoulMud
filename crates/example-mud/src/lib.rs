//! example-mud â€” the vanilla GRIM game a MUD author starts from.
//!
//! The world seed is exposed as a library so the end-to-end test harness
//! (`tests/`) boots the *same* world the binary ships, rather than a
//! divergent copy.

mod consumables;
pub mod savage_rules;
pub mod savage_soul;
pub mod seed;
mod seed_doors;
pub mod shop;

pub mod help;

mod desk;

mod gate;

mod royal_oak;
mod trainer;
mod trainer_ai;

mod conditions;
mod hack;
mod medical;
pub mod population;
mod preferences;

mod crafting;

mod morlock;

mod client_protocols;
mod combat_prose;
pub mod intermud;
mod zebedee;

mod grapevine;

pub use grapevine::DisableGrapevine;

pub mod species;

pub mod creator;

mod client_auth;
mod game_commands;

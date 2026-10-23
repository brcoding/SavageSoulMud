//! example-mud â€” the vanilla GRIM game a MUD author starts from.
//!
//! The world seed is exposed as a library so the end-to-end test harness
//! (`tests/`) boots the *same* world the binary ships, rather than a
//! divergent copy.

pub mod seed;
pub mod shop;
pub mod savage_rules;
pub mod savage_soul;
mod consumables;
mod seed_doors;

pub mod help;

mod desk;

mod gate;

mod royal_oak;
mod trainer;
mod trainer_ai;

mod preferences;
mod conditions;
pub mod population;
mod medical;
mod hack;

mod crafting;

mod morlock;

pub mod intermud;
mod zebedee;
mod client_protocols;
mod combat_prose;

mod grapevine;

pub use grapevine::DisableGrapevine;

pub mod species;

pub mod creator;

mod client_auth;
mod game_commands;

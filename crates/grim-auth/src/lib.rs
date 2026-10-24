//! `grim-auth` — the pre-game phase (`AuthPlugin`).
//!
//! Owns the login / account-creation / character-select / MOTD flow that runs
//! before a session enters the world. Still driven by `grim_core::ClientState`
//! (the scene-stack model remains deferred — see ARCHITECTURE.md §8); it is the
//! pre-game layer above the session core in `grim-scene`.
//!
//! Cohesive concerns live in sibling modules; this file is a shell that wires
//! them together and re-exports the public plugin.

mod accessibility;
mod account;
mod build_mode;
mod character_select;
mod creation;
mod creation_defaults;
mod finalize;
mod greeter;
mod input;
mod login;
mod new_account;
mod params;
mod password;
mod plugin;
mod throttle;
mod world_entry;

pub mod validation;

pub use accessibility::AccessibilityOnboarding;
pub use build_mode::CharacterBuildMode;
pub use greeter::LoginVersion;
pub use plugin::AuthPlugin;
pub use throttle::ReconnectLimits;
pub use validation::ReservedNamePrefixes;

pub use creation_defaults::{CharacterCreationDefaults, StartingCredits};

mod client_login;
pub use client_login::{AuthSystems, LoginCredentials, LoginResult};

#[cfg(test)]
mod tests;

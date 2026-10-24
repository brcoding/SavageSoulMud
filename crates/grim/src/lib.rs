//! GRIM — Game Runtime for Interactive Multiplayer.
//!
//! This crate is a **facade**: it depends on the subsystem crates, re-exports
//! their public surface, and offers a default plugin group. A MUD author who
//! wants the defaults depends on `grim` alone; one who wants to swap a piece
//! depends on the subsystem crates directly. Nothing here is privileged.

#![allow(ambiguous_glob_reexports)]

pub use bevy::prelude::*;
pub use grim_core::*;

// Placement Phase 1/2a relocated these domain types out of the
// `grim-core` god-node into their owning subsystem crates. Re-export
// them at the crate root (and via `prelude`) so `grim::X` / `grim::prelude::X`
// stay stable downstream.
pub mod prelude;
pub use grim_actor::{
    Actor, AttemptWalk, Character, Creature, DarkVision, InRoom, Linkdead, OutputHistory, Player,
    Role, StoredCharacter, StoredObject,
};
pub use grim_auth::{
    AccessibilityOnboarding, AuthPlugin, AuthSystems, CharacterBuildMode,
    CharacterCreationDefaults, LoginCredentials, LoginResult, LoginVersion, ReservedNamePrefixes,
    StartingCredits,
};
pub use grim_channel::LastWhisperFrom;
pub use grim_config::{ConfigDef, ConfigRegistry, Scope as ConfigScope};
pub use grim_object::{CarriedBy, Object, ObjectPlugin};
pub use grim_scene::{ConnectedAt, IdleConfig, SceneSystems};
pub use grim_script::{
    compile, CompiledTrigger, ScriptPlugin, ScriptTriggers, TriggerDef, TriggerKind,
};
pub use grim_social::{SocialDef, SocialDir, SocialPerformed, SocialRegistry};
pub use grim_world::{
    render_map, Area, ClassDef, ClassRegistry, Door, Doors, Exits, LightSource, MapConfig, RaceDef,
    RaceRegistry, Room, RoomDarkness, RoomLocation, StartingRoom,
};

// Transport-agnostic networking primitives (Connection + wire events).
pub use grim_networking::{self as networking, *};
pub use grim_networking_telnet::{TelnetCapabilities, TelnetInput, TelnetOutput, TelnetSystems};

// The text catalog. `tr` is re-exported at the crate root so `grim::tr` (the
// function) and `grim::tr!` (the macro, via #[macro_export]) both resolve.
pub use grim_text::tr;

// Command resolution.
pub use grim_command::{CommandRegistry, Contest};

// Target parsing + keyword query (shared by every verb that names something).
pub use grim_target::{parse_target, query, rank_match, ParseOptions, Selector, TargetSpec};

// Command events (semantic intents).
pub use grim_command_events::*;

pub mod plugins;
pub use plugins::*;

// The default plugin groups. Definitions live in their own module so `lib.rs`
// stays declarations + re-exports only (module-size convention).
mod plugin_groups;
pub use plugin_groups::{GrimDefaultPlugins, GrimHeadlessPlugins};

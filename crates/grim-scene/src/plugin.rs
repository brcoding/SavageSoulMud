//! `ScenePlugin`: wires up the session-lifecycle resources, message types, and
//! systems that make up the scene subsystem. The pre-game flow (login /
//! creation / character-select / MOTD) lives in the auth crate; this plugin owns
//! the in-game input dispatch, output formatting, and copyover resume.

use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::*;
use grim_core::events::{
    DoorEvent, EngineCommand, GlobalEcho, InfoMessage, ItemEvent, LinkdeadAnnounce, LookEntity,
    LookRoom, MoveEvent, OocEvent, RecallEvent, SayEvent, ServerBroadcast, TransferEvent,
    YellEvent,
};
use grim_core::events::{LoginAnnounce, LogoutAnnounce};
use grim_networking::{ConnectionOutput, ConnectionResumed, DisconnectRequest};
use grim_world::{ClassRegistry, RaceRegistry};

use crate::command::process_command_queue;
use crate::idle::{check_idle, touch_sessions_on_input, IdleConfig};
use crate::input::handle_ingame_input;
use crate::item_output::{format_item_events, format_look_pack, format_transfer_events};
use crate::output::{capture_output, format_output, format_server_broadcast};
use crate::parser;
use crate::recall_output::format_recall;
use crate::resume::handle_connection_resumed;
use crate::session::JustEnteredWorld;

/// Public ordering handle for the scene's in-game input dispatch. The auth
/// pre-game input system runs `.before(SceneSystems::InGameInput)` so a line
/// that advances a session into the world is consumed there and not
/// re-dispatched as an in-game command the same tick (see [`JustEnteredWorld`]
/// and `input.rs`).
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneSystems {
    /// Activity stamping + AFK clearing ([`touch_sessions_on_input`]). Runs
    /// before every dispatcher so a returning line clears AFK before it is
    /// interpreted. The auth pre-game system runs `.after` this set.
    TouchInput,
    /// The in-game input dispatcher ([`handle_ingame_input`]).
    InGameInput,
    Visibility,
    Output,
    Staff,
}

pub struct ScenePlugin;
impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, crate::staff::sync_testing);
        app.configure_sets(
            Update,
            (
                SceneSystems::Visibility,
                SceneSystems::InGameInput,
                SceneSystems::Staff,
                SceneSystems::Output,
            )
                .chain(),
        );
        app.add_systems(
            Update,
            (
                crate::staff::sync_testing.in_set(SceneSystems::Visibility),
                crate::staff::handle_staff
                    .after(process_command_queue)
                    .in_set(SceneSystems::Staff),
            ),
        );
        crate::ban::register(app);
        app.insert_resource(parser::command_registry());
        // Scene writes character JSON on `quit` and reads it on copyover resume,
        // so it needs the persistence directory. init_resource so ScenePlugin
        // stands alone; if PersistencePlugin is also present its identical
        // default is a no-op.
        app.init_resource::<grim_persistence::PersistenceConfig>();
        // The `ban` command and the resume refusal read the blocklist.
        // init_resource so ScenePlugin stands alone; when PersistencePlugin is
        // also present its Startup load replaces this empty default with the
        // `bans.json` contents.
        app.init_resource::<grim_persistence::BanList>();
        // Playable races/classes: the WHO list reads their abbreviations. The
        // character-creation flow (auth crate) also reads them and validates the
        // effective set is non-empty; both plugins init_resource idempotently.
        app.init_resource::<RaceRegistry>();
        app.init_resource::<ClassRegistry>();
        // Player settings the look minimap resolves (`minimap` on/off).
        // init_resource so ScenePlugin stands alone; ActorPlugin seeds the
        // `minimap` definition into the same registry when present.
        app.init_resource::<grim_config::ConfigRegistry>();
        // Wiznet prefs (master + per-category on/off switches). Seeded here
        // because the `wiznet` command and the broadcast live in this crate.
        {
            let mut registry = app
                .world_mut()
                .resource_mut::<grim_config::ConfigRegistry>();
            crate::wiznet::seed_registry(&mut registry);
        }
        // Per-tick set of connections a pre-game handler advanced to InGame; the
        // in-game input system consults it to avoid re-dispatching the line that
        // triggered the transition (see input.rs).
        app.init_resource::<JustEnteredWorld>();
        // Idle thresholds (seconds); an author overrides by inserting a custom
        // value. init_resource so ScenePlugin stands alone.
        app.init_resource::<IdleConfig>();
        // Admin snapshot backing the offline half of the `wizlist`: read once
        // at startup (a reboot refreshes it). init_resource so the `Res` in
        // `SessionRes` exists before the loader replaces it.
        app.init_resource::<crate::wizlist::WizlistAdmins>();
        app.add_systems(Startup, crate::wizlist::load_wizlist_admins);
        crate::editor::register(app);
        crate::wiznet::register(app);
        app.add_message::<ConnectionOutput>()
            .add_message::<ConnectionResumed>()
            .add_message::<DisconnectRequest>()
            .add_message::<EngineCommand>()
            .add_message::<LookRoom>()
            .add_message::<LookEntity>()
            .add_message::<SayEvent>()
            .add_message::<YellEvent>()
            .add_message::<OocEvent>()
            .add_message::<GlobalEcho>()
            .add_message::<MoveEvent>()
            .add_message::<DoorEvent>()
            .add_message::<RecallEvent>()
            .add_message::<ItemEvent>()
            .add_message::<TransferEvent>()
            .add_message::<InfoMessage>()
            .add_message::<LoginAnnounce>()
            .add_message::<LogoutAnnounce>()
            .add_message::<LinkdeadAnnounce>()
            .add_message::<ServerBroadcast>()
            .add_message::<grim_networking::WiznetAlert>()
            .add_systems(
                Update,
                (
                    handle_connection_resumed.before(SceneSystems::Visibility),
                    touch_sessions_on_input
                        .in_set(SceneSystems::TouchInput)
                        .before(SceneSystems::InGameInput),
                    handle_ingame_input.in_set(SceneSystems::InGameInput),
                    // After dispatch: the sweep observes post-input state, so a
                    // returning line (touch just cleared AFK, refreshed activity)
                    // is never re-flagged in the same tick.
                    check_idle.after(SceneSystems::InGameInput),
                    process_command_queue,
                    format_output.in_set(SceneSystems::Output),
                    format_item_events.in_set(SceneSystems::Output),
                    format_transfer_events.in_set(SceneSystems::Output),
                    // The pack block belongs below the description `format_output`
                    // just wrote for the same `LookEntity`.
                    format_look_pack
                        .after(format_output)
                        .in_set(SceneSystems::Output),
                    format_server_broadcast.in_set(SceneSystems::Output),
                    format_recall.in_set(SceneSystems::Output),
                    capture_output,
                ),
            );
    }
}

//! System-parameter bundles.
//!
//! Bevy caps a system at 16 parameters. Several scene systems need more
//! resources/queries than that, so cohesive groups are bundled into
//! `#[derive(SystemParam)]` structs here and shared across the in-game
//! dispatcher, resume, and output systems.

use crate::session::ConnectedAt;
use bevy::prelude::*;
use grim_actor::{Actor, Character, Creature, InRoom, Linkdead};
use grim_core::components::{Client, Name as GrimName};
use grim_core::events::{Command, DoorEvent, LinkdeadAnnounce, LoginAnnounce, LogoutAnnounce};
use grim_world::{Area, ClassRegistry, RaceRegistry, Room, RoomLocation};

/// The online-characters query shared by the WHO / WHERE renderers: each player
/// entity with its display name, current room, optional [`Actor`] base
/// (race/level/gender WHO stats), optional [`Character`] (admin/title/restrings
/// WHO stats), optional [`Creature`] mob marker (the WHERE being filter), and
/// optional [`ConnectedAt`] (the WHO connect-time sort tiebreak).
pub(crate) type PlayerChars<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static GrimName,
        &'static InRoom,
        Option<&'static Actor>,
        Option<&'static Character>,
        Option<&'static Creature>,
        Option<&'static ConnectedAt>,
    ),
>;

/// Session-scoped resources bundled into one `SystemParam` so the in-game
/// dispatcher can take the command registry as a real `Res` without exceeding
/// Bevy's 16-parameter system limit. The race/class registries feed the WHO
/// column abbreviations; the wizlist snapshot feeds the offline half of the
/// `wizlist`.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct SessionRes<'w> {
    pub(crate) persistence: Res<'w, grim_persistence::PersistenceConfig>,
    pub(crate) registry: Res<'w, grim_command::CommandRegistry<Command>>,
    pub(crate) races: Res<'w, RaceRegistry>,
    pub(crate) classes: Res<'w, ClassRegistry>,
    pub(crate) wizlist: Res<'w, crate::wizlist::WizlistAdmins>,
    pub(crate) time: Res<'w, Time>,
}

/// Rooms + areas bundled so placement code can resolve a persisted
/// [`RoomLocation`] (stable area/room `friendly_id`s) to the *current* room
/// entity. Bundled as one `SystemParam` to stay within Bevy's 16-parameter
/// system limit, and it also serves the plain room lookups the dispatcher does.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct RoomResolver<'w, 's> {
    pub(crate) rooms: Query<'w, 's, (Entity, &'static Room, &'static GrimName)>,
    pub(crate) areas: Query<'w, 's, &'static Area>,
}

impl RoomResolver<'_, '_> {
    /// The room entity matching this location's area + room `friendly_id`s, if
    /// it exists in the currently-loaded world.
    pub(crate) fn resolve(&self, loc: &RoomLocation) -> Option<Entity> {
        self.rooms.iter().find_map(|(e, r, _)| {
            let area_matches = self
                .areas
                .get(r.area)
                .map(|a| a.friendly_id == loc.area)
                .unwrap_or(false);
            (r.friendly_id == loc.room && area_matches).then_some(e)
        })
    }

    /// Where to place a character entering the world: their persisted
    /// `last_room` if it still resolves, else the starting room. New characters
    /// (no `last_room`) and characters whose room no longer exists both fall
    /// back to `starting`.
    pub(crate) fn placement(&self, last_room: Option<&RoomLocation>, starting: Entity) -> Entity {
        last_room
            .and_then(|loc| self.resolve(loc))
            .unwrap_or(starting)
    }
}

/// Everything `format_output` reads besides its inputs: the session announces
/// plus the door facts plus the live-session and linkdead lookups presence
/// rendering needs. Bundled because that system is at the 16-parameter ceiling.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct OutputReads<'w, 's> {
    pub(crate) visibility: grim_actor::visibility::Visibility<'w,'s>,
    pub(crate) terminals: Query<'w, 's, &'static grim_networking::TerminalSize>,
    pub(crate) login: MessageReader<'w, 's, LoginAnnounce>,
    pub(crate) logout: MessageReader<'w, 's, LogoutAnnounce>,
    pub(crate) linkdead: MessageReader<'w, 's, LinkdeadAnnounce>,
    pub(crate) doors: MessageReader<'w, 's, DoorEvent>,
    pub(crate) clients: Query<'w, 's, &'static Client>,
    pub(crate) linkdead_chars: Query<'w, 's, &'static Linkdead>,
}

//! System-parameter bundles for the pre-game flow.
//!
//! Bevy caps a system at 16 parameters. The login/creation dispatcher and the
//! world-entry placement need more resources/queries than that, so cohesive
//! groups are bundled into `#[derive(SystemParam)]` structs here.
//!
//! These are grim-auth's OWN params over the underlying components/resources —
//! deliberately NOT grim-scene's param structs (coupling to scene's internal
//! `SystemParam`s is avoided; only genuinely shared domain helpers like
//! `grim_scene::formatter` are called across the crate boundary).

use bevy::prelude::*;
use grim_actor::{Actor, Character, InRoom, Linkdead, OutputHistory, Player};
use grim_core::components::Name as GrimName;
use grim_core::events::LinkdeadAnnounce;
use grim_networking::{DisconnectRequest, WiznetAlert};
use grim_scene::ConnectedAt;
use grim_world::{Area, ClassRegistry, RaceRegistry, Room, RoomLocation, StartingRoom};

use crate::validation::ReservedNamePrefixes;

/// The online-characters query shared by the post-login auto-look: each player
/// entity with its display name, current room, optional [`Actor`] base, optional
/// [`Character`], and optional [`ConnectedAt`]. Mirrors the scene WHO query shape
/// so the MOTD hand-off reads the same columns.
pub(crate) type PlayerChars<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static GrimName,
        &'static InRoom,
        Option<&'static Actor>,
        Option<&'static Character>,
        Option<&'static ConnectedAt>,
    ),
>;

/// Session-scoped resources the login/creation flow reads, bundled into one
/// `SystemParam` so the pre-game dispatcher stays within Bevy's 16-parameter
/// system limit. Unlike the scene's `SessionRes`, this carries no command
/// registry — the pre-game phase never parses in-game commands.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct SessionRes<'w> {
    pub(crate) accessibility: Option<Res<'w, crate::AccessibilityOnboarding>>,
    pub(crate) creation_defaults: Option<Res<'w, crate::CharacterCreationDefaults>>,
    pub(crate) build_mode: Option<Res<'w, crate::CharacterBuildMode>>,
    pub(crate) starting: Res<'w, StartingRoom>,
    pub(crate) persistence: Res<'w, grim_persistence::PersistenceConfig>,
    pub(crate) bans: Res<'w, grim_persistence::BanList>,
    pub(crate) reserved: Res<'w, ReservedNamePrefixes>,
    pub(crate) races: Res<'w, RaceRegistry>,
    pub(crate) classes: Res<'w, ClassRegistry>,
}

/// Rooms + areas bundled so placement code can resolve a persisted
/// [`RoomLocation`] to the *current* room entity. Bundled as one `SystemParam`
/// to stay within Bevy's 16-parameter limit.
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
    /// `last_room` if it still resolves, else the starting room.
    pub(crate) fn placement(&self, last_room: Option<&RoomLocation>, starting: Entity) -> Entity {
        last_room
            .and_then(|loc| self.resolve(loc))
            .unwrap_or(starting)
    }
}

/// Everything [`crate::world_entry::enter_world_by_name`] needs to place a
/// character in the world (reconnect / takeover / spawn-from-disk), bundled into
/// one `SystemParam`. Shared by the login-by-name, character-select, and
/// legacy-backfill (class-pick) paths.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct WorldEntry<'w, 's> {
    pub(crate) characters: Query<
        'w,
        's,
        (
            Entity,
            &'static Character,
            &'static Actor,
            &'static GrimName,
        ),
    >,
    pub(crate) players: Query<'w, 's, &'static Player>,
    pub(crate) linkdead: Query<'w, 's, &'static Linkdead>,
    pub(crate) rooms: RoomResolver<'w, 's>,
    pub(crate) histories: Query<'w, 's, &'static mut OutputHistory>,
    pub(crate) announce_linkdead: MessageWriter<'w, LinkdeadAnnounce>,
    pub(crate) disconnect: MessageWriter<'w, DisconnectRequest>,
    pub(crate) alerts: MessageWriter<'w, WiznetAlert>,
}

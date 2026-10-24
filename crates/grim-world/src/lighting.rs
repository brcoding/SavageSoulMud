//! Being-free light and darkness markers.

/// Opt-in room darkness; games author which rooms lack illumination.
#[derive(bevy::prelude::Component)]
pub struct RoomDarkness;
/// A lit portable or placed light; ownership/placement determines its room.
#[derive(bevy::prelude::Component)]
pub struct LightSource;

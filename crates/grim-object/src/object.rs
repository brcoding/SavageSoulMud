//! Object placement: the [`Object`] marker and the [`CarriedBy`] carrier link.
//!
//! An object is on the ground while it carries the actor layer's `InRoom` and
//! carried while it carries [`CarriedBy`] — the get/drop handlers swap the two
//! atomically, so no tick ever sees both (a room listing would show it) or
//! neither (it would vanish).

use bevy::prelude::*;

/// Marker for a pickable thing. Carries the shared descriptive components:
/// `Name` (short name, shown in inventory and pickup lines), `Keywords` (get/
/// drop matching), `RoomDescription` (long line under the room description),
/// and optionally `Description` (`look <target>` paragraphs).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct Object;

/// The carrier of a carried object. Present instead of `InRoom` — never
/// alongside it (see the module docs).
pub use grim_core::components::CarriedBy;

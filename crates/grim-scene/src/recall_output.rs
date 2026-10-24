//! Recall echoes: per-recipient room broadcasts for a [`RecallEvent`].
//!
//! A separate system from [`crate::output::format_output`] because that one
//! is already at Bevy's system-parameter ceiling (see
//! [`crate::output::format_server_broadcast`]. The room left sees the
//! attempt, then the disappearance; the room entered sees a recall-marked
//! arrival. The recaller is excluded everywhere — they get the arrival
//! description instead.

use bevy::prelude::*;
use grim_core::components::Name as GrimName;
use grim_core::events::RecallEvent;
use grim_networking::ConnectionOutput;
use grim_text::tr;

use crate::output::{broadcast_from, Occupants};

/// Render one recall: attempt + disappearance to the room left, a
/// recall-marked arrival to the room entered, skipping the recaller.
fn emit_recall(
    ev: &RecallEvent,
    names: &Query<&GrimName>,
    room_occupants: &Occupants,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    let Ok(actor_name) = names.get(ev.actor) else {
        return;
    };
    let attempt = tr!("recall.leave.attempt", name = actor_name.0.as_str());
    broadcast_from(ev.from, &[ev.actor], ev.actor, &attempt, room_occupants, outputs);
    let vanish = tr!("recall.leave.vanish", name = actor_name.0.as_str());
    broadcast_from(ev.from, &[ev.actor], ev.actor, &vanish, room_occupants, outputs);
    let arrive = tr!("recall.arrive", name = actor_name.0.as_str());
    broadcast_from(ev.to, &[ev.actor], ev.actor, &arrive, room_occupants, outputs);
}

/// Broadcast every pending [`RecallEvent`] to its two rooms.
pub(crate) fn format_recall(
    mut recall_events: MessageReader<RecallEvent>,
    names: Query<&GrimName>,
    room_occupants: Occupants,
    mut outputs: MessageWriter<ConnectionOutput>,
) {
    for ev in recall_events.read() {
        emit_recall(ev, &names, &room_occupants, &mut outputs);
    }
}

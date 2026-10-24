//! broadcast implementation, split from the parent subsystem.

use super::*;

/// An actor's actions are visible only to recipients who can perceive them.
pub(crate) fn broadcast_from(
    room: Entity,
    exclude: &[Entity],
    source: Entity,
    text: &str,
    occupants: &Occupants,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    for (entity, ir, player, ..) in occupants.iter() {
        if ir.room != room || exclude.contains(&entity) || !occupants.sees(entity, source) {
            continue;
        }
        if let Some(player) = player {
            outputs.write(ConnectionOutput {
                prepend_newline: true,
                ..ConnectionOutput::new(player.connection, text)
            });
        }
    }
}

/// Out-of-band server messages (shutdown warnings) to every connected player.
/// A separate system from `format_output` because that one is already at Bevy's
/// system-parameter ceiling.
pub(crate) fn format_server_broadcast(
    mut broadcasts: MessageReader<ServerBroadcast>,
    occupants: Occupants,
    mut outputs: MessageWriter<ConnectionOutput>,
) {
    for ev in broadcasts.read() {
        broadcast_global(&ev.text, &occupants, &mut outputs);
    }
}

/// Send text to every connected player standing in `room`. Objects share the
/// query but never speak, so they are excluded from the audience (ground
/// objects have no `Player` either, but the explicit cut keeps the render
/// contract obvious).
pub(super) fn broadcast_room(
    text: &str,
    room: Entity,
    occupants: &Occupants,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    for entity in in_room(
        room,
        occupants
            .iter()
            .filter(|(_, _, _, _, _, o)| o.is_none())
            .map(|(e, ir, ..)| (e, ir.room)),
        None,
    ) {
        if let Ok((_, _, Some(p), ..)) = occupants.get(entity) {
            outputs.write(ConnectionOutput {
                prepend_newline: true,
                ..ConnectionOutput::new(p.connection, text.to_string())
            });
        }
    }
}

/// Send text to every connected player.
pub(super) fn broadcast_global(
    text: &str,
    occupants: &Occupants,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    for (_, _, player, _, _, _) in occupants.iter() {
        if let Some(p) = player {
            outputs.write(ConnectionOutput {
                prepend_newline: true,
                ..ConnectionOutput::new(p.connection, text.to_string())
            });
        }
    }
}

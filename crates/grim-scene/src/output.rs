//! Engine → client output formatting and per-recipient broadcast: turns game
//! events (look/say/yell/ooc/move/info + announces) into `ConnectionOutput`,
//! captures output for linkdead replay, and fans server broadcasts out.

use bevy::prelude::*;
use grim_actor::{in_room, Character, InRoom, OutputHistory, Player};
use grim_channel::{ChannelMessage, ChannelRegistry};
use grim_config::ConfigRegistry;
use grim_core::components::{Description, Name as GrimName, RoomDescription};
use grim_core::events::{
    DoorEvent, GlobalEcho, InfoMessage, LookEntity, LookRoom, MoveEvent, ServerBroadcast,
};
use grim_networking::ConnectionOutput;
use grim_object::Object;
use grim_text::tr;
use grim_world::Room;

use crate::channel_output::emit_channel;
use crate::formatter;
use crate::look_output::{emit_look_entity, emit_look_room, RoomLinks};
use crate::params::OutputReads;

/// Room-occupant query shape, shared by every broadcast helper below.
pub(crate) type Occupants<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static InRoom,
        Option<&'static Player>,
        &'static GrimName,
        Option<&'static RoomDescription>,
        Option<&'static Object>,
    ),
>;

#[allow(clippy::too_many_arguments)]
pub(crate) fn format_output(
    mut look_room_events: MessageReader<LookRoom>,
    mut look_entity_events: MessageReader<LookEntity>,
    mut channel_events: MessageReader<ChannelMessage>,
    mut move_events: MessageReader<MoveEvent>,
    mut info_events: MessageReader<InfoMessage>,
    mut gecho_events: MessageReader<GlobalEcho>,
    mut reads: OutputReads,
    channel_registry: Res<ChannelRegistry>,
    config_registry: Res<ConfigRegistry>,
    rooms: Query<(Entity, &Room, &GrimName)>,
    room_occupants: Occupants,
    room_exits: RoomLinks,
    names: Query<&GrimName>,
    descriptions: Query<(Option<&Description>, Option<&grim_core::components::ItemCondition>, Option<&Object>)>,
    characters: Query<&Character>,
    mut outputs: MessageWriter<ConnectionOutput>,
) {
    // ── Login / Logout / Linkdead announces (same-room only) ──
    // Login and linkdead resolve the room by subject entity: the subject is
    // alive at render time, so identity beats the name lookup (NPCs share
    // the name query). Logout carries its room: the quitter is despawned.
    for ev in reads.login.read() {
        if let Ok((_, ir, _, _, _, _)) = room_occupants.get(ev.subject) {
            broadcast_room(
                &format!("{} has connected.\n", ev.name),
                ir.room,
                &room_occupants,
                &mut outputs,
            );
        }
    }
    for ev in reads.logout.read() {
        // The quitter's entity is already despawned: the room rides on the
        // message instead of a lookup.
        broadcast_room(
            &format!("{} has disconnected.\n", ev.name),
            ev.room,
            &room_occupants,
            &mut outputs,
        );
    }
    for ev in reads.linkdead.read() {
        if let Ok((_, ir, _, _, _, _)) = room_occupants.get(ev.subject) {
            let formatted = formatter::format_linkdead(&ev.name, ev.reconnecting);
            broadcast_room(&formatted, ir.room, &room_occupants, &mut outputs);
        }
    }
    // Causal order, not arrival order: attempt-phase speech (channel) and
    // direct lines (info) are written before the movement system writes the
    // arrival description, so they render first — a farewell precedes the new
    // room, never dangles after it. Cross-actor simultaneity stays
    // arbitrary-but-deterministic; same-tick causality is what we keep.
    for ev in channel_events.read() {
        emit_channel(
            ev,
            &channel_registry,
            &names,
            &room_occupants,
            &rooms,
            &characters,
            &mut outputs,
        );
    }
    for ev in info_events.read() {
        emit_info(ev, &room_occupants, &mut outputs);
    }
    for ev in look_room_events.read() {
        emit_look_room(
            ev,
            &rooms,
            &room_occupants,
            &room_exits,
            &characters,
            &reads.clients,
            &reads.terminals,
            &reads.linkdead_chars,
            &config_registry,
            &reads.visibility,
            &mut outputs,
        );
    }
    for ev in look_entity_events.read() {
        emit_look_entity(ev, &room_occupants, &names, &descriptions, &reads.visibility, &mut outputs);
    }
    for ev in move_events.read() {
        emit_move(ev, &names, &room_occupants, &mut outputs);
    }
    for ev in reads.doors.read() {
        emit_door(ev, &names, &room_occupants, &mut outputs);
    }
    for ev in gecho_events.read() {
        emit_gecho(ev, &names, &characters, &room_occupants, &mut outputs);
    }
}

/// The connection entity for a recipient, from its `Player`, else the entity.
/// A linkdead recipient has no `Player`, so this falls back to the entity (its
/// `OutputHistory` buffers what is written there — see [`capture_output`]).
pub(crate) fn find_conn(target: Entity, room_occupants: &Occupants) -> Entity {
    room_occupants
        .get(target)
        .ok()
        .and_then(|(_, _, p, _, _, _)| p.map(|p| p.connection))
        .unwrap_or(target)
}

fn emit_move(
    ev: &MoveEvent,
    names: &Query<&GrimName>,
    room_occupants: &Occupants,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    let Ok(actor_name) = names.get(ev.actor) else {
        return;
    };
    let dir_str = ev.direction.to_string();
    let leave_msg = formatter::format_move(&actor_name.0, &dir_str, true);
    broadcast_to_room(ev.from, &[ev.actor], &leave_msg, room_occupants, outputs);
    let arrive_msg = formatter::format_move(&actor_name.0, &dir_str, false);
    broadcast_to_room(ev.to, &[ev.actor], &arrive_msg, room_occupants, outputs);
}

/// Per-recipient door rendering: the actor gets the first-party line, the
/// actor's room gets the attributed line, and the far room hears the door
/// move on its own side (recipient-relative direction + sentence-case name).
fn emit_door(
    ev: &DoorEvent,
    names: &Query<&GrimName>,
    room_occupants: &Occupants,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    let Ok(actor_name) = names.get(ev.actor) else {
        return;
    };
    let dir = ev.direction.to_string();
    // Actor: "You open the privy door to the east."
    let first_key = if ev.opened {
        "door.open.first"
    } else {
        "door.close.first"
    };
    let conn = find_conn(ev.actor, room_occupants);
    outputs.write(ConnectionOutput {
        prepend_newline: true,
        ..ConnectionOutput::new(
            conn,
            tr!(first_key, name = ev.name.as_str(), heading = dir.as_str()),
        )
    });
    // Same-room witnesses: "Alice opens the privy door to the east."
    let third_key = if ev.opened {
        "door.open.third"
    } else {
        "door.close.third"
    };
    let witness = tr!(
        third_key,
        actor = actor_name.0.as_str(),
        name = ev.name.as_str(),
        heading = dir.as_str()
    );
    broadcast_to_room(ev.from, &[ev.actor], &witness, room_occupants, outputs);
    // Far room: "The privy door opens to the west." — recipient-relative
    // direction, sentence-cased after any leading colour markup.
    let far_key = if ev.opened {
        "door.open.far"
    } else {
        "door.close.far"
    };
    let far_dir = ev.direction.opposite().to_string();
    let capped = grim_color::capitalize_first(&ev.name);
    let far = tr!(far_key, name = capped.as_str(), heading = far_dir.as_str());
    broadcast_to_room(ev.to, &[], &far, room_occupants, outputs);
}

fn emit_info(
    ev: &InfoMessage,
    room_occupants: &Occupants,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    let conn = find_conn(ev.target, room_occupants);
    outputs.write(ConnectionOutput {
        prepend_newline: true,
        ..ConnectionOutput::new(conn, ev.text.clone())
    });
}

/// Admin `gecho`: broadcast to every connected player in the world, including
/// the sender. Another admin sees it attributed (`Name> text`); the sender and
/// non-admins see the raw text. Rendering is per-recipient, so this cannot be
/// formatted once and broadcast.
fn emit_gecho(
    ev: &GlobalEcho,
    names: &Query<&GrimName>,
    characters: &Query<&Character>,
    room_occupants: &Occupants,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    let sender_name = names.get(ev.actor).ok().map(|n| n.0.clone());
    let raw = formatter::format_gecho(None, &ev.text);
    let attributed = sender_name
        .as_deref()
        .map(|n| formatter::format_gecho(Some(n), &ev.text));
    for (entity, _, player, _, _, _) in room_occupants.iter() {
        let Some(p) = player else {
            continue;
        };
        let conn = p.connection;
        let is_other_admin = entity != ev.actor
            && characters
                .get(entity)
                .map(Character::is_admin)
                .unwrap_or(false);
        let text = match (is_other_admin, &attributed) {
            (true, Some(a)) => a.clone(),
            _ => raw.clone(),
        };
        outputs.write(ConnectionOutput {
            prepend_newline: true,
            ..ConnectionOutput::new(conn, text)
        });
    }
}

/// Find the Connection entity for a character, using their Player component.
/// Falls back to the input entity if no Player component found.
#[allow(dead_code)]
fn find_connection(entity: Entity, players: &Query<&Player>) -> Entity {
    players.get(entity).map(|p| p.connection).unwrap_or(entity)
}

/// Capture every `ConnectionOutput` into the connection's `OutputHistory` for
/// linkdead replay on reconnect.
pub(crate) fn capture_output(
    mut output: MessageReader<ConnectionOutput>,
    mut histories: Query<&mut OutputHistory>,
) {
    for ev in output.read() {
        if let Ok(mut history) = histories.get_mut(ev.connection) {
            history.push(&ev.text);
        }
    }
}
/// Send text to every player in the given room, skipping `exclude`.
/// Membership funnels through the shared [`in_room`] helper (mapped from the
/// rich `Occupants` tuple); the player check stays local.
pub(crate) fn broadcast_to_room(
    room: Entity,
    exclude: &[Entity],
    text: &str,
    occupants: &Occupants,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    for entity in in_room(room, occupants.iter().map(|(e, ir, ..)| (e, ir.room)), None) {
        if exclude.contains(&entity) {
            continue;
        }
        if let Ok((_, _, Some(p), ..)) = occupants.get(entity) {
            outputs.write(ConnectionOutput {
                prepend_newline: true,
                ..ConnectionOutput::new(p.connection, text.to_string())
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
fn broadcast_room(
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
fn broadcast_global(
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

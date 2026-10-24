//! Per-recipient render of object events: pickups/drops ([`ItemEvent`]),
//! pack transfers ([`TransferEvent`]), and the being pack block on
//! `look <target>`.
//!
//! Separate systems from [`format_output`](crate::output::format_output)
//! because that one is already at Bevy's system-parameter ceiling.
//! `format_look_pack` additionally runs ordered after it, so the pack block
//! always lands below the description it belongs to.

use bevy::prelude::*;
use grim_actor::{Character, Creature};
use grim_core::components::Name as GrimName;
use grim_core::events::{ItemEvent, ItemKind, LookEntity, TransferEvent, TransferKind};
use grim_networking::ConnectionOutput;
use grim_object::{CarriedBy, Object};
use grim_text::tr;

use crate::output::{broadcast_from, find_conn, Occupants};

/// Object pickup/drop: per-recipient render of an [`ItemEvent`]. The actor
/// sees the first-party line ("You pick up …"); everyone else in the room
/// sees it attributed ("<name> picks up …").
pub(crate) fn format_item_events(
    mut items: MessageReader<ItemEvent>,
    room_occupants: Occupants,
    mut outputs: MessageWriter<ConnectionOutput>,
) {
    for ev in items.read() {
        let (first_key, third_key) = match ev.kind {
            ItemKind::Pickup => ("item.pickup.first", "item.pickup.third"),
            ItemKind::Drop => ("item.drop.first", "item.drop.third"),
        };
        let conn = find_conn(ev.actor, &room_occupants);
        outputs.write(ConnectionOutput {
            echo: None,
            ..ConnectionOutput::new(conn, tr!(first_key, short = ev.short.as_str()))
        });
        let third = tr!(
            third_key,
            name = ev.actor_name.as_str(),
            short = ev.short.as_str()
        );
        broadcast_from(ev.room, &[ev.actor], ev.actor, &third, &room_occupants, &mut outputs);
    }
}

/// Pack transfers (`give`/`steal`): per-recipient render of a
/// [`TransferEvent`]. The mover sees first-party ("You give …"), the other
/// party second-party ("… gives you …" / "… steals your …"), and the rest of
/// the room third-party ("… gives … to …"). Every line names both parties
/// and the item.
pub(crate) fn format_transfer_events(
    mut transfers: MessageReader<TransferEvent>,
    room_occupants: Occupants,
    characters: Query<&Character>,
    mut outputs: MessageWriter<ConnectionOutput>,
) {
    for ev in transfers.read() {
        let (first_key, target_key, third_key) = match ev.kind {
            TransferKind::Give => ("item.give.first", "item.give.target", "item.give.third"),
            TransferKind::Steal => ("item.steal.first", "item.steal.target", "item.steal.third"),
        };
        let mover_conn = find_conn(ev.mover, &room_occupants);
        outputs.write(ConnectionOutput {
            echo: None,
            ..ConnectionOutput::new(
                mover_conn,
                tr!(
                    first_key,
                    short = ev.short.as_str(),
                    other = ev.other_name.as_str()
                ),
            )
        });
        // Creatures have no connection to write to (and no output history to
        // buffer into); the second-party line is for characters only.
        if characters.get(ev.other).is_ok() {
            let other_conn = find_conn(ev.other, &room_occupants);
            outputs.write(ConnectionOutput {
                echo: None,
                ..ConnectionOutput::new(
                    other_conn,
                    tr!(
                        target_key,
                        name = ev.mover_name.as_str(),
                        short = ev.short.as_str()
                    ),
                )
            });
        }
        let third = tr!(
            third_key,
            name = ev.mover_name.as_str(),
            short = ev.short.as_str(),
            other = ev.other_name.as_str()
        );
        broadcast_from(
            ev.room,
            &[ev.mover, ev.other], if room_occupants.hidden(ev.other){ev.other}else{ev.mover},
            &third,
            &room_occupants,
            &mut outputs,
        );
    }
}

/// Being inventory on `look <target>`: after a character's or creature's
/// description, a blank line and their pack — the same listing the subject
/// would see from `inv`, with the header addressed to the looker ("You are
/// carrying" for self, "<name> is carrying" otherwise). Non-beings (objects,
/// fixtures) show no pack.
pub(crate) fn format_look_pack(
    mut looks: MessageReader<LookEntity>,
    room_occupants: Occupants,
    names: Query<&GrimName>,
    pack: Query<(&GrimName, &CarriedBy), With<Object>>,
    characters: Query<&Character>,
    creatures: Query<&Creature>,
    mut outputs: MessageWriter<ConnectionOutput>,
) {
    for ev in looks.read() {
        if !room_occupants.sees(ev.target,ev.subject) {continue;}
        if characters.get(ev.subject).is_err() && creatures.get(ev.subject).is_err() {
            continue;
        }
        let Ok(subject_name) = names.get(ev.subject) else {
            continue;
        };
        let mut shorts: Vec<&str> = pack
            .iter()
            .filter(|(_, held)| held.carrier == ev.subject)
            .map(|(nm, _)| nm.0.as_str())
            .collect();
        shorts.sort();
        let text = if shorts.is_empty() {
            if ev.subject == ev.target {
                tr!("inventory.empty")
            } else {
                tr!("look.pack.empty", name = subject_name.0.as_str())
            }
        } else {
            let mut out = if ev.subject == ev.target {
                tr!("inventory.list.header")
            } else {
                tr!("look.pack.header", name = subject_name.0.as_str())
            };
            for short in shorts {
                out.push_str(&tr!("inventory.list.row", short = short));
            }
            out
        };
        let conn = find_conn(ev.target, &room_occupants);
        outputs.write(ConnectionOutput {
            prepend_newline: true,
            ..ConnectionOutput::new(conn, text)
        });
    }
}

//! Room occupancy: the shared "who is in this room" vocabulary.
//!
//! New systems declare the canonical [`Beings`] alias instead of inventing a
//! tuple. Two predicates funnel every room-scoped query through one
//! definition:
//!
//! - [`beings_in`] — addressable beings in `room` from a [`Beings`] query: a
//!   [`Creature`], or a [`Character`] with [`Player`] or [`Linkdead`]
//!   presence. Offline PCs are never addressable.
//! - [`in_room`] — the generic core for query shapes that carry extra columns
//!   (render data, trigger state): map to `(Entity, room)` pairs at the call
//!   site instead of reimplementing the predicate.
//!
//! Both take `except` for the beings-minus-self case. `self` is resolved at
//! the parse layer (before querying), never inside these helpers.

use bevy::prelude::*;
use grim_core::components::{Keywords, Name as GrimName};

use crate::character::Character;
use crate::placement::InRoom;
use crate::{Creature, Linkdead, Player};

/// The canonical beings query shape: identity, placement, display name, and
/// the marker columns that decide addressability.
pub type Beings<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static InRoom,
        &'static GrimName,
        Option<&'static Keywords>,
        Option<&'static Character>,
        Option<&'static Creature>,
        Option<&'static Player>,
        Option<&'static Linkdead>,
    ),
>;

/// Whether a being with this marker combination is addressable: a creature,
/// or a character with live or linkdead presence.
pub fn is_addressable(character: bool, creature: bool, player: bool, linkdead: bool) -> bool {
    creature || (character && (player || linkdead))
}

/// Addressable beings in `room` (see [`is_addressable`]), optionally
/// excluding one entity.
pub fn beings_in(room: Entity, beings: &Beings, except: Option<Entity>) -> Vec<Entity> {
    beings
        .iter()
        .filter(|(e, ir, _, _, ch, cr, p, l)| {
            ir.room == room
                && Some(*e) != except
                && is_addressable(ch.is_some(), cr.is_some(), p.is_some(), l.is_some())
        })
        .map(|(e, ..)| e)
        .collect()
}

/// Every occupant of `room`, optionally excluding one entity. The generic
/// core: map richer tuples to `(Entity, room)` pairs at the call site.
pub fn in_room(
    room: Entity,
    occupants: impl IntoIterator<Item = (Entity, Entity)>,
    except: Option<Entity>,
) -> Vec<Entity> {
    occupants
        .into_iter()
        .filter(|(e, r)| *r == room && Some(*e) != except)
        .map(|(e, _)| e)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stub_character() -> Character {
        Character {
            credits: 100,
            id: grim_core::GrimId::new(),
            account_id: grim_core::GrimId::new(),
            created_at: chrono::Utc::now(),
            last_room: None,
            roles: Vec::new(),
            class: String::new(),
            title: None,
            restrings: std::collections::HashMap::new(),
            config: std::collections::HashMap::new(),
            game_state: Default::default(),
        }
    }

    fn spawn_pc(app: &mut App, room: Entity, online: bool, linkdead: bool) -> Entity {
        let mut e =
            app.world_mut()
                .spawn((GrimName("Pc".into()), InRoom { room }, stub_character()));
        if online {
            e.insert(Player {
                connection: Entity::PLACEHOLDER,
            });
        }
        if linkdead {
            e.insert(Linkdead);
        }
        e.id()
    }

    #[test]
    fn in_room_lists_occupants_minus_except() {
        let mut app = App::new();
        let room = app.world_mut().spawn_empty().id();
        let elsewhere = app.world_mut().spawn_empty().id();
        let a = app.world_mut().spawn(InRoom { room }).id();
        let b = app.world_mut().spawn(InRoom { room }).id();
        let _far = app.world_mut().spawn(InRoom { room: elsewhere }).id();
        let mut pairs = app.world_mut().query::<(Entity, &InRoom)>();
        let pairs: Vec<(Entity, Entity)> = pairs
            .iter(app.world())
            .map(|(e, ir)| (e, ir.room))
            .collect();
        assert_eq!(in_room(room, pairs, Some(a)), vec![b]);
    }

    #[test]
    fn addressability_rule() {
        assert!(is_addressable(false, true, false, false));
        assert!(is_addressable(true, false, true, false));
        assert!(is_addressable(true, false, false, true));
        assert!(!is_addressable(true, false, false, false));
        assert!(!is_addressable(false, false, false, false));
    }

    #[derive(Message)]
    struct Probe {
        room: Entity,
        except: Option<Entity>,
    }

    #[derive(Message, Clone, PartialEq, Debug)]
    struct ProbeHit(Vec<Entity>);

    fn probe(mut events: MessageReader<Probe>, beings: Beings, mut out: MessageWriter<ProbeHit>) {
        for ev in events.read() {
            out.write(ProbeHit(beings_in(ev.room, &beings, ev.except)));
        }
    }

    #[test]
    fn beings_in_skips_offline_pcs() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<Probe>().add_message::<ProbeHit>();
        app.add_systems(Update, probe);
        let room = app.world_mut().spawn_empty().id();
        let elsewhere = app.world_mut().spawn_empty().id();
        let mob = app
            .world_mut()
            .spawn((GrimName("Mob".into()), InRoom { room }, Creature))
            .id();
        let online = spawn_pc(&mut app, room, true, false);
        let dead = spawn_pc(&mut app, room, false, true);
        let offline = spawn_pc(&mut app, room, false, false);
        let far = spawn_pc(&mut app, elsewhere, true, false);
        app.world_mut().write_message(Probe { room, except: None });
        app.update();
        let msgs = app.world().resource::<Messages<ProbeHit>>();
        let mut cursor = msgs.get_cursor();
        let mut got: Vec<Entity> = cursor
            .read(msgs)
            .flat_map(|h| h.0.iter().copied())
            .collect();
        got.sort();
        let mut want = vec![mob, online, dead];
        want.sort();
        assert_eq!(got, want);
        assert!(!got.contains(&offline));
        assert!(!got.contains(&far));
    }
}

//! `open` / `close`: flip the door guarding an exit, on both sides.
//!
//! Both linked rooms carry their own copy of the door (see
//! `grim_world::Doors`); the handler flips the pair. When the reverse exit
//! does not point back, only the actor's side flips — a one-sided door, not
//! a failure.
//!
//! Reads the actor's [`Character`]/[`InRoom`] plus `grim_world`'s room
//! topology (`Exits`/`Doors`), emits [`DoorEvent`] for per-recipient
//! rendering in `grim-scene`.

use bevy::prelude::*;
use grim_color::capitalize_first;
use grim_core::events::{Command, DoorEvent, EngineCommand, InfoMessage};
use grim_text::tr;
use grim_world::{Doors, Exits};

use crate::placement::InRoom;

/// `open` / `close`: flip the door on that exit, in message order.
///
/// One system reads `EngineCommand` once so an `open` + `close` pair for the
/// same door in one tick resolves in the order the player typed it, not in
/// system-execution order.
fn handle_door(
    mut engine: MessageReader<EngineCommand>,
    mut info: MessageWriter<InfoMessage>,
    mut doors_events: MessageWriter<DoorEvent>,
    inroom: Query<&InRoom>,
    exits: Query<&Exits>,
    mut doors: Query<&mut Doors>,
) {
    for cmd in engine.read() {
        let (direction, open) = match cmd.command {
            Command::Open { direction } => (direction, true),
            Command::Close { direction } => (direction, false),
            _ => continue,
        };
        set_door(
            cmd.client,
            direction,
            open,
            &mut info,
            &mut doors_events,
            &inroom,
            &exits,
            &mut doors,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn set_door(
    actor: Entity,
    direction: grim_core::cardinal::Cardinal,
    open: bool,
    info: &mut MessageWriter<InfoMessage>,
    doors_events: &mut MessageWriter<DoorEvent>,
    inroom: &Query<&InRoom>,
    exits: &Query<&Exits>,
    doors: &mut Query<&mut Doors>,
) {
    let from = match inroom.get(actor) {
        Ok(ir) => ir.room,
        Err(_) => return,
    };
    // Fail closed: no exit here means no door here, never free passage.
    let to = match exits
        .get(from)
        .ok()
        .and_then(|e| e.exits.get(&direction).copied())
    {
        Some(to) => to,
        None => {
            info.write(InfoMessage {
                target: actor,
                text: tr!("door.error.no_door"),
            });
            return;
        }
    };
    let name = match doors.get(from).ok().and_then(|d| d.doors.get(&direction)) {
        Some(door) => (door.name.clone(), door.open),
        None => {
            info.write(InfoMessage {
                target: actor,
                text: tr!("door.error.no_door"),
            });
            return;
        }
    };
    // Mirror bookkeeping, resolved before any mutation: when the reverse exit
    // points back, the far `Doors` entry must exist. A linked-back room with
    // no far door is a data bug — fail closed (reply, touch nothing) rather
    // than leave the pair divergent.
    let back = direction.opposite();
    let reverse_ok = exits.get(to).ok().and_then(|e| e.exits.get(&back).copied()) == Some(from);
    let far_open = reverse_ok
        .then(|| {
            doors
                .get(to)
                .ok()
                .and_then(|d| d.doors.get(&back))
                .map(|door| door.open)
        })
        .flatten();
    if reverse_ok && far_open.is_none() {
        info.write(InfoMessage {
            target: actor,
            text: tr!("door.error.no_door"),
        });
        return;
    }
    // Already-state only when both sides agree: a divergent far side heals
    if name.1 == open && (!reverse_ok || far_open == Some(open)) {
        let capped = capitalize_first(&name.0);
        info.write(InfoMessage {
            target: actor,
            text: if open {
                tr!("door.open.already", name = capped.as_str())
            } else {
                tr!("door.close.already", name = capped.as_str())
            },
        });
        return;
    }
    if let Ok(mut own) = doors.get_mut(from) {
        if let Some(door) = own.doors.get_mut(&direction) {
            door.open = open;
        }
    }
    // Mirror the far side: the reverse exit's door flips too, when the link
    // points back at us (the entry's existence was checked above).
    if reverse_ok {
        if let Ok(mut far) = doors.get_mut(to) {
            if let Some(door) = far.doors.get_mut(&back) {
                door.open = open;
            }
        }
    }
    doors_events.write(DoorEvent {
        actor,
        from,
        to,
        direction,
        opened: open,
        name: name.0,
    });
}

/// Wire the `open`/`close` handler and the delivery messages it owns. The
/// `DoorEvent` facts it emits are registered by `grim_world::WorldPlugin`.
pub(crate) fn register(app: &mut App) {
    app.add_message::<EngineCommand>()
        .add_message::<InfoMessage>()
        .add_systems(Update, handle_door);
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_core::cardinal::Cardinal;
    use grim_core::character::Gender;
    use grim_core::components::Name as GrimName;
    use grim_core::GrimId;
    use grim_world::Area;
    use std::collections::HashMap;

    use crate::actor::Actor;
    use crate::character::Role;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(grim_world::WorldPlugin);
        register(&mut app);
        app
    }

    fn spawn_room(app: &mut App, exits: Exits, doors: Doors) -> Entity {
        let area = app
            .world_mut()
            .spawn(Area {
                id: GrimId::new(),
                friendly_id: "haven".into(),
                name: "haven".into(),
            })
            .id();
        app.world_mut()
            .spawn((
                grim_world::Room {
                    id: GrimId::new(),
                    friendly_id: "room".into(),
                    name: "room".into(),
                    description: String::new(),
                    area,
                },
                exits,
                doors,
            ))
            .id()
    }

    fn spawn_actor_in(app: &mut App, room: Entity) -> Entity {
        app.world_mut()
            .spawn((
                InRoom { room },
                GrimName("Hero".into()),
                Actor {
                    race: String::new(),
                    level: 1,
                    gender: Gender::Neutral,
                },
                crate::character::Character {
                    credits: 100,
                    id: GrimId::new(),
                    account_id: GrimId::new(),
                    created_at: chrono::Utc::now(),
                    last_room: None,
                    roles: vec![Role::Admin],
                    class: String::new(),
                    title: None,
                    restrings: HashMap::new(),
                    config: HashMap::new(),
                    game_state: Default::default(),
                },
            ))
            .id()
    }

    fn link(app: &mut App, a: Entity, b: Entity) {
        app.world_mut()
            .get_mut::<Exits>(a)
            .unwrap()
            .exits
            .insert(Cardinal::East, b);
        app.world_mut()
            .get_mut::<Exits>(b)
            .unwrap()
            .exits
            .insert(Cardinal::West, a);
    }

    fn hang(app: &mut App, room: Entity, dir: Cardinal, open: bool) {
        hang_hidden(app, room, dir, open, false);
    }

    fn hang_hidden(app: &mut App, room: Entity, dir: Cardinal, open: bool, hidden: bool) {
        app.world_mut()
            .get_mut::<Doors>(room)
            .unwrap()
            .doors
            .insert(
                dir,
                grim_world::Door {
                    name: "the privy door".into(),
                    keywords: vec!["privy door".into()],
                    open,
                    hidden,
                },
            );
    }

    fn send(app: &mut App, actor: Entity, command: Command) {
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command,
        });
        app.update();
    }

    fn infos(app: &App) -> Vec<String> {
        let messages = app.world().resource::<Messages<InfoMessage>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).map(|m| m.text.clone()).collect()
    }

    fn door_events(app: &App) -> Vec<(bool, String)> {
        let messages = app.world().resource::<Messages<DoorEvent>>();
        let mut cursor = messages.get_cursor();
        cursor
            .read(messages)
            .map(|e| (e.opened, e.name.clone()))
            .collect()
    }

    #[test]
    fn open_flips_both_sides_and_emits_fact() {
        let mut app = test_app();
        let a = spawn_room(&mut app, Exits::default(), Doors::default());
        let b = spawn_room(&mut app, Exits::default(), Doors::default());
        link(&mut app, a, b);
        hang(&mut app, a, Cardinal::East, false);
        hang(&mut app, b, Cardinal::West, false);
        let actor = spawn_actor_in(&mut app, a);
        send(
            &mut app,
            actor,
            Command::Open {
                direction: Cardinal::East,
            },
        );
        assert!(app.world().get::<Doors>(a).unwrap().doors[&Cardinal::East].open);
        assert!(app.world().get::<Doors>(b).unwrap().doors[&Cardinal::West].open);
        assert_eq!(
            door_events(&app),
            vec![(true, "the privy door".to_string())]
        );
        assert!(infos(&app).is_empty());
    }

    #[test]
    fn open_with_no_door_replies_no_door() {
        let mut app = test_app();
        let a = spawn_room(&mut app, Exits::default(), Doors::default());
        let b = spawn_room(&mut app, Exits::default(), Doors::default());
        link(&mut app, a, b);
        let actor = spawn_actor_in(&mut app, a);
        send(
            &mut app,
            actor,
            Command::Open {
                direction: Cardinal::East,
            },
        );
        assert_eq!(
            infos(&app),
            vec!["There is no door in that direction.\n".to_string()]
        );
        assert!(door_events(&app).is_empty());
    }

    #[test]
    fn open_with_no_exit_replies_no_door() {
        let mut app = test_app();
        let a = spawn_room(&mut app, Exits::default(), Doors::default());
        let actor = spawn_actor_in(&mut app, a);
        send(
            &mut app,
            actor,
            Command::Close {
                direction: Cardinal::North,
            },
        );
        assert_eq!(
            infos(&app),
            vec!["There is no door in that direction.\n".to_string()]
        );
    }

    #[test]
    fn reopen_replies_already_open() {
        let mut app = test_app();
        let a = spawn_room(&mut app, Exits::default(), Doors::default());
        let b = spawn_room(&mut app, Exits::default(), Doors::default());
        link(&mut app, a, b);
        hang(&mut app, a, Cardinal::East, true);
        hang(&mut app, b, Cardinal::West, true);
        let actor = spawn_actor_in(&mut app, a);
        send(
            &mut app,
            actor,
            Command::Open {
                direction: Cardinal::East,
            },
        );
        assert!(infos(&app)
            .iter()
            .any(|t| t.contains("The privy door is already open")));
        assert!(door_events(&app).is_empty());
    }

    #[test]
    fn close_flips_and_reclose_replies_already_closed() {
        let mut app = test_app();
        let a = spawn_room(&mut app, Exits::default(), Doors::default());
        let b = spawn_room(&mut app, Exits::default(), Doors::default());
        link(&mut app, a, b);
        hang(&mut app, a, Cardinal::East, true);
        hang(&mut app, b, Cardinal::West, true);
        let actor = spawn_actor_in(&mut app, a);
        send(
            &mut app,
            actor,
            Command::Close {
                direction: Cardinal::East,
            },
        );
        assert!(!app.world().get::<Doors>(a).unwrap().doors[&Cardinal::East].open);
        assert!(!app.world().get::<Doors>(b).unwrap().doors[&Cardinal::West].open);
        assert_eq!(
            door_events(&app),
            vec![(false, "the privy door".to_string())]
        );
        send(
            &mut app,
            actor,
            Command::Close {
                direction: Cardinal::East,
            },
        );
        assert!(infos(&app)
            .iter()
            .any(|t| t.contains("The privy door is already closed")));
    }

    #[test]
    fn one_sided_door_flips_own_side_only() {
        // Reverse exit missing: the actor's side still opens, no far mirror.
        let mut app = test_app();
        let a = spawn_room(&mut app, Exits::default(), Doors::default());
        let b = spawn_room(&mut app, Exits::default(), Doors::default());
        app.world_mut()
            .get_mut::<Exits>(a)
            .unwrap()
            .exits
            .insert(Cardinal::East, b);
        hang(&mut app, a, Cardinal::East, false);
        let actor = spawn_actor_in(&mut app, a);
        send(
            &mut app,
            actor,
            Command::Open {
                direction: Cardinal::East,
            },
        );
        assert!(app.world().get::<Doors>(a).unwrap().doors[&Cardinal::East].open);
        assert_eq!(door_events(&app).len(), 1);
    }

    #[test]
    fn linked_back_room_without_far_door_rejects_without_mutation() {
        // Reverse exit points back but the far `Doors` entry is missing:
        // fail closed — neither side mutates, no fact fires.
        let mut app = test_app();
        let a = spawn_room(&mut app, Exits::default(), Doors::default());
        let b = spawn_room(&mut app, Exits::default(), Doors::default());
        link(&mut app, a, b);
        hang(&mut app, a, Cardinal::East, false);
        let actor = spawn_actor_in(&mut app, a);
        send(
            &mut app,
            actor,
            Command::Open {
                direction: Cardinal::East,
            },
        );
        assert!(!app.world().get::<Doors>(a).unwrap().doors[&Cardinal::East].open);
        assert_eq!(
            infos(&app),
            vec!["There is no door in that direction.\n".to_string()]
        );
        assert!(door_events(&app).is_empty());
    }

    #[test]
    fn divergent_far_side_heals_instead_of_replying_already() {
        // Local already open but far closed: open again heals the far side.
        let mut app = test_app();
        let a = spawn_room(&mut app, Exits::default(), Doors::default());
        let b = spawn_room(&mut app, Exits::default(), Doors::default());
        link(&mut app, a, b);
        hang(&mut app, a, Cardinal::East, true);
        hang(&mut app, b, Cardinal::West, false);
        let actor = spawn_actor_in(&mut app, a);
        send(
            &mut app,
            actor,
            Command::Open {
                direction: Cardinal::East,
            },
        );
        assert!(app.world().get::<Doors>(b).unwrap().doors[&Cardinal::West].open);
        assert_eq!(
            door_events(&app),
            vec![(true, "the privy door".to_string())]
        );
        assert!(infos(&app).is_empty());
    }

    #[test]
    fn open_then_close_in_one_tick_resolves_in_message_order() {
        // Both commands queued before the tick: last writer wins.
        let mut app = test_app();
        let a = spawn_room(&mut app, Exits::default(), Doors::default());
        let b = spawn_room(&mut app, Exits::default(), Doors::default());
        link(&mut app, a, b);
        hang(&mut app, a, Cardinal::East, false);
        hang(&mut app, b, Cardinal::West, false);
        let actor = spawn_actor_in(&mut app, a);
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Open {
                direction: Cardinal::East,
            },
        });
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Close {
                direction: Cardinal::East,
            },
        });
        app.update();
        assert!(!app.world().get::<Doors>(a).unwrap().doors[&Cardinal::East].open);
        assert!(!app.world().get::<Doors>(b).unwrap().doors[&Cardinal::West].open);
        assert_eq!(
            door_events(&app),
            vec![
                (true, "the privy door".to_string()),
                (false, "the privy door".to_string())
            ]
        );
    }
}

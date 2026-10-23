//! Movement: walking an exit (`move`) and the admin `goto` teleport, plus the
//! shared [`place_actor`] seam every "put actor in room X" path routes through
//! (also used by the player `recall` in [`super::recall`]). Both read the
//! actor's [`Character`]/[`InRoom`] and resolve destinations against
//! `grim_world`'s room topology + address lookups.

use bevy::prelude::*;
use grim_core::cardinal::Cardinal;
use grim_core::events::{Command, EngineCommand, InfoMessage, LookRoom, MoveEvent};
use grim_text::tr;
use grim_world::{
    resolve_room_address, room_location, Area, Doors, Exits, Room, RoomLocation, RoomLookup,
};

use crate::character::Character;
use crate::placement::InRoom;
use crate::transition::{AttemptEnter, AttemptLeave, AttemptWalk, Enter, Leave};

/// Build a room's persisted [`RoomLocation`] from its `Room` + `Area` records.
/// `handle_move` reaches this shape via `grim_world::room_location` (which holds
/// `Query<&Room>`/`Query<&Area>`); `handle_goto` holds `Query<(Entity, &Room)>`
/// for address resolution, so it decomposes to the same records and shares this
/// one builder — keeping both paths in agreement if `RoomLocation` grows a field.
pub(super) fn persisted_location(room: &Room, area: &Area) -> RoomLocation {
    RoomLocation {
        area: area.friendly_id.clone(),
        room: room.friendly_id.clone(),
    }
}

/// The single seam every "put actor in room X" path routes through: set the
/// actor's `InRoom` and refresh their persisted location. Per ADR-0001 the
/// location update is a property of the *destination*, not of how the actor
/// arrived, so walk and `goto` (and, later, summon/recall/login) share it. `loc`
/// is the destination's persisted [`RoomLocation`], precomputed by the caller.
///
/// Persists only `last_room` today. ADR-0001's `last_canonical_room` is not a
/// field yet; while every room is Canonical (no instancing) the two would be
/// equal, so it is deferred to the instancing work rather than added dead here.
pub(super) fn place_actor(
    actor: Entity,
    to: Entity,
    loc: Option<RoomLocation>,
    inroom: &mut Query<&mut InRoom>,
    characters: &mut Query<&mut Character>,
) {
    if let Ok(mut ir) = inroom.get_mut(actor) {
        ir.room = to;
    }
    if let Some(loc) = loc {
        if let Ok(mut character) = characters.get_mut(actor) {
            character.last_room = Some(loc);
        }
    }
}
/// Whether a closed door blocks `actor`'s walk from `from` toward `direction`:
/// replies and returns true, else false. A closed *hidden* door reuses the
/// `move.no_exit` catalog key byte-identically (never leaks the secret); a
/// closed visible door replies "%{Name} is closed." (sentence-capitalized).
/// Runs inside the queued placement closure (world access only) so the check
/// is atomic with placement.
fn door_blocks(world: &mut World, actor: Entity, from: Entity, direction: Cardinal) -> bool {
    let door = world
        .get::<Doors>(from)
        .and_then(|d| d.doors.get(&direction))
        .filter(|door| !door.open)
        .map(|door| (door.hidden, grim_color::capitalize_first(&door.name)));
    if let Some((hidden, name)) = door {
        let text = if hidden {
            tr!("move.no_exit")
        } else {
            grim_text::tr("door.error.closed", &[("name", name.as_str())])
        };
        world
            .resource_mut::<Messages<InfoMessage>>()
            .write(InfoMessage {
                target: actor,
                text,
            });
        return true;
    }
    false
}

/// A committed room transition waiting for its facts to fire next tick.
#[derive(Debug, Clone, Copy)]
pub(super) struct RoomFact {
    pub(super) actor: Entity,
    pub(super) from: Entity,
    pub(super) to: Entity,
}

/// Facts committed by placement but not yet fired. Double-buffered: the
/// orchestrator pushes to `incoming`, `fire_pending_facts` fires `ready` and
/// rotates, so facts always fire the tick *after* the arrival they follow —
/// in a later flush, whatever the command-flush timing is.
#[derive(Resource, Default)]
pub(crate) struct PendingFacts {
    ready: Vec<RoomFact>,
    pub(super) incoming: Vec<RoomFact>,
}

/// Fire last tick's committed facts. Chained after the orchestrators so the
/// buffer rotation is deterministic: facts fire exactly one tick after the
/// placement that committed them.
fn fire_pending_facts(mut pending: ResMut<PendingFacts>, mut commands: Commands) {
    let PendingFacts { ready, incoming } = std::mem::take(&mut *pending);
    for fact in ready {
        commands.trigger(Leave {
            actor: fact.actor,
            room: fact.from,
        });
        commands.trigger(Enter {
            actor: fact.actor,
            room: fact.to,
        });
    }
    pending.ready = incoming;
}

/// `move <direction>`: traverse an exit. Validation runs here; the phased
/// point of no return runs in a queued closure so the attempt triggers fire
/// synchronously (`trigger_ref`) with placement in one atomic step: walk and
/// room attempts first (vetoable — a denied move never places), then
/// placement, the `MoveEvent` fact (the "walking happened" event the game
/// acts on), and the automatic look at the destination. The committed
/// `Leave`/`Enter` facts queue into `PendingFacts` and fire next tick (see
/// `fire_pending_facts`), so their speech lands in a later flush than the
/// arrival. Also refreshes the character's persisted `last_room` so a
pub(crate) fn handle_move(
    mut engine: MessageReader<EngineCommand>,
    mut commands: Commands,
    inroom: Query<&InRoom>,
    exits: Query<&Exits>,
    rooms: Query<&Room>,
    areas: Query<&Area>,
) {
    for cmd in engine.read() {
        let Command::Move { direction } = cmd.command else {
            continue;
        };
        let actor = cmd.client;
        let from = match inroom.get(actor) {
            Ok(ir) => ir.room,
            Err(_) => continue,
        };
        match exits.get(from) {
            Ok(room_exits) => match room_exits.exits.get(&direction).copied() {
                Some(to) => {
                    // Keep the persisted location current on every step so an
                    // unexpected restart or copyover resumes the character in the
                    // room they actually walked to, not a stale one.
                    let loc = room_location(to, &rooms, &areas);
                    commands.queue(move |world: &mut World| {
                        // Serial moves: two intents for one actor can queue in
                        // the same tick (two sessions, a teleport mid-step).
                        // The snapshots above are stale if another closure
                        // already placed this actor — fail closed and drop the
                        // intent rather than walking from the wrong room.
                        let current = world.get::<InRoom>(actor).map(|ir| ir.room);
                        if current != Some(from) {
                            return;
                        }
                        // A closed door blocks the walk (checked inside the
                        // atomic closure, not at dispatch — same TOCTOU shape
                        // as the serial-move guard above): reply and stop.
                        if door_blocks(world, actor, from, direction) {
                            return;
                        }
                        let mut walk = AttemptWalk {
                            actor,
                            room: from,
                            direction,
                            denied: false,
                        };
                        world.trigger_ref(&mut walk);
                        if walk.denied {
                            return;
                        }
                        let mut leave = AttemptLeave {
                            actor,
                            room: from,
                            denied: false,
                        };
                        world.trigger_ref(&mut leave);
                        if leave.denied {
                            return;
                        }
                        let mut enter = AttemptEnter {
                            actor,
                            room: to,
                            denied: false,
                        };
                        world.trigger_ref(&mut enter);
                        if enter.denied {
                            return;
                        }
                        // Placement (the `place_actor` seam, inlined: inside a
                        // queued closure there are no queries, only the world).
                        if let Some(mut ir) = world.get_mut::<InRoom>(actor) {
                            ir.room = to;
                        }
                        if let Some(loc) = loc {
                            if let Some(mut character) = world.get_mut::<Character>(actor) {
                                character.last_room = Some(loc);
                            }
                        }
                        world
                            .resource_mut::<Messages<MoveEvent>>()
                            .write(MoveEvent {
                                actor,
                                from,
                                to,
                                direction,
                            });
                        // Facts fire next tick (see `fire_pending_facts`), so
                        // their speech lands in a later flush than the arrival.
                        world
                            .resource_mut::<PendingFacts>()
                            .incoming
                            .push(RoomFact { actor, from, to });
                        world.resource_mut::<Messages<LookRoom>>().write(LookRoom {
                            target: actor,
                            room: to,
                        });
                    });
                }
                None => {
                    commands.write_message(InfoMessage {
                        target: actor,
                        text: tr!("move.no_exit"),
                    });
                }
            },
            Err(_) => {
                commands.write_message(InfoMessage {
                    target: actor,
                    text: tr!("move.no_exit"),
                });
            }
        }
    }
}

/// `goto <address>`: admin-only teleport. Resolves the address through
/// [`resolve_room_address`] and places the actor via the shared [`place_actor`]
/// seam, then shows the destination room.
///
/// Admin-gated here as defense in depth: the dispatcher already masks `goto` as
/// an unknown command for non-admins, so a well-behaved session never sends this
/// for one. A `goto` from a non-client source with no admin character is refused
/// silently (emitting anything would leak that the command exists).
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_goto(
    mut engine: MessageReader<EngineCommand>,
    mut commands: Commands,
    mut inroom: Query<&mut InRoom>,
    rooms: Query<(Entity, &Room)>,
    areas: Query<(Entity, &Area)>,
    mut characters: Query<&mut Character>,
    mut look_room: MessageWriter<LookRoom>,
    mut info: MessageWriter<InfoMessage>,
    mut pending: ResMut<PendingFacts>,
) {
    for cmd in engine.read() {
        let Command::Goto { target } = &cmd.command else {
            continue;
        };
        let actor = cmd.client;
        let is_admin = characters.get(actor).map(|c| c.is_admin()).unwrap_or(false);
        if !is_admin {
            continue;
        }
        match resolve_room_address(target, &rooms, &areas) {
            RoomLookup::Found(to) => {
                let from = inroom.get(actor).map(|ir| ir.room).ok();
                let loc = rooms.get(to).ok().and_then(|(_, r)| {
                    areas
                        .get(r.area)
                        .ok()
                        .map(|(_, a)| persisted_location(r, a))
                });
                if let Some(from) = from.filter(|from| *from != to) {
                    // Greetings, not veto: teleports are admin tools, so the
                    // attempts fire (deferred) but denial is never consulted.
                    commands.trigger(AttemptLeave {
                        actor,
                        room: from,
                        denied: false,
                    });
                    commands.trigger(AttemptEnter {
                        actor,
                        room: to,
                        denied: false,
                    });
                }
                place_actor(actor, to, loc, &mut inroom, &mut characters);
                if let Some(from) = from.filter(|from| *from != to) {
                    pending.incoming.push(RoomFact { actor, from, to });
                }
                look_room.write(LookRoom {
                    target: actor,
                    room: to,
                });
            }
            RoomLookup::NotFound => {
                // `target` is raw admin input; escape it so it can't inject
                // colour markup into the reply (only the admin's own session,
                // but keep the invariant that interpolated input is escaped).
                info.write(InfoMessage {
                    target: actor,
                    text: format!("No room matches '{}'.\n", grim_color::escape_codes(target)),
                });
            }
            RoomLookup::Ambiguous(candidates) => {
                // List every candidate with its distinguishing ids so the admin
                // can re-issue `goto` against a unique one (an entity or grim id).
                let mut text = String::from("Select an option...\n");
                for e in candidates {
                    if let Ok((_, r)) = rooms.get(e) {
                        text.push_str(&room_ident_line(e, r));
                        text.push('\n');
                    }
                }
                info.write(InfoMessage {
                    target: actor,
                    text,
                });
            }
        }
    }
}

/// One disambiguation line for a room: `Name (entity:… grim:… slug:…)`. Matches
/// the admin room-title debug format. (A future instance id would slot in here.)
fn room_ident_line(entity: Entity, room: &Room) -> String {
    format!(
        "{} (entity:{} grim:{} slug:{})",
        room.name,
        entity.to_bits(),
        room.id,
        room.friendly_id
    )
}

/// Wire the `move` and `goto` handlers and the input/delivery messages they
/// own. The room-transition moments are trigger *events* (`AttemptWalk`/
/// `AttemptLeave`/`AttemptEnter`/`Leave`/`Enter`), which need no
/// registration — observers (e.g. `grim-script`) attach directly. The
/// world-happening events they emit (`MoveEvent`/`LookRoom`) are registered
/// by `grim_world::WorldPlugin`. Committed facts fire a tick after placement
/// via `fire_pending_facts`, chained here so the rotation is deterministic.
pub(crate) fn register(app: &mut App) {
    app.add_message::<EngineCommand>()
        .add_message::<InfoMessage>()
        .init_resource::<PendingFacts>()
        .add_systems(Update, (handle_move, fire_pending_facts).chain());
    app.add_systems(Update, handle_goto);
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_core::character::Gender;
    // The project display-name component, aliased to dodge Bevy's prelude `Name`
    // (the glob above brings Bevy's in scope). See AGENTS.md.
    use grim_core::components::Name as GrimName;
    use grim_core::GrimId;

    use crate::actor::Actor;
    use crate::character::Role;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(grim_world::WorldPlugin);
        register(&mut app);
        crate::commands::recall::register(&mut app);
        app.init_resource::<walking::TransitionLog>();
        app.add_observer(walking::log_attempt_walk);
        app.add_observer(walking::log_attempt_leave);
        app.add_observer(walking::log_attempt_enter);
        app.add_observer(walking::log_leave);
        app.add_observer(walking::log_enter);
        app
    }

    fn look_room_count(app: &App) -> usize {
        let messages = app.world().resource::<Messages<LookRoom>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).count()
    }

    /// Spawn an area + a room in it, returning the room entity. `friendly_id`s
    /// are the stable storage keys `last_room` records.
    fn spawn_room(app: &mut App, area_fid: &str, room_fid: &str, exits: Exits) -> Entity {
        let area = app
            .world_mut()
            .spawn(Area {
                id: GrimId::new(),
                friendly_id: area_fid.into(),
                name: area_fid.into(),
            })
            .id();
        app.world_mut()
            .spawn((
                Room {
                    id: GrimId::new(),
                    friendly_id: room_fid.into(),
                    name: room_fid.into(),
                    description: String::new(),
                    area,
                },
                exits,
            ))
            .id()
    }

    fn spawn_actor_in(app: &mut App, room: Entity, admin: bool) -> Entity {
        let roles = if admin { vec![Role::Admin] } else { Vec::new() };
        app.world_mut()
            .spawn((
                InRoom { room },
                GrimName("Admin".into()),
                Actor {
                    race: String::new(),
                    level: 1,
                    gender: Gender::Neutral,
                },
                Character {
                    credits: 100,
                    id: GrimId::new(),
                    account_id: GrimId::new(),
                    created_at: chrono::Utc::now(),
                    last_room: None,
                    roles,
                    class: String::new(),
                    title: None,
                    restrings: std::collections::HashMap::new(),
                    config: std::collections::HashMap::new(),
                    game_state: Default::default(),
                },
            ))
            .id()
    }
    fn room_of(app: &App, actor: Entity) -> Entity {
        app.world().get::<InRoom>(actor).unwrap().room
    }

    fn send_goto(app: &mut App, actor: Entity, target: &str) {
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Goto {
                target: target.into(),
            },
        });
        app.update();
    }

    fn send_recall(app: &mut App, actor: Entity) {
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Recall,
        });
        app.update();
    }

    fn send_move(app: &mut App, actor: Entity, direction: Cardinal) {
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Move { direction },
        });
        app.update();
    }

    fn hang_door(app: &mut App, room: Entity, dir: Cardinal, open: bool) {
        hang_hidden_door(app, room, dir, open, false);
    }

    fn hang_hidden_door(app: &mut App, room: Entity, dir: Cardinal, open: bool, hidden: bool) {
        if app.world().get::<Doors>(room).is_none() {
            app.world_mut().entity_mut(room).insert(Doors::default());
        }
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

    #[test]
    fn closed_door_blocks_move_without_placement_or_events() {
        let mut app = test_app();
        let a = spawn_room(&mut app, "haven", "tavern", Exits::default());
        let b = spawn_room(&mut app, "haven", "privy", Exits::default());
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
        hang_door(&mut app, a, Cardinal::East, false);
        let before_looks = look_room_count(&app);
        let actor = spawn_actor_in(&mut app, a, false);
        send_move(&mut app, actor, Cardinal::East);
        assert_eq!(room_of(&app, actor), a);
        assert!(info_texts(&app)
            .iter()
            .any(|t| t.contains("The privy door is closed")));
        assert_eq!(look_room_count(&app), before_looks);
        assert_eq!(move_event_count(&app), 0);
    }

    #[test]
    fn hidden_closed_door_refuses_as_no_exit() {
        // A secret door refuses exactly like a non-exit: no door name leaks.
        let mut app = test_app();
        let a = spawn_room(&mut app, "haven", "tavern", Exits::default());
        let b = spawn_room(&mut app, "haven", "cellar", Exits::default());
        app.world_mut()
            .get_mut::<Exits>(a)
            .unwrap()
            .exits
            .insert(Cardinal::Down, b);
        hang_hidden_door(&mut app, a, Cardinal::Down, false, true);
        let actor = spawn_actor_in(&mut app, a, false);
        send_move(&mut app, actor, Cardinal::Down);
        assert_eq!(room_of(&app, actor), a);
        assert!(
            info_texts(&app)
                .iter()
                .any(|t| t == "You can't go that way.\n"),
            "hidden refusal must equal the no-exit reply; got: {:?}",
            info_texts(&app)
        );
    }

    #[test]
    fn hidden_open_door_allows_move() {
        // Guessed-and-opened secrets walk normally (discovery by movement).
        let mut app = test_app();
        let a = spawn_room(&mut app, "haven", "tavern", Exits::default());
        let b = spawn_room(&mut app, "haven", "cellar", Exits::default());
        app.world_mut()
            .get_mut::<Exits>(a)
            .unwrap()
            .exits
            .insert(Cardinal::Down, b);
        app.world_mut()
            .get_mut::<Exits>(b)
            .unwrap()
            .exits
            .insert(Cardinal::Up, a);
        hang_hidden_door(&mut app, a, Cardinal::Down, true, true);
        hang_hidden_door(&mut app, b, Cardinal::Up, true, false);
        let actor = spawn_actor_in(&mut app, a, false);
        send_move(&mut app, actor, Cardinal::Down);
        assert_eq!(room_of(&app, actor), b);
    }

    #[test]
    fn open_door_allows_move() {
        let mut app = test_app();
        let a = spawn_room(&mut app, "haven", "tavern", Exits::default());
        let b = spawn_room(&mut app, "haven", "privy", Exits::default());
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
        hang_door(&mut app, a, Cardinal::East, true);
        let actor = spawn_actor_in(&mut app, a, false);
        send_move(&mut app, actor, Cardinal::East);
        app.update();
        assert_eq!(room_of(&app, actor), b);
    }

    fn info_texts(app: &App) -> Vec<String> {
        let messages = app.world().resource::<Messages<InfoMessage>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).map(|m| m.text.clone()).collect()
    }

    fn move_event_count(app: &App) -> usize {
        let messages = app.world().resource::<Messages<MoveEvent>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).count()
    }

    fn recall_events(app: &App) -> Vec<(Entity, Entity, Entity)> {
        let messages = app
            .world()
            .resource::<Messages<grim_core::events::RecallEvent>>();
        let mut cursor = messages.get_cursor();
        cursor
            .read(messages)
            .map(|e| (e.actor, e.from, e.to))
            .collect()
    }

    // ── walking an exit ──────────────────────────────────────────────
    mod walking {
        use super::*;

        #[test]
        fn move_valid_exit_updates_in_room() {
            let mut app = test_app();
            let room2 = app.world_mut().spawn(()).id();
            let mut exits = Exits::default();
            exits.exits.insert(Cardinal::North, room2);
            let room1 = app.world_mut().spawn(exits).id();
            let actor = app.world_mut().spawn(InRoom { room: room1 }).id();
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Move {
                    direction: Cardinal::North,
                },
            });
            app.update();
            assert_eq!(app.world().get::<InRoom>(actor).unwrap().room, room2);
            {
                let messages = app.world().resource::<Messages<MoveEvent>>();
                let mut cursor = messages.get_cursor();
                let mut iter = cursor.read(messages);
                let ev = iter.next().expect("expected one MoveEvent");
                assert_eq!(ev.from, room1);
                assert_eq!(ev.to, room2);
                assert!(iter.next().is_none(), "expected exactly one MoveEvent");
            }
            assert_eq!(look_room_count(&app), 1);
        }

        /// Observer-fired transition log: the synchronous record of which
        /// moments fired, in firing order. Wired in [`super::test_app`].
        #[derive(Resource, Default)]
        pub(super) struct TransitionLog(pub(super) Vec<(String, Entity, Entity)>);

        pub(super) fn log_attempt_walk(e: On<AttemptWalk>, mut log: ResMut<TransitionLog>) {
            let ev = e.event();
            log.0.push(("attempt_walk".into(), ev.actor, ev.room));
        }

        pub(super) fn log_attempt_leave(e: On<AttemptLeave>, mut log: ResMut<TransitionLog>) {
            let ev = e.event();
            log.0.push(("attempt_leave".into(), ev.actor, ev.room));
        }

        pub(super) fn log_attempt_enter(e: On<AttemptEnter>, mut log: ResMut<TransitionLog>) {
            let ev = e.event();
            log.0.push(("attempt_enter".into(), ev.actor, ev.room));
        }

        pub(super) fn log_leave(e: On<Leave>, mut log: ResMut<TransitionLog>) {
            let ev = e.event();
            log.0.push(("leave".into(), ev.actor, ev.room));
        }

        pub(super) fn log_enter(e: On<Enter>, mut log: ResMut<TransitionLog>) {
            let ev = e.event();
            log.0.push(("enter".into(), ev.actor, ev.room));
        }

        /// The (attempt_walk, attempt_leave, attempt_enter, leave, enter) counts.
        pub(super) fn transition_events(app: &App) -> (usize, usize, usize, usize, usize) {
            let log = app.world().resource::<TransitionLog>();
            let mut counts = (0, 0, 0, 0, 0);
            for (kind, _, _) in &log.0 {
                match kind.as_str() {
                    "attempt_walk" => counts.0 += 1,
                    "attempt_leave" => counts.1 += 1,
                    "attempt_enter" => counts.2 += 1,
                    "leave" => counts.3 += 1,
                    "enter" => counts.4 += 1,
                    _ => {}
                }
            }
            counts
        }

        #[test]
        fn move_valid_exit_emits_attempts_then_facts_in_order() {
            let mut app = test_app();
            let room2 = app.world_mut().spawn(()).id();
            let mut exits = Exits::default();
            exits.exits.insert(Cardinal::North, room2);
            let room1 = app.world_mut().spawn(exits).id();
            let actor = app.world_mut().spawn(InRoom { room: room1 }).id();
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Move {
                    direction: Cardinal::North,
                },
            });
            app.update();
            // Attempts fire (and placement lands) before any fact: facts
            // always fire a later tick, whatever the command-flush timing.
            assert_eq!(transition_events(&app), (1, 1, 1, 0, 0));
            assert_eq!(room_of(&app, actor), room2);
            app.update();
            app.update();
            assert_eq!(transition_events(&app), (1, 1, 1, 1, 1));
            // Attempts fire before facts, in phase order.
            let log = app.world().resource::<TransitionLog>();
            let kinds: Vec<&str> = log.0.iter().map(|(k, _, _)| k.as_str()).collect();
            assert_eq!(
                kinds,
                [
                    "attempt_walk",
                    "attempt_leave",
                    "attempt_enter",
                    "leave",
                    "enter"
                ]
            );
            assert_eq!((log.0[1].1, log.0[1].2), (actor, room1));
            assert_eq!((log.0[4].1, log.0[4].2), (actor, room2));
        }

        #[test]
        fn denied_attempt_leave_blocks_placement() {
            let mut app = test_app();
            let room2 = app.world_mut().spawn(()).id();
            let mut exits = Exits::default();
            exits.exits.insert(Cardinal::North, room2);
            let room1 = app.world_mut().spawn(exits).id();
            let actor = app.world_mut().spawn(InRoom { room: room1 }).id();
            app.add_observer(|mut e: On<AttemptLeave>| e.event_mut().denied = true);
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Move {
                    direction: Cardinal::North,
                },
            });
            app.update();
            app.update();
            app.update();
            assert_eq!(room_of(&app, actor), room1, "denied move must not place");
            assert_eq!(transition_events(&app), (1, 1, 0, 0, 0));
            assert_eq!(look_room_count(&app), 0, "denied move shows no arrival");
        }

        #[test]
        fn denied_attempt_enter_blocks_placement() {
            let mut app = test_app();
            let room2 = app.world_mut().spawn(()).id();
            let mut exits = Exits::default();
            exits.exits.insert(Cardinal::North, room2);
            let room1 = app.world_mut().spawn(exits).id();
            let actor = app.world_mut().spawn(InRoom { room: room1 }).id();
            app.add_observer(|mut e: On<AttemptEnter>| e.event_mut().denied = true);
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Move {
                    direction: Cardinal::North,
                },
            });
            app.update();
            app.update();
            app.update();
            assert_eq!(room_of(&app, actor), room1, "denied move must not place");
            assert_eq!(transition_events(&app), (1, 1, 1, 0, 0));
        }

        #[test]
        fn denied_attempt_walk_blocks_everything_after_it() {
            let mut app = test_app();
            let room2 = app.world_mut().spawn(()).id();
            let mut exits = Exits::default();
            exits.exits.insert(Cardinal::North, room2);
            let room1 = app.world_mut().spawn(exits).id();
            let actor = app.world_mut().spawn(InRoom { room: room1 }).id();
            app.add_observer(|mut e: On<AttemptWalk>| e.event_mut().denied = true);
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Move {
                    direction: Cardinal::North,
                },
            });
            app.update();
            app.update();
            app.update();
            assert_eq!(room_of(&app, actor), room1, "denied move must not place");
            assert_eq!(transition_events(&app), (1, 0, 0, 0, 0));
            assert_eq!(look_room_count(&app), 0, "denied move shows no arrival");
        }

        #[test]
        fn second_queued_move_for_same_actor_is_stale_and_skipped() {
            let mut app = test_app();
            let room2 = app.world_mut().spawn(()).id();
            let room3 = app.world_mut().spawn(()).id();
            let mut exits = Exits::default();
            exits.exits.insert(Cardinal::North, room2);
            exits.exits.insert(Cardinal::East, room3);
            let room1 = app.world_mut().spawn(exits).id();
            let actor = app.world_mut().spawn(InRoom { room: room1 }).id();
            // Both intents validate against room1 and queue; the north
            // closure places first, so the east closure finds the actor
            // already gone and must drop rather than walk from room1.
            for direction in [Cardinal::North, Cardinal::East] {
                app.world_mut().write_message(EngineCommand {
                    client: actor,
                    command: Command::Move { direction },
                });
            }
            app.update();
            app.update();
            app.update();
            assert_eq!(room_of(&app, actor), room2);
            let messages = app.world().resource::<Messages<MoveEvent>>();
            let mut cursor = messages.get_cursor();
            let mut iter = cursor.read(messages);
            let ev = iter.next().expect("exactly the north move");
            assert_eq!((ev.from, ev.to), (room1, room2));
            assert!(iter.next().is_none(), "stale east move must not fire");
        }

        #[test]
        fn move_no_exit_emits_no_transitions() {
            let mut app = test_app();
            let room1 = app.world_mut().spawn(Exits::default()).id();
            let actor = app.world_mut().spawn(InRoom { room: room1 }).id();
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Move {
                    direction: Cardinal::North,
                },
            });
            app.update();
            assert_eq!(transition_events(&app), (0, 0, 0, 0, 0));
        }

        #[test]
        fn move_no_exit_emits_info_message() {
            let mut app = test_app();
            let room1 = app.world_mut().spawn(Exits::default()).id();
            let actor = app.world_mut().spawn(InRoom { room: room1 }).id();
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Move {
                    direction: Cardinal::North,
                },
            });
            app.update();
            assert_eq!(app.world().get::<InRoom>(actor).unwrap().room, room1);
            {
                let messages = app.world().resource::<Messages<InfoMessage>>();
                let mut cursor = messages.get_cursor();
                let mut iter = cursor.read(messages);
                let ev = iter.next().expect("expected one InfoMessage");
                assert_eq!(ev.text, "You can't go that way.\n");
                assert!(iter.next().is_none(), "expected exactly one InfoMessage");
            }
        }

        #[test]
        fn move_updates_character_last_room_to_destination_friendly_ids() {
            let mut app = test_app();
            let room2 = spawn_room(&mut app, "town", "market", Exits::default());
            let mut exits = Exits::default();
            exits.exits.insert(Cardinal::North, room2);
            let room1 = spawn_room(&mut app, "town", "square", exits);
            let actor = app
                .world_mut()
                .spawn((
                    InRoom { room: room1 },
                    GrimName("Walker".into()),
                    Actor {
                        race: String::new(),
                        level: 1,
                        gender: Gender::Neutral,
                    },
                    Character {
                        credits: 100,
                        id: GrimId::new(),
                        account_id: GrimId::new(),
                        created_at: chrono::Utc::now(),
                        last_room: None,
                        roles: Vec::new(),
                        class: String::new(),
                        title: None,
                        restrings: std::collections::HashMap::new(),
                        config: std::collections::HashMap::new(),
                        game_state: Default::default(),
                    },
                ))
                .id();
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Move {
                    direction: Cardinal::North,
                },
            });
            app.update();
            let loc = app
                .world()
                .get::<Character>(actor)
                .unwrap()
                .last_room
                .clone()
                .expect("last_room should be set after moving");
            assert_eq!(loc.area, "town");
            assert_eq!(loc.room, "market");
        }

        #[test]
        fn move_without_in_room_is_ignored() {
            // An actor with no `InRoom` placement can't move; the handler skips
            // it rather than erroring.
            let mut app = test_app();
            let actor = app.world_mut().spawn(()).id();
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Move {
                    direction: Cardinal::North,
                },
            });
            app.update();
            let messages = app.world().resource::<Messages<InfoMessage>>();
            let mut cursor = messages.get_cursor();
            assert_eq!(cursor.read(messages).count(), 0);
            assert_eq!(look_room_count(&app), 0);
        }

        #[test]
        fn move_from_room_without_exits_component_blocks() {
            // The current room has no `Exits` component at all → the exits query
            // errors and the move is refused with "You can't go that way".
            let mut app = test_app();
            let room = app.world_mut().spawn(()).id();
            let actor = app.world_mut().spawn(InRoom { room }).id();
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Move {
                    direction: Cardinal::North,
                },
            });
            app.update();
            assert_eq!(app.world().get::<InRoom>(actor).unwrap().room, room);
            let messages = app.world().resource::<Messages<InfoMessage>>();
            let mut cursor = messages.get_cursor();
            let mut iter = cursor.read(messages);
            assert_eq!(iter.next().unwrap().text, "You can't go that way.\n");
            assert!(iter.next().is_none());
        }

        #[test]
        fn move_without_character_component_does_not_panic() {
            // A non-character actor (e.g. an NPC) can move; the last_room update
            // is simply skipped rather than erroring.
            let mut app = test_app();
            let room2 = spawn_room(&mut app, "town", "market", Exits::default());
            let mut exits = Exits::default();
            exits.exits.insert(Cardinal::North, room2);
            let room1 = spawn_room(&mut app, "town", "square", exits);
            let actor = app.world_mut().spawn(InRoom { room: room1 }).id();
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Move {
                    direction: Cardinal::North,
                },
            });
            app.update();
            assert_eq!(app.world().get::<InRoom>(actor).unwrap().room, room2);
        }
    }

    // ── admin goto + address resolution ──────────────────────────────
    mod goto {
        use super::*;

        #[test]
        fn goto_bare_slug_moves_admin_and_updates_last_room() {
            let mut app = test_app();
            let dest = spawn_room(&mut app, "town", "market", Exits::default());
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            send_goto(&mut app, actor, "market");
            assert_eq!(room_of(&app, actor), dest);
            assert_eq!(look_room_count(&app), 1);
            let loc = app
                .world()
                .get::<Character>(actor)
                .unwrap()
                .last_room
                .clone()
                .expect("goto should refresh last_room");
            assert_eq!((loc.area.as_str(), loc.room.as_str()), ("town", "market"));
        }

        #[test]
        fn goto_emits_transitions_between_rooms() {
            let mut app = test_app();
            let dest = spawn_room(&mut app, "town", "market", Exits::default());
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            send_goto(&mut app, actor, "market");
            assert_eq!(room_of(&app, actor), dest);
            // Teleports skip the walk intent but greet both rooms; facts land later.
            assert_eq!(super::walking::transition_events(&app), (0, 1, 1, 0, 0));
            app.update();
            app.update();
            assert_eq!(super::walking::transition_events(&app), (0, 1, 1, 1, 1));
            let log = app.world().resource::<super::walking::TransitionLog>();
            assert_eq!((log.0[0].1, log.0[0].2), (actor, start));
            assert_eq!((log.0[3].1, log.0[3].2), (actor, dest));
        }

        #[test]
        fn goto_same_room_emits_no_transitions() {
            let mut app = test_app();
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            send_goto(&mut app, actor, "square");
            assert_eq!(room_of(&app, actor), start);
            assert_eq!(super::walking::transition_events(&app), (0, 0, 0, 0, 0));
        }

        #[test]
        fn goto_area_room_slug_disambiguates() {
            let mut app = test_app();
            let _town_market = spawn_room(&mut app, "town", "market", Exits::default());
            let forest_market = spawn_room(&mut app, "forest", "market", Exits::default());
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            send_goto(&mut app, actor, "forest:market");
            assert_eq!(room_of(&app, actor), forest_market);
        }

        #[test]
        fn goto_bare_slug_matching_two_areas_lists_candidates() {
            let mut app = test_app();
            spawn_room(&mut app, "town", "market", Exits::default());
            spawn_room(&mut app, "forest", "market", Exits::default());
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            send_goto(&mut app, actor, "market");
            assert_eq!(room_of(&app, actor), start, "ambiguous goto must not move");
            let text = info_texts(&app).join("");
            assert!(text.contains("Select an option..."));
            // One detail line per candidate, each carrying its ids.
            let lines: Vec<&str> = text.lines().filter(|l| l.contains("entity:")).collect();
            assert_eq!(lines.len(), 2, "expected two candidates, got:\n{text}");
            assert!(lines
                .iter()
                .all(|l| l.contains("grim:") && l.contains("slug:market")));
        }

        #[test]
        fn goto_area_room_all_token_permutations_resolve() {
            // area side ∈ {entity, grim, slug} × room side ∈ {entity, grim, slug}.
            let mut app = test_app();
            let market = spawn_room(&mut app, "town", "market", Exits::default());
            let start = spawn_room(&mut app, "forest", "clearing", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);

            let area = app.world().get::<Room>(market).unwrap().area;
            let area_tokens = [
                area.to_bits().to_string(),
                app.world().get::<Area>(area).unwrap().id.to_string(),
                "town".to_string(),
            ];
            let room_tokens = [
                market.to_bits().to_string(),
                app.world().get::<Room>(market).unwrap().id.to_string(),
                "market".to_string(),
            ];
            for a in &area_tokens {
                for r in &room_tokens {
                    app.world_mut().get_mut::<InRoom>(actor).unwrap().room = start;
                    send_goto(&mut app, actor, &format!("{a}:{r}"));
                    assert_eq!(
                        room_of(&app, actor),
                        market,
                        "address {a}:{r} should resolve"
                    );
                }
            }
        }

        #[test]
        fn goto_bare_all_token_forms_resolve() {
            // bare room token ∈ {entity, grim, slug}.
            let mut app = test_app();
            let market = spawn_room(&mut app, "town", "market", Exits::default());
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            let tokens = [
                market.to_bits().to_string(),
                app.world().get::<Room>(market).unwrap().id.to_string(),
                "market".to_string(),
            ];
            for t in &tokens {
                app.world_mut().get_mut::<InRoom>(actor).unwrap().room = start;
                send_goto(&mut app, actor, t);
                assert_eq!(
                    room_of(&app, actor),
                    market,
                    "bare token {t} should resolve"
                );
            }
        }

        #[test]
        fn goto_area_room_slug_ambiguous_within_area_lists_candidates() {
            // Two rooms sharing a slug in the SAME area (e.g. instanced) → the
            // `<area>:<room>` slug path is itself ambiguous and lists both.
            let mut app = test_app();
            let a = spawn_room(&mut app, "town", "market", Exits::default());
            let area = app.world().get::<Room>(a).unwrap().area;
            // A second "market" room in the very same area.
            app.world_mut().spawn((
                Room {
                    id: GrimId::new(),
                    friendly_id: "market".into(),
                    name: "Town Square".into(),
                    description: String::new(),
                    area,
                },
                GrimName("Town Square".into()),
                Exits::default(),
            ));
            let start = spawn_room(&mut app, "forest", "clearing", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            send_goto(&mut app, actor, "town:market");
            assert_eq!(
                room_of(&app, actor),
                start,
                "ambiguous area:room must not move"
            );
            let text = info_texts(&app).join("");
            assert!(text.contains("Select an option..."));
            assert_eq!(text.lines().filter(|l| l.contains("entity:")).count(), 2);
        }

        #[test]
        fn goto_area_room_not_found_variants() {
            let mut app = test_app();
            spawn_room(&mut app, "town", "market", Exits::default());
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            for addr in ["nowhere:market", "town:nowhere"] {
                app.world_mut().get_mut::<InRoom>(actor).unwrap().room = start;
                send_goto(&mut app, actor, addr);
                assert_eq!(room_of(&app, actor), start, "{addr} must not move");
            }
            assert!(info_texts(&app)
                .iter()
                .any(|t| t.contains("No room matches")));
        }

        #[test]
        fn goto_by_entity_id_moves() {
            let mut app = test_app();
            let dest = spawn_room(&mut app, "town", "market", Exits::default());
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            send_goto(&mut app, actor, &dest.to_bits().to_string());
            assert_eq!(room_of(&app, actor), dest);
        }

        #[test]
        fn goto_by_room_grim_id_moves() {
            let mut app = test_app();
            let dest = spawn_room(&mut app, "town", "market", Exits::default());
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            let gid = app.world().get::<Room>(dest).unwrap().id.to_string();
            send_goto(&mut app, actor, &gid);
            assert_eq!(room_of(&app, actor), dest);
        }

        #[test]
        fn goto_by_area_grim_id_and_room_slug() {
            let mut app = test_app();
            let dest = spawn_room(&mut app, "town", "market", Exits::default());
            let start = spawn_room(&mut app, "forest", "clearing", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            let area_gid = {
                let area = app.world().get::<Room>(dest).unwrap().area;
                app.world().get::<Area>(area).unwrap().id.to_string()
            };
            send_goto(&mut app, actor, &format!("{area_gid}:market"));
            assert_eq!(room_of(&app, actor), dest);
        }

        #[test]
        fn goto_unknown_slug_reports_not_found() {
            let mut app = test_app();
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            send_goto(&mut app, actor, "nowhere");
            assert_eq!(room_of(&app, actor), start);
            assert!(info_texts(&app)
                .iter()
                .any(|t| t.contains("No room matches 'nowhere'")));
        }

        #[test]
        fn goto_numeric_that_is_not_a_live_room_falls_through() {
            // A well-formed but dead entity id parses, misses the live-room check,
            // and falls through to the (also-missing) slug lookup → NotFound.
            let mut app = test_app();
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let ghost = app.world_mut().spawn(()).id();
            let bits = ghost.to_bits();
            app.world_mut().despawn(ghost);
            let actor = spawn_actor_in(&mut app, start, true);
            send_goto(&mut app, actor, &bits.to_string());
            assert_eq!(room_of(&app, actor), start);
            assert!(info_texts(&app)
                .iter()
                .any(|t| t.contains("No room matches")));
        }

        #[test]
        fn goto_empty_target_reports_not_found() {
            let mut app = test_app();
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, start, true);
            send_goto(&mut app, actor, "");
            assert_eq!(room_of(&app, actor), start);
            assert!(!info_texts(&app).is_empty());
        }

        #[test]
        fn goto_is_ignored_for_non_admin() {
            let mut app = test_app();
            let dest = spawn_room(&mut app, "town", "market", Exits::default());
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let _ = dest;
            let actor = spawn_actor_in(&mut app, start, false);
            send_goto(&mut app, actor, "market");
            assert_eq!(room_of(&app, actor), start, "non-admin goto must not move");
            assert_eq!(look_room_count(&app), 0);
            assert!(
                info_texts(&app).is_empty(),
                "non-admin goto must stay silent"
            );
        }
    }

    // ── player recall ──────────────────────────────────────────
    mod recall {
        use super::*;

        #[test]
        fn recall_moves_non_admin_to_haven_square_and_updates_last_room() {
            // Recall is a player verb: no admin gate, unlike goto.
            let mut app = test_app();
            let square = spawn_room(&mut app, "haven", "square", Exits::default());
            let start = spawn_room(&mut app, "haven", "tavern", Exits::default());
            let actor = spawn_actor_in(&mut app, start, false);
            send_recall(&mut app, actor);
            assert_eq!(room_of(&app, actor), square);
            assert_eq!(look_room_count(&app), 1);
            // The room echoes render from this event, not from a `MoveEvent`.
            assert_eq!(recall_events(&app), vec![(actor, start, square)]);
            assert_eq!(move_event_count(&app), 0);
            let loc = app
                .world()
                .get::<Character>(actor)
                .unwrap()
                .last_room
                .clone()
                .expect("recall should refresh last_room");
            assert_eq!((loc.area.as_str(), loc.room.as_str()), ("haven", "square"));
        }

        #[test]
        fn recall_emits_transitions_between_rooms() {
            let mut app = test_app();
            let square = spawn_room(&mut app, "haven", "square", Exits::default());
            let start = spawn_room(&mut app, "haven", "tavern", Exits::default());
            let actor = spawn_actor_in(&mut app, start, false);
            send_recall(&mut app, actor);
            assert_eq!(room_of(&app, actor), square);
            // Teleports skip the walk intent but greet both rooms; facts land later.
            assert_eq!(super::walking::transition_events(&app), (0, 1, 1, 0, 0));
            app.update();
            app.update();
            assert_eq!(super::walking::transition_events(&app), (0, 1, 1, 1, 1));
            assert_eq!(move_event_count(&app), 0);
        }

        #[test]
        fn recall_already_there_replies_without_moving() {
            let mut app = test_app();
            let square = spawn_room(&mut app, "haven", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, square, false);
            send_recall(&mut app, actor);
            assert_eq!(room_of(&app, actor), square);
            assert_eq!(info_texts(&app), vec!["You are already there.\n"]);
            assert_eq!(look_room_count(&app), 0);
            assert_eq!(super::walking::transition_events(&app), (0, 0, 0, 0, 0));
            assert!(recall_events(&app).is_empty());
        }

        #[test]
        fn recall_without_in_room_is_ignored() {
            // No placement, no recall: skipped silently, like `handle_move` —
            // no `last_room` rewrite, no look at a room the actor is not in.
            let mut app = test_app();
            spawn_room(&mut app, "haven", "square", Exits::default());
            let actor = app.world_mut().spawn_empty().id();
            send_recall(&mut app, actor);
            assert!(info_texts(&app).is_empty());
            assert_eq!(look_room_count(&app), 0);
            assert_eq!(move_event_count(&app), 0);
            assert!(recall_events(&app).is_empty());
        }

        #[test]
        fn recall_without_haven_square_fails_closed() {
            // A world with no `haven:square` (renamed area data): the actor
            // stays put and gets a reply, never a panic or a stray look.
            let mut app = test_app();
            let start = spawn_room(&mut app, "town", "square", Exits::default());
            let actor = spawn_actor_in(&mut app, start, false);
            send_recall(&mut app, actor);
            assert_eq!(room_of(&app, actor), start);
            assert_eq!(info_texts(&app), vec!["Nothing happens.\n"]);
            assert_eq!(look_room_count(&app), 0);
            assert_eq!(super::walking::transition_events(&app), (0, 0, 0, 0, 0));
        }
    }
}

//! `look` rendering: turn a `look` command into the room/entity description
//! events (`LookRoom`/`LookEntity`) or a "not here" info message. Reads the
//! actor's [`InRoom`] placement and the [`Name`] of entities sharing the room;
//! targets resolve through [`grim_target`] (see [`find_subject`]).

use bevy::prelude::*;
use grim_core::components::{CarriedBy, Keywords, Name};
use grim_core::events::{Command, EngineCommand, InfoMessage, LookEntity, LookRoom};
use grim_target::{parse_target, query, ParseOptions};

use crate::occupants::in_room;
use crate::placement::InRoom;

/// `look` / `look <target>`: show the actor's room or a named entity in it.
#[allow(clippy::too_many_arguments)] // Bevy injects the independent query/message parameters.
pub(crate) fn handle_look(
    mut engine: MessageReader<EngineCommand>,
    inroom: Query<&InRoom>,
    visibility: crate::visibility::Visibility,
    named: Query<(Entity, &InRoom, &Name, Option<&Keywords>)>,
    carried: Query<(Entity, &CarriedBy, &Name, Option<&Keywords>)>,
    mut look_room: MessageWriter<LookRoom>,
    mut look_entity: MessageWriter<LookEntity>,
    mut info: MessageWriter<InfoMessage>,
) {
    for cmd in engine.read() {
        let Command::Look { target } = &cmd.command else {
            continue;
        };
        let actor = cmd.client;
        let Ok(actor_room) = inroom.get(actor) else {
            continue;
        };
        match target {
            None => {
                look_room.write(LookRoom {
                    target: actor,
                    room: actor_room.room,
                });
            }
            Some(name) => {
                let subject = find_subject(name, actor, actor_room.room, &named, &visibility)
                    .or_else(|| {
                        let spec = parse_target(name, ParseOptions::BEING)?;
                        query(
                            &spec,
                            carried
                                .iter()
                                .filter(|(_, held, _, _)| held.carrier == actor)
                                .map(|(e, _, n, k)| {
                                    (e, n.0.as_str(), k.map(|k| k.0.as_slice()).unwrap_or(&[]))
                                }),
                        )
                        .into_iter()
                        .next()
                    });
                match subject {
                    Some(subject) => {
                        look_entity.write(LookEntity {
                            target: actor,
                            subject,
                        });
                    }
                    None => {
                        info.write(InfoMessage {
                            target: actor,
                            text: "You don't see that here.\n".into(),
                        });
                    }
                }
            }
        }
    }
}

/// Resolve a `look` target to an entity in `room`. `self` is always the actor;
/// otherwise the [`ParseOptions::BEING`] spec picks the winner: the best
/// ranked candidate, or the Nth (`look 2.goblin`). Ranking is exact name,
/// then exact keyword, then name/keyword prefix with the shortest name first
/// (so `wrack` finds `Wrack` over `Wrackus` regardless of order, and `wracku`
/// disambiguates). Ties fall through to the lowest entity id, keeping
/// resolution deterministic.
fn find_subject(
    raw: &str,
    actor: Entity,
    room: Entity,
    named: &Query<(Entity, &InRoom, &Name, Option<&Keywords>)>,
    visibility: &crate::visibility::Visibility,
) -> Option<Entity> {
    if raw.trim().eq_ignore_ascii_case("self") {
        return Some(actor);
    }
    let spec = parse_target(raw, ParseOptions::BEING)?;
    let here = in_room(room, named.iter().map(|(e, ir, _, _)| (e, ir.room)), None);
    query(
        &spec,
        here.into_iter()
            .filter(|e| visibility.sees_actor(actor, *e))
            .filter_map(|e| {
                let (_, _, name, keywords) = named.get(e).ok()?;
                Some((
                    e,
                    name.0.as_str(),
                    keywords.map(|k| k.0.as_slice()).unwrap_or(&[]),
                ))
            }),
    )
    .into_iter()
    .next()
}

/// Wire the `look` handler and the input/delivery messages it owns. The
/// world-happening events it emits (`LookRoom`/`LookEntity`) are registered by
/// `grim_world::WorldPlugin`.
pub(crate) fn register(app: &mut App) {
    app.add_message::<EngineCommand>()
        .add_message::<InfoMessage>()
        .add_systems(Update, handle_look);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(grim_world::WorldPlugin);
        register(&mut app);
        app
    }

    fn look_room_count(app: &App) -> usize {
        let messages = app.world().resource::<Messages<LookRoom>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).count()
    }

    #[test]
    fn look_no_target_emits_look_room() {
        let mut app = test_app();
        let room = app.world_mut().spawn(()).id();
        let actor = app.world_mut().spawn(InRoom { room }).id();
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Look { target: None },
        });
        app.update();
        let messages = app.world().resource::<Messages<LookRoom>>();
        let mut cursor = messages.get_cursor();
        let mut iter = cursor.read(messages);
        let ev = iter.next().expect("expected one LookRoom");
        assert_eq!(ev.target, actor);
        assert_eq!(ev.room, room);
        assert!(iter.next().is_none(), "expected exactly one LookRoom");
    }

    #[test]
    fn look_valid_target_emits_look_entity() {
        let mut app = test_app();
        let room = app.world_mut().spawn(()).id();
        let actor = app
            .world_mut()
            .spawn((InRoom { room }, Name("hero".into())))
            .id();
        let goblin = app
            .world_mut()
            .spawn((InRoom { room }, Name("goblin".into())))
            .id();
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Look {
                target: Some("goblin".into()),
            },
        });
        app.update();
        let messages = app.world().resource::<Messages<LookEntity>>();
        let mut cursor = messages.get_cursor();
        let mut iter = cursor.read(messages);
        let ev = iter.next().expect("expected one LookEntity");
        assert_eq!(ev.target, actor);
        assert_eq!(ev.subject, goblin);
        assert!(iter.next().is_none(), "expected exactly one LookEntity");
    }

    #[test]
    fn look_self_targets_own_character() {
        let mut app = test_app();
        let room = app.world_mut().spawn(()).id();
        let actor = app
            .world_mut()
            .spawn((InRoom { room }, Name("hero".into())))
            .id();
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Look {
                target: Some("self".into()),
            },
        });
        app.update();
        let messages = app.world().resource::<Messages<LookEntity>>();
        let mut cursor = messages.get_cursor();
        let mut iter = cursor.read(messages);
        let ev = iter.next().expect("expected one LookEntity");
        assert_eq!(ev.target, actor);
        assert_eq!(ev.subject, actor);
        assert!(iter.next().is_none(), "expected exactly one LookEntity");
    }

    #[test]
    fn look_keyword_matches_creature() {
        let mut app = test_app();
        let room = app.world_mut().spawn(()).id();
        let actor = app
            .world_mut()
            .spawn((InRoom { room }, Name("hero".into())))
            .id();
        let smith = app
            .world_mut()
            .spawn((
                InRoom { room },
                Name("Grimmok Ironhand".into()),
                Keywords(vec!["grimmok".into(), "smith".into()]),
            ))
            .id();
        for target in ["SMITH", "smi"] {
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Look {
                    target: Some(target.into()),
                },
            });
        }
        app.update();
        let messages = app.world().resource::<Messages<LookEntity>>();
        let mut cursor = messages.get_cursor();
        let mut iter = cursor.read(messages);
        for _ in 0..2 {
            let ev = iter.next().expect("expected one LookEntity");
            assert_eq!(ev.target, actor);
            assert_eq!(ev.subject, smith);
        }
        assert!(iter.next().is_none(), "expected exactly two LookEntity");
    }

    fn look_subjects(app: &mut App, actor: Entity, targets: &[&str]) -> Vec<Entity> {
        for target in targets {
            app.world_mut().write_message(EngineCommand {
                client: actor,
                command: Command::Look {
                    target: Some((*target).into()),
                },
            });
        }
        app.update();
        let messages = app.world().resource::<Messages<LookEntity>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).map(|ev| ev.subject).collect()
    }

    #[test]
    fn look_exact_beats_partial_and_shortest_prefix_wins() {
        let mut app = test_app();
        let room = app.world_mut().spawn(()).id();
        let actor = app
            .world_mut()
            .spawn((InRoom { room }, Name("hero".into())))
            .id();
        // Longer name spawned first: ranking (not order) must decide.
        let wrackus = app
            .world_mut()
            .spawn((InRoom { room }, Name("Wrackus".into())))
            .id();
        let wrack = app
            .world_mut()
            .spawn((InRoom { room }, Name("Wrack".into())))
            .id();
        assert_eq!(
            look_subjects(&mut app, actor, &["wrack", "wracku", "WRACK"]),
            vec![wrack, wrackus, wrack]
        );
    }

    #[test]
    fn look_exact_keyword_beats_name_prefix() {
        let mut app = test_app();
        let room = app.world_mut().spawn(()).id();
        let actor = app
            .world_mut()
            .spawn((InRoom { room }, Name("hero".into())))
            .id();
        let _smithson = app
            .world_mut()
            .spawn((InRoom { room }, Name("Smithson".into())))
            .id();
        let smith = app
            .world_mut()
            .spawn((
                InRoom { room },
                Name("Grimmok Ironhand".into()),
                Keywords(vec!["smith".into()]),
            ))
            .id();
        assert_eq!(look_subjects(&mut app, actor, &["smith"]), vec![smith]);
    }

    #[test]
    fn look_without_in_room_is_ignored() {
        // An actor with no `InRoom` placement produces no look output.
        let mut app = test_app();
        let actor = app.world_mut().spawn(()).id();
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Look { target: None },
        });
        app.update();
        assert_eq!(look_room_count(&app), 0);
        let messages = app.world().resource::<Messages<InfoMessage>>();
        let mut cursor = messages.get_cursor();
        assert_eq!(cursor.read(messages).count(), 0);
    }

    #[test]
    fn look_invalid_target_emits_info_message() {
        let mut app = test_app();
        let room = app.world_mut().spawn(()).id();
        let actor = app
            .world_mut()
            .spawn((InRoom { room }, Name("hero".into())))
            .id();
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Look {
                target: Some("ghost".into()),
            },
        });
        app.update();
        {
            let messages = app.world().resource::<Messages<InfoMessage>>();
            let mut cursor = messages.get_cursor();
            let mut iter = cursor.read(messages);
            let ev = iter.next().expect("expected one InfoMessage");
            assert_eq!(ev.target, actor);
            assert_eq!(ev.text, "You don't see that here.\n");
            assert!(iter.next().is_none(), "expected exactly one InfoMessage");
        }
        assert_eq!(look_room_count(&app), 0);
    }

    #[test]
    fn look_offset_selects_second_ranked() {
        let mut app = test_app();
        let room = app.world_mut().spawn(()).id();
        let actor = app
            .world_mut()
            .spawn((InRoom { room }, Name("hero".into())))
            .id();
        // Exact ("goblin") outranks prefix ("goblins"), so `2.goblin` finds
        // the prefixed one.
        let goblins = app
            .world_mut()
            .spawn((InRoom { room }, Name("goblins".into())))
            .id();
        let goblin = app
            .world_mut()
            .spawn((InRoom { room }, Name("goblin".into())))
            .id();
        assert_eq!(
            look_subjects(&mut app, actor, &["goblin", "2.goblin"]),
            vec![goblin, goblins]
        );
    }

    #[test]
    fn look_quoted_group_matches_every_term() {
        let mut app = test_app();
        let room = app.world_mut().spawn(()).id();
        let actor = app
            .world_mut()
            .spawn((InRoom { room }, Name("hero".into())))
            .id();
        // No keywords: only the multi-term AND reaches the inner word.
        let lantern = app
            .world_mut()
            .spawn((InRoom { room }, Name("brass lantern".into())))
            .id();
        assert_eq!(
            look_subjects(&mut app, actor, &["\"brass lantern\""]),
            vec![lantern]
        );
    }
}

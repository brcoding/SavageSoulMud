//! `give <item> <target>`: hand carried objects to a being in the room.
//!
//! The item side is a [`grim_target`] spec ([`ParseOptions::ITEM`]) — `give
//! all sword bob` hands over every sword — while the being side takes a
//! [`ParseOptions::BEING`] spec (one recipient, `2.bob` for the second).
//! Player characters accept; creatures refuse ("They don't want that item.").
//! Each move emits one [`TransferEvent`], which the scene layer renders
//! per-recipient (first-party to the giver, second-party to the recipient,
//! third-party to the rest of the room). Misses answer the giver directly
//! with an [`InfoMessage`].

use bevy::prelude::*;
use grim_actor::placement::InRoom;
use grim_actor::{beings_in, Beings};
use grim_core::components::Name as GrimName;
use grim_core::events::{Command, EngineCommand, InfoMessage, TransferEvent, TransferKind};
use grim_target::{parse_target, query, ParseOptions, TargetSpec};
use grim_text::tr;

use crate::object::CarriedBy;
use crate::persist::Carried;

/// The being in `room` matching `spec` (never `actor`): exact name, exact
/// keyword, then shortest-prefix name, ties to the lowest entity id.
/// Returns the entity, its display name, and whether it is a creature.
/// Beings are the canonical shape (PCs online/linkdead + creatures).
/// Self-dealing is the caller's job (`is_self`); this only excludes the actor.
pub(crate) fn find_being(
    spec: &TargetSpec,
    room: Entity,
    actor: Entity,
    beings: &Beings,
    visibility: &grim_actor::visibility::Visibility,
) -> Option<(Entity, String, bool)> {
    let here = beings_in(room, beings, Some(actor));
    let found = query(
        spec,
        here.into_iter()
            .filter(|e| visibility.sees_actor(actor, *e) && visibility.sees_actor(*e, actor))
            .filter_map(|e| {
                let (_, _, nm, kw, ..) = beings.get(e).ok()?;
                Some((e, nm.0.as_str(), kw.map(|k| k.0.as_slice()).unwrap_or(&[])))
            }),
    );
    let entity = found.into_iter().next()?;
    let (_, _, name, _, _, is_creature, _, _) = beings.get(entity).ok()?;
    Some((entity, name.0.clone(), is_creature.is_some()))
}

/// `give <item> <target>`: move the matching carried objects into a being's
/// pack. PCs accept; creatures refuse.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_give(
    mut engine: MessageReader<EngineCommand>,
    mut commands: Commands,
    inroom: Query<&InRoom>,
    carried: Carried,
    beings: Beings,
    visibility: grim_actor::visibility::Visibility,
    names: Query<&GrimName>,
    mut transfers: MessageWriter<TransferEvent>,
    mut info: MessageWriter<InfoMessage>,
) {
    for cmd in engine.read() {
        let Command::Give { item, target } = &cmd.command else {
            continue;
        };
        let actor = cmd.client;
        let Ok(actor_room) = inroom.get(actor) else {
            continue;
        };
        let Ok(actor_name) = names.get(actor) else {
            continue;
        };
        // Your own hands first: `self` (or your own name) can never receive.
        if is_self(target, &actor_name.0) {
            info.write(InfoMessage {
                target: actor,
                text: tr!("item.give.self"),
            });
            continue;
        }
        let Some(item_spec) = parse_target(item, ParseOptions::ITEM) else {
            info.write(InfoMessage {
                target: actor,
                text: tr!("item.give.not_carried"),
            });
            continue;
        };
        let held = query(
            &item_spec,
            carried
                .iter()
                .filter(|(_, _, _, _, _, _, held)| held.carrier == actor)
                .map(|(e, nm, _, kw, _, _, _)| {
                    (e, nm.0.as_str(), kw.map(|k| k.0.as_slice()).unwrap_or(&[]))
                }),
        );
        if held.is_empty() {
            info.write(InfoMessage {
                target: actor,
                text: tr!("item.give.not_carried"),
            });
            continue;
        };
        let Some(being_spec) = parse_target(target, ParseOptions::BEING) else {
            info.write(InfoMessage {
                target: actor,
                text: tr!("item.give.not_here"),
            });
            continue;
        };
        match find_being(&being_spec, actor_room.room, actor, &beings, &visibility) {
            Some((recipient, recipient_name, false)) => {
                for entity in held {
                    let Ok((_, name, _, _, _, _, _)) = carried.get(entity) else {
                        continue;
                    };
                    let short = name.0.clone();
                    commands
                        .entity(entity)
                        .insert(CarriedBy { carrier: recipient });
                    transfers.write(TransferEvent {
                        mover: actor,
                        mover_name: actor_name.0.clone(),
                        other: recipient,
                        other_name: recipient_name.clone(),
                        room: actor_room.room,
                        short,
                        kind: TransferKind::Give,
                    });
                }
            }
            Some((_, _, true)) => {
                info.write(InfoMessage {
                    target: actor,
                    text: tr!("item.give.refused"),
                });
            }
            None => {
                info.write(InfoMessage {
                    target: actor,
                    text: tr!("item.give.not_here"),
                });
            }
        }
    }
}

/// `self`, or the actor's own exact name, as a transfer peer.
pub(crate) fn is_self(target: &str, actor_name: &str) -> bool {
    let want = target.trim().to_lowercase();
    want == "self" || want == actor_name.to_lowercase()
}

/// Wire the `give` handler and the messages it reads. [`TransferEvent`] is
/// registered by the plugin (shared with `steal`).
pub(crate) fn register(app: &mut App) {
    app.add_message::<EngineCommand>()
        .add_message::<InfoMessage>()
        .add_systems(Update, handle_give);
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_actor::{Character, Creature, Player};
    use grim_core::components::Keywords;
    use grim_core::GrimId;

    use crate::object::Object;

    fn character() -> Character {
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
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<TransferEvent>();
        register(&mut app);
        app
    }

    fn room_with_giver(app: &mut App) -> (Entity, Entity) {
        let room = app.world_mut().spawn_empty().id();
        let giver = app
            .world_mut()
            .spawn((
                GrimName("Alice".into()),
                character(),
                Player {
                    connection: Entity::PLACEHOLDER,
                },
                InRoom { room },
            ))
            .id();
        (room, giver)
    }

    fn spawn_pc(app: &mut App, room: Entity, name: &str) -> Entity {
        app.world_mut()
            .spawn((
                GrimName(name.into()),
                character(),
                Player {
                    connection: Entity::PLACEHOLDER,
                },
                InRoom { room },
            ))
            .id()
    }

    fn spawn_mob(app: &mut App, room: Entity, name: &str) -> Entity {
        app.world_mut()
            .spawn((GrimName(name.into()), Creature, InRoom { room }))
            .id()
    }
    fn give_item(app: &mut App, holder: Entity, name: &str, keywords: &[&str]) -> Entity {
        app.world_mut()
            .spawn((
                Object,
                GrimName(name.into()),
                Keywords(keywords.iter().map(|k| k.to_string()).collect()),
                CarriedBy { carrier: holder },
            ))
            .id()
    }

    fn transfers(app: &App) -> Vec<TransferEvent> {
        let messages = app.world().resource::<Messages<TransferEvent>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).cloned().collect()
    }

    fn infos(app: &App) -> Vec<(Entity, String)> {
        let messages = app.world().resource::<Messages<InfoMessage>>();
        let mut cursor = messages.get_cursor();
        cursor
            .read(messages)
            .map(|m| (m.target, m.text.clone()))
            .collect()
    }

    fn give(app: &mut App, giver: Entity, item: &str, target: &str) {
        app.world_mut().write_message(EngineCommand {
            client: giver,
            command: Command::Give {
                item: item.into(),
                target: target.into(),
            },
        });
        app.update();
    }

    #[test]
    fn give_moves_item_and_emits_event() {
        let mut app = test_app();
        let (room, giver) = room_with_giver(&mut app);
        let recipient = spawn_pc(&mut app, room, "Bob");
        let obj = give_item(&mut app, giver, "brass lantern", &["lantern"]);
        give(&mut app, giver, "lantern", "bob");

        assert_eq!(
            app.world().get::<CarriedBy>(obj).unwrap().carrier,
            recipient
        );
        let evs = transfers(&app);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, TransferKind::Give);
        assert_eq!(evs[0].mover, giver);
        assert_eq!(evs[0].mover_name, "Alice");
        assert_eq!(evs[0].other, recipient);
        assert_eq!(evs[0].other_name, "Bob");
        assert_eq!(evs[0].short, "brass lantern");
        assert!(infos(&app).is_empty());
    }

    #[test]
    fn give_to_creature_is_refused_and_kept() {
        let mut app = test_app();
        let (room, giver) = room_with_giver(&mut app);
        spawn_mob(&mut app, room, "Goblin");
        let obj = give_item(&mut app, giver, "coin", &["coin"]);
        give(&mut app, giver, "coin", "goblin");

        assert_eq!(app.world().get::<CarriedBy>(obj).unwrap().carrier, giver);
        assert!(transfers(&app).is_empty());
        assert_eq!(
            infos(&app),
            vec![(giver, "They don't want that item.\n".to_string())]
        );
    }

    #[test]
    fn give_misses_unheld_item() {
        let mut app = test_app();
        let (room, giver) = room_with_giver(&mut app);
        spawn_pc(&mut app, room, "Bob");
        give(&mut app, giver, "lantern", "bob");

        assert!(transfers(&app).is_empty());
        assert_eq!(
            infos(&app),
            vec![(giver, "You aren't carrying that.\n".to_string())]
        );
    }

    #[test]
    fn give_misses_absent_target() {
        let mut app = test_app();
        let (_room, giver) = room_with_giver(&mut app);
        give_item(&mut app, giver, "coin", &["coin"]);
        give(&mut app, giver, "coin", "nobody");

        assert!(transfers(&app).is_empty());
        assert_eq!(
            infos(&app),
            vec![(giver, "They aren't here.\n".to_string())]
        );
    }

    #[test]
    fn give_to_other_room_is_not_here() {
        let mut app = test_app();
        let (_room, giver) = room_with_giver(&mut app);
        let elsewhere = app.world_mut().spawn_empty().id();
        spawn_pc(&mut app, elsewhere, "Bob");
        give_item(&mut app, giver, "coin", &["coin"]);
        give(&mut app, giver, "coin", "bob");

        assert!(transfers(&app).is_empty());
        assert_eq!(infos(&app).len(), 1);
    }

    #[test]
    fn give_to_self_is_refused() {
        let mut app = test_app();
        let (_room, giver) = room_with_giver(&mut app);
        give_item(&mut app, giver, "coin", &["coin"]);
        give(&mut app, giver, "coin", "self");

        assert!(transfers(&app).is_empty());
        assert_eq!(
            infos(&app),
            vec![(giver, "You can't give something to yourself.\n".to_string())]
        );
    }

    #[test]
    fn give_all_hands_over_every_match() {
        let mut app = test_app();
        let (room, giver) = room_with_giver(&mut app);
        let recipient = spawn_pc(&mut app, room, "Bob");
        let a = give_item(&mut app, giver, "coin", &["coin"]);
        let b = give_item(&mut app, giver, "coin", &["coin"]);
        give(&mut app, giver, "all coin", "bob");

        for obj in [a, b] {
            assert_eq!(
                app.world().get::<CarriedBy>(obj).unwrap().carrier,
                recipient
            );
        }
        let evs = transfers(&app);
        assert_eq!(evs.len(), 2);
        assert!(evs.iter().all(|ev| ev.kind == TransferKind::Give));
        assert!(infos(&app).is_empty());
    }

    #[test]
    fn give_quantity_caps_at_count() {
        let mut app = test_app();
        let (room, giver) = room_with_giver(&mut app);
        let recipient = spawn_pc(&mut app, room, "Bob");
        let (a, b, c) = (
            give_item(&mut app, giver, "coin", &["coin"]),
            give_item(&mut app, giver, "coin", &["coin"]),
            give_item(&mut app, giver, "coin", &["coin"]),
        );
        give(&mut app, giver, "2*coin", "bob");

        let moved: Vec<_> = [a, b, c]
            .into_iter()
            .filter(|obj| app.world().get::<CarriedBy>(*obj).unwrap().carrier == recipient)
            .collect();
        assert_eq!(moved.len(), 2);
        assert_eq!(transfers(&app).len(), 2);
        assert!(infos(&app).is_empty());
    }

    #[test]
    fn give_quoted_item_matches_every_term() {
        let mut app = test_app();
        let (room, giver) = room_with_giver(&mut app);
        let recipient = spawn_pc(&mut app, room, "Bob");
        let obj = give_item(&mut app, giver, "brass lantern", &[]);
        give(&mut app, giver, "\"brass lantern\"", "bob");

        assert_eq!(
            app.world().get::<CarriedBy>(obj).unwrap().carrier,
            recipient
        );
        assert_eq!(transfers(&app).len(), 1);
        assert!(infos(&app).is_empty());
    }

    #[test]
    fn give_offset_being_picks_second_ranked() {
        let mut app = test_app();
        let (room, giver) = room_with_giver(&mut app);
        // Exact ("Bob") outranks prefix ("Bobby"), so `2.bob` is Bobby.
        let bobby = spawn_pc(&mut app, room, "Bobby");
        spawn_pc(&mut app, room, "Bob");
        let obj = give_item(&mut app, giver, "coin", &["coin"]);
        give(&mut app, giver, "coin", "2.bob");
        assert_eq!(app.world().get::<CarriedBy>(obj).unwrap().carrier, bobby);
        let evs = transfers(&app);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].other, bobby);
        assert!(infos(&app).is_empty());
    }
}

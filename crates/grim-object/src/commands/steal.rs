//! `steal <item> <target>`: take objects from a being's pack in the room.
//!
//! The item side is a [`grim_target`] spec ([`ParseOptions::ITEM`]) — `steal
//! all coin bob` empties their coins — while the victim resolves through the
//! shared [`find_being`](super::give::find_being) helper. Existence checks
//! only — no skill checks (example workflow). Each move emits one
//! [`TransferEvent`], rendered per-recipient like `give`. Misses answer the
//! thief with an [`InfoMessage`].

use bevy::prelude::*;
use grim_actor::placement::InRoom;
use grim_core::components::Name as GrimName;
use grim_core::events::{Command, EngineCommand, InfoMessage, TransferEvent, TransferKind};
use grim_target::{parse_target, query, ParseOptions};
use grim_text::tr;

use crate::object::CarriedBy;

use super::give::{find_being, is_self};
use crate::persist::Carried;
use grim_actor::Beings;

/// `steal <item> <target>`: move the matching objects from the victim's pack
/// into the thief's.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_steal(
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
        let Command::Steal { item, target } = &cmd.command else {
            continue;
        };
        let actor = cmd.client;
        let Ok(actor_room) = inroom.get(actor) else {
            continue;
        };
        let Ok(actor_name) = names.get(actor) else {
            continue;
        };
        if is_self(target, &actor_name.0) {
            info.write(InfoMessage {
                target: actor,
                text: tr!("item.steal.self"),
            });
            continue;
        }
        // The victim first: they must be here.
        let Some(being_spec) = parse_target(target, ParseOptions::BEING) else {
            info.write(InfoMessage {
                target: actor,
                text: tr!("item.steal.not_here"),
            });
            continue;
        };
        let Some((victim, victim_name, _)) =
            find_being(&being_spec, actor_room.room, actor, &beings, &visibility)
        else {
            info.write(InfoMessage {
                target: actor,
                text: tr!("item.steal.not_here"),
            });
            continue;
        };
        // Then their pack: they must carry a match.
        let Some(item_spec) = parse_target(item, ParseOptions::ITEM) else {
            info.write(InfoMessage {
                target: actor,
                text: tr!("item.steal.not_found"),
            });
            continue;
        };
        let loot = query(
            &item_spec,
            carried
                .iter()
                .filter(|(_, _, _, _, _, _, held)| held.carrier == victim)
                .map(|(e, nm, _, kw, _, _, _)| {
                    (e, nm.0.as_str(), kw.map(|k| k.0.as_slice()).unwrap_or(&[]))
                }),
        );
        if loot.is_empty() {
            info.write(InfoMessage {
                target: actor,
                text: tr!("item.steal.not_found"),
            });
            continue;
        }
        for entity in loot {
            let Ok((_, name, _, _, _, _, _)) = carried.get(entity) else {
                continue;
            };
            let short = name.0.clone();
            commands.entity(entity).insert(CarriedBy { carrier: actor });
            transfers.write(TransferEvent {
                mover: actor,
                mover_name: actor_name.0.clone(),
                other: victim,
                other_name: victim_name.clone(),
                room: actor_room.room,
                short,
                kind: TransferKind::Steal,
            });
        }
    }
}

/// Wire the `steal` handler and the messages it reads. [`TransferEvent`] is
/// registered by the plugin (shared with `give`).
pub(crate) fn register(app: &mut App) {
    app.add_message::<EngineCommand>()
        .add_message::<InfoMessage>()
        .add_systems(Update, handle_steal);
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
        // `handle_give` is not registered here; give tests live in their own
        // module. Steal-only registration keeps these tests focused.
        app
    }

    fn room_with_thief(app: &mut App) -> (Entity, Entity) {
        let room = app.world_mut().spawn_empty().id();
        let thief = app
            .world_mut()
            .spawn((
                GrimName("Wrack".into()),
                character(),
                Player {
                    connection: Entity::PLACEHOLDER,
                },
                InRoom { room },
            ))
            .id();
        (room, thief)
    }

    fn spawn_victim(app: &mut App, room: Entity, name: &str, creature: bool) -> Entity {
        let mut entity = app.world_mut().spawn((
            GrimName(name.into()),
            InRoom { room },
            Player {
                connection: Entity::PLACEHOLDER,
            },
        ));
        if creature {
            entity.insert(Creature);
        } else {
            entity.insert(character());
        }
        entity.id()
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

    fn steal(app: &mut App, thief: Entity, item: &str, target: &str) {
        app.world_mut().write_message(EngineCommand {
            client: thief,
            command: Command::Steal {
                item: item.into(),
                target: target.into(),
            },
        });
        app.update();
    }

    #[test]
    fn steal_moves_pack_item_and_emits_event() {
        let mut app = test_app();
        let (room, thief) = room_with_thief(&mut app);
        let victim = spawn_victim(&mut app, room, "Bob", false);
        let obj = give_item(&mut app, victim, "brass lantern", &["lantern"]);
        steal(&mut app, thief, "lantern", "bob");

        assert_eq!(app.world().get::<CarriedBy>(obj).unwrap().carrier, thief);
        let evs = transfers(&app);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].kind, TransferKind::Steal);
        assert_eq!(evs[0].mover, thief);
        assert_eq!(evs[0].mover_name, "Wrack");
        assert_eq!(evs[0].other, victim);
        assert_eq!(evs[0].other_name, "Bob");
        assert_eq!(evs[0].short, "brass lantern");
        assert!(infos(&app).is_empty());
    }

    #[test]
    fn steal_from_creature_works() {
        let mut app = test_app();
        let (room, thief) = room_with_thief(&mut app);
        let mob = spawn_victim(&mut app, room, "Goblin", true);
        let obj = give_item(&mut app, mob, "coin", &["coin"]);
        steal(&mut app, thief, "coin", "goblin");

        assert_eq!(app.world().get::<CarriedBy>(obj).unwrap().carrier, thief);
        assert_eq!(transfers(&app).len(), 1);
    }

    #[test]
    fn steal_misses_victim_first() {
        let mut app = test_app();
        let (_room, thief) = room_with_thief(&mut app);
        steal(&mut app, thief, "lantern", "nobody");

        assert!(transfers(&app).is_empty());
        assert_eq!(
            infos(&app),
            vec![(thief, "They aren't here.\n".to_string())]
        );
    }

    #[test]
    fn steal_misses_unheld_item() {
        let mut app = test_app();
        let (room, thief) = room_with_thief(&mut app);
        spawn_victim(&mut app, room, "Bob", false);
        steal(&mut app, thief, "lantern", "bob");

        assert!(transfers(&app).is_empty());
        assert_eq!(
            infos(&app),
            vec![(thief, "They aren't carrying that.\n".to_string())]
        );
    }

    #[test]
    fn steal_from_self_is_refused() {
        let mut app = test_app();
        let (_room, thief) = room_with_thief(&mut app);
        steal(&mut app, thief, "lantern", "self");

        assert!(transfers(&app).is_empty());
        assert_eq!(
            infos(&app),
            vec![(thief, "You can't steal from yourself.\n".to_string())]
        );
    }

    #[test]
    fn steal_all_takes_every_match() {
        let mut app = test_app();
        let (room, thief) = room_with_thief(&mut app);
        let victim = spawn_victim(&mut app, room, "Bob", false);
        let a = give_item(&mut app, victim, "coin", &["coin"]);
        let b = give_item(&mut app, victim, "coin", &["coin"]);
        steal(&mut app, thief, "all coin", "bob");

        for obj in [a, b] {
            assert_eq!(app.world().get::<CarriedBy>(obj).unwrap().carrier, thief);
        }
        let evs = transfers(&app);
        assert_eq!(evs.len(), 2);
        assert!(evs.iter().all(|ev| ev.kind == TransferKind::Steal));
        assert!(infos(&app).is_empty());
    }
}

//! `reply <text>`: whisper the last player who whispered you
//! ([`LastWhisperFrom`](crate::whisper::LastWhisperFrom)). Fails gracefully if
//! no one has, or if they've left.

use bevy::prelude::*;
use grim_core::components::Name;
use grim_core::events::{Command, EngineCommand, InfoMessage};

use crate::whisper::{deliver_whisper, LastWhisperFrom, LivePc};

pub(crate) fn handle_reply(
    mut engine: MessageReader<EngineCommand>,
    last: Query<&LastWhisperFrom>,
    // A repliable target is a PC still in the world (online or linkdead), i.e.
    // one that still has `Character` and a `Player`/`Linkdead` marker; a
    // fully-quit character is despawned and a stale `Character`-only entity is
    // excluded.
    players: Query<(), LivePc>,
    names: Query<&Name>,
    visibility: grim_actor::visibility::Visibility,
    mut info: MessageWriter<InfoMessage>,
    mut commands: Commands,
) {
    for cmd in engine.read() {
        let Command::Reply { text } = &cmd.command else {
            continue;
        };
        let actor = cmd.client;

        let Ok(&LastWhisperFrom(target)) = last.get(actor) else {
            info.write(InfoMessage {
                target: actor,
                text: "You have no one to reply to.\n".into(),
            });
            continue;
        };
        // Linkdead is fine (they'll see it on return); only a target that has
        // fully left the world (no longer a player entity) can't be replied to.
        if players.get(target).is_err()
            || !visibility.sees_actor(actor, target)
            || !visibility.sees_actor(target, actor)
        {
            info.write(InfoMessage {
                target: actor,
                text: "They are no longer here to reply to.\n".into(),
            });
            continue;
        }

        deliver_whisper(actor, target, text, &names, &mut info, &mut commands);
    }
}

/// Wire the `reply` handler and the messages it reads/emits.
pub(crate) fn register(app: &mut App) {
    app.add_message::<EngineCommand>()
        .add_message::<InfoMessage>()
        .add_systems(Update, handle_reply);
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_actor::{Character, Linkdead, Player};
    use grim_core::GrimId;

    fn character(roles: Vec<grim_actor::Role>) -> Character {
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
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        // reply reads LastWhisperFrom that tell sets, so wire both handlers.
        crate::commands::tell::register(&mut app);
        register(&mut app);
        app
    }

    fn spawn_player(app: &mut App, name: &str) -> Entity {
        app.world_mut()
            .spawn((
                Name(name.into()),
                character(Vec::new()),
                Player {
                    connection: Entity::PLACEHOLDER,
                },
            ))
            .id()
    }

    fn infos(app: &App) -> Vec<(Entity, String)> {
        let m = app.world().resource::<Messages<InfoMessage>>();
        let mut c = m.get_cursor();
        c.read(m).map(|i| (i.target, i.text.clone())).collect()
    }

    #[test]
    fn reply_answers_last_whisperer() {
        let mut app = test_app();
        let alice = spawn_player(&mut app, "Alice");
        let bob = spawn_player(&mut app, "Bob");
        // Alice whispers Bob → sets Bob's LastWhisperFrom = Alice.
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Tell {
                target: "Bob".into(),
                text: "hi".into(),
            },
        });
        app.update(); // flush the deferred LastWhisperFrom insert
        app.world_mut().write_message(EngineCommand {
            client: bob,
            command: Command::Reply { text: "hey".into() },
        });
        app.update();
        let msgs = infos(&app);
        assert!(msgs.contains(&(bob, "You tell Alice 'hey'\n".to_string())));
        assert!(msgs.contains(&(alice, "Bob tells you 'hey'\n".to_string())));
    }

    #[test]
    fn reply_with_no_prior_whisper_is_reported() {
        let mut app = test_app();
        let alice = spawn_player(&mut app, "Alice");
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Reply { text: "hi".into() },
        });
        app.update();
        let msgs = infos(&app);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].1.contains("no one to reply to"));
    }

    #[test]
    fn reply_to_departed_player_is_reported() {
        let mut app = test_app();
        let alice = spawn_player(&mut app, "Alice");
        let bob = spawn_player(&mut app, "Bob");
        // Bob whispers Alice → Alice's LastWhisperFrom = Bob.
        app.world_mut().write_message(EngineCommand {
            client: bob,
            command: Command::Tell {
                target: "Alice".into(),
                text: "hi".into(),
            },
        });
        app.update();
        // Bob fully leaves the world (quit → despawn).
        app.world_mut().despawn(bob);
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Reply {
                text: "you there?".into(),
            },
        });
        app.update();
        let msgs = infos(&app);
        assert!(msgs
            .iter()
            .any(|(t, txt)| *t == alice && txt.contains("no longer here")));
    }

    #[test]
    fn reply_reaches_linkdead_player() {
        // A linkdead PC (Character + Linkdead, no Player) is still repliable.
        let mut app = test_app();
        let alice = spawn_player(&mut app, "Alice");
        let bob = spawn_player(&mut app, "Bob");
        // Bob whispers Alice → Alice.LastWhisperFrom = Bob.
        app.world_mut().write_message(EngineCommand {
            client: bob,
            command: Command::Tell {
                target: "Alice".into(),
                text: "hi".into(),
            },
        });
        app.update();
        // Bob goes linkdead: loses Player, gains Linkdead, still in-world.
        app.world_mut()
            .entity_mut(bob)
            .remove::<Player>()
            .insert(Linkdead);
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Reply {
                text: "still there?".into(),
            },
        });
        app.update();
        let msgs = infos(&app);
        assert!(msgs
            .iter()
            .any(|(t, txt)| *t == alice && txt.contains("You tell Bob")));
        assert!(msgs
            .iter()
            .any(|(t, txt)| *t == bob && txt.contains("Alice tells you")));
    }

    #[test]
    fn reply_to_character_only_entity_is_refused() {
        // A stale `Character`-only entity (no `Player`, no `Linkdead`) is not a
        // live PC; `LastWhisperFrom` pointing at it must not be repliable.
        let mut app = test_app();
        let alice = spawn_player(&mut app, "Alice");
        let ghost = app
            .world_mut()
            .spawn((Name("Ghost".into()), character(Vec::new())))
            .id();
        app.world_mut()
            .entity_mut(alice)
            .insert(LastWhisperFrom(ghost));
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Reply { text: "?".into() },
        });
        app.update();
        assert!(infos(&app)
            .iter()
            .any(|(t, txt)| *t == alice && txt.contains("no longer here")));
    }
}

//! `tell <target> <text>` (alias `whisper`): a private message to one player.
//! `target` matches by the shared keyword rank (case-insensitive exact name,
//! then name prefix) among connected players; `self` targets the sender.

use bevy::prelude::*;
use grim_core::components::Name;
use grim_core::events::{Command, EngineCommand, InfoMessage};
use grim_target::rank_match;

use crate::whisper::{deliver_whisper, LivePc};

pub(crate) fn handle_tell(
    mut engine: MessageReader<EngineCommand>,
    // Player characters, online or linkdead: a PC carries `Character` + `Name`
    // and is either connected (`Player`) or `Linkdead`. Requiring one of those
    // two markers excludes a stale/half-built `Character`-only entity, and mobs
    // (which carry `Creature`, not `Character`) are excluded outright.
    players: Query<(Entity, &Name), LivePc>,
    names: Query<&Name>,
    mut info: MessageWriter<InfoMessage>,
    mut commands: Commands,
) {
    for cmd in engine.read() {
        let Command::Tell { target, text } = &cmd.command else {
            continue;
        };
        let actor = cmd.client;

        let recipient = if target.eq_ignore_ascii_case("self") {
            Some(actor)
        } else {
            let want = target.to_lowercase();
            // Match any player in the world, including linkdead ones — a whisper
            // to a linkdead player is fine; they'll see it when they return.
            // Ranked (exact beats prefix, shortest name wins), ties to the
            // lowest entity id — deterministic under duplicate names.
            players
                .iter()
                .filter_map(|(e, n)| {
                    rank_match(&n.0, &[], std::slice::from_ref(&want))
                        .map(|rank| (rank, e.to_bits(), e))
                })
                .min()
                .map(|(_, _, e)| e)
        };

        let Some(recipient) = recipient else {
            info.write(InfoMessage {
                target: actor,
                text: format!("No one named '{target}' is here to tell.\n"),
            });
            continue;
        };

        deliver_whisper(actor, recipient, text, &names, &mut info, &mut commands);
    }
}

/// Wire the `tell` handler and the messages it reads/emits.
pub(crate) fn register(app: &mut App) {
    app.add_message::<EngineCommand>()
        .add_message::<InfoMessage>()
        .add_systems(Update, handle_tell);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::whisper::LastWhisperFrom;
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
        register(&mut app);
        app
    }

    /// An online PC: `Name + Character + Player`. `tell`/`reply` resolve targets
    /// by `Character` (PCs, online or linkdead), so the target carries one.
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
    fn tell_fuzzy_matches_and_messages_both_parties() {
        let mut app = test_app();
        let alice = spawn_player(&mut app, "Alice");
        let wrack = spawn_player(&mut app, "Wrack");
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Tell {
                target: "wr".into(), // case-insensitive prefix
                text: "hi".into(),
            },
        });
        app.update();
        let msgs = infos(&app);
        assert!(msgs.contains(&(alice, "You tell Wrack 'hi'\n".to_string())));
        assert!(msgs.contains(&(wrack, "Alice tells you 'hi'\n".to_string())));
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn tell_exact_name_beats_longer_prefix() {
        let mut app = test_app();
        let alice = spawn_player(&mut app, "Alice");
        spawn_player(&mut app, "Wrackus");
        let wrack = spawn_player(&mut app, "Wrack");
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Tell {
                target: "wrack".into(),
                text: "hi".into(),
            },
        });
        app.update();
        let msgs = infos(&app);
        assert!(msgs.contains(&(alice, "You tell Wrack 'hi'\n".to_string())));
        assert!(msgs.contains(&(wrack, "Alice tells you 'hi'\n".to_string())));
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn tell_self_only_echoes_to_sender() {
        let mut app = test_app();
        let alice = spawn_player(&mut app, "Alice");
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Tell {
                target: "self".into(),
                text: "note".into(),
            },
        });
        app.update();
        let msgs = infos(&app);
        assert_eq!(msgs, vec![(alice, "You tell Alice 'note'\n".to_string())]);
    }

    #[test]
    fn tell_self_allows_reply_to_self() {
        let mut app = test_app();
        // Also register reply handler since we're testing the full flow
        crate::commands::reply::register(&mut app);

        let alice = spawn_player(&mut app, "Alice");
        // Alice whispers to herself
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Tell {
                target: "self".into(),
                text: "note to self".into(),
            },
        });
        app.update();
        // Alice replies to herself
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Reply {
                text: "reply to note".into(),
            },
        });
        app.update();
        let msgs = infos(&app);
        // Both messages go to Alice
        assert!(msgs.contains(&(alice, "You tell Alice 'note to self'\n".to_string())));
        assert!(msgs.contains(&(alice, "You tell Alice 'reply to note'\n".to_string())));
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn tell_unknown_target_reports_to_sender() {
        let mut app = test_app();
        let alice = spawn_player(&mut app, "Alice");
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Tell {
                target: "nobody".into(),
                text: "hi".into(),
            },
        });
        app.update();
        let msgs = infos(&app);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].0, alice);
        assert!(msgs[0].1.contains("No one named 'nobody'"));
    }

    #[test]
    fn tell_reaches_linkdead_player() {
        // A linkdead player is still in the world and can be told; they'll see
        // it when they return.
        let mut app = test_app();
        let alice = spawn_player(&mut app, "Alice");
        // Linkdead now means NO `Player` (present-only-while-connected). A
        // linkdead PC keeps `Name + Character` and carries the `Linkdead` marker.
        let bob = app
            .world_mut()
            .spawn((Name("Bob".into()), character(Vec::new()), Linkdead))
            .id();
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Tell {
                target: "Bob".into(),
                text: "hi".into(),
            },
        });
        app.update();
        let msgs = infos(&app);
        assert!(msgs.contains(&(alice, "You tell Bob 'hi'\n".to_string())));
        assert!(msgs.contains(&(bob, "Alice tells you 'hi'\n".to_string())));
    }

    #[test]
    fn tell_and_reply_exclude_character_only_entity() {
        // A stale `Character`-only entity (no `Player`, no `Linkdead`) is not a
        // live PC and must be excluded from tell targeting.
        let mut app = test_app();
        let alice = spawn_player(&mut app, "Alice");
        let _ghost = app
            .world_mut()
            .spawn((Name("Ghost".into()), character(Vec::new())))
            .id();
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Tell {
                target: "Ghost".into(),
                text: "hi".into(),
            },
        });
        app.update();
        assert!(infos(&app)
            .iter()
            .any(|(t, txt)| *t == alice && txt.contains("No one named 'Ghost'")));
    }

    /// `LastWhisperFrom` is set on the recipient after delivery; sanity-check it
    /// here since `tell` is the write side (`reply` in `reply.rs` reads it).
    #[test]
    fn tell_sets_last_whisper_from_on_recipient() {
        let mut app = test_app();
        let alice = spawn_player(&mut app, "Alice");
        let bob = spawn_player(&mut app, "Bob");
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Command::Tell {
                target: "Bob".into(),
                text: "hi".into(),
            },
        });
        app.update();
        assert_eq!(app.world().get::<LastWhisperFrom>(bob).unwrap().0, alice);
    }
}

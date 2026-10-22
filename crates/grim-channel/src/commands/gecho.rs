//! Admin `gecho <text>`: world-wide echo. Admin gating happens at dispatch (see
//! the grim-scene dispatcher); by the time it reaches here the command is
//! authorized. Rendering — including per-recipient attribution — is
//! `format_output`'s job.

use bevy::prelude::*;
use grim_actor::Character;
use grim_core::events::{Command, EngineCommand, GlobalEcho};

pub(crate) fn handle_gecho(
    mut engine: MessageReader<EngineCommand>,
    mut echo: MessageWriter<GlobalEcho>,
    characters: Query<&Character>,
) {
    for cmd in engine.read() {
        let Command::Gecho { text } = &cmd.command else {
            continue;
        };
        // Defense in depth, mirroring `handle_goto` / `handle_shutdown_command`.
        // Dispatch already masks `gecho` as unknown for non-admins, so a
        // well-behaved session never sends this for a non-admin. If one arrives
        // from another command source, fail closed and stay silent.
        let is_admin = characters
            .get(cmd.client)
            .map(Character::is_admin)
            .unwrap_or(false);
        if !is_admin {
            continue;
        }
        echo.write(GlobalEcho {
            actor: cmd.client,
            text: text.clone(),
        });
    }
}

/// Wire the `gecho` handler and the messages it reads/emits.
pub(crate) fn register(app: &mut App) {
    app.add_message::<EngineCommand>()
        .add_message::<GlobalEcho>()
        .add_systems(Update, handle_gecho);
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_actor::Role;
    use grim_core::GrimId;

    fn character(roles: Vec<Role>) -> Character {
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

    #[test]
    fn gecho_emits_global_echo() {
        let mut app = test_app();
        let actor = app.world_mut().spawn(character(vec![Role::Admin])).id();
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Gecho {
                text: "server reboot soon".into(),
            },
        });
        app.update();
        let messages = app.world().resource::<Messages<GlobalEcho>>();
        let mut cursor = messages.get_cursor();
        let mut iter = cursor.read(messages);
        let ev = iter.next().expect("expected one GlobalEcho");
        assert_eq!(ev.actor, actor);
        assert_eq!(ev.text, "server reboot soon");
        assert!(iter.next().is_none(), "expected exactly one GlobalEcho");
    }

    #[test]
    fn gecho_from_non_admin_is_ignored() {
        // Defense in depth: even if a `Gecho` reaches the handler for a
        // non-admin (no `Character`/no admin role), no `GlobalEcho` is emitted.
        let mut app = test_app();
        let actor = app.world_mut().spawn(()).id();
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Gecho {
                text: "should not broadcast".into(),
            },
        });
        app.update();
        let messages = app.world().resource::<Messages<GlobalEcho>>();
        let mut cursor = messages.get_cursor();
        assert!(
            cursor.read(messages).next().is_none(),
            "non-admin gecho must emit no GlobalEcho"
        );
    }
}

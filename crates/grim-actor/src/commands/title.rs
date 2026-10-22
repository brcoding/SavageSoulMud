//! The `title` command: set or clear the actor's WHO title, stored on their
//! [`Character`] (persisted via the existing save-on-disconnect).

use bevy::prelude::*;
use grim_core::events::{Command, EngineCommand, InfoMessage};

use crate::character::Character;

/// Maximum title length, in characters.
const MAX_TITLE_LEN: usize = 60;

/// `title <text>` sets the actor's title (rejected over [`MAX_TITLE_LEN`]
/// chars); a bare `title` (empty text) clears it. Mutates `Character.title` and
/// confirms to the actor. A non-character actor is silently ignored.
pub(crate) fn handle_title(
    mut engine: MessageReader<EngineCommand>,
    mut characters: Query<&mut Character>,
    mut info: MessageWriter<InfoMessage>,
) {
    for cmd in engine.read() {
        let Command::Title { text } = &cmd.command else {
            continue;
        };
        let actor = cmd.client;
        let Ok(mut character) = characters.get_mut(actor) else {
            continue;
        };
        let trimmed = text.trim();
        let reply = if trimmed.is_empty() {
            character.title = None;
            "Your title has been cleared.\n".to_string()
        } else if trimmed.chars().count() > MAX_TITLE_LEN {
            // Reject without mutating: the old title (if any) stands.
            format!("A title may be at most {MAX_TITLE_LEN} characters.\n")
        } else {
            character.title = Some(trimmed.to_string());
            // Escape the echo so a title can't inject colour into the reply.
            format!("Your title is now: {}\n", grim_color::escape_codes(trimmed))
        };
        info.write(InfoMessage {
            target: actor,
            text: reply,
        });
    }
}

/// Wire the `title` handler and the messages it reads/emits.
pub(crate) fn register(app: &mut App) {
    app.add_message::<EngineCommand>()
        .add_message::<InfoMessage>()
        .add_systems(Update, handle_title);
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_core::GrimId;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        register(&mut app);
        app
    }

    fn spawn_char(app: &mut App) -> Entity {
        app.world_mut()
            .spawn(Character {
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
            })
            .id()
    }

    fn send_title(app: &mut App, actor: Entity, text: &str) {
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Title { text: text.into() },
        });
        app.update();
    }

    fn info_texts(app: &App) -> Vec<String> {
        let m = app.world().resource::<Messages<InfoMessage>>();
        let mut c = m.get_cursor();
        c.read(m).map(|i| i.text.clone()).collect()
    }

    #[test]
    fn title_sets_and_trims() {
        let mut app = test_app();
        let hero = spawn_char(&mut app);
        send_title(&mut app, hero, "  the Bold  ");
        assert_eq!(
            app.world().get::<Character>(hero).unwrap().title.as_deref(),
            Some("the Bold")
        );
        assert!(info_texts(&app).iter().any(|t| t.contains("the Bold")));
    }

    #[test]
    fn title_bare_clears() {
        let mut app = test_app();
        let hero = spawn_char(&mut app);
        send_title(&mut app, hero, "the Bold");
        send_title(&mut app, hero, "");
        assert!(app.world().get::<Character>(hero).unwrap().title.is_none());
        assert!(info_texts(&app).iter().any(|t| t.contains("cleared")));
    }

    #[test]
    fn title_over_sixty_chars_is_rejected() {
        let mut app = test_app();
        let hero = spawn_char(&mut app);
        let long = "x".repeat(61);
        send_title(&mut app, hero, &long);
        assert!(
            app.world().get::<Character>(hero).unwrap().title.is_none(),
            "over-length title must not be set"
        );
        assert!(info_texts(&app)
            .iter()
            .any(|t| t.contains("at most 60 characters")));
    }

    #[test]
    fn title_exactly_sixty_chars_is_accepted() {
        let mut app = test_app();
        let hero = spawn_char(&mut app);
        let sixty = "y".repeat(60);
        send_title(&mut app, hero, &sixty);
        assert_eq!(
            app.world().get::<Character>(hero).unwrap().title.as_deref(),
            Some(sixty.as_str())
        );
    }

    #[test]
    fn title_reject_keeps_previous_title() {
        let mut app = test_app();
        let hero = spawn_char(&mut app);
        send_title(&mut app, hero, "the Kept");
        send_title(&mut app, hero, &"z".repeat(61));
        assert_eq!(
            app.world().get::<Character>(hero).unwrap().title.as_deref(),
            Some("the Kept"),
            "a rejected over-length title leaves the old one intact"
        );
    }

    #[test]
    fn title_for_non_character_is_ignored() {
        let mut app = test_app();
        let ghost = app.world_mut().spawn(()).id();
        send_title(&mut app, ghost, "spooky");
        assert!(info_texts(&app).is_empty());
    }
}

//! Optional one-time preference question at the authenticated world-entry seam.

use bevy::prelude::*;
use grim_actor::{Actor, Character, InRoom, OutputHistory, Player, StoredCharacter};
use grim_core::components::{Client, ClientState, Name as GrimName};
use grim_core::events::{LoginAnnounce, LookRoom};
use grim_networking::ConnectionOutput;
use grim_persistence::PersistenceConfig;
use grim_scene::JustEnteredWorld;

/// Enable a one-time screen-reader question for each character. Disabled by
/// default so games opt into this onboarding step explicitly.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct AccessibilityOnboarding(pub bool);

const QUESTION: &str = "Do you use a screen reader? [y/N] ";

/// True means entry must pause (including an unresolved character: fail closed).
pub(crate) fn ask_if_needed(
    client: &mut Client,
    characters: &Query<(Entity, &Character, &Actor, &GrimName)>,
    outputs: &mut MessageWriter<ConnectionOutput>,
) -> bool {
    let Some(entity) = client.character else {
        return true;
    };
    let Ok((_, character, _, _)) = characters.get(entity) else {
        return true;
    };
    if matches!(
        character.config.get("screenreader").map(String::as_str),
        Some("on" | "off")
    ) {
        return false;
    }
    client.state = ClientState::AccessibilityPrompt;
    outputs.write(ConnectionOutput::new(client.connection, QUESTION));
    true
}

fn choice(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" | "on" | "screenreader on" => Some(true),
        "" | "n" | "no" | "off" | "screenreader off" => Some(false),
        _ => None,
    }
}

pub(crate) fn answer(
    client_entity: Entity,
    client: &Client,
    text: &str,
    commands: &mut Commands,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    let Some(enabled) = choice(text) else {
        outputs.write(ConnectionOutput::new(
            client.connection,
            format!("Please answer yes or no.\n{QUESTION}"),
        ));
        return;
    };
    // Snapshot at deferred-application time, after any pending inventory or
    // character changes. Never insert an old Character clone over newer state.
    commands.queue(move |world: &mut World| finish(world, client_entity, enabled));
}

fn finish(world: &mut World, client_entity: Entity, enabled: bool) {
    let Some(client) = world.get::<Client>(client_entity) else {
        return;
    };
    if client.state != ClientState::AccessibilityPrompt {
        return;
    }
    let (Some(entity), conn) = (client.character, client.connection) else {
        return;
    };
    // A stale/taken-over connection must never persist somebody else's settings.
    if !world
        .get::<Player>(entity)
        .is_some_and(|p| p.connection == conn)
    {
        return;
    }
    let Some(name) = world.get::<GrimName>(entity).cloned() else {
        return;
    };
    let Some(actor) = world.get::<Actor>(entity).cloned() else {
        return;
    };
    let Some(mut character) = world.get::<Character>(entity).cloned() else {
        return;
    };
    character.config.insert(
        "screenreader".into(),
        if enabled { "on" } else { "off" }.into(),
    );
    if enabled {
        for (key, value) in [
            ("minimap", "off"),
            ("colors", "off"),
            ("prompt", "accessible"),
            ("combatlog", "brief"),
        ] {
            character.config.insert(key.into(), value.into());
        }
    }
    let mut stored = StoredCharacter::from_components(&name, &actor, &character);
    {
        let mut pack =
            world.query_filtered::<grim_object::persist::Held, With<grim_object::Object>>();
        stored.inventory = grim_object::persist::snapshot(pack.iter(world), entity);
    }
    let Some(config) = world.get_resource::<PersistenceConfig>() else {
        return;
    };
    if let Err(error) = persist_choice(config, &stored) {
        error!("Could not save accessibility preferences: {error}");
        world.write_message(ConnectionOutput::new(
            conn,
            "Could not save your preference. Please answer again.\n",
        ));
        return;
    }
    world.entity_mut(entity).insert(character);
    if let Some(mut client) = world.get_mut::<Client>(client_entity) {
        client.state = ClientState::InGame;
        client.input_queue.clear();
        client.command_cooldown = Timer::from_seconds(0.5, TimerMode::Once);
    }
    world.entity_mut(conn).insert(OutputHistory::with_max(100));
    world.resource_mut::<JustEnteredWorld>().0.insert(conn);
    grim_scene::push_ingame_scene(&mut world.commands(), client_entity);
    world.write_message(ConnectionOutput::new(
        conn,
        if enabled {
            "Screen reader mode on. Use screenreader off to change it.\n"
        } else {
            "Screen reader mode off. Use screenreader on to change it.\n"
        },
    ));
    world.write_message(LoginAnnounce {
        subject: entity,
        name: name.0,
    });
    if let Some(room) = world.get::<InRoom>(entity).map(|r| r.room) {
        world.write_message(LookRoom {
            target: entity,
            room,
        });
    }
}

fn persist_choice(config: &PersistenceConfig, stored: &StoredCharacter) -> std::io::Result<()> {
    use std::io::Write;
    // Authenticated character names are letters-only; retain the guard here
    // because disk state can also have been edited outside the engine.
    if stored.name.is_empty() || !stored.name.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid character name",
        ));
    }
    std::fs::create_dir_all(config.characters_dir())?;
    let path = config
        .characters_dir()
        .join(format!("{}.json", stored.name));
    let temporary = path.with_extension("accessibility.tmp");
    let bytes = serde_json::to_vec_pretty(stored)?;
    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::choice;

    #[test]
    fn accepts_plain_answers_and_explicit_commands() {
        assert_eq!(choice(" YES "), Some(true));
        assert_eq!(choice("screenreader on"), Some(true));
        assert_eq!(choice(""), Some(false));
        assert_eq!(choice("screenreader off"), Some(false));
        assert_eq!(choice("look"), None);
    }
}

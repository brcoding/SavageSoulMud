//! answers implementation, split from the parent subsystem.

use super::*;

pub(super) fn answer_character_equipment(
    conn: Entity,
    char_entity: Entity,
    characters: &Query<(Entity, &Character, &Actor, &GrimName)>,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    if let Ok((_, character, _, _)) = characters.get(char_entity) {
        if let Some(display) = character
            .game_state
            .get("equipment_display")
            .and_then(|v| v.as_str())
        {
            outputs.write(ConnectionOutput::new(conn, display));
        } else {
            answer_equipment(conn, outputs);
        }
    }
}

pub(super) fn answer_session_sockets(
    conn: Entity,
    char_entity: Entity,
    characters: &Query<(Entity, &Character, &Actor, &GrimName)>,
    snapshot: &[ClientSnapshot],
    connections: &Query<&Connection>,
    accounts: &Query<&Account>,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    // Admin-gated + masked like shutdown/goto/gecho, but answered
    // session-locally (no engine round-trip): a non-admin gets the
    // exact unknown-command reply so the verb is not leaked.
    let is_admin = character_is_admin(characters, char_entity);
    if is_admin {
        outputs.write(ConnectionOutput {
            echo: None,
            ..ConnectionOutput::new(
                conn,
                format_sockets(snapshot, connections, characters, accounts),
            )
        });
    } else {
        outputs.write(ConnectionOutput {
            echo: None,
            ..ConnectionOutput::new(conn, tr!("error.unknown_command"))
        });
    }
}

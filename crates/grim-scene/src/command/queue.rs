//! queue implementation, split from the parent subsystem.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn process_command_queue(
    time: Res<Time>,
    mut clients: Query<(Entity, &mut Client)>,
    mut engine_commands: MessageWriter<EngineCommand>,
    mut announce_logout: MessageWriter<LogoutAnnounce>,
    mut disconnect: MessageWriter<DisconnectRequest>,
    player_chars: Query<(Entity, &GrimName, &InRoom)>,
    characters: Query<(&GrimName, &Actor, &Character)>,
    persistence: Res<grim_persistence::PersistenceConfig>,
    mut commands: Commands,
    pack: grim_object::persist::Carried,
) {
    for (entity, mut client) in clients.iter_mut() {
        let conn = client.connection;
        if client.state != ClientState::InGame {
            continue;
        }
        client.command_cooldown.tick(time.delta());
        if !client.command_cooldown.is_finished() {
            continue;
        }
        if let Some(cmd) = client.input_queue.pop_front() {
            if matches!(&cmd, Command::Quit) {
                let char_name = client
                    .character
                    .and_then(|c| player_chars.get(c).ok())
                    .map(|(_, n, _)| n.0.clone())
                    .unwrap_or_else(|| "Someone".into());
                let char_room = client
                    .character
                    .and_then(|c| player_chars.get(c).ok())
                    .map(|(_, _, ir)| ir.room);
                // `quit` is an intentional logout: save the character to disk,
                // then DESPAWN its entity entirely — a logged-out character lives
                // only on disk and is re-loaded on next login. This is NOT
                // linkdead: linkdead is only for an *unexpected* socket drop (see
                // `save_on_disconnect`). The Client is despawned too, so the
                // ensuing `ConnectionClosed` finds no client and no linkdead is set.
                if let Some(char_entity) = client.character {
                    if let Ok((name, actor, ch)) = characters.get(char_entity) {
                        let mut stored = StoredCharacter::from_components(name, actor, ch);
                        stored.inventory = grim_object::persist::snapshot_pack(&pack, char_entity);
                        let path = persistence
                            .characters_dir()
                            .join(format!("{}.json", name.0));
                        let _ = std::fs::create_dir_all(persistence.characters_dir());
                        if let Ok(json) = serde_json::to_string_pretty(&stored) {
                            let _ = std::fs::write(path, json);
                        }
                    }
                    // The pack went to disk with the character; despawn the live
                    // objects too, or their `CarriedBy` dangles at this entity.
                    // They re-spawn from the snapshot on next login.
                    let held: Vec<Entity> = pack
                        .iter()
                        .filter(|(_, _, _, _, _, _, held)| held.carrier == char_entity)
                        .map(|(e, _, _, _, _, _, _)| e)
                        .collect();
                    grim_object::persist::unload(&mut commands, held.into_iter());
                    commands.entity(char_entity).despawn();
                }
                commands.entity(entity).despawn();
                // The entity is gone after this: the room rides on the message.
                let hidden = client
                    .character
                    .and_then(|e| characters.get(e).ok())
                    .is_some_and(|(_, _, c)| c.is_hidden());
                if let Some(room) = char_room.filter(|_| !hidden) {
                    announce_logout.write(LogoutAnnounce {
                        name: char_name.clone(),
                        room,
                    });
                }
                info!("Character '{}' quit", char_name);
                disconnect.write(DisconnectRequest { connection: conn });
                continue;
            }
            engine_commands.write(EngineCommand {
                client: client.character.unwrap_or(entity),
                command: cmd,
            });
            // Start cooldown for next command
            client.command_cooldown.reset();
        }
    }
}

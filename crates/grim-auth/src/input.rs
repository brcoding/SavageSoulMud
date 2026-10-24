//! The pre-game input dispatcher: routes each input line to the handler for the
//! session's current pre-game [`ClientState`]. This is one half of the routing
//! split — the in-game half lives in `grim_scene`. Any client already `InGame`
//! is skipped here (scene owns it); every pre-game state is handled here.
//!
//! ## Intra-tick transition guard
//! A single line can advance a client from a pre-game state to `InGame` (the
//! MOTD ENTER, or a login-by-name that reconnects a linkdead character). That
//! same [`ConnectionInput`] must NOT then be re-processed as an in-game command
//! by the scene in-game system in the same tick. This system runs BEFORE the
//! scene in-game system (ordered via [`grim_scene::SceneSystems::InGameInput`])
//! and records every connection it advances to `InGame` in
//! [`grim_scene::JustEnteredWorld`]; the scene system skips the triggering line
//! for those connections. The set is cleared at the top of this system each
//! tick, so it only ever holds this tick's transitions.

use bevy::prelude::*;
use grim_core::components::{Account, Client, ClientState};
use grim_core::events::{LoginAnnounce, LookRoom};
use grim_networking::{Connection, ConnectionInput, ConnectionOutput};
use grim_scene::JustEnteredWorld;

use crate::character_select as character;
use crate::creation;
use crate::login;
use crate::params::{PlayerChars, SessionRes, WorldEntry};

// A flat dispatch: one match arm per pre-game ClientState, each delegating to
// its handler. Long by nature (a state table) — waived, not split.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn handle_pregame_input(
    mut inputs: MessageReader<ConnectionInput>,
    mut credentials: MessageReader<crate::LoginCredentials>,
    mut results: MessageWriter<crate::LoginResult>,
    mut clients: Query<(Entity, &mut Client)>,
    mut accounts: Query<(Entity, &mut Account)>,
    connections: Query<&Connection>,
    player_chars: PlayerChars,
    res: SessionRes,
    mut just_entered: ResMut<JustEnteredWorld>,
    mut commands: Commands,
    mut outputs: MessageWriter<ConnectionOutput>,
    mut look_room: MessageWriter<LookRoom>,
    mut announce_login: MessageWriter<LoginAnnounce>,
    mut world: WorldEntry,
) {
    // Only ever hold this tick's pre-game → InGame transitions (see module doc).
    just_entered.0.clear();
    let lines: Vec<_> = inputs
        .read()
        .map(|ev| ConnectionInput {
            connection: ev.connection,
            text: ev.text.clone(),
        })
        .collect();
    let manual: std::collections::HashSet<_> = lines.iter().map(|ev| ev.connection).collect();
    let requests: Vec<_> = credentials
        .read()
        .map(|c| {
            (
                ConnectionInput {
                    connection: c.connection,
                    text: String::new(),
                },
                Some((c.account.clone(), c.password.clone())),
            )
        })
        .chain(lines.into_iter().map(|ev| (ev, None)))
        .collect();
    for (ev, credential) in requests {
        let Some((client_entity, mut client)) = clients
            .iter_mut()
            .find(|(_, c)| c.connection == ev.connection)
        else {
            continue;
        };
        let text = ev.text.as_str();
        let conn = client.connection;
        let peer = connections
            .get(conn)
            .map(|c| c.addr.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        // Match on a clone of the state so each handler can freely mutate the
        // borrowed `Client` (including its `state`) without a borrow conflict.
        let structured = credential.is_some();
        if let Some((account, password)) = credential {
            // Manual input wins a mixed batch; never reinterpret a queued typed
            // password as gameplay after structured authentication advances state.
            if manual.contains(&conn) {
                results.write(crate::LoginResult {
                    connection: conn,
                    success: false,
                    message: "Continue interactive sign-in.".into(),
                });
                continue;
            }
            if client.state != ClientState::LoginPrompt {
                results.write(crate::LoginResult {
                    connection: conn,
                    success: false,
                    message: "Continue the current login or character selection.".into(),
                });
                continue;
            }
            let valid = !account.is_empty()
                && account.len() <= 320
                && password.len() <= 1024
                && !password.is_empty()
                && !account.chars().any(char::is_control)
                && !password.chars().any(char::is_control);
            let resolved = if valid {
                crate::client_login::resolve(
                    account.trim(),
                    &accounts,
                    &world.characters,
                    &res.persistence,
                )
            } else {
                None
            };
            if let Some((identifier, selected)) = resolved {
                crate::password::authenticate(
                    &mut client,
                    client_entity,
                    conn,
                    &peer,
                    &password,
                    &identifier,
                    selected,
                    &accounts,
                    &world.characters,
                    &world.players,
                    &world.linkdead,
                    &mut world.histories,
                    &world.rooms,
                    &res,
                    &mut commands,
                    &mut outputs,
                    &mut world.announce_linkdead,
                    &mut world.disconnect,
                    &mut world.alerts,
                );
            }
            let success = client.account.is_some()
                && matches!(
                    client.state,
                    ClientState::CharacterSelect
                        | ClientState::MotdPrompt
                        | ClientState::InGame
                        | ClientState::SelectGender { .. }
                );
            results.write(crate::LoginResult {
                connection: conn,
                success,
                message: if success {
                    String::new()
                } else {
                    "Invalid credentials or access denied.".into()
                },
            });
            outputs.write(ConnectionOutput {
                echo: Some(true),
                ..ConnectionOutput::new(conn, "")
            });
            if !success {
                client.state = ClientState::Disconnecting;
                world
                    .disconnect
                    .write(grim_networking::DisconnectRequest { connection: conn });
            }
        } else {
            match client.state.clone() {
                // In-game input is the scene system's job; disconnects are terminal.
                ClientState::InGame | ClientState::Disconnecting => continue,
                ClientState::LoginPrompt => login::login_prompt(
                    &mut client,
                    conn,
                    text,
                    &accounts,
                    &world.characters,
                    &world.linkdead,
                    &res.persistence,
                    &mut outputs,
                ),
                ClientState::ConfirmCreate { identifier } => {
                    login::confirm_create(&mut client, conn, text, identifier, &mut outputs);
                }
                ClientState::NewAccountPrompt => {
                    login::new_account_prompt(&mut client, conn, text, &accounts, &mut outputs);
                }
                ClientState::PasswordPrompt {
                    identifier,
                    is_new,
                    character,
                } => login::password_prompt(
                    login::PasswordPromptArgs {
                        identifier,
                        is_new,
                        character,
                    },
                    client_entity,
                    &mut client,
                    conn,
                    peer.as_str(),
                    text,
                    &accounts,
                    &world.characters,
                    &world.players,
                    &world.linkdead,
                    &mut world.histories,
                    &world.rooms,
                    &res,
                    &mut commands,
                    &mut outputs,
                    &mut world.announce_linkdead,
                    &mut world.disconnect,
                    &mut world.alerts,
                ),
                ClientState::CharacterSelect => character::character_select(
                    client_entity,
                    &mut client,
                    conn,
                    text,
                    &accounts,
                    &world.characters,
                    &world.players,
                    &world.linkdead,
                    &mut world.histories,
                    &world.rooms,
                    &res,
                    &mut commands,
                    &mut outputs,
                    &mut world.announce_linkdead,
                    &mut world.disconnect,
                    &mut world.alerts,
                ),
                ClientState::CreateCharacter => {
                    creation::create_character(&mut client, conn, text, &res, &mut outputs);
                }
                ClientState::SelectGender { name } => {
                    creation::select_gender(&mut client, conn, text, name, &res, &mut outputs);
                }
                ClientState::SelectRace { name, gender } => {
                    creation::select_race(
                        &mut client,
                        conn,
                        text,
                        name,
                        gender,
                        &res,
                        &mut outputs,
                        &mut accounts,
                        &mut commands,
                        &mut world,
                    );
                }
                ClientState::SelectClass { name, gender, race } => creation::select_class(
                    &mut client,
                    conn,
                    text,
                    name,
                    gender,
                    race,
                    &mut accounts,
                    &res,
                    &mut commands,
                    &mut outputs,
                    &mut world,
                ),
                ClientState::AccessibilityPrompt => crate::accessibility::answer(
                    client_entity,
                    &client,
                    text,
                    &mut commands,
                    &mut outputs,
                ),
                ClientState::MotdPrompt => {
                    if res.accessibility.as_ref().is_some_and(|r| r.0)
                        && crate::accessibility::ask_if_needed(
                            &mut client,
                            &world.characters,
                            &mut outputs,
                        )
                    {
                        continue;
                    }
                    character::motd_prompt(
                        &mut client,
                        &world.characters,
                        &player_chars,
                        &mut commands,
                        &mut announce_login,
                        &mut look_room,
                    );
                }
            }
        }

        // If this line advanced the session into the world, push its in-game
        // scene (once — an already-InGame session `continue`s above before
        // reaching here) and record its connection so the scene in-game system
        // skips the same line this tick.
        if client.state == ClientState::InGame {
            // Direct linkdead reconnects skip the MOTD, but still get the
            // question once. Copyover resumes never pass through this system.
            if res.accessibility.as_ref().is_some_and(|r| r.0)
                && crate::accessibility::ask_if_needed(&mut client, &world.characters, &mut outputs)
            {
                continue;
            }
            grim_scene::push_ingame_scene(&mut commands, client_entity);
            if !structured {
                just_entered.0.insert(ev.connection);
            }
        }
    }
}

//! Password prompt + authentication: empty input aborts to the login prompt,
//! `is_new` creates the account, otherwise one password attempt. Split out of
//! `login.rs` under the module line cap.

use bevy::prelude::*;
use grim_actor::{Actor, Character, Linkdead, OutputHistory, Player};
use grim_core::components::{Account, Client, ClientState, Name as GrimName};
use grim_core::events::LinkdeadAnnounce;
use grim_networking::{
    admin_log, ConnectionOutput, DisconnectRequest, WiznetAlert, WiznetCategory,
};
use grim_persistence::load_character_by_name;
use grim_text::tr;

use crate::account;
use crate::character_select as character;
use crate::creation;
use crate::params::{RoomResolver, SessionRes};
use crate::validation::verify_password;
use crate::world_entry;

/// The destructured `PasswordPrompt` state, passed as one argument so the
/// dispatcher stays under clippy's argument limit at the call site.
pub(crate) struct PasswordPromptArgs {
    pub(crate) identifier: String,
    pub(crate) is_new: bool,
    pub(crate) character: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn password_prompt(
    args: PasswordPromptArgs,
    client_entity: Entity,
    client: &mut Client,
    conn: Entity,
    peer: &str,
    text: &str,
    accounts: &Query<(Entity, &mut Account)>,
    characters: &Query<(Entity, &Character, &Actor, &GrimName)>,
    players: &Query<&Player>,
    linkdead: &Query<&Linkdead>,
    histories: &mut Query<&mut OutputHistory>,
    rooms: &RoomResolver,
    res: &SessionRes,
    commands: &mut Commands,
    outputs: &mut MessageWriter<ConnectionOutput>,
    announce_linkdead: &mut MessageWriter<LinkdeadAnnounce>,
    disconnect: &mut MessageWriter<DisconnectRequest>,
    alerts: &mut MessageWriter<WiznetAlert>,
) {
    let PasswordPromptArgs {
        identifier,
        is_new,
        character,
    } = args;
    let auto_select = character;
    if text.trim().is_empty() {
        client.state = ClientState::LoginPrompt;
        outputs.write(ConnectionOutput {
            echo: Some(true),
            ..ConnectionOutput::new(conn, tr!("login.prompt"))
        });
        return;
    }
    if is_new {
        account::create_account(
            client,
            client_entity,
            conn,
            text,
            &identifier,
            &res.persistence,
            characters,
            accounts,
            players,
            linkdead,
            commands,
            outputs,
        );
    } else {
        authenticate(
            client,
            client_entity,
            conn,
            peer,
            text,
            &identifier,
            auto_select,
            accounts,
            characters,
            players,
            linkdead,
            histories,
            rooms,
            res,
            commands,
            outputs,
            announce_linkdead,
            disconnect,
            alerts,
        );
    }
}

/// Verify the password for an existing account, then enter the world
/// directly (login-by-name auto-select) or show the character menu. One
/// attempt: a wrong password disconnects and emits a security wiznet.
#[allow(clippy::too_many_arguments)]
pub(crate) fn authenticate(
    client: &mut Client,
    client_entity: Entity,
    conn: Entity,
    peer: &str,
    text: &str,
    identifier: &str,
    auto_select: Option<String>,
    accounts: &Query<(Entity, &mut Account)>,
    characters: &Query<(Entity, &Character, &Actor, &GrimName)>,
    players: &Query<&Player>,
    linkdead: &Query<&Linkdead>,
    histories: &mut Query<&mut OutputHistory>,
    rooms: &RoomResolver,
    res: &SessionRes,
    commands: &mut Commands,
    outputs: &mut MessageWriter<ConnectionOutput>,
    announce_linkdead: &mut MessageWriter<LinkdeadAnnounce>,
    disconnect: &mut MessageWriter<DisconnectRequest>,
    alerts: &mut MessageWriter<WiznetAlert>,
) {
    let account_found = accounts
        .iter()
        .find(|(_, a)| a.identifier == *identifier)
        .map(|(e, a)| (e, a.id));
    match account_found {
        Some((account_entity, account_id)) => {
            let ok = accounts
                .get(account_entity)
                .map(|(_, a)| verify_password(text.trim(), &a.password_hash))
                .unwrap_or(false);
            if ok {
                // Banned accounts are refused before any game state loads.
                if world_entry::refuse_banned_account(
                    accounts, identifier, &res.bans, conn, outputs, disconnect, alerts,
                ) {
                    return;
                }
                client.account = Some(account_entity);
                if let Some(name) = auto_select {
                    // A legacy character (no race/class yet) is routed through the
                    // creation picker once before entering — same as the
                    // menu-selection path (character_select). Otherwise: straight
                    // into the world (reconnect / takeover / spawn).
                    let legacy = load_character_by_name(&res.persistence, &name)
                        .map(|c| {
                            c.account_id == account_id && c.race.is_empty() && c.class.is_empty()
                        })
                        .unwrap_or(false);
                    if legacy {
                        creation::start_gender_pick(client, conn, name, outputs);
                    } else {
                        world_entry::enter_world_by_name(
                            conn,
                            client,
                            account_id,
                            &name,
                            commands,
                            characters,
                            players,
                            linkdead,
                            histories,
                            rooms,
                            res.starting.0,
                            &res.persistence,
                            &res.bans,
                            outputs,
                            announce_linkdead,
                            disconnect,
                            alerts,
                        );
                    }
                } else {
                    client.state = ClientState::CharacterSelect;
                    character::show_character_menu(
                        client_entity,
                        client,
                        characters,
                        accounts,
                        outputs,
                        linkdead,
                        players,
                        &res.persistence,
                    );
                }
            } else {
                // One attempt: enter the terminal state FIRST so a pipelined
                // burst in the same tick cannot test further passwords, then
                // report, alert the wizards, and sever the socket. The
                // transport restores echo on its own when the connection
                // closes.
                client.state = ClientState::Disconnecting;
                warn!("Authentication rejected: incorrect password from {peer}");
                outputs.write(ConnectionOutput {
                    echo: Some(true),
                    ..ConnectionOutput::new(conn, tr!("login.bad_password"))
                });
                if let Some(name) = auto_select.as_deref() {
                    admin_log!(
                        alerts,
                        WiznetCategory::Security,
                        "Invalid password for character {name} from {peer}"
                    );
                } else {
                    admin_log!(
                        alerts,
                        WiznetCategory::Security,
                        "Invalid password for account {identifier} from {peer}"
                    );
                }
                disconnect.write(DisconnectRequest { connection: conn });
            }
        }
        None => {
            client.state = ClientState::LoginPrompt;
            outputs.write(ConnectionOutput {
                echo: Some(true),
                ..ConnectionOutput::new(conn, tr!("login.prompt"))
            });
        }
    }
}

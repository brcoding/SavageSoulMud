//! In-game command handling: parse a line for a session in `InGame`, answer the
//! session-local commands (who/where/sockets/commands/areas), gate admin-only
//! and drain the per-client queue into engine commands under a cooldown.

use bevy::prelude::*;
use grim_actor::{Actor, Character, InRoom, Linkdead, StoredCharacter};
use grim_core::components::{Account, Client, ClientState, Description, Name as GrimName};
use grim_core::events::{Command, DescOp, EngineCommand, LogoutAnnounce};
use grim_networking::{Connection, ConnectionOutput, DisconnectRequest};
use grim_persistence::PersistenceConfig;
use grim_text::tr;

use crate::finger;
use crate::formatter;
use crate::params::{PlayerChars, RoomResolver, SessionRes};
use crate::parser;
use crate::sockets::{format_sockets, ClientSnapshot};
use crate::who::{afk_chars, format_areas, format_where, format_who};
use crate::wizlist::format_wizlist;
mod queue;
pub(crate) use queue::process_command_queue;
mod answers;
use answers::{answer_character_equipment, answer_session_sockets};

/// `equipment` stays a dummy (no worn-items system yet). Factored out of
/// [`handle_ingame`] to hold that dispatch table under the line budget.
fn answer_equipment(conn: Entity, outputs: &mut MessageWriter<ConnectionOutput>) {
    outputs.write(ConnectionOutput {
        echo: None,
        ..ConnectionOutput::new(conn, tr!("equipment.empty"))
    });
}

/// Answer the `wizlist` from live characters plus the startup admin snapshot.
/// Factored out of [`handle_ingame`] to hold that dispatch table under the
/// line budget (same reason as [`answer_equipment`]).
fn answer_wizlist(
    conn: Entity,
    player_chars: &PlayerChars,
    linkdead: &Query<&Linkdead>,
    snapshot: &[ClientSnapshot],
    res: &SessionRes,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    outputs.write(ConnectionOutput {
        echo: None,
        ..ConnectionOutput::new(
            conn,
            format_wizlist(
                snapshot
                    .iter()
                    .find(|s| s.connection == conn)
                    .and_then(|s| s.character),
                player_chars,
                linkdead,
                &afk_chars(snapshot),
                res,
                &res.wizlist,
            ),
        )
    });
}

/// Whether the actor behind `char_entity` holds the admin role. A missing
/// actor reads as non-admin (fail closed). Shared by the session-local
/// (`sockets`) and engine-queued (`shutdown`/`goto`/`gecho`/`ban`) gates so
/// every admin check stays one lookup.
fn character_is_admin(
    characters: &Query<(Entity, &Character, &Actor, &GrimName)>,
    char_entity: Entity,
) -> bool {
    characters
        .get(char_entity)
        .map(|(_, c, _, _)| c.is_admin())
        .unwrap_or(false)
}

/// Registry names masked as unknown for non-admins (see `dispatch_admin_gated`
/// and the session-local `sockets` gate). The `commands` grid hides these
/// from non-admins so the list only shows what the actor can use.
const ADMIN_GATED_NAMES: &[&str] = &[
    "wizinvis",
    "testaccount",
    "shutdown",
    "goto",
    "gecho",
    "ban",
    "sockets",
    "wiznet",
    "intermud",
    "i3",
    "imud",
    "zebedee",
    "i2",
    "grapevine",
    "gv",
];

/// Answer `commands`: the registry grid minus the `socials` section (listed
/// by its own lister) and minus admin verbs for non-admins. What remains is
/// exactly what this actor can type. Factored out of [`handle_ingame`] to
/// hold that dispatch table under the line budget.
fn answer_commands(
    conn: Entity,
    char_entity: Entity,
    characters: &Query<(Entity, &Character, &Actor, &GrimName)>,
    res: &SessionRes,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    let socials: std::collections::HashSet<String> = res
        .registry
        .names_in_section("social")
        .into_iter()
        .collect();
    let is_admin = character_is_admin(characters, char_entity);
    let names: Vec<String> = res
        .registry
        .names()
        .into_iter()
        .filter(|n| !socials.contains(n) && (is_admin || !ADMIN_GATED_NAMES.contains(&n.as_str())))
        .collect();
    outputs.write(ConnectionOutput {
        echo: None,
        ..ConnectionOutput::new(conn, formatter::format_commands(&names))
    });
}

/// Answer `who` from the online (non-linkdead) player list plus the snapshot-derived AFK set.
/// Factored out of [`handle_ingame`] to hold that dispatcher under the line
/// budget (same reason as [`answer_equipment`]).
fn answer_who(
    conn: Entity,
    player_chars: &PlayerChars,
    linkdead: &Query<&Linkdead>,
    snapshot: &[ClientSnapshot],
    res: &SessionRes,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    outputs.write(ConnectionOutput {
        echo: None,
        ..ConnectionOutput::new(
            conn,
            format_who(
                player_chars,
                linkdead,
                &afk_chars(snapshot),
                res,
                snapshot
                    .iter()
                    .find(|s| s.connection == conn)
                    .and_then(|s| s.character),
            ),
        )
    });
}

/// Flag the session AFK with the `afk.set` notice. Any later input line clears
/// it via the touch system. Factored out of [`handle_ingame`] for the line
/// budget (same reason as [`answer_equipment`]).
fn flag_afk(client: &mut Client, conn: Entity, outputs: &mut MessageWriter<ConnectionOutput>) {
    client.afk = true;
    outputs.write(ConnectionOutput {
        echo: None,
        ..ConnectionOutput::new(conn, tr!("afk.set"))
    });
}

/// Resolve game-authored restrictions before queueing a physical action.
fn action_restriction<'a>(cmd: &Command, character: &'a Character) -> Option<&'a str> {
    let blocked = matches!(
        cmd,
        Command::Move { .. }
            | Command::Recall
            | Command::Open { .. }
            | Command::Close { .. }
            | Command::Get { .. }
            | Command::Drop { .. }
            | Command::Give { .. }
            | Command::Steal { .. }
            | Command::Buy { .. }
            | Command::Sell { .. }
    );
    let allow_drop = matches!(cmd, Command::Drop { .. })
        && character
            .game_state
            .get("action_restriction_allow_drop")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    if !blocked || allow_drop {
        return None;
    }
    character
        .game_state
        .get("action_restriction")
        .and_then(|v| v.as_str())
}

/// InGame: parse the line (honouring `!` repeat), answer session-local commands
/// directly, admin-gate + mask the admin verbs, and queue everything else.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_ingame(
    client: &mut Client,
    conn: Entity,
    text: &str,
    characters: &Query<(Entity, &Character, &Actor, &GrimName)>,
    player_chars: &PlayerChars,
    descriptions: &Query<&Description>,
    persistence: &PersistenceConfig,
    linkdead: &Query<&Linkdead>,
    rooms: &RoomResolver,
    res: &SessionRes,
    snapshot: &[ClientSnapshot],
    connections: &Query<&Connection>,
    accounts: &Query<&Account>,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    let Some(char_entity) = client.character else {
        return;
    };
    let Some(text_to_parse) = repeated_input(client, text, conn, outputs) else {
        return;
    };

    if let Some(cmd) = parser::parse_command(&res.registry, text_to_parse) {
        let restriction = characters
            .get(char_entity)
            .ok()
            .and_then(|(_, character, _, _)| action_restriction(&cmd, character));
        if let Some(reason) = restriction {
            outputs.write(ConnectionOutput::new(conn, format!("{reason}\n")));
            return;
        }
        // Update last_input for future "!" repeats (store only non-"!" input)
        client.last_input = Some(text_to_parse.to_string());
        // Handle special commands immediately
        match &cmd {
            Command::Who => answer_who(conn, player_chars, linkdead, snapshot, res, outputs),
            Command::Afk => flag_afk(client, conn, outputs),
            Command::Wizlist => {
                answer_wizlist(conn, player_chars, linkdead, snapshot, res, outputs);
            }
            Command::Where => {
                outputs.write(ConnectionOutput {
                    echo: None,
                    ..ConnectionOutput::new(conn, format_where(char_entity, player_chars, rooms))
                });
            }
            Command::Finger { target } => answer_finger(
                conn,
                char_entity,
                target,
                player_chars,
                descriptions,
                persistence,
                outputs,
            ),
            Command::Desc { op: DescOp::Edit } => {
                crate::editor::open_desc_edit(client, conn, char_entity, descriptions, outputs);
            }
            Command::Equipment => {
                answer_character_equipment(conn, char_entity, characters, outputs)
            }
            Command::Commands => answer_commands(conn, char_entity, characters, res, outputs),
            Command::Areas => {
                outputs.write(ConnectionOutput {
                    echo: None,
                    ..ConnectionOutput::new(conn, format_areas(rooms))
                });
            }
            Command::Sockets => answer_session_sockets(
                conn,
                char_entity,
                characters,
                snapshot,
                connections,
                accounts,
                outputs,
            ),
            Command::SavageSoul { ref verb, .. }
                if ["intermud", "i3", "imud", "zebedee", "i2", "grapevine", "gv"]
                    .contains(&verb.as_str()) =>
            {
                let is_admin = character_is_admin(characters, char_entity)
                    && characters
                        .get(char_entity)
                        .is_ok_and(|(_, c, _, _)| !c.is_hidden());
                dispatch_admin_gated(cmd, is_admin, conn, &mut client.input_queue, outputs);
            }
            Command::WizInvis { .. }
            | Command::TestAccount { .. }
            | Command::Shutdown { .. }
            | Command::Reboot { .. }
            | Command::Copyover { .. }
            | Command::Goto { .. }
            | Command::Gecho { .. }
            | Command::Ban { .. }
            | Command::Wiznet { .. } => {
                // Admin-gated + masked: a non-admin must not learn the command
                // exists, so respond exactly as for an unknown command — same
                // text, same framing (a direct ConnectionOutput, no prepended
                // newline; the engine's InfoMessage path would add a leading
                // newline and leak the difference). One shared helper keeps every
                // admin-gated command byte-identical for non-admins.
                let is_admin = character_is_admin(characters, char_entity);
                dispatch_admin_gated(cmd, is_admin, conn, &mut client.input_queue, outputs);
            }
            _ => {
                // All other commands go through the queue to enforce cooldown
                client.input_queue.push_back(cmd);
            }
        }
    } else if text.trim().is_empty() {
        // Blank line — write a newline to trigger prompt on flush
        outputs.write(ConnectionOutput {
            echo: None,
            ..ConnectionOutput::new(conn, " ")
        });
    } else {
        outputs.write(ConnectionOutput {
            echo: None,
            ..ConnectionOutput::new(conn, tr!("error.unknown_command"))
        });
    }
}

/// Dispatch an admin-gated command: an admin's is queued for the engine; a
/// non-admin gets the exact unknown-command reply (same text, same direct
/// framing — no leading newline) so the command's existence is not leaked.
/// Shared by every admin-gated command so their masked responses stay
/// byte-identical.
pub(crate) fn dispatch_admin_gated(
    cmd: Command,
    is_admin: bool,
    conn: Entity,
    queue: &mut std::collections::VecDeque<Command>,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    if is_admin {
        queue.push_back(cmd);
    } else {
        outputs.write(ConnectionOutput {
            echo: None,
            ..ConnectionOutput::new(conn, tr!("error.unknown_command"))
        });
    }
}

fn repeated_input<'a>(
    client: &'a Client,
    text: &'a str,
    conn: Entity,
    outputs: &mut MessageWriter<ConnectionOutput>,
) -> Option<&'a str> {
    Some(if text.trim() == "!" {
        if let Some(ref last_input) = client.last_input {
            last_input.as_str()
        } else {
            outputs.write(ConnectionOutput {
                echo: None,
                ..ConnectionOutput::new(conn, "No previous command to repeat.\n")
            });
            return None;
        }
    } else {
        text
    })
}

fn answer_finger(
    conn: Entity,
    char_entity: Entity,
    target: &str,
    player_chars: &PlayerChars,
    descriptions: &Query<&Description>,
    persistence: &PersistenceConfig,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    outputs.write(ConnectionOutput {
        echo: None,
        ..ConnectionOutput::new(
            conn,
            finger::format(target, player_chars, descriptions, persistence, char_entity),
        )
    });
}

//! The admin-only `sockets` list: one row per live session (connection id,
//! address, session state, character, account), answered session-locally by
//! `handle_ingame` (no engine round-trip). All author-facing text resolves
//! through the `grim-text` catalog, which also escapes the interpolated
//! connection values (account identifiers are emails — their `@` would
//! otherwise read as colour markup).

use bevy::prelude::*;
use grim_actor::{Actor, Character};
use grim_core::components::{Account, ClientState, Name as GrimName};
use grim_networking::Connection;
use grim_text::tr;

use crate::formatter::{self, SocketRow};

/// A per-tick snapshot of every session, collected by `handle_ingame_input`
/// before dispatching lines. Passed down so the `sockets` list can render all
/// sessions without a second `Client` query — which would conflict with the
/// `&mut Client` borrow the dispatcher holds (Bevy rejects shared + mutable
/// access to one component in a system).
#[derive(Debug, Clone)]
pub(crate) struct ClientSnapshot {
    pub(crate) client: Entity,
    pub(crate) connection: Entity,
    pub(crate) state: ClientState,
    pub(crate) account: Option<Entity>,
    pub(crate) character: Option<Entity>,
    /// Whether the session is flagged AFK (copied from `Client::afk`).
    pub(crate) afk: bool,
    /// Seconds since the session's last input, or `None` when never seen.
    pub(crate) idle_secs: Option<u64>,
}

/// Short session-state label for the `sockets` list, resolved through the
/// catalog. The match stays exhaustive over [`ClientState`] so a new session
/// state fails to compile until it gets a label here.
fn client_state_label(state: &ClientState) -> String {
    match state {
        ClientState::InGame => tr!("sockets.state.ingame"),
        ClientState::LoginPrompt => tr!("sockets.state.login"),
        ClientState::PasswordPrompt { .. } => tr!("sockets.state.password"),
        ClientState::ConfirmCreate { .. } => tr!("sockets.state.confirm"),
        ClientState::NewAccountPrompt => tr!("sockets.state.newaccount"),
        ClientState::Disconnecting => tr!("sockets.state.disconnecting"),
        ClientState::CharacterSelect => tr!("sockets.state.select"),
        ClientState::CreateCharacter => tr!("sockets.state.newchar"),
        ClientState::SelectGender { .. } => tr!("sockets.state.gender"),
        ClientState::SelectRace { .. } => tr!("sockets.state.race"),
        ClientState::SelectClass { .. } => tr!("sockets.state.class"),
        ClientState::MotdPrompt => tr!("sockets.state.motd"),
        ClientState::AccessibilityPrompt => "Accessibility".into(),
    }
}

/// The admin-only `sockets` list: one row per live session, sorted by
/// connection id. A session whose `Connection` is gone is skipped (there is
/// no address to show); missing character/account names render as `"-"`.
pub(crate) fn format_sockets(
    snapshot: &[ClientSnapshot],
    connections: &Query<&Connection>,
    characters: &Query<(Entity, &Character, &Actor, &GrimName)>,
    accounts: &Query<&Account>,
) -> String {
    let mut rows: Vec<SocketRow> = snapshot
        .iter()
        .filter_map(|s| {
            let conn = connections.get(s.connection).ok()?;
            let character = s
                .character
                .and_then(|e| characters.get(e).ok())
                .map(|(_, _, _, n)| n.0.clone())
                .unwrap_or_else(|| "-".to_string());
            let account = s
                .account
                .and_then(|e| accounts.get(e).ok())
                .map(|a| a.identifier.clone())
                .unwrap_or_else(|| "-".to_string());
            Some(SocketRow {
                id: conn.id,
                addr: conn.addr.to_string(),
                state: client_state_label(&s.state),
                character,
                account,
                idle_secs: s.idle_secs,
            })
        })
        .collect();
    rows.sort_by_key(|r| r.id);
    formatter::format_sockets_list(&rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_core::components::Gender;

    /// Every `ClientState` arm resolves to its catalog label, so no session
    /// state can reach the admin output unlabelled or unreviewed.
    #[test]
    fn every_session_state_has_a_label() {
        let cases: Vec<(ClientState, &str)> = vec![
            (ClientState::InGame, "InGame"),
            (ClientState::LoginPrompt, "Login"),
            (
                ClientState::PasswordPrompt {
                    identifier: String::new(),
                    is_new: false,
                    character: None,
                },
                "Password",
            ),
            (
                ClientState::ConfirmCreate {
                    identifier: String::new(),
                },
                "Confirm",
            ),
            (ClientState::NewAccountPrompt, "NewAccount"),
            (ClientState::Disconnecting, "Disconnecting"),
            (ClientState::CharacterSelect, "Select"),
            (ClientState::CreateCharacter, "NewChar"),
            (
                ClientState::SelectGender {
                    name: String::new(),
                },
                "Gender",
            ),
            (
                ClientState::SelectRace {
                    name: String::new(),
                    gender: Gender::Neutral,
                },
                "Race",
            ),
            (
                ClientState::SelectClass {
                    name: String::new(),
                    gender: Gender::Neutral,
                    race: String::new(),
                },
                "Class",
            ),
            (ClientState::MotdPrompt, "MOTD"),
        ];
        for (state, expected) in cases {
            assert_eq!(client_state_label(&state), expected);
        }
    }
}

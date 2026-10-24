//! Account creation: validate the chosen password, persist a new account
//! to disk + ECS, and show the (empty) character menu. Split out of
//! `login.rs` under the module line cap; called once from the password
//! prompt's new-account path.

use bevy::prelude::*;
use chrono::Utc;
use grim_actor::{Actor, Character, Linkdead, Player};
use grim_core::components::{Account, Client, ClientState, Name as GrimName};
use grim_core::GrimId;
use grim_networking::ConnectionOutput;
use grim_persistence::PersistenceConfig;
use grim_text::tr;

use crate::character_select as character;
use crate::validation::{hash_password, validate_password};

/// Validate the chosen password, persist a new account to disk + ECS, and show
/// the (empty) character menu.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_account(
    client: &mut Client,
    client_entity: Entity,
    conn: Entity,
    text: &str,
    identifier: &str,
    persistence: &PersistenceConfig,
    characters: &Query<(Entity, &Character, &Actor, &GrimName)>,
    accounts: &Query<(Entity, &mut Account)>,
    players: &Query<&Player>,
    linkdead: &Query<&Linkdead>,
    commands: &mut Commands,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    match validate_password(text.trim()) {
        Ok(()) => {
            let account = Account {
                testing: false,
                id: GrimId::new(),
                identifier: identifier.to_string(),
                password_hash: hash_password(text.trim()),
                characters: vec![],
                created_at: Utc::now(),
            };
            // Save to disk immediately
            let path = persistence
                .accounts_dir()
                .join(format!("{}.json", account.id));
            let _ = std::fs::create_dir_all(persistence.accounts_dir());
            if let Ok(json) = serde_json::to_string_pretty(&account) {
                let _ = std::fs::write(path, json);
            }
            let account_entity = commands.spawn(account).id();
            client.account = Some(account_entity);
            client.state = ClientState::CharacterSelect;
            // Restore echo before showing menu
            outputs.write(ConnectionOutput {
                echo: Some(true),
                ..ConnectionOutput::new(conn, "")
            });
            character::show_character_menu(
                client_entity,
                client,
                characters,
                accounts,
                outputs,
                linkdead,
                players,
                persistence,
            );
        }
        Err(e) => {
            // The transport auto-restored echo when the rejected password was
            // submitted, so re-mask before re-prompting.
            outputs.write(ConnectionOutput {
                echo: Some(false),
                ..ConnectionOutput::new(
                    conn,
                    format!("Invalid password: {}\n{}", e, tr!("login.choose_password")),
                )
            });
        }
    }
}

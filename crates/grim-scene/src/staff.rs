//! Staff-managed test accounts and persistent wizard invisibility.
use bevy::prelude::*;
use grim_actor::{Character, Role};
use grim_core::components::{Account, Name};
use grim_core::events::{Command, EngineCommand, InfoMessage};
use grim_persistence::PersistenceConfig;
use grim_text::tr;
use std::collections::HashMap;

pub(crate) fn stored_character(
    config: &PersistenceConfig,
    stored: &grim_actor::StoredCharacter,
) -> Character {
    let (_, _, mut character) = stored.clone().into_components();
    if let Some(account) = std::fs::read(
        config
            .accounts_dir()
            .join(format!("{}.json", stored.account_id)),
    )
    .ok()
    .and_then(|bytes| serde_json::from_slice::<Account>(&bytes).ok())
    {
        character.roles.retain(|r| *r != Role::Tester);
        if account.testing {
            character.roles.push(Role::Tester);
        }
    }
    character
}

pub(crate) fn sync_testing(accounts: Query<&Account>, mut characters: Query<&mut Character>) {
    let flags: HashMap<_, _> = accounts.iter().map(|a| (a.id, a.testing)).collect();
    for mut character in &mut characters {
        if let Some(testing) = flags.get(&character.account_id) {
            if character.is_tester() == *testing {
                continue;
            }
            character.roles.retain(|r| *r != Role::Tester);
            if *testing {
                character.roles.push(Role::Tester);
            }
        }
    }
}

fn save_account(config: &PersistenceConfig, account: &Account) -> std::io::Result<()> {
    let path = config.accounts_dir().join(format!("{}.json", account.id));
    let temp = path.with_extension("json.staff-tmp");
    std::fs::create_dir_all(config.accounts_dir())?;
    std::fs::write(&temp, serde_json::to_vec_pretty(account)?)?;
    std::fs::rename(temp, path)
}

fn change_account(
    target: &str,
    enabled: bool,
    characters: &mut Query<(&Name, &mut Character)>,
    accounts: &mut Query<&mut Account>,
    config: &PersistenceConfig,
) -> String {
    let id = characters
        .iter()
        .find(|(name, _)| name.0.eq_ignore_ascii_case(target))
        .map(|(_, c)| c.account_id)
        .or_else(|| {
            grim_persistence::load_character_by_name(config, target).map(|c| c.account_id)
        });
    if let Some(id) = id {
        if let Some(mut account) = accounts.iter_mut().find(|a| a.id == id) {
            let previous = account.testing;
            account.testing = enabled;
            if save_account(config, &account).is_err() {
                account.testing = previous;
                tr!("staff.save_failed")
            } else {
                for (_, mut c) in characters.iter_mut() {
                    if c.account_id == id {
                        c.roles.retain(|r| *r != Role::Tester);
                        if enabled {
                            c.roles.push(Role::Tester);
                        }
                    }
                }
                if enabled {
                    tr!("staff.test_on", name = target)
                } else {
                    tr!("staff.test_off", name = target)
                }
            }
        } else {
            tr!("staff.account_missing")
        }
    } else {
        tr!("staff.character_missing")
    }
}

pub(crate) fn handle_staff(
    mut input: MessageReader<EngineCommand>,
    mut characters: Query<(&Name, &mut Character)>,
    mut accounts: Query<&mut Account>,
    config: Res<PersistenceConfig>,
    mut info: MessageWriter<InfoMessage>,
) {
    for event in input.read() {
        if !matches!(
            event.command,
            Command::WizInvis { .. } | Command::TestAccount { .. }
        ) {
            continue;
        }
        let actor = event.client;
        let Ok((_, character)) = characters.get(actor) else {
            continue;
        };
        if !character.is_admin() {
            info.write(InfoMessage {
                target: actor,
                text: tr!("error.unknown_command"),
            });
            continue;
        }
        let text = match &event.command {
            Command::WizInvis { enabled } => {
                let (_, mut character) = characters.get_mut(actor).unwrap();
                if character.is_tester() {
                    tr!("staff.test_always_hidden")
                } else {
                    let value = enabled.unwrap_or(!character.is_hidden());
                    character.game_state.insert("wizinvis".into(), value.into());
                    if value {
                        tr!("staff.invis_on")
                    } else {
                        tr!("staff.invis_off")
                    }
                }
            }
            Command::TestAccount { target, enabled } => {
                change_account(target, *enabled, &mut characters, &mut accounts, &config)
            }
            _ => unreachable!(),
        };
        info.write(InfoMessage {
            target: actor,
            text,
        });
    }
}

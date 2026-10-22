//! Character creation: name entry and the gender → race → class picker, ending
//! in a persisted level-1 character. The `CharacterSelect` / menu side lives in
//! [`crate::character`]; this module owns the `CreateCharacter` / `SelectGender`
//! / `SelectRace` / `SelectClass` state handlers.

use bevy::prelude::*;
use grim_core::components::{Account, Client, ClientState, Gender};
use grim_networking::ConnectionOutput;
use grim_persistence::load_character_by_name;

use crate::finalize;
use crate::params::{SessionRes, WorldEntry};
use crate::validation::{is_name_reserved, validate_character_name};
use grim_scene::formatter::{self, MenuItem};

/// The three fixed genders, paired with display name + slug for the menu.
const GENDERS: [(Gender, &str, &str); 3] = [
    (Gender::Male, "Male", "male"),
    (Gender::Female, "Female", "female"),
    (Gender::Neutral, "Neutral", "neutral"),
];

pub(crate) fn create_character(
    client: &mut Client,
    conn: Entity,
    text: &str,
    res: &SessionRes,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    match validate_character_name(text.trim()) {
        Ok(name) if is_name_reserved(&name, &res.reserved.0) => {
            outputs.write(ConnectionOutput {
                echo: None,
                ..ConnectionOutput::new(
                    conn,
                    "That name is reserved.\nEnter a name for your new character: ",
                )
            });
        }
        Ok(name) if load_character_by_name(&res.persistence, &name).is_some() => {
            outputs.write(ConnectionOutput {
                echo: None,
                ..ConnectionOutput::new(
                    conn,
                    "That name is already taken.\nEnter a name for your new character: ",
                )
            });
        }
        Ok(name) => {
            // Name accepted — begin the gender → race → class picker.
            start_gender_pick(client, conn, name, outputs);
        }
        Err(e) => {
            outputs.write(ConnectionOutput {
                echo: None,
                ..ConnectionOutput::new(
                    conn,
                    format!("Invalid name: {}\nEnter a name for your character: ", e),
                )
            });
        }
    }
}

// ─── Creation picker: gender → race → class ──────────────────────────

/// The gender selection menu (three fixed options).
fn gender_menu() -> String {
    let items: Vec<MenuItem> = GENDERS
        .iter()
        .map(|(_, name, slug)| MenuItem {
            name,
            slug,
            description: "",
        })
        .collect();
    formatter::format_selection_menu("Gender", &items, "Choose a gender: ")
}

/// Begin the gender → race → class picker for `name`: flip to `SelectGender`
/// and send the gender menu. Shared by new-character creation and legacy
/// backfill — a character created before races/classes existed
/// (`race`/`class` empty on disk) routed through the picker once at login (see
/// [`crate::character::character_select`]).
pub(crate) fn start_gender_pick(
    client: &mut Client,
    conn: Entity,
    name: String,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    client.state = ClientState::SelectGender { name };
    outputs.write(ConnectionOutput {
        echo: None,
        ..ConnectionOutput::new(conn, gender_menu())
    });
}

/// The race selection menu, built from the [`grim_world::RaceRegistry`].
fn race_menu(res: &SessionRes) -> String {
    let items: Vec<MenuItem> = res
        .races
        .iter()
        .map(|r| MenuItem {
            name: &r.name,
            slug: &r.slug,
            description: &r.description,
        })
        .collect();
    formatter::format_selection_menu("Race", &items, "Choose a race: ")
}

/// The class selection menu — only the tier-1 (creatable) classes, each with
/// its one-line description.
fn class_menu(res: &SessionRes) -> String {
    let items: Vec<MenuItem> = res
        .classes
        .creatable()
        .map(|c| MenuItem {
            name: &c.name,
            slug: &c.slug,
            description: &c.description,
        })
        .collect();
    formatter::format_selection_menu("Class", &items, "Choose a class: ")
}

/// SelectGender: resolve the pick to a [`Gender`] and advance to race
/// selection, or re-prompt the gender menu on invalid input.
#[allow(clippy::too_many_arguments)]
pub(crate) fn select_gender(
    client: &mut Client,
    conn: Entity,
    text: &str,
    name: String,
    res: &SessionRes,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    let items: Vec<MenuItem> = GENDERS
        .iter()
        .map(|(_, n, slug)| MenuItem {
            name: n,
            slug,
            description: "",
        })
        .collect();
    match formatter::parse_menu_choice(text, &items) {
        Some(i) => {
            let gender = GENDERS[i].0;
            client.state = ClientState::SelectRace { name, gender };
            outputs.write(ConnectionOutput {
                echo: None,
                ..ConnectionOutput::new(conn, race_menu(res))
            });
        }
        None => reprompt(conn, gender_menu(), outputs),
    }
}

/// SelectRace: resolve the race, then finish a classless build or select a class,
/// or re-prompt the race menu on invalid input.
#[allow(clippy::too_many_arguments)]
pub(crate) fn select_race(
    client: &mut Client,
    conn: Entity,
    text: &str,
    name: String,
    gender: Gender,
    res: &SessionRes,
    outputs: &mut MessageWriter<ConnectionOutput>,
    accounts: &mut Query<(Entity, &mut Account)>,
    commands: &mut Commands,
    world: &mut WorldEntry,
) {
    let items: Vec<MenuItem> = res
        .races
        .iter()
        .map(|r| MenuItem {
            name: &r.name,
            slug: &r.slug,
            description: &r.description,
        })
        .collect();
    match formatter::parse_menu_choice(text, &items) {
        Some(i) => {
            let race = res.races.0[i].slug.clone();
            if res
                .build_mode
                .as_ref()
                .is_some_and(|mode| **mode == crate::CharacterBuildMode::SavageSoul)
            {
                let class = "savage-soul".to_string();
                if finalize::account_owns_named(accounts, client.account, &name, &res.persistence) {
                    finalize::backfill_and_enter(
                        client, conn, name, gender, race, class, accounts, res, commands, outputs,
                        world,
                    );
                } else {
                    finalize::finalize_character(
                        client, conn, name, gender, race, class, accounts, res, commands, outputs,
                    );
                }
                return;
            }
            client.state = ClientState::SelectClass { name, gender, race };
            outputs.write(ConnectionOutput {
                echo: None,
                ..ConnectionOutput::new(conn, class_menu(res))
            });
        }
        None => reprompt(conn, race_menu(res), outputs),
    }
}

/// SelectClass: resolve the pick to a tier-1 class slug, then finish the build.
/// Two modes, discriminated by account ownership (no state flag — the closed
/// [`ClientState`] enum stays intact):
///
/// - **new character** — the account does NOT already own a character with this
///   `name` → [`finalize_character`] creates + spawns it (→ MOTD).
/// - **legacy backfill** — the account already owns `name` (a pre-race/class
///   character routed here from the menu) → [`backfill_and_enter`] writes the
///   chosen build to its on-disk JSON and enters the world via the normal path.
///
/// Re-prompts on invalid input.
#[allow(clippy::too_many_arguments)]
pub(crate) fn select_class(
    client: &mut Client,
    conn: Entity,
    text: &str,
    name: String,
    gender: Gender,
    race: String,
    accounts: &mut Query<(Entity, &mut Account)>,
    res: &SessionRes,
    commands: &mut Commands,
    outputs: &mut MessageWriter<ConnectionOutput>,
    world: &mut WorldEntry,
) {
    // Only tier-1 classes are creatable; index into that same filtered list.
    let creatable: Vec<_> = res.classes.creatable().collect();
    let items: Vec<MenuItem> = creatable
        .iter()
        .map(|c| MenuItem {
            name: &c.name,
            slug: &c.slug,
            description: &c.description,
        })
        .collect();
    match formatter::parse_menu_choice(text, &items) {
        Some(i) => {
            let class = creatable[i].slug.clone();
            if finalize::account_owns_named(accounts, client.account, &name, &res.persistence) {
                finalize::backfill_and_enter(
                    client, conn, name, gender, race, class, accounts, res, commands, outputs,
                    world,
                );
            } else {
                finalize::finalize_character(
                    client, conn, name, gender, race, class, accounts, res, commands, outputs,
                );
            }
        }
        None => reprompt(conn, class_menu(res), outputs),
    }
}

/// Re-send a menu without advancing state, prefixed with a rejection notice.
fn reprompt(conn: Entity, menu: String, outputs: &mut MessageWriter<ConnectionOutput>) {
    outputs.write(ConnectionOutput {
        echo: None,
        ..ConnectionOutput::new(conn, format!("Please choose one of the options.\n{menu}"))
    });
}

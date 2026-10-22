//! The `config` command: list, show, and set registered player settings.

use crate::character::Character;
use bevy::prelude::*;
use grim_config::ConfigRegistry;
use grim_core::events::{Command, EngineCommand, InfoMessage};
use grim_text::tr;

/// `config` lists every registered setting with its resolved value and valid
/// values; `config <key>` cycles to the next valid value (wrapping around);
/// `config <key> <value>` validates against the registry and stores the
/// canonical value on [`Character::config`] (persisted by the existing
/// save-on-move/disconnect paths). Keys match case-insensitively; unknown keys
/// and invalid values answer with the valid options. A non-character actor is
/// silently ignored.
pub(crate) fn handle_config(
    mut engine: MessageReader<EngineCommand>,
    mut characters: Query<&mut Character>,
    registry: Res<ConfigRegistry>,
    mut info: MessageWriter<InfoMessage>,
) {
    for cmd in engine.read() {
        let Command::Config { key, value } = &cmd.command else {
            continue;
        };
        let actor = cmd.client;
        let Ok(mut character) = characters.get_mut(actor) else {
            continue;
        };
        let text = match (key, value) {
            (None, _) => list_text(&registry, &character),
            (Some(key), None) => match registry.get(&key.to_lowercase()) {
                Some(def) => {
                    let current = registry
                        .resolve(&character.config, &def.key)
                        .unwrap_or(def.default.as_str())
                        .to_owned();
                    let next = if def.valid.is_empty() {
                        None
                    } else {
                        let pos = def
                            .valid
                            .iter()
                            .position(|v| *v == current)
                            .map(|i| (i + 1) % def.valid.len())
                            .unwrap_or(0);
                        def.valid.get(pos).map(String::as_str)
                    };
                    match next {
                        Some(next) => {
                            character.config.insert(def.key.clone(), next.into());
                            tr!(
                                "config.cycled",
                                name = def.key.as_str(),
                                value = next,
                                old = current.as_str()
                            )
                        }
                        None => tr!(
                            "config.bad_value",
                            name = def.key.as_str(),
                            value = current.as_str(),
                            options = ""
                        ),
                    }
                }
                None => unknown_text(&registry, &character, key),
            },
            (Some(key), Some(value)) => {
                let normalized = key.to_lowercase();
                match registry.get(&normalized) {
                    Some(def) => match def.canonical(value) {
                        Some(canonical) => {
                            character.config.insert(def.key.clone(), canonical.into());
                            tr!("config.set", name = def.key.as_str(), value = canonical)
                        }
                        None => {
                            let options = def.valid.join(", ");
                            tr!(
                                "config.bad_value",
                                name = def.key.as_str(),
                                value = value.as_str(),
                                options = options.as_str()
                            )
                        }
                    },
                    None => unknown_text(&registry, &character, key),
                }
            }
        };
        info.write(InfoMessage {
            target: actor,
            text,
        });
    }
}

/// Full settings list: header plus one sorted row per registered setting,
/// each with its valid values.
fn list_text(registry: &ConfigRegistry, character: &Character) -> String {
    let mut out = tr!("config.list.header");
    for def in registry.all() {
        let resolved = registry
            .resolve(&character.config, &def.key)
            .unwrap_or(def.default.as_str());
        let options = def.valid.join("|");
        out.push_str(&tr!(
            "config.list.row",
            name = def.key.as_str(),
            value = resolved,
            options = options.as_str()
        ));
    }
    out
}

/// Unknown key: the error plus the full list, so the player sees what exists.
fn unknown_text(registry: &ConfigRegistry, character: &Character, key: &str) -> String {
    tr!("config.unknown_key", name = key) + &list_text(registry, character)
}

/// Wire the `config` handler and the messages it reads/emits.
pub(crate) fn register(app: &mut App) {
    app.add_message::<EngineCommand>()
        .add_message::<InfoMessage>()
        .add_systems(Update, handle_config);
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_config::{ConfigDef, Scope};
    use grim_core::GrimId;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<ConfigRegistry>();
        app.world_mut()
            .resource_mut::<ConfigRegistry>()
            .register(ConfigDef {
                key: "minimap".into(),
                valid: vec!["on".into(), "off".into()],
                default: "on".into(),
                scope: Scope::Character,
            });
        register(&mut app);
        app
    }

    fn spawn_char(app: &mut App) -> Entity {
        app.world_mut()
            .spawn(Character {
                credits: 100,
                id: GrimId::new(),
                account_id: GrimId::new(),
                created_at: chrono::Utc::now(),
                last_room: None,
                roles: Vec::new(),
                class: String::new(),
                title: None,
                restrings: std::collections::HashMap::new(),
                config: std::collections::HashMap::new(),
                game_state: Default::default(),
            })
            .id()
    }

    fn send_config(app: &mut App, actor: Entity, key: Option<&str>, value: Option<&str>) {
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Config {
                key: key.map(str::to_string),
                value: value.map(str::to_string),
            },
        });
        app.update();
    }

    fn info_texts(app: &App) -> Vec<String> {
        let m = app.world().resource::<Messages<InfoMessage>>();
        let mut c = m.get_cursor();
        c.read(m).map(|i| i.text.clone()).collect()
    }

    #[test]
    fn bare_config_lists_settings_with_values_and_options() {
        let mut app = test_app();
        let hero = spawn_char(&mut app);
        send_config(&mut app, hero, None, None);
        assert_eq!(
            info_texts(&app),
            vec!["Config options:\n  minimap: on - [on|off]\n"]
        );
    }

    #[test]
    fn config_key_cycles_to_next_value() {
        let mut app = test_app();
        let hero = spawn_char(&mut app);
        // Default on → off, naming the previous value; case-insensitive key.
        send_config(&mut app, hero, Some("MINIMAP"), None);
        assert_eq!(
            info_texts(&app),
            vec!["minimap set to off. (was previously on)\n"]
        );
        // Wraps around: off → on.
        send_config(&mut app, hero, Some("minimap"), None);
        assert_eq!(
            info_texts(&app),
            vec![
                "minimap set to off. (was previously on)\n",
                "minimap set to on. (was previously off)\n"
            ]
        );
        let character = app.world().get::<Character>(hero).expect("hero");
        assert_eq!(character.config.get("minimap"), Some(&"on".to_string()));
    }

    #[test]
    fn config_set_stores_canonical_value() {
        let mut app = test_app();
        let hero = spawn_char(&mut app);
        send_config(&mut app, hero, Some("minimap"), Some("OFF"));
        assert_eq!(info_texts(&app), vec!["minimap set to off.\n"]);
        let character = app.world().get::<Character>(hero).expect("hero");
        assert_eq!(character.config.get("minimap"), Some(&"off".to_string()));
    }

    #[test]
    fn config_set_rejects_invalid_value() {
        let mut app = test_app();
        let hero = spawn_char(&mut app);
        send_config(&mut app, hero, Some("minimap"), Some("sideways"));
        assert_eq!(
            info_texts(&app),
            vec!["Invalid value \"sideways\" for minimap. Valid values: on, off.\n"]
        );
        let character = app.world().get::<Character>(hero).expect("hero");
        assert!(!character.config.contains_key("minimap"));
    }

    #[test]
    fn config_unknown_key_lists_options() {
        let mut app = test_app();
        let hero = spawn_char(&mut app);
        send_config(&mut app, hero, Some("frobnicate"), None);
        assert_eq!(
            info_texts(&app),
            vec!["Unknown config option: frobnicate.\nConfig options:\n  minimap: on - [on|off]\n"]
        );
    }

    #[test]
    fn config_for_non_character_is_ignored() {
        let mut app = test_app();
        let drifter = app.world_mut().spawn(()).id();
        send_config(&mut app, drifter, None, None);
        assert!(info_texts(&app).is_empty());
    }
}

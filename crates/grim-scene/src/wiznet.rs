//! Wiznet: admin-alert preferences, the `wiznet` command, and the alert
//! broadcast. Alerts are produced everywhere via `admin_log!` into
//! `WiznetAlert`; this module owns the receiving end — the per-character
//! prefs (three `grim-config` settings, persisted on `Character.config`),
//! the engine-queued command, and the per-admin broadcast.
//!
//! Prefs resolve through the registry with fail-safe defaults, exactly
//! like `minimap`: master `wiznet` (on), `wiznet.security` (on),
//! `wiznet.logins` (off — socket churn is noisy). The broadcast line is
//! plain `format!`, not a catalog entry: bodies are pre-built operational
//! data (addresses, names — including `@`-bearing account identifiers),
use bevy::prelude::*;
use grim_actor::{Character, Player};
use grim_config::{ConfigDef, ConfigRegistry, Scope};
use grim_core::components::{Client, Name as GrimName};
use grim_core::events::{
    Command, EngineCommand, InfoMessage, LinkdeadAnnounce, LoginAnnounce, LogoutAnnounce,
};
use grim_networking::{Connection, ConnectionOutput, WiznetAlert, WiznetCategory};
use grim_text::tr;

/// Master wiznet switch; `off` silences every category for the character.
pub(crate) const WIZNET_MASTER: &str = "wiznet";
/// Security alerts: guard trips, shed, throttle and ban refusals.
pub(crate) const WIZNET_SECURITY: &str = "wiznet.security";
/// Socket churn and world entry.
pub(crate) const WIZNET_LOGINS: &str = "wiznet.logins";

/// Seed the three wiznet settings. Called from `ScenePlugin::build`, mirroring
/// the `minimap` seeding in `ActorPlugin`.
pub(crate) fn seed_registry(registry: &mut ConfigRegistry) {
    for (key, default) in [
        (WIZNET_MASTER, "on"),
        (WIZNET_SECURITY, "on"),
        (WIZNET_LOGINS, "off"),
    ] {
        registry.register(ConfigDef {
            key: key.into(),
            valid: vec!["on".into(), "off".into()],
            default: default.into(),
            scope: Scope::Character,
        });
    }
}

/// Wire the `wiznet` handler and the broadcast. Message registration lives
/// in `ScenePlugin` (which owns the messages); cf. `ban::register`.
pub(crate) fn register(app: &mut App) {
    app.add_systems(Update, (handle_wiznet, broadcast_wiznet));
}

/// `wiznet [on|off|security|logins]`, engine-queued behind the admin gate
/// (masked at dispatch, re-checked here). Bare lists, `on|off` sets the
/// master switch, a category name flips it, anything else shows usage.
fn handle_wiznet(
    mut engine: MessageReader<EngineCommand>,
    mut characters: Query<(Entity, &mut Character, &GrimName)>,
    registry: Res<ConfigRegistry>,
    mut info: MessageWriter<InfoMessage>,
) {
    for cmd in engine.read() {
        let Command::Wiznet { arg } = &cmd.command else {
            continue;
        };
        let Ok((_, mut character, _)) = characters.get_mut(cmd.client) else {
            continue;
        };
        // Belt-and-braces: dispatch gated + masked, but a queued command
        // must never answer a non-admin.
        if !character.is_admin() {
            continue;
        }
        let lowered = arg.as_deref().map(|a| a.to_lowercase());
        let text = match lowered.as_deref() {
            None => list_text(&registry, &character),
            Some("on") => set_pref(&registry, &mut character, WIZNET_MASTER, "on"),
            Some("off") => set_pref(&registry, &mut character, WIZNET_MASTER, "off"),
            Some("security") => toggle_pref(&registry, &mut character, WIZNET_SECURITY),
            Some("logins") => toggle_pref(&registry, &mut character, WIZNET_LOGINS),
            Some(_) => tr!("wiznet.usage"),
        };
        info.write(InfoMessage {
            target: cmd.client,
            text,
        });
    }
}

/// The `wiznet` list: every category with its resolved value.
fn list_text(registry: &ConfigRegistry, character: &Character) -> String {
    let mut out = tr!("wiznet.list.header");
    for key in [WIZNET_MASTER, WIZNET_SECURITY, WIZNET_LOGINS] {
        let Some(def) = registry.get(key) else {
            continue;
        };
        let resolved = registry
            .resolve(&character.config, key)
            .unwrap_or(&def.default);
        let options = def.valid.join("|");
        out.push_str(&tr!(
            "wiznet.list.row",
            name = def.key.as_str(),
            value = resolved,
            options = options.as_str()
        ));
    }
    out
}

/// Set one wiznet key through the registry (canonical form).
fn set_pref(
    registry: &ConfigRegistry,
    character: &mut Character,
    key: &str,
    value: &str,
) -> String {
    let Some(def) = registry.get(key) else {
        return tr!("wiznet.usage");
    };
    let Some(canonical) = def.canonical(value) else {
        return tr!("wiznet.usage");
    };
    character.config.insert(def.key.clone(), canonical.into());
    tr!("wiznet.set", name = def.key.as_str(), value = canonical)
}

/// Flip one wiznet category to its other valid value.
fn toggle_pref(registry: &ConfigRegistry, character: &mut Character, key: &str) -> String {
    let next = match registry.resolve(&character.config, key) {
        Some("off") => "on",
        _ => "off",
    };
    set_pref(registry, character, key, next)
}

/// Fan every alert and login-transition notice out to the admins watching
/// its category: online (`Player`), admin, master on, category on.
#[allow(clippy::too_many_arguments)]
fn broadcast_wiznet(
    mut alerts: MessageReader<WiznetAlert>,
    mut logins: MessageReader<LoginAnnounce>,
    mut logouts: MessageReader<LogoutAnnounce>,
    mut linkdeads: MessageReader<LinkdeadAnnounce>,
    online: Query<(Entity, &GrimName, &Character, &Player)>,
    clients: Query<&Client>,
    characters: Query<(Entity, &GrimName, &Character)>,
    connections: Query<&Connection>,
    registry: Res<ConfigRegistry>,
    mut outputs: MessageWriter<ConnectionOutput>,
) {
    let mut pending: Vec<(WiznetCategory, String)> = Vec::new();
    for alert in alerts.read() {
        pending.push((alert.category, alert.text.clone()));
    }
    for ev in logins.read() {
        pending.push((
            WiznetCategory::Logins,
            format!(
                "{} entered the world{}",
                ev.name,
                addr_suffix(&ev.name, &clients, &characters, &connections)
            ),
        ));
    }
    for ev in logouts.read() {
        pending.push((
            WiznetCategory::Logins,
            format!(
                "{} quit{}",
                ev.name,
                addr_suffix(&ev.name, &clients, &characters, &connections)
            ),
        ));
    }
    for ev in linkdeads.read() {
        let what = if ev.reconnecting {
            "reconnected"
        } else {
            "went linkdead"
        };
        pending.push((
            WiznetCategory::Logins,
            format!(
                "{} {what}{}",
                ev.name,
                addr_suffix(&ev.name, &clients, &characters, &connections)
            ),
        ));
    }
    for (category, text) in pending {
        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        let line = format!("{ts} [wiznet:{category}] {text}\n");
        for conn in wiznet_audience(&online, &registry, category) {
            // Unsolicited like any other game event: stay off the prompt line.
            outputs.write(ConnectionOutput {
                prepend_newline: true,
                ..ConnectionOutput::new(conn, line.clone())
            });
        }
    }
}

/// Connections of online admins with the master switch and `category` on.
fn wiznet_audience(
    online: &Query<(Entity, &GrimName, &Character, &Player)>,
    registry: &ConfigRegistry,
    category: WiznetCategory,
) -> Vec<Entity> {
    let key = match category {
        WiznetCategory::Security => WIZNET_SECURITY,
        WiznetCategory::Logins => WIZNET_LOGINS,
    };
    online
        .iter()
        .filter(|(_, _, character, _)| {
            character.is_admin()
                && registry.resolve(&character.config, WIZNET_MASTER) == Some("on")
                && registry.resolve(&character.config, key) == Some("on")
        })
        .map(|(_, _, _, player)| player.connection)
        .collect()
}

/// `" from {addr}"` for a character with a live session, else `""`.
/// Best-effort join for announce-derived lines (the socket may already be
/// gone); never fails the line.
fn addr_suffix(
    name: &str,
    clients: &Query<&Client>,
    characters: &Query<(Entity, &GrimName, &Character)>,
    connections: &Query<&Connection>,
) -> String {
    characters
        .iter()
        .find(|(_, n, _)| n.0 == name)
        .and_then(|(entity, _, _)| clients.iter().find(|c| c.character == Some(entity)))
        .and_then(|client| connections.get(client.connection).ok())
        .map(|conn| format!(" from {}", conn.addr))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_actor::{Role, StoredCharacter};
    use grim_core::GrimId;

    fn wiznet_app() -> App {
        let mut app = App::new();
        app.add_message::<EngineCommand>()
            .add_message::<InfoMessage>()
            .add_message::<WiznetAlert>()
            .add_message::<LoginAnnounce>()
            .add_message::<LogoutAnnounce>()
            .add_message::<LinkdeadAnnounce>()
            .add_message::<ConnectionOutput>();
        app.init_resource::<ConfigRegistry>();
        {
            let mut registry = app.world_mut().resource_mut::<ConfigRegistry>();
            seed_registry(&mut registry);
        }
        app.add_systems(Update, (handle_wiznet, broadcast_wiznet));
        app
    }

    fn spawn_conn(app: &mut App, id: usize, port: u16) -> Entity {
        app.world_mut()
            .spawn(Connection {
                id,
                addr: format!("127.0.0.1:{port}").parse().unwrap(),
                echo_hidden: false,
            })
            .id()
    }

    fn spawn_char(app: &mut App, name: &str, admin: bool, conn: Entity) -> Entity {
        let roles = if admin { vec![Role::Admin] } else { vec![] };
        let (gname, actor, character) = StoredCharacter {
            credits: 100,
            id: GrimId::new(),
            account_id: GrimId::new(),
            name: name.into(),
            created_at: chrono::Utc::now(),
            last_room: None,
            roles,
            gender: grim_core::components::Gender::Neutral,
            race: String::new(),
            class: String::new(),
            level: 1,
            title: None,
            restrings: std::collections::HashMap::new(),
            config: std::collections::HashMap::new(),
            game_state: Default::default(),
            inventory: Vec::new(),
        }
        .into_components();
        app.world_mut()
            .spawn((gname, actor, character, Player { connection: conn }))
            .id()
    }

    fn send_wiznet(app: &mut App, actor: Entity, arg: Option<&str>) {
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Wiznet {
                arg: arg.map(str::to_string),
            },
        });
        app.update();
    }

    fn infos(app: &App) -> Vec<String> {
        let msgs = app.world().resource::<Messages<InfoMessage>>();
        let mut cursor = msgs.get_cursor();
        cursor.read(msgs).map(|m| m.text.clone()).collect()
    }

    fn outputs_for(app: &App, conn: Entity) -> Vec<String> {
        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        cursor
            .read(msgs)
            .filter(|m| m.connection == conn)
            .map(|m| m.text.clone())
            .collect()
    }

    #[test]
    fn seed_registers_three_settings_with_defaults() {
        let mut app = App::new();
        app.init_resource::<ConfigRegistry>();
        {
            let mut registry = app.world_mut().resource_mut::<ConfigRegistry>();
            seed_registry(&mut registry);
        }
        let registry = app.world().resource::<ConfigRegistry>();
        assert!(registry.get("wiznet").is_some());
        assert!(registry.get("wiznet.security").is_some());
        assert!(registry.get("wiznet.logins").is_some());
        assert_eq!(
            registry.resolve(&std::collections::HashMap::new(), "wiznet"),
            Some("on")
        );
        assert_eq!(
            registry.resolve(&std::collections::HashMap::new(), "wiznet.logins"),
            Some("off")
        );
    }

    #[test]
    fn wiznet_lists_categories_with_states() {
        let mut app = wiznet_app();
        let conn = spawn_conn(&mut app, 1, 4001);
        let admin = spawn_char(&mut app, "Root", true, conn);
        send_wiznet(&mut app, admin, None);
        let texts = infos(&app);
        assert_eq!(texts.len(), 1);
        assert!(
            texts[0].starts_with("Wiznet options:\n"),
            "got:\n{}",
            texts[0]
        );
        assert!(
            texts[0].contains("wiznet: on - [on|off]"),
            "got:\n{}",
            texts[0]
        );
        assert!(
            texts[0].contains("wiznet.security: on - [on|off]"),
            "got:\n{}",
            texts[0]
        );
        assert!(
            texts[0].contains("wiznet.logins: off - [on|off]"),
            "got:\n{}",
            texts[0]
        );
    }

    #[test]
    fn wiznet_master_set_and_category_toggle() {
        let mut app = wiznet_app();
        let conn = spawn_conn(&mut app, 1, 4001);
        let admin = spawn_char(&mut app, "Root", true, conn);
        send_wiznet(&mut app, admin, Some("off"));
        assert_eq!(
            infos(&app).last().cloned(),
            Some("wiznet set to off.\n".into())
        );
        send_wiznet(&mut app, admin, Some("SECURITY"));
        // Messages older than two updates flush: assert the latest only.
        assert_eq!(
            infos(&app).last().cloned(),
            Some("wiznet.security set to off.\n".into())
        );
        send_wiznet(&mut app, admin, Some("security"));
        assert_eq!(
            infos(&app).last().cloned(),
            Some("wiznet.security set to on.\n".into())
        );
        let character = app.world().get::<Character>(admin).unwrap();
        assert_eq!(
            character.config.get("wiznet.security"),
            Some(&"on".to_string())
        );
    }

    #[test]
    fn wiznet_unknown_arg_shows_usage() {
        let mut app = wiznet_app();
        let conn = spawn_conn(&mut app, 1, 4001);
        let admin = spawn_char(&mut app, "Root", true, conn);
        send_wiznet(&mut app, admin, Some("frobnicate"));
        assert_eq!(
            infos(&app),
            vec!["Usage: wiznet [on|off|security|logins]\n"]
        );
    }

    #[test]
    fn wiznet_nonadmin_gets_nothing() {
        let mut app = wiznet_app();
        let conn = spawn_conn(&mut app, 1, 4001);
        let player = spawn_char(&mut app, "Bob", false, conn);
        send_wiznet(&mut app, player, None);
        assert!(infos(&app).is_empty());
    }

    #[test]
    fn broadcast_reaches_opted_in_admins_only() {
        let mut app = wiznet_app();
        let conn_a = spawn_conn(&mut app, 1, 4001);
        let conn_b = spawn_conn(&mut app, 2, 4002);
        let conn_c = spawn_conn(&mut app, 3, 4003);
        let _a = spawn_char(&mut app, "Root", true, conn_a);
        let b = spawn_char(&mut app, "Deputy", true, conn_b);
        let _c = spawn_char(&mut app, "Bob", false, conn_c);
        // Deputy opts out of security.
        app.world_mut()
            .get_mut::<Character>(b)
            .unwrap()
            .config
            .insert("wiznet.security".into(), "off".into());
        app.world_mut().write_message(WiznetAlert {
            category: WiznetCategory::Security,
            text: "connection flood: 101 connects".into(),
        });
        app.update();
        let a_out = outputs_for(&app, conn_a).join("");
        let b_out = outputs_for(&app, conn_b).join("");
        let c_out = outputs_for(&app, conn_c).join("");
        assert!(
            a_out.contains("[wiznet:security] connection flood: 101 connects"),
            "opted-in admin hears it; got: {a_out:?}"
        );
        assert!(
            b_out.is_empty(),
            "opted-out admin hears nothing; got: {b_out:?}"
        );
        assert!(c_out.is_empty(), "non-admin hears nothing; got: {c_out:?}");
    }

    #[test]
    fn broadcast_login_announce_carries_addr() {
        let mut app = wiznet_app();
        let conn_a = spawn_conn(&mut app, 1, 4001);
        let conn_h = spawn_conn(&mut app, 2, 4567);
        let a = spawn_char(&mut app, "Root", true, conn_a);
        let hero = spawn_char(&mut app, "Hero", false, conn_h);
        // Root opts into logins (default off); Hero's session join feeds
        // the addr lookup.
        app.world_mut()
            .get_mut::<Character>(a)
            .unwrap()
            .config
            .insert("wiznet.logins".into(), "on".into());
        let session = app.world_mut().spawn(Client::new(conn_h)).id();
        app.world_mut()
            .entity_mut(session)
            .get_mut::<Client>()
            .unwrap()
            .character = Some(hero);
        app.world_mut().write_message(LoginAnnounce {
            subject: hero,
            name: "Hero".into(),
        });
        app.update();
        let a_out = outputs_for(&app, conn_a).join("");
        assert!(
            a_out.contains("[wiznet:logins] Hero entered the world from 127.0.0.1:4567"),
            "got: {a_out:?}"
        );
    }

    #[test]
    fn broadcast_lines_carry_utc_timestamp_prefix() {
        let mut app = wiznet_app();
        let conn = spawn_conn(&mut app, 1, 4001);
        let _admin = spawn_char(&mut app, "Root", true, conn);
        app.world_mut().write_message(WiznetAlert {
            category: WiznetCategory::Security,
            text: "conn 7 tripped".into(),
        });
        app.update();
        let out = outputs_for(&app, conn).join("");
        let line = out.lines().next().expect("one wiznet line");
        // `YYYY-MM-DDTHH:mm:ssZ [wiznet:security] conn 7 tripped`
        assert_eq!(&line[4..5], "-", "got: {line:?}");
        assert_eq!(&line[7..8], "-", "got: {line:?}");
        assert_eq!(&line[10..11], "T", "got: {line:?}");
        assert_eq!(&line[13..14], ":", "got: {line:?}");
        assert_eq!(&line[16..17], ":", "got: {line:?}");
        assert_eq!(&line[19..20], "Z", "got: {line:?}");
        assert!(
            line[20..].starts_with(" [wiznet:security] conn 7 tripped"),
            "tag and body intact after the timestamp; got: {line:?}"
        );
    }
}

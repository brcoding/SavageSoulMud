//! The admin `ban` command: list, add, and remove blocklist entries.
//!
//! Queued admin-gated + masked like the other admin verbs (a non-admin sees
//! the exact unknown-command reply), then handled here off the engine queue
//! with a defense-in-depth admin re-check. An `add` persists to `bans.json`
//! and kicks every live session it matches; the auth crate refuses banned
//! identities at the login boundary, and a reboot reloads the file.

use bevy::log::warn;
use bevy::prelude::*;
use grim_actor::{Actor, Character};
use grim_core::components::{Account, Client, Name as GrimName};
use grim_core::events::{BanKind, BanOp, Command, EngineCommand, InfoMessage};
use grim_networking::{Connection, ConnectionOutput, DisconnectRequest};
use grim_persistence::{BanEntry, BanList, PersistenceConfig};
use grim_text::tr;

/// Parse `ban list [type]` / `ban add <type> <pattern>` /
/// `ban remove <type> <pattern>`. Types are `ip` / `account` / `character`
/// (case-insensitive); `add`/`remove` take exactly one pattern token. Anything
/// else is `None` (unknown command), matching the other admin verbs.
pub(crate) fn parse_ban(rest: &str) -> Option<Command> {
    let (verb, args) = rest.split_once(' ').unwrap_or((rest, ""));
    if verb.eq_ignore_ascii_case("list") {
        let filter = args.trim();
        if filter.is_empty() {
            return Some(Command::Ban {
                op: BanOp::List { filter: None },
            });
        }
        if filter.split_whitespace().count() == 1 {
            return BanKind::parse(filter).map(|kind| Command::Ban {
                op: BanOp::List { filter: Some(kind) },
            });
        }
        return None;
    }
    if verb.eq_ignore_ascii_case("add") || verb.eq_ignore_ascii_case("remove") {
        let mut parts = args.split_whitespace();
        let kind = parts.next().and_then(BanKind::parse)?;
        let pattern = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
        let op = if verb.eq_ignore_ascii_case("add") {
            BanOp::Add {
                kind,
                pattern: pattern.to_string(),
            }
        } else {
            BanOp::Remove {
                kind,
                pattern: pattern.to_string(),
            }
        };
        return Some(Command::Ban { op });
    }
    None
}

/// `ban ...`: admin-gated (defense in depth — dispatch gates first, so a
/// well-behaved session never sends this for a non-admin; a foreign command
/// source fails closed and silent here to avoid leaking the verb).
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_ban_command(
    mut engine: MessageReader<EngineCommand>,
    characters: Query<(Entity, &Character, &Actor, &GrimName)>,
    clients: Query<(Entity, &Client)>,
    connections: Query<&Connection>,
    accounts: Query<&Account>,
    mut bans: ResMut<BanList>,
    persistence: Res<PersistenceConfig>,
    mut info: MessageWriter<InfoMessage>,
    mut outputs: MessageWriter<ConnectionOutput>,
    mut disconnect: MessageWriter<DisconnectRequest>,
) {
    for cmd in engine.read() {
        let Command::Ban { op } = &cmd.command else {
            continue;
        };
        let actor = cmd.client;
        let (is_admin, admin_name) = match characters.get(actor) {
            Ok((_, c, _, n)) => (c.is_admin(), n.0.clone()),
            Err(_) => continue,
        };
        if !is_admin {
            continue;
        }
        match op {
            BanOp::List { filter } => {
                let entries = bans.list(*filter);
                info.write(InfoMessage {
                    target: actor,
                    text: format_ban_list(&entries),
                });
            }
            BanOp::Add { kind, pattern } => {
                if !BanList::valid_pattern(*kind, pattern) {
                    info.write(InfoMessage {
                        target: actor,
                        text: tr!("ban.invalid_pattern", scope = kind.as_str()),
                    });
                    continue;
                }
                // Mutate a candidate and persist it first: the live list, the
                // confirmation, and the kicks all land only when the save
                // succeeds, so a failed write can never desync memory from
                // `bans.json` (which would revert on the next boot).
                let mut candidate = bans.clone();
                if !candidate.add(*kind, pattern, &admin_name) {
                    info.write(InfoMessage {
                        target: actor,
                        text: tr!("ban.exists"),
                    });
                    continue;
                }
                if let Err(e) = candidate.save(&persistence.dir) {
                    warn!("ban: failed to save blocklist: {e}");
                    info.write(InfoMessage {
                        target: actor,
                        text: tr!("ban.save_failed"),
                    });
                    continue;
                }
                *bans = candidate;
                let stored = BanList::normalize(*kind, pattern);
                info.write(InfoMessage {
                    target: actor,
                    text: tr!("ban.added", scope = kind.as_str(), pattern = stored),
                });
                kick_matches(
                    &bans,
                    *kind,
                    &clients,
                    &connections,
                    &accounts,
                    &characters,
                    &mut outputs,
                    &mut disconnect,
                );
            }
            BanOp::Remove { kind, pattern } => {
                // Same candidate discipline as `add`: the live list changes
                // only once the removal is durable.
                let mut candidate = bans.clone();
                if !candidate.remove(*kind, pattern) {
                    info.write(InfoMessage {
                        target: actor,
                        text: tr!("ban.not_found"),
                    });
                    continue;
                }
                if let Err(e) = candidate.save(&persistence.dir) {
                    warn!("ban: failed to save blocklist: {e}");
                    info.write(InfoMessage {
                        target: actor,
                        text: tr!("ban.save_failed"),
                    });
                    continue;
                }
                *bans = candidate;
                let stored = BanList::normalize(*kind, pattern);
                info.write(InfoMessage {
                    target: actor,
                    text: tr!("ban.removed", scope = kind.as_str(), pattern = stored),
                });
            }
        }
    }
}

/// Render the `ban list` reply: the catalog empty line, or one header plus a
/// row per entry (kind, pattern, ban date, creating admin).
fn format_ban_list(entries: &[&BanEntry]) -> String {
    if entries.is_empty() {
        return tr!("ban.list.empty");
    }
    let total = entries.len().to_string();
    let mut out = tr!("ban.list.header", total = total);
    for e in entries {
        let date = e.created_at.format("%Y-%m-%d").to_string();
        out.push_str(&tr!(
            "ban.list.row",
            scope = e.kind.as_str(),
            pattern = e.pattern,
            date = date,
            author = e.created_by
        ));
    }
    out
}

/// Sever every live session the new ban matches: the per-kind banned message
/// on the socket, then the close request. The ensuing `ConnectionClosed`
/// marks the character linkdead through the normal disconnect path.
#[allow(clippy::too_many_arguments)]
fn kick_matches(
    bans: &BanList,
    kind: BanKind,
    clients: &Query<(Entity, &Client)>,
    connections: &Query<&Connection>,
    accounts: &Query<&Account>,
    characters: &Query<(Entity, &Character, &Actor, &GrimName)>,
    outputs: &mut MessageWriter<ConnectionOutput>,
    disconnect: &mut MessageWriter<DisconnectRequest>,
) {
    let msg = match kind {
        BanKind::Ip => tr!("ban.banned.ip"),
        BanKind::Account => tr!("ban.banned.account"),
        BanKind::Character => tr!("ban.banned.character"),
    };
    for (_, client) in clients.iter() {
        let hit = match kind {
            BanKind::Ip => connections
                .get(client.connection)
                .is_ok_and(|c| bans.is_ip_banned(&c.addr.ip())),
            BanKind::Account => client
                .account
                .and_then(|a| accounts.get(a).ok())
                .is_some_and(|a| bans.is_account_banned(a)),
            BanKind::Character => client
                .character
                .and_then(|e| characters.get(e).ok())
                .is_some_and(|(_, _, _, n)| bans.is_character_banned(&n.0)),
        };
        if hit {
            outputs.write(ConnectionOutput::new(client.connection, msg.clone()));
            disconnect.write(DisconnectRequest {
                connection: client.connection,
            });
        }
    }
}

/// Wire the `ban` handler and the messages it reads/emits.
pub(crate) fn register(app: &mut App) {
    app.add_message::<EngineCommand>()
        .add_message::<InfoMessage>()
        .add_message::<DisconnectRequest>()
        .add_systems(Update, handle_ban_command);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use grim_core::components::Account;
    use grim_core::GrimId;
    use std::net::SocketAddr;

    fn test_app(dir: &std::path::Path) -> App {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(PersistenceConfig {
            dir: dir.to_path_buf(),
        });
        app.insert_resource(BanList::default());
        register(&mut app);
        app.add_message::<ConnectionOutput>();
        app
    }

    fn unique_dir(tag: &str) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!("grim-scene-ban-{tag}-{}-{n}", std::process::id()))
    }

    fn spawn_admin(app: &mut App, name: &str, conn: Entity) -> Entity {
        let actor = app
            .world_mut()
            .spawn((
                Character {
                    credits: 100,
                    id: GrimId::new(),
                    account_id: GrimId::new(),
                    created_at: Utc::now(),
                    last_room: None,
                    roles: vec![grim_actor::Role::Admin],
                    class: String::new(),
                    title: None,
                    restrings: Default::default(),
                    config: Default::default(),
                    game_state: Default::default(),
                },
                Actor {
                    race: "human".into(),
                    level: 1,
                    gender: grim_core::character::Gender::Neutral,
                },
                GrimName(name.into()),
            ))
            .id();
        app.world_mut().spawn(Client {
            character: Some(actor),
            ..Client::new(conn)
        });
        actor
    }

    fn spawn_conn(app: &mut App, id: usize, addr: &str) -> Entity {
        app.world_mut()
            .spawn(Connection {
                id,
                addr: addr.parse::<SocketAddr>().unwrap(),
                echo_hidden: false,
            })
            .id()
    }

    fn infos(app: &App) -> Vec<String> {
        let messages = app.world().resource::<Messages<InfoMessage>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).map(|m| m.text.clone()).collect()
    }

    fn disconnects(app: &App) -> Vec<Entity> {
        let messages = app.world().resource::<Messages<DisconnectRequest>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).map(|m| m.connection).collect()
    }

    fn send(app: &mut App, actor: Entity, op: BanOp) {
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Ban { op },
        });
        app.update();
    }

    #[test]
    fn non_admin_ban_is_silently_ignored() {
        let dir = unique_dir("gate");
        let mut app = test_app(&dir);
        let conn = spawn_conn(&mut app, 1, "127.0.0.1:4001");
        let actor = app
            .world_mut()
            .spawn((
                Character {
                    credits: 100,
                    id: GrimId::new(),
                    account_id: GrimId::new(),
                    created_at: Utc::now(),
                    last_room: None,
                    roles: vec![],
                    class: String::new(),
                    title: None,
                    restrings: Default::default(),
                    config: Default::default(),
                    game_state: Default::default(),
                },
                Actor {
                    race: "human".into(),
                    level: 1,
                    gender: grim_core::character::Gender::Neutral,
                },
                GrimName("Pleb".into()),
            ))
            .id();
        app.world_mut().spawn(Client {
            character: Some(actor),
            ..Client::new(conn)
        });
        send(&mut app, actor, BanOp::List { filter: None });
        assert!(infos(&app).is_empty());
        assert!(disconnects(&app).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_empty_then_add_then_list_shows_row() {
        let dir = unique_dir("list");
        let mut app = test_app(&dir);
        let conn = spawn_conn(&mut app, 1, "127.0.0.1:4001");
        let actor = spawn_admin(&mut app, "Root", conn);
        send(&mut app, actor, BanOp::List { filter: None });
        assert_eq!(infos(&app), vec!["No bans.\n"]);
        send(
            &mut app,
            actor,
            BanOp::Add {
                kind: BanKind::Character,
                pattern: "Villain".into(),
            },
        );
        let got = infos(&app);
        assert_eq!(got.len(), 2);
        assert_eq!(got[1], "Ban added: character villain\n");
        // Persisted to the single file.
        let stored: Vec<BanEntry> =
            serde_json::from_str(&std::fs::read_to_string(BanList::bans_file(&dir)).unwrap())
                .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].created_by, "Root");
        send(
            &mut app,
            actor,
            BanOp::List {
                filter: Some(BanKind::Ip),
            },
        );
        assert_eq!(infos(&app)[2], "No bans.\n");
        send(&mut app, actor, BanOp::List { filter: None });
        let listed = &infos(&app)[3];
        assert!(
            listed.contains("character villain (banned ") && listed.contains(" by Root)"),
            "row shows kind, pattern, date, creator; got: {listed}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_duplicate_and_invalid_pattern_are_rejected() {
        let dir = unique_dir("reject");
        let mut app = test_app(&dir);
        let conn = spawn_conn(&mut app, 1, "127.0.0.1:4001");
        let actor = spawn_admin(&mut app, "Root", conn);
        send(
            &mut app,
            actor,
            BanOp::Add {
                kind: BanKind::Ip,
                pattern: "999.1.1.1".into(),
            },
        );
        assert_eq!(infos(&app), vec!["Invalid pattern for ip ban.\n"]);
        send(
            &mut app,
            actor,
            BanOp::Add {
                kind: BanKind::Ip,
                pattern: "10.*".into(),
            },
        );
        send(
            &mut app,
            actor,
            BanOp::Add {
                kind: BanKind::Ip,
                pattern: "10.*".into(),
            },
        );
        assert_eq!(infos(&app)[2], "That ban already exists.\n");
        send(
            &mut app,
            actor,
            BanOp::Remove {
                kind: BanKind::Ip,
                pattern: "11.*".into(),
            },
        );
        assert_eq!(infos(&app)[3], "No such ban.\n");
        send(
            &mut app,
            actor,
            BanOp::Remove {
                kind: BanKind::Ip,
                pattern: "10.*".into(),
            },
        );
        assert_eq!(infos(&app)[4], "Ban removed: ip 10.*\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_save_changes_nothing_and_kicks_nobody() {
        let dir = unique_dir("savefail");
        let mut app = test_app(&dir);
        // Point the persistence dir at a FILE, so `bans.json` can never be
        // written: every save fails.
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"nope").unwrap();
        app.world_mut().resource_mut::<PersistenceConfig>().dir = blocker;
        let admin_conn = spawn_conn(&mut app, 1, "127.0.0.1:4001");
        let admin = spawn_admin(&mut app, "Root", admin_conn);
        let victim_conn = spawn_conn(&mut app, 2, "10.1.2.3:5000");
        let _victim = spawn_admin(&mut app, "Victim", victim_conn);
        send(
            &mut app,
            admin,
            BanOp::Add {
                kind: BanKind::Ip,
                pattern: "10.*".into(),
            },
        );
        assert_eq!(
            infos(&app),
            vec!["Ban could not be saved; nothing changed.\n"]
        );
        assert!(
            app.world().resource::<BanList>().list(None).is_empty(),
            "failed save leaves the live list untouched"
        );
        assert!(disconnects(&app).is_empty(), "failed save kicks nobody");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_kicks_matching_sessions_only() {
        let dir = unique_dir("kick");
        let mut app = test_app(&dir);
        let admin_conn = spawn_conn(&mut app, 1, "127.0.0.1:4001");
        let admin = spawn_admin(&mut app, "Root", admin_conn);
        // Victim on a banned subnet; bystander elsewhere. Both hold sessions
        // but never send commands — only Root's `ban add` runs.
        let victim_conn = spawn_conn(&mut app, 2, "10.1.2.3:5000");
        let _victim = spawn_admin(&mut app, "Victim", victim_conn);
        let calm_conn = spawn_conn(&mut app, 3, "192.168.0.9:5001");
        let _calm = spawn_admin(&mut app, "Calm", calm_conn);
        send(
            &mut app,
            admin,
            BanOp::Add {
                kind: BanKind::Ip,
                pattern: "10.*".into(),
            },
        );
        assert_eq!(disconnects(&app), vec![victim_conn]);
        let outputs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = outputs.get_cursor();
        let texts: Vec<(Entity, String)> = cursor
            .read(outputs)
            .map(|o| (o.connection, o.text.clone()))
            .collect();
        assert!(
            texts
                .iter()
                .any(|(c, t)| *c == victim_conn && t.contains("Your IP address has been banned")),
            "victim sees the IP ban message; got: {texts:?}"
        );
        assert!(
            texts.iter().all(|(c, _)| *c != calm_conn),
            "bystander untouched; got: {texts:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn character_ban_kicks_named_session() {
        let dir = unique_dir("charkick");
        let mut app = test_app(&dir);
        let admin_conn = spawn_conn(&mut app, 1, "127.0.0.1:4001");
        let admin = spawn_admin(&mut app, "Root", admin_conn);
        let doomed_conn = spawn_conn(&mut app, 2, "127.0.0.1:4002");
        let _doomed = spawn_admin(&mut app, "Doomed", doomed_conn);
        send(
            &mut app,
            admin,
            BanOp::Add {
                kind: BanKind::Character,
                pattern: "DOOMED".into(),
            },
        );
        assert_eq!(disconnects(&app), vec![doomed_conn]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(unused_must_use)]
    fn account_ban_kicks_that_accounts_sessions() {
        let dir = unique_dir("acctkick");
        let mut app = test_app(&dir);
        let admin_conn = spawn_conn(&mut app, 1, "127.0.0.1:4001");
        let admin = spawn_admin(&mut app, "Root", admin_conn);
        let acct = Account {
            testing: false,
            id: GrimId::new(),
            identifier: "doomed@example.com".into(),
            password_hash: "hash".into(),
            characters: vec![],
            created_at: Utc::now(),
        };
        let acct_e = app.world_mut().spawn(acct).id();
        let target_conn = spawn_conn(&mut app, 2, "127.0.0.1:4002");
        app.world_mut().spawn(Client {
            account: Some(acct_e),
            ..Client::new(target_conn)
        });
        send(
            &mut app,
            admin,
            BanOp::Add {
                kind: BanKind::Account,
                pattern: "doomed@example.com".into(),
            },
        );
        assert_eq!(disconnects(&app), vec![target_conn]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

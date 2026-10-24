//! Unit tests for the scene subsystem, grouped into concern-named submodules.
//! Kept inline (private-item access) — shared fixtures live at this module's
//! root; each nested `mod` covers one concern and pulls them in via `super::*`.
//! The pre-game (login/creation/select/MOTD) suites moved to the auth crate;
//! what remains exercises copyover resume, output formatting, and in-game
//! dispatch.

use bevy::prelude::*;
use chrono::Utc;
use grim_actor::{
    Actor, Character, Creature, InRoom, Linkdead, OutputHistory, Player, Role, StoredCharacter,
};
use grim_channel::{Channel, ChannelMessage, ChannelPlugin};
use grim_core::components::Name as GrimName;
use grim_core::components::*;
use grim_core::events::*;
use grim_object::{CarriedBy, Object};
// Explicit named import shadows the glob'd `bevy::prelude::Command` trait.
use grim_core::events::Command;
use grim_core::GrimId;
use grim_networking::{
    Connection, ConnectionEstablished, ConnectionInput, ConnectionOutput, DisconnectRequest,
    WiznetAlert, WiznetCategory,
};
use grim_persistence::{BanList, PersistenceConfig, PersistencePlugin};
use grim_world::{Area, Room, StartingRoom, WorldPlugin};
use std::net::SocketAddr;

use crate::scene_stack::{InGameScene, SceneStack};
use crate::ScenePlugin;

// ─── Shared fixtures ─────────────────────

/// A unique temp dir per call, so parallel tests never share on-disk state.
fn unique_dir(tag: &str) -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("grim-scene-{tag}-{}-{}", std::process::id(), n))
}

/// A fresh app on a per-test temp `PersistenceConfig` dir. Uses a unique dir
/// rather than the process-CWD `data/` dir so parallel tests (e.g. the `quit`
/// path, which saves a character to disk) never clobber each other.
fn test_app() -> App {
    let dir = unique_dir("app");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("characters")).unwrap();
    std::fs::create_dir_all(dir.join("accounts")).unwrap();
    let mut app = App::new();
    app.insert_resource(PersistenceConfig { dir });
    app.add_plugins(MinimalPlugins);
    app.add_plugins(WorldPlugin);
    app.add_plugins(ChannelPlugin);
    app.add_plugins(PersistencePlugin);
    app.add_plugins(ScenePlugin);
    // Telnet protocol messages not registered by the above plugins
    app.add_message::<ConnectionEstablished>()
        .add_message::<ConnectionInput>();
    app
}

fn spawn_room(app: &mut App) -> Entity {
    app.world_mut()
        .spawn((
            Room {
                id: GrimId::new(),
                friendly_id: "room1".into(),
                name: "Room".into(),
                description: "A room.".into(),
                area: Entity::PLACEHOLDER,
            },
            GrimName("Room".into()),
        ))
        .id()
}

fn spawn_area(app: &mut App, friendly_id: &str, name: &str) -> Entity {
    app.world_mut()
        .spawn(Area {
            id: GrimId::new(),
            friendly_id: friendly_id.into(),
            name: name.into(),
        })
        .id()
}

fn spawn_room_in(app: &mut App, area: Entity, friendly_id: &str, name: &str) -> Entity {
    app.world_mut()
        .spawn((
            Room {
                id: GrimId::new(),
                friendly_id: friendly_id.into(),
                name: name.into(),
                description: "A room.".into(),
                area,
            },
            GrimName(name.into()),
        ))
        .id()
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

fn named_character(name: &str) -> StoredCharacter {
    let mut stored = make_character(vec![]);
    stored.name = name.into();
    stored
}

fn spawn_ingame_in(app: &mut App, conn: Entity, stored: StoredCharacter, room: Entity) -> Entity {
    let char_entity = spawn_ingame(app, conn, stored);
    app.world_mut()
        .entity_mut(char_entity)
        .insert(InRoom { room });
    char_entity
}

fn spawn_ingame(app: &mut App, conn: Entity, stored: StoredCharacter) -> Entity {
    let (name, actor, character) = stored.into_components();
    let char_entity = app
        .world_mut()
        .spawn((
            name,
            actor,
            character,
            InRoom {
                room: Entity::PLACEHOLDER,
            },
            Player { connection: conn },
        ))
        .id();
    let mut client = Client::new(conn);
    client.state = ClientState::InGame;
    client.character = Some(char_entity);
    let session = app.world_mut().spawn(client).id();
    // Mirror the world-entry push: in-game scene as child + stack top.
    let scene = app.world_mut().spawn(InGameScene).id();
    app.world_mut().entity_mut(session).add_child(scene);
    app.world_mut()
        .entity_mut(session)
        .insert(SceneStack(vec![scene]));
    char_entity
}

/// A flat [`StoredCharacter`] fixture ("Hero" by default). The post-split disk
/// surface; split into `Name + Actor + Character` when spawning a live entity.
fn make_character(roles: Vec<Role>) -> StoredCharacter {
    StoredCharacter {
        credits: 100,
        id: GrimId::new(),
        name: "Hero".into(),
        account_id: GrimId::new(),
        created_at: Utc::now(),
        last_room: None,
        roles,
        gender: Gender::Neutral,
        race: String::new(),
        class: String::new(),
        level: 1,
        title: None,
        restrings: std::collections::HashMap::new(),
        config: std::collections::HashMap::new(),
        game_state: Default::default(),
        inventory: Vec::new(),
    }
}

// ─── Copyover resume ─────────────────────

mod reconnect {
    use super::*;
    use grim_networking::ConnectionResumed;

    /// Copyover resume of a linkdead resident: the resident has `Character +
    /// Linkdead` and no `Player`; resuming it must attach a `Player` AND clear
    /// `Linkdead` in the same step, so it ends online and not linkdead.
    #[test]
    fn resume_linkdead_resident_clears_linkdead_and_attaches_player() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));

        let account = Account {
            testing: false,
            id: GrimId::new(),
            identifier: "resume@example.com".into(),
            password_hash: String::new(),
            characters: vec![],
            created_at: Utc::now(),
        };
        let account_id = account.id;
        app.world_mut().spawn(account);

        let mut stored = make_character(Vec::new());
        stored.name = "Test".into();
        stored.account_id = account_id;
        let (name, actor, character) = stored.into_components();
        let char_entity = app
            .world_mut()
            .spawn((name, actor, character, InRoom { room }, Linkdead))
            .id();

        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 7,
                addr: "127.0.0.1:12377".parse::<SocketAddr>().unwrap(),
                echo_hidden: false,
            })
            .id();
        app.world_mut().write_message(ConnectionResumed {
            connection: conn,
            character: "Test".into(),
            addr: "127.0.0.1:12377".parse().unwrap(),
        });
        app.update();

        let player = app
            .world()
            .get::<Player>(char_entity)
            .expect("resume must attach a Player (online)");
        assert_eq!(
            player.connection, conn,
            "resume must attach the resumed connection, not some other"
        );
        assert!(
            app.world().get::<Linkdead>(char_entity).is_none(),
            "resume must clear Linkdead — not both online and linkdead"
        );
    }

    // ─── Banned resume ─────────────────────
    mod ban_resume {
        use super::*;
        use grim_networking::ConnectionResumed;
        /// A banned character never resumes: no `Player`, no `Client`, the socket
        /// refused with the ban message and a disconnect.
        #[test]
        fn resume_refuses_banned_character() {
            let mut app = test_app();
            let room = spawn_room(&mut app);
            app.world_mut().insert_resource(StartingRoom(room));

            let account = Account {
                testing: false,
                id: GrimId::new(),
                identifier: "banned-resume@example.com".into(),
                password_hash: String::new(),
                characters: vec![],
                created_at: Utc::now(),
            };
            let account_id = account.id;
            app.world_mut().spawn(account);

            let mut stored = make_character(Vec::new());
            stored.name = "Doomed".into();
            stored.account_id = account_id;
            let (name, actor, character) = stored.into_components();
            let char_entity = app
                .world_mut()
                .spawn((name, actor, character, InRoom { room }, Linkdead))
                .id();
            // Run Startup so `PersistencePlugin` loads the (empty) `BanList`.
            app.update();
            app.world_mut()
                .resource_mut::<BanList>()
                .add(BanKind::Character, "doomed", "Root");

            let conn = app
                .world_mut()
                .spawn(Connection {
                    id: 8,
                    addr: "127.0.0.1:12378".parse::<SocketAddr>().unwrap(),
                    echo_hidden: false,
                })
                .id();
            app.world_mut().write_message(ConnectionResumed {
                connection: conn,
                character: "Doomed".into(),
                addr: "127.0.0.1:12378".parse().unwrap(),
            });
            app.update();

            assert!(
                app.world().get::<Player>(char_entity).is_none(),
                "banned resume attaches no Player"
            );
            let mut clients = app.world_mut().query::<&Client>();
            assert!(
                clients
                    .iter(app.world())
                    .find(|c| c.connection == conn)
                    .is_none(),
                "banned resume spawns no Client"
            );
            let msgs = app.world().resource::<Messages<ConnectionOutput>>();
            let mut cursor = msgs.get_cursor();
            assert!(
                cursor
                    .read(msgs)
                    .any(|o| o.connection == conn
                        && o.text.contains("Your character has been banned")),
                "banned resume shows the ban message"
            );
            let dus = app.world().resource::<Messages<DisconnectRequest>>();
            let mut dc = dus.get_cursor();
            assert!(
                dc.read(dus).any(|d| d.connection == conn),
                "banned resume severs the socket"
            );
        }

        /// A banned account never resumes, even for an unbanned character name.
        #[test]
        fn resume_refuses_banned_account() {
            let mut app = test_app();
            let room = spawn_room(&mut app);
            app.world_mut().insert_resource(StartingRoom(room));

            let account = Account {
                testing: false,
                id: GrimId::new(),
                identifier: "doomed-acct@example.com".into(),
                password_hash: String::new(),
                characters: vec![],
                created_at: Utc::now(),
            };
            let account_id = account.id;
            app.world_mut().spawn(account);

            let mut stored = make_character(Vec::new());
            stored.name = "Clean".into();
            stored.account_id = account_id;
            let (name, actor, character) = stored.into_components();
            let char_entity = app
                .world_mut()
                .spawn((name, actor, character, InRoom { room }, Linkdead))
                .id();
            // Run Startup so `PersistencePlugin` loads the (empty) `BanList`.
            app.update();
            app.world_mut().resource_mut::<BanList>().add(
                BanKind::Account,
                "doomed-acct@example.com",
                "Root",
            );

            let conn = app
                .world_mut()
                .spawn(Connection {
                    id: 9,
                    addr: "127.0.0.1:12379".parse::<SocketAddr>().unwrap(),
                    echo_hidden: false,
                })
                .id();
            app.world_mut().write_message(ConnectionResumed {
                connection: conn,
                character: "Clean".into(),
                addr: "127.0.0.1:12379".parse().unwrap(),
            });
            app.update();

            assert!(
                app.world().get::<Player>(char_entity).is_none(),
                "banned-account resume attaches no Player"
            );
            let msgs = app.world().resource::<Messages<ConnectionOutput>>();
            let mut cursor = msgs.get_cursor();
            assert!(
                cursor.read(msgs).any(
                    |o| o.connection == conn && o.text.contains("Your account has been banned")
                ),
                "banned-account resume shows the ban message"
            );
        }

        /// A banned IP never resumes, even for an otherwise clean identity.
        #[test]
        fn resume_refuses_banned_ip() {
            let mut app = test_app();
            let room = spawn_room(&mut app);
            app.world_mut().insert_resource(StartingRoom(room));

            let account = Account {
                testing: false,
                id: GrimId::new(),
                identifier: "clean-resume@example.com".into(),
                password_hash: String::new(),
                characters: vec![],
                created_at: Utc::now(),
            };
            let account_id = account.id;
            app.world_mut().spawn(account);

            let mut stored = make_character(Vec::new());
            stored.name = "Clean".into();
            stored.account_id = account_id;
            let (name, actor, character) = stored.into_components();
            let char_entity = app
                .world_mut()
                .spawn((name, actor, character, InRoom { room }, Linkdead))
                .id();
            // Run Startup so `PersistencePlugin` loads the (empty) `BanList`.
            app.update();
            app.world_mut()
                .resource_mut::<BanList>()
                .add(BanKind::Ip, "10.*", "Root");

            let conn = app
                .world_mut()
                .spawn(Connection {
                    id: 10,
                    addr: "10.9.9.9:12380".parse::<SocketAddr>().unwrap(),
                    echo_hidden: false,
                })
                .id();
            app.world_mut().write_message(ConnectionResumed {
                connection: conn,
                character: "Clean".into(),
                addr: "10.9.9.9:12380".parse().unwrap(),
            });
            app.update();

            assert!(
                app.world().get::<Player>(char_entity).is_none(),
                "banned-IP resume attaches no Player"
            );
            let msgs = app.world().resource::<Messages<ConnectionOutput>>();
            let mut cursor = msgs.get_cursor();
            assert!(
                cursor
                    .read(msgs)
                    .any(|o| o.connection == conn
                        && o.text.contains("Your IP address has been banned")),
                "banned-IP resume shows the ban message"
            );
            let alerts = app.world().resource::<Messages<WiznetAlert>>();
            let mut cursor = alerts.get_cursor();
            assert!(
                cursor
                    .read(alerts)
                    .any(|a| a.category == WiznetCategory::Security && a.text.contains("10.9.9.9")),
                "banned-IP resume emits a security alert with the peer IP"
            );
        }
    }
}

// ─── Output formatting & broadcast ───────

mod output_format {
    use super::*;

    /// Verify that format_output broadcasts ChannelMessage to room occupants.
    #[test]
    fn format_output_say_broadcast() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));

        let actor_conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let observer_conn = app
            .world_mut()
            .spawn(Connection {
                id: 2,
                addr: "127.0.0.1:12346".parse().unwrap(),
                echo_hidden: false,
            })
            .id();

        let actor = app
            .world_mut()
            .spawn((
                GrimName("Hero".into()),
                InRoom { room },
                Player {
                    connection: actor_conn,
                },
                OutputHistory::with_max(100),
            ))
            .id();
        let _observer = app
            .world_mut()
            .spawn((
                GrimName("Bystander".into()),
                InRoom { room },
                Player {
                    connection: observer_conn,
                },
                OutputHistory::with_max(100),
            ))
            .id();

        app.world_mut().write_message(ChannelMessage {
            origin: None,

            hidden_origin: false,
            channel: Channel {
                name: "say".to_string(),
                scope: grim_core::channel::Scope::Room,
                identify: grim_core::channel::Identify::Perceived,
                toggleable: false,
                speak: grim_core::channel::SpeakEligibility::All,
                listen: grim_core::channel::ListenEligibility::All,
                key: "channel.say".to_string(),
            },
            actor,
            text: "hello".into(),
        });
        app.world_mut().write_message(InfoMessage {
            target: actor,
            text: "You say, 'hello'\n".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let outputs: Vec<&ConnectionOutput> = cursor.read(msgs).collect();

        assert!(
            outputs
                .iter()
                .any(|o| o.connection == observer_conn && o.text.contains("Hero says")),
            "observer should get broadcast"
        );
        assert!(
            outputs
                .iter()
                .any(|o| o.connection == actor_conn && o.text.contains("You say")),
            "actor should get echo"
        );
    }

    /// Spawn bundle for an in-world character in output-format tests.
    fn char_bundle(
        name: &str,
        conn: Entity,
        room: Entity,
        roles: Vec<Role>,
    ) -> (GrimName, Actor, Character, InRoom, Player, OutputHistory) {
        let (gname, actor, character) = StoredCharacter {
            credits: 100,
            id: GrimId::new(),
            account_id: GrimId::new(),
            name: name.into(),
            created_at: Utc::now(),
            last_room: None,
            roles,
            gender: Gender::Neutral,
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
        (
            gname,
            actor,
            character,
            InRoom { room },
            Player { connection: conn },
            OutputHistory::with_max(100),
        )
    }

    /// Visible echoes reach everyone; hidden actions remain silent to mortals.
    /// Other admins always receive attribution, while the sender sees raw text.
    #[test]
    fn format_output_gecho_respects_attribution_and_hidden_actions() {
        for mode in ["visible", "wizinvis", "tester"] {
            let mut app = test_app();
            let room = spawn_room(&mut app);
            app.world_mut().insert_resource(StartingRoom(room));

            let mk_conn = |app: &mut App, id: usize, port: u16| -> Entity {
                app.world_mut()
                    .spawn(Connection {
                        id,
                        addr: format!("127.0.0.1:{port}").parse().unwrap(),
                        echo_hidden: false,
                    })
                    .id()
            };
            let sender_conn = mk_conn(&mut app, 1, 12345);
            let admin2_conn = mk_conn(&mut app, 2, 12346);
            let normal_conn = mk_conn(&mut app, 3, 12347);

            let sender = app
                .world_mut()
                .spawn(char_bundle("Boss", sender_conn, room, vec![Role::Admin]))
                .id();
            let _admin2 = app
                .world_mut()
                .spawn(char_bundle("Deputy", admin2_conn, room, vec![Role::Admin]))
                .id();
            let _normal = app
                .world_mut()
                .spawn(char_bundle("Peon", normal_conn, room, Vec::new()))
                .id();

            match mode {
                "wizinvis" => {
                    app.world_mut()
                        .get_mut::<Character>(sender)
                        .unwrap()
                        .game_state
                        .insert("wizinvis".into(), serde_json::json!(true));
                }
                "tester" => {
                    app.world_mut()
                        .get_mut::<Character>(sender)
                        .unwrap()
                        .roles
                        .push(Role::Tester);
                }
                _ => {}
            }
            app.world_mut().write_message(GlobalEcho {
                actor: sender,
                text: "reboot".into(),
            });
            app.update();

            let msgs = app.world().resource::<Messages<ConnectionOutput>>();
            let mut cursor = msgs.get_cursor();
            let outputs: Vec<&ConnectionOutput> = cursor.read(msgs).collect();
            let text_for = |conn: Entity| -> String {
                outputs
                    .iter()
                    .filter(|o| o.connection == conn)
                    .map(|o| o.text.clone())
                    .collect()
            };

            // Sender sees raw text, not attributed to themselves.
            let sender_text = text_for(sender_conn);
            assert!(sender_text.contains("reboot"), "sender should see the echo");
            assert!(
                !sender_text.contains("Boss>"),
                "sender should not see own name prefix"
            );
            // Another admin sees it attributed.
            assert!(
                text_for(admin2_conn).contains("Boss> reboot"),
                "other admin should see attributed echo"
            );
            // Non-admin sees raw text only.
            let normal_text = text_for(normal_conn);
            assert_eq!(
                normal_text.contains("reboot"),
                mode == "visible",
                "hidden staff actions must stay silent: {mode}"
            );
            assert!(
                !normal_text.contains("Boss>"),
                "non-admin should not see attribution"
            );
        }
    }

    /// Verify that format_output handles LoginAnnounce: only the subject's
    /// room hears it.
    #[test]
    fn format_output_login_announce() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        let elsewhere = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));

        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let hero = app
            .world_mut()
            .spawn((
                GrimName("Hero".into()),
                InRoom { room },
                Player { connection: conn },
                OutputHistory::with_max(100),
            ))
            .id();
        let far_conn = app
            .world_mut()
            .spawn(Connection {
                id: 2,
                addr: "127.0.0.1:12346".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        app.world_mut().spawn((
            GrimName("Bob".into()),
            InRoom { room: elsewhere },
            Player {
                connection: far_conn,
            },
            OutputHistory::with_max(100),
        ));

        app.world_mut().write_message(LoginAnnounce {
            subject: hero,
            name: "Hero".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let outputs: Vec<&ConnectionOutput> = cursor.read(msgs).collect();
        assert!(
            outputs
                .iter()
                .any(|o| o.connection == conn && o.text.contains("Hero has connected")),
            "same room should announce login"
        );
        assert!(
            outputs
                .iter()
                .all(|o| o.connection != far_conn || !o.text.contains("has connected")),
            "other rooms must not hear it"
        );
    }

    /// A `ServerBroadcast` reaches every connected player (shutdown warnings).
    #[test]
    fn server_broadcast_reaches_connected_players() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));

        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        app.world_mut().spawn((
            GrimName("Hero".into()),
            InRoom { room },
            Player { connection: conn },
            OutputHistory::with_max(100),
        ));

        app.world_mut().write_message(ServerBroadcast {
            text: "{R[SERVER]{x Restarting in {Y15{x seconds.\n".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let outputs: Vec<&ConnectionOutput> = cursor.read(msgs).collect();
        assert!(
            outputs
                .iter()
                .any(|o| o.connection == conn && o.text.contains("Restarting in")),
            "connected player should receive the broadcast"
        );
    }

    /// Verify that format_output handles LogoutAnnounce: only the room on
    /// the message hears it (the quitter's entity is already despawned).
    #[test]
    fn format_output_logout_announce() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        let elsewhere = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));

        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        app.world_mut().spawn((
            GrimName("Witness".into()),
            InRoom { room },
            Player { connection: conn },
            OutputHistory::with_max(100),
        ));
        let far_conn = app
            .world_mut()
            .spawn(Connection {
                id: 2,
                addr: "127.0.0.1:12346".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        app.world_mut().spawn((
            GrimName("Bob".into()),
            InRoom { room: elsewhere },
            Player {
                connection: far_conn,
            },
            OutputHistory::with_max(100),
        ));

        app.world_mut().write_message(LogoutAnnounce {
            name: "Hero".into(),
            room,
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let outputs: Vec<&ConnectionOutput> = cursor.read(msgs).collect();
        assert!(
            outputs
                .iter()
                .any(|o| o.connection == conn && o.text.contains("Hero has disconnected")),
            "same room should announce logout"
        );
        assert!(
            outputs
                .iter()
                .all(|o| o.connection != far_conn || !o.text.contains("has disconnected")),
            "other rooms must not hear it"
        );
    }

    /// Verify that format_output handles LinkdeadAnnounce: only the
    /// subject's room hears it.
    #[test]
    fn format_output_linkdead_announce() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        let elsewhere = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));

        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let hero = app
            .world_mut()
            .spawn((
                GrimName("Hero".into()),
                InRoom { room },
                Player { connection: conn },
                OutputHistory::with_max(100),
            ))
            .id();
        let far_conn = app
            .world_mut()
            .spawn(Connection {
                id: 2,
                addr: "127.0.0.1:12346".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        app.world_mut().spawn((
            GrimName("Bob".into()),
            InRoom { room: elsewhere },
            Player {
                connection: far_conn,
            },
            OutputHistory::with_max(100),
        ));

        app.world_mut().write_message(LinkdeadAnnounce {
            subject: hero,
            name: "Hero".into(),
            reconnecting: true,
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let outputs: Vec<&ConnectionOutput> = cursor.read(msgs).collect();
        assert!(
            outputs
                .iter()
                .any(|o| o.connection == conn && o.text.contains("Hero has reconnected")),
            "same room should announce reconnect"
        );
        assert!(
            outputs
                .iter()
                .all(|o| o.connection != far_conn || !o.text.contains("has reconnected")),
            "other rooms must not hear it"
        );
    }

    // ── format_output: look_room with missing room ──
    #[test]
    fn format_output_look_room_room_not_found() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));

        // Write LookRoom for a non-existent room → should not panic
        app.world_mut().write_message(LookRoom {
            target: Entity::PLACEHOLDER,
            room: Entity::PLACEHOLDER,
        });
        app.update();

        // No crash = success
    }

    // ── format_output: admins see room ids in the title, players don't ──
    #[test]
    fn format_output_admin_sees_room_ids() {
        for (admin, expect_ids) in [(true, true), (false, false)] {
            let mut app = test_app();
            let room = spawn_room(&mut app);
            app.world_mut().insert_resource(StartingRoom(room));
            let conn = app
                .world_mut()
                .spawn(Connection {
                    id: 1,
                    addr: "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
                    echo_hidden: false,
                })
                .id();
            let roles = if admin { vec![Role::Admin] } else { vec![] };
            let target = spawn_ingame(&mut app, conn, make_character(roles));
            app.world_mut().write_message(LookRoom { target, room });
            app.update();

            let msgs = app.world().resource::<Messages<ConnectionOutput>>();
            let mut cursor = msgs.get_cursor();
            let text: String = cursor
                .read(msgs)
                .filter(|o| o.connection == conn)
                .map(|o| o.text.clone())
                .collect();

            if expect_ids {
                assert!(
                    text.contains("entity:")
                        && text.contains("grim:")
                        && text.contains("slug:room1"),
                    "admin should see room ids; got:\n{text}"
                );
            } else {
                assert!(
                    !text.contains("entity:") && !text.contains("grim:") && !text.contains("slug:"),
                    "normal player must not see room ids; got:\n{text}"
                );
            }
        }
    }

    // ── format_output: look_entity with missing subject name ──
    #[test]
    fn format_output_look_entity_not_found() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));

        // Entity with no GrimName component → lookup fails, format_output continues
        let nameless = app.world_mut().spawn_empty().id();
        app.world_mut().write_message(LookEntity {
            target: Entity::PLACEHOLDER,
            subject: nameless,
        });
        app.update();

        // No crash = success
    }

    // ── format_output: move broadcasts to from/to rooms ──
    #[test]
    fn format_output_move_broadcasts() {
        let mut app = test_app();
        let from_room = spawn_room(&mut app);
        let to_room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(from_room));

        let actor_conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let observer_conn = app
            .world_mut()
            .spawn(Connection {
                id: 2,
                addr: "127.0.0.1:12346".parse().unwrap(),
                echo_hidden: false,
            })
            .id();

        let actor = app
            .world_mut()
            .spawn((
                GrimName("Mover".into()),
                InRoom { room: from_room },
                Player {
                    connection: actor_conn,
                },
                OutputHistory::with_max(100),
            ))
            .id();
        let _observer = app
            .world_mut()
            .spawn((
                GrimName("Watcher".into()),
                InRoom { room: from_room },
                Player {
                    connection: observer_conn,
                },
                OutputHistory::with_max(100),
            ))
            .id();

        app.world_mut().write_message(MoveEvent {
            actor,
            from: from_room,
            to: to_room,
            direction: grim_core::cardinal::Cardinal::North,
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let outputs: Vec<&ConnectionOutput> = cursor.read(msgs).collect();
        // Observer in from_room should see departure
        assert!(
            outputs
                .iter()
                .any(|o| o.connection == observer_conn && o.text.contains("Mover leaves")),
            "Observer should see departure message"
        );
    }

    // ── format_output: door facts render per-recipient ──
    #[test]
    fn format_output_door_broadcasts() {
        let mut app = test_app();
        let from_room = spawn_room(&mut app);
        let to_room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(from_room));
        let mk_conn = |app: &mut App, id: usize| {
            app.world_mut()
                .spawn(Connection {
                    id,
                    addr: format!("127.0.0.1:{id}").parse().unwrap(),
                    echo_hidden: false,
                })
                .id()
        };
        let mk_watcher = |app: &mut App, room: Entity, name: &str, conn: Entity| {
            app.world_mut()
                .spawn((
                    GrimName(name.into()),
                    InRoom { room },
                    Player { connection: conn },
                    OutputHistory::with_max(100),
                ))
                .id()
        };
        let actor_conn = mk_conn(&mut app, 4001);
        let witness_conn = mk_conn(&mut app, 4002);
        let far_conn = mk_conn(&mut app, 4003);
        let actor = mk_watcher(&mut app, from_room, "Alice", actor_conn);
        mk_watcher(&mut app, from_room, "Witness", witness_conn);
        mk_watcher(&mut app, to_room, "Far", far_conn);
        app.world_mut().write_message(DoorEvent {
            actor,
            from: from_room,
            to: to_room,
            direction: grim_core::cardinal::Cardinal::East,
            opened: true,
            name: "the privy door".into(),
        });
        app.update();
        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let outputs: Vec<&ConnectionOutput> = cursor.read(msgs).collect();
        let text_for = |conn: Entity| {
            outputs
                .iter()
                .filter(|o| o.connection == conn)
                .map(|o| o.text.clone())
                .collect::<Vec<_>>()
                .join("\n")
        };
        // Actor + witness name their side; the far room hears the opposite
        // side with a sentence-cased door name.
        assert!(text_for(actor_conn).contains("You open the privy door to the east."));
        assert!(text_for(witness_conn).contains("Alice opens the privy door to the east."));
        assert!(text_for(far_conn).contains("The privy door opens to the west."));
    }

    // ── recall echoes: attempt + disappearance left, marked arrival right ──
    #[test]
    fn format_output_recall_broadcasts() {
        let mut app = test_app();
        let from_room = spawn_room(&mut app);
        let to_room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(from_room));
        let mk_conn = |app: &mut App, id: usize| {
            app.world_mut()
                .spawn(Connection {
                    id,
                    addr: "127.0.0.1:12345".parse().unwrap(),
                    echo_hidden: false,
                })
                .id()
        };
        let mk_watcher = |app: &mut App, room: Entity, name: &str, conn: Entity| {
            app.world_mut()
                .spawn((
                    GrimName(name.into()),
                    InRoom { room },
                    Player { connection: conn },
                    OutputHistory::with_max(100),
                ))
                .id()
        };
        let actor_conn = mk_conn(&mut app, 1);
        let left_conn = mk_conn(&mut app, 2);
        let arrived_conn = mk_conn(&mut app, 3);
        let actor = mk_watcher(&mut app, from_room, "Mover", actor_conn);
        mk_watcher(&mut app, from_room, "LeftBehind", left_conn);
        mk_watcher(&mut app, to_room, "Greeter", arrived_conn);

        app.world_mut().write_message(RecallEvent {
            actor,
            from: from_room,
            to: to_room,
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let outputs: Vec<&ConnectionOutput> = cursor.read(msgs).collect();
        let texts = |conn: Entity| {
            outputs
                .iter()
                .filter(|o| o.connection == conn)
                .map(|o| o.text.as_str())
                .collect::<Vec<_>>()
                .join("")
        };
        // The room left sees the attempt, then the disappearance, in order.
        let left = texts(left_conn);
        let attempt = left.find("closes their eyes").expect("attempt echo");
        let vanish = left.find("disappears").expect("vanish echo");
        assert!(attempt < vanish, "attempt precedes disappearance:\n{left}");
        // The room entered sees a recall-marked arrival, not a walk-in.
        let arrived = texts(arrived_conn);
        assert!(
            arrived.contains("appears in a flash of light"),
            "arrival should be recall-marked:\n{arrived}"
        );
        // The recaller sees neither echo (they get the arrival description).
        let own = texts(actor_conn);
        assert!(
            !own.contains("disappears") && !own.contains("flash of light"),
            "recaller must not see their own echoes:\n{own}"
        );
    }

    // ── transfer events: mover / other / room all see named lines ──
    #[test]
    fn format_transfer_events_echoes_all_parties() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let mk_conn = |app: &mut App, id: usize| {
            app.world_mut()
                .spawn(Connection {
                    id,
                    addr: "127.0.0.1:12345".parse().unwrap(),
                    echo_hidden: false,
                })
                .id()
        };
        let mover_conn = mk_conn(&mut app, 1);
        let other_conn = mk_conn(&mut app, 2);
        let watcher_conn = mk_conn(&mut app, 3);
        let mover = app
            .world_mut()
            .spawn((
                GrimName("Alice".into()),
                InRoom { room },
                Player {
                    connection: mover_conn,
                },
            ))
            .id();
        let (_bob_name, bob_actor, bob_char) = make_character(Vec::new()).into_components();
        let other = app
            .world_mut()
            .spawn((
                GrimName("Bob".into()),
                bob_actor,
                bob_char,
                InRoom { room },
                Player {
                    connection: other_conn,
                },
            ))
            .id();
        app.world_mut().spawn((
            GrimName("Cara".into()),
            InRoom { room },
            Player {
                connection: watcher_conn,
            },
        ));

        for kind in [TransferKind::Give, TransferKind::Steal] {
            app.world_mut().write_message(TransferEvent {
                mover,
                mover_name: "Alice".into(),
                other,
                other_name: "Bob".into(),
                room,
                short: "brass lantern".into(),
                kind,
            });
        }
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let mut by_conn: std::collections::HashMap<Entity, String> =
            std::collections::HashMap::new();
        for o in cursor.read(msgs) {
            by_conn.entry(o.connection).or_default().push_str(&o.text);
        }
        let mover_text = &by_conn[&mover_conn];
        assert!(
            mover_text.contains("You give brass lantern to Bob"),
            "mover give line; got:\n{mover_text}"
        );
        assert!(
            mover_text.contains("You steal brass lantern from Bob"),
            "mover steal line; got:\n{mover_text}"
        );
        let other_text = &by_conn[&other_conn];
        assert!(
            other_text.contains("Alice gives you brass lantern"),
            "recipient line; got:\n{other_text}"
        );
        assert!(
            other_text.contains("Alice steals your brass lantern"),
            "victim line; got:\n{other_text}"
        );
        let watcher_text = &by_conn[&watcher_conn];
        assert!(
            watcher_text.contains("Alice gives brass lantern to Bob"),
            "room give line; got:\n{watcher_text}"
        );
        assert!(
            watcher_text.contains("Alice steals brass lantern from Bob"),
            "room steal line; got:\n{watcher_text}"
        );
    }

    // ── look pack: beings show inventory below the description ──
    #[test]
    fn look_shows_subject_pack_below_description() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let viewer = spawn_ingame(&mut app, conn, make_character(Vec::new()));
        app.world_mut().entity_mut(viewer).insert(InRoom { room });
        let mut bob = make_character(Vec::new());
        bob.name = "Bob".into();
        let (bob_name, bob_actor, bob_char) = bob.into_components();
        let bob_entity = app
            .world_mut()
            .spawn((
                bob_name,
                bob_actor,
                bob_char,
                Description(vec!["A sturdy warrior.".into()]),
                InRoom { room },
            ))
            .id();
        app.world_mut().spawn((
            Object,
            GrimName("brass lantern".into()),
            CarriedBy {
                carrier: bob_entity,
            },
        ));

        app.world_mut().write_message(LookEntity {
            target: viewer,
            subject: bob_entity,
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let mine: Vec<(bool, String)> = cursor
            .read(msgs)
            .filter(|o| o.connection == conn)
            .map(|o| (o.prepend_newline, o.text.clone()))
            .collect();
        assert_eq!(mine.len(), 2, "description plus pack; got:\n{mine:?}");
        assert!(
            mine[0].1.contains("A sturdy warrior."),
            "description first; got:\n{mine:?}"
        );
        // `prepend_newline` renders as the blank line on the wire (see
        // `render_output`): the pack block lands below the description.
        assert!(mine[1].0, "pack block is wire-separated; got:\n{mine:?}");
        assert!(
            mine[1].1.contains("Bob is carrying:\n  brass lantern"),
            "pack listing; got:\n{mine:?}"
        );
    }

    // ── look_room presence lines: AFK players are marked ──
    #[test]
    fn look_room_marks_afk_players() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let mk_conn = |app: &mut App, id: usize| {
            app.world_mut()
                .spawn(Connection {
                    id,
                    addr: "127.0.0.1:12345".parse().unwrap(),
                    echo_hidden: false,
                })
                .id()
        };
        let viewer_conn = mk_conn(&mut app, 1);
        let viewer = spawn_ingame(&mut app, viewer_conn, make_character(Vec::new()));
        let mut zara = make_character(Vec::new());
        zara.name = "Zara".into();
        let zara_conn = mk_conn(&mut app, 2);
        let zara_entity = spawn_ingame(&mut app, zara_conn, zara);
        for e in [viewer, zara_entity] {
            app.world_mut().entity_mut(e).insert(InRoom { room });
        }
        // Flag Zara's session AFK (as the `afk` verb or idle sweep would).
        {
            let mut qs = app.world_mut().query::<&mut Client>();
            let world = app.world_mut();
            let mut client = qs
                .iter_mut(world)
                .find(|c| c.character == Some(zara_entity))
                .expect("zara session");
            client.afk = true;
        }

        app.world_mut().write_message(LookRoom {
            target: viewer,
            room,
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let text: String = cursor
            .read(msgs)
            .filter(|o| o.connection == viewer_conn)
            .map(|o| o.text.clone())
            .collect();
        assert!(
            text.contains("Zara (AFK) is standing here."),
            "AFK marker on presence line; got:\n{text}"
        );
    }

    // ── look_room presence lines: linkdead players are marked ──
    #[test]
    fn look_room_marks_linkdead_players() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let mk_conn = |app: &mut App, id: usize| {
            app.world_mut()
                .spawn(Connection {
                    id,
                    addr: "127.0.0.1:12345".parse().unwrap(),
                    echo_hidden: false,
                })
                .id()
        };
        let viewer_conn = mk_conn(&mut app, 1);
        let viewer = spawn_ingame(&mut app, viewer_conn, make_character(Vec::new()));
        let mut zara = make_character(Vec::new());
        zara.name = "Zara".into();
        let zara_conn = mk_conn(&mut app, 2);
        let zara_entity = spawn_ingame(&mut app, zara_conn, zara);
        for e in [viewer, zara_entity] {
            app.world_mut().entity_mut(e).insert(InRoom { room });
        }
        // Drop Zara's session: linkdead (Character, no Player) + InRoom.
        app.world_mut()
            .entity_mut(zara_entity)
            .remove::<Player>()
            .insert(Linkdead);

        app.world_mut().write_message(LookRoom {
            target: viewer,
            room,
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let text: String = cursor
            .read(msgs)
            .filter(|o| o.connection == viewer_conn)
            .map(|o| o.text.clone())
            .collect();
        assert!(
            text.contains("Zara (Linkdead) is standing here."),
            "Linkdead marker on presence line; got:\n{text}"
        );
    }

    // ── look_room presence lines: players above creatures, sorted ──
    #[test]
    fn look_room_lists_players_above_creatures() {
        use grim_actor::Creature;

        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let mk_conn = |app: &mut App, id: usize| {
            app.world_mut()
                .spawn(Connection {
                    id,
                    addr: "127.0.0.1:12345".parse().unwrap(),
                    echo_hidden: false,
                })
                .id()
        };
        let viewer_conn = mk_conn(&mut app, 1);
        let viewer = spawn_ingame(&mut app, viewer_conn, make_character(Vec::new()));
        // The other player, sorted by name (Zara after Hero, irrelevant here).
        let mut zara = make_character(Vec::new());
        zara.name = "Zara".into();
        let zara_conn = mk_conn(&mut app, 2);
        let zara_entity = spawn_ingame(&mut app, zara_conn, zara);
        for e in [viewer, zara_entity] {
            app.world_mut()
                .entity_mut(e)
                .insert(InRoom { room })
                .insert(Description(vec!["A hero.".into()]));
        }
        // Creatures: one with a long room line, one falling back.
        app.world_mut().spawn((
            GrimName("Grimmok Ironhand".into()),
            Creature,
            RoomDescription("Grimmok Ironhand stands here, hammering metal.".into()),
            InRoom { room },
        ));
        app.world_mut()
            .spawn((GrimName("Goblin".into()), Creature, InRoom { room }));

        app.world_mut().write_message(LookRoom {
            target: viewer,
            room,
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let text: String = cursor
            .read(msgs)
            .filter(|o| o.connection == viewer_conn)
            .map(|o| o.text.clone())
            .collect();
        assert!(
            !text.contains("Also here:"),
            "no summary line; got:\n{text}"
        );
        let zara_at = text.find("Zara is standing here.").expect("player line");
        let goblin_at = text.find("{GGoblin{x is here.").expect("fallback line");
        let grimmok_at = text
            .find("{GGrimmok Ironhand{x stands here, hammering metal.")
            .expect("room line");
        assert!(
            zara_at < goblin_at && zara_at < grimmok_at,
            "players sort above creatures; got:\n{text}"
        );
        assert!(
            goblin_at < grimmok_at,
            "creatures sort by name; got:\n{text}"
        );
    }

    // ── look_room presence lines: objects below creatures, no blank line ──
    #[test]
    fn look_room_lists_objects_below_creatures() {
        use grim_actor::Creature;

        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let viewer = spawn_ingame(&mut app, conn, make_character(Vec::new()));
        app.world_mut().entity_mut(viewer).insert(InRoom { room });
        app.world_mut().spawn((
            GrimName("Grimmok Ironhand".into()),
            Creature,
            RoomDescription("Grimmok Ironhand stands here, hammering metal.".into()),
            InRoom { room },
        ));
        app.world_mut().spawn((
            Object,
            GrimName("brass lantern".into()),
            RoomDescription("A brass lantern rests here.".into()),
            InRoom { room },
        ));
        // Carried objects never list, even held by someone in the room.
        app.world_mut().spawn((
            Object,
            GrimName("coin".into()),
            RoomDescription("A coin glints.".into()),
            CarriedBy { carrier: viewer },
        ));

        app.world_mut().write_message(LookRoom {
            target: viewer,
            room,
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let text: String = cursor
            .read(msgs)
            .filter(|o| o.connection == conn)
            .map(|o| o.text.clone())
            .collect();
        let creature_at = text
            .find("{GGrimmok Ironhand{x stands here, hammering metal.")
            .expect("creature line");
        let object_at = text
            .find("A brass lantern rests here.")
            .expect("object line");
        assert!(
            creature_at < object_at,
            "objects list under creatures; got:\n{text}"
        );
        assert!(
            text.contains("hammering metal.\n           {mA brass lantern rests here.{x"),
            "no blank line between creatures and objects; got:\n{text}"
        );
        assert!(
            !text.contains("coin"),
            "carried objects never list; got:\n{text}"
        );
    }

    // ── look_entity joins paragraphs with single newlines ──
    #[test]
    fn look_entity_joins_paragraphs_with_newlines() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let viewer = spawn_ingame(&mut app, conn, make_character(Vec::new()));
        let subject = app
            .world_mut()
            .spawn((
                GrimName("Statue".into()),
                Description(vec!["Para one.".into(), "Para two.".into()]),
                InRoom { room },
            ))
            .id();

        app.world_mut().write_message(LookEntity {
            target: viewer,
            subject,
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let text: String = cursor
            .read(msgs)
            .filter(|o| o.connection == conn)
            .map(|o| o.text.clone())
            .collect();
        assert!(
            text.contains("Statue\nPara one.\nPara two.\n"),
            "paragraphs join with single newlines; got:\n{text}"
        );
    }

    // ── look_room: closed doors list under Doors, open doors under Exits ──
    #[test]
    fn look_room_splits_closed_doors_from_exits() {
        use grim_core::cardinal::Cardinal;
        use grim_world::{Door, Doors, Exits};
        use std::collections::HashMap;

        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        // All six exits, scrambled; east + down are closed doors, north is
        // an open door (stays in Exits), the rest are plain exits.
        let far = spawn_room(&mut app);
        let mut exits = HashMap::new();
        for dir in [
            Cardinal::Down,
            Cardinal::West,
            Cardinal::South,
            Cardinal::East,
            Cardinal::North,
            Cardinal::Up,
        ] {
            exits.insert(dir, far);
        }
        app.world_mut().entity_mut(room).insert(Exits { exits });
        let mut doors = HashMap::new();
        for (dir, open) in [
            (Cardinal::East, false),
            (Cardinal::Down, false),
            (Cardinal::North, true),
        ] {
            doors.insert(
                dir,
                Door {
                    name: "a door".into(),
                    keywords: vec![],
                    open,
                    hidden: false,
                },
            );
        }
        app.world_mut().entity_mut(room).insert(Doors { doors });
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let viewer = spawn_ingame(&mut app, conn, make_character(Vec::new()));
        // Bypass the minimap gutter so the lines assert plain.
        app.world_mut()
            .get_mut::<Character>(viewer)
            .unwrap()
            .config
            .insert("minimap".into(), "off".into());
        app.world_mut().write_message(LookRoom {
            target: viewer,
            room,
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let text: String = cursor
            .read(msgs)
            .filter(|o| o.connection == conn)
            .map(|o| o.text.clone())
            .collect();
        assert!(
            text.contains("Exits: {gnorth{x {gsouth{x {gwest{x {gup{x  Doors: {geast{x {gdown{x\n"),
            "open door + plain exits in NESWUD order on one line; got:\n{text}"
        );
    }

    // ── look_room: hidden exits list under Secret for admins only ──
    #[test]
    fn look_room_hides_secret_exits_from_players() {
        use grim_core::cardinal::Cardinal;
        use grim_world::{Door, Doors, Exits};
        use std::collections::HashMap;

        for (admin, expected) in [
            (false, "Exits: {gnorth{x  Doors: none\n"),
            (true, "Exits: {gnorth{x  Doors: none  Secret: {gdown{x\n"),
        ] {
            let mut app = test_app();
            let room = spawn_room(&mut app);
            app.world_mut().insert_resource(StartingRoom(room));
            let far = spawn_room(&mut app);
            let mut exits = HashMap::new();
            exits.insert(Cardinal::North, far);
            exits.insert(Cardinal::Down, far);
            app.world_mut().entity_mut(room).insert(Exits { exits });
            let mut doors = HashMap::new();
            doors.insert(
                Cardinal::Down,
                Door {
                    name: "a secret door".into(),
                    keywords: vec![],
                    open: false,
                    hidden: true,
                },
            );
            app.world_mut().entity_mut(room).insert(Doors { doors });
            let conn = app
                .world_mut()
                .spawn(Connection {
                    id: 1,
                    addr: "127.0.0.1:12345".parse().unwrap(),
                    echo_hidden: false,
                })
                .id();
            let roles = if admin { vec![Role::Admin] } else { vec![] };
            let viewer = spawn_ingame(&mut app, conn, make_character(roles));
            app.world_mut()
                .get_mut::<Character>(viewer)
                .unwrap()
                .config
                .insert("minimap".into(), "off".into());
            app.world_mut().write_message(LookRoom {
                target: viewer,
                room,
            });
            app.update();

            let msgs = app.world().resource::<Messages<ConnectionOutput>>();
            let mut cursor = msgs.get_cursor();
            let text: String = cursor
                .read(msgs)
                .filter(|o| o.connection == conn)
                .map(|o| o.text.clone())
                .collect();
            assert!(
                text.contains(expected),
                "admin={admin}: expected {expected:?}; got:\n{text}"
            );
            if !admin {
                assert!(!text.contains("down"), "secret must not leak; got:\n{text}");
            }
        }
    }

    // ── look_room: an open secret lists under Exits (closed ones hide) ──
    #[test]
    fn look_room_open_secret_lists_under_exits() {
        use grim_core::cardinal::Cardinal;
        use grim_world::{Door, Doors, Exits};
        use std::collections::HashMap;

        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let far = spawn_room(&mut app);
        let mut exits = HashMap::new();
        exits.insert(Cardinal::Down, far);
        app.world_mut().entity_mut(room).insert(Exits { exits });
        let mut doors = HashMap::new();
        doors.insert(
            Cardinal::Down,
            Door {
                name: "a secret door".into(),
                keywords: vec![],
                open: true,
                hidden: true,
            },
        );
        app.world_mut().entity_mut(room).insert(Doors { doors });
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let viewer = spawn_ingame(&mut app, conn, make_character(Vec::new()));
        app.world_mut()
            .get_mut::<Character>(viewer)
            .unwrap()
            .config
            .insert("minimap".into(), "off".into());
        app.world_mut().write_message(LookRoom {
            target: viewer,
            room,
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let text: String = cursor
            .read(msgs)
            .filter(|o| o.connection == conn)
            .map(|o| o.text.clone())
            .collect();
        assert!(
            text.contains("Exits: {gdown{x  Doors: none\n"),
            "open secret lists under Exits like any open door; got:\n{text}"
        );
    }
}

// ─── In-game command dispatch ────────────

mod ingame_commands {
    use super::*;

    // ── handle_client_input: InGame with unknown command ──
    #[test]
    fn ingame_unknown_command_shows_error() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));

        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();

        let char_entity = app
            .world_mut()
            .spawn((
                GrimName("Hero".into()),
                InRoom { room },
                Player { connection: conn },
            ))
            .id();

        let mut client = Client::new(conn);
        client.state = ClientState::InGame;
        client.character = Some(char_entity);
        let session = app.world_mut().spawn(client).id();
        let scene = app.world_mut().spawn(InGameScene).id();
        app.world_mut().entity_mut(session).add_child(scene);
        app.world_mut()
            .entity_mut(session)
            .insert(SceneStack(vec![scene]));

        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "blargh".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let outputs: Vec<&ConnectionOutput> = cursor.read(msgs).collect();
        assert!(
            outputs
                .iter()
                .any(|o| o.connection == conn && o.text.contains("Unknown command")),
            "Unknown command should show error message"
        );
    }

    /// Same-tick open: `desc edit` followed by a text line in one update must
    /// append the line, not dispatch it as a command. (A deferred open — e.g.
    /// via a component inserted by `Commands` — would miss the second line,
    /// which would execute as an ordinary command instead.)
    #[test]
    fn ingame_editor_open_is_visible_to_next_line_same_tick() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let char_entity = spawn_ingame(&mut app, conn, make_character(Vec::new()));
        app.world_mut()
            .entity_mut(char_entity)
            .insert(InRoom { room })
            .insert(Description(vec!["Seed line.".into()]));

        // Both lines land before a single update: open, then a line that
        // would otherwise dispatch (and move!) as a command.
        for text in ["desc edit", "north"] {
            app.world_mut().write_message(ConnectionInput {
                connection: conn,
                text: text.into(),
            });
        }
        app.update();

        // The editor opened with the seed line preloaded, and "north"
        // appended to the buffer instead of walking anywhere.
        let mut clients = app.world_mut().query::<&Client>();
        let editor = clients
            .iter(app.world())
            .find(|c| c.character == Some(char_entity))
            .and_then(|c| c.editor.clone())
            .expect("editor open");
        assert_eq!(
            editor.buffer,
            vec!["Seed line.".to_string(), "north".into()]
        );
        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        assert!(
            cursor
                .read(msgs)
                .all(|o| !o.text.contains("Unknown command")),
            "second line must not dispatch as a command"
        );
        let mut inroom = app.world_mut().query::<&InRoom>();
        assert_eq!(
            inroom.get(app.world(), char_entity).unwrap().room,
            room,
            "no movement happened"
        );
    }

    /// Same-tick close: `@save` followed by `look` in one update must
    /// dispatch the look (queued for cooldown) rather than swallow it as
    /// editor text. (A deferred close would still find the session and eat it.)
    #[test]
    fn ingame_editor_close_is_visible_to_next_line_same_tick() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let char_entity = spawn_ingame(&mut app, conn, make_character(Vec::new()));
        app.world_mut()
            .entity_mut(char_entity)
            .insert(InRoom { room });

        // Force the session into the editor with a preloaded line.
        {
            let mut clients = app.world_mut().query::<&mut Client>();
            let mut client = clients
                .iter_mut(app.world_mut())
                .find(|c| c.character == Some(char_entity))
                .expect("session");
            client.editor = Some(grim_core::components::EditorSession {
                character: char_entity,
                kind: grim_core::events::EditorKind::Description,
                buffer: vec!["Saved line.".into()],
            });
        }
        for text in ["@save", "look"] {
            app.world_mut().write_message(ConnectionInput {
                connection: conn,
                text: text.into(),
            });
        }
        app.update();

        // Closed synchronously: the save carried only the preloaded line, and
        // `look` queued as a command for the cooldown drain.
        let dones = app.world().resource::<Messages<EditorDone>>();
        let mut cursor = dones.get_cursor();
        let done = cursor.read(dones).next().expect("EditorDone");
        assert_eq!(done.lines, Some(vec!["Saved line.".to_string()]));
        let mut clients = app.world_mut().query::<&Client>();
        let client = clients
            .iter(app.world())
            .find(|c| c.character == Some(char_entity))
            .expect("session");
        assert!(client.editor.is_none(), "closed");
        assert!(
            client
                .input_queue
                .iter()
                .any(|c| matches!(c, Command::Look { .. })),
            "look queued as a command, not editor text"
        );
    }

    /// A non-admin `shutdown` is indistinguishable from an unknown command:
    /// same text, and the same framing (direct output, no prepended newline).
    #[test]
    fn ingame_shutdown_masked_for_non_admin() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        spawn_ingame(&mut app, conn, make_character(Vec::new()));

        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "shutdown 30".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let out = cursor
            .read(msgs)
            .find(|o| o.connection == conn)
            .expect("expected a response");
        assert_eq!(out.text, "Unknown command. Type 'commands' for a list.\n");
        assert!(!out.prepend_newline, "must match unknown-command framing");

        // And the command was not forwarded to the engine.
        let engine = app.world().resource::<Messages<EngineCommand>>();
        assert_eq!(engine.get_cursor().read(engine).count(), 0);
    }

    /// An admin `shutdown` is accepted (queued), never masked.
    #[test]
    fn ingame_shutdown_allowed_for_admin() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        spawn_ingame(&mut app, conn, make_character(vec![Role::Admin]));

        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "shutdown 30".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        assert!(
            !cursor
                .read(msgs)
                .any(|o| o.connection == conn && o.text.contains("Unknown command")),
            "admin shutdown must not be masked"
        );

        // Positively confirm it was accepted (a silent drop would still pass the
        // not-masked check above). Depending on the command cooldown, after one
        // update it is either still queued or already dispatched as an
        // EngineCommand — accept either so the test doesn't depend on timing.
        let engine = app.world().resource::<Messages<EngineCommand>>();
        let dispatched = engine
            .get_cursor()
            .read(engine)
            .any(|e| matches!(e.command, Command::Shutdown { seconds: 30 }));
        let mut clients = app.world_mut().query::<&Client>();
        let queued = clients
            .iter(app.world())
            .find(|c| c.connection == conn)
            .is_some_and(|c| {
                matches!(
                    c.input_queue.front(),
                    Some(Command::Shutdown { seconds: 30 })
                )
            });
        assert!(
            queued || dispatched,
            "admin shutdown should be queued or dispatched, not dropped"
        );
    }

    /// A non-admin `reboot` is masked exactly like an unknown command and never
    /// forwarded to the engine.
    #[test]
    fn ingame_reboot_masked_for_non_admin() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        spawn_ingame(&mut app, conn, make_character(Vec::new()));

        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "reboot 10".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let out = cursor
            .read(msgs)
            .find(|o| o.connection == conn)
            .expect("expected a response");
        assert_eq!(out.text, "Unknown command. Type 'commands' for a list.\n");
        assert!(!out.prepend_newline, "must match unknown-command framing");

        let engine = app.world().resource::<Messages<EngineCommand>>();
        assert_eq!(engine.get_cursor().read(engine).count(), 0);
    }

    /// An admin `reboot` is accepted (queued), never masked.
    #[test]
    fn ingame_reboot_allowed_for_admin() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        spawn_ingame(&mut app, conn, make_character(vec![Role::Admin]));

        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "reboot 10".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        assert!(
            !cursor
                .read(msgs)
                .any(|o| o.connection == conn && o.text.contains("Unknown command")),
            "admin reboot must not be masked"
        );

        let engine = app.world().resource::<Messages<EngineCommand>>();
        let dispatched = engine
            .get_cursor()
            .read(engine)
            .any(|e| matches!(e.command, Command::Reboot { seconds: 10 }));
        let mut clients = app.world_mut().query::<&Client>();
        let queued = clients
            .iter(app.world())
            .find(|c| c.connection == conn)
            .is_some_and(|c| {
                matches!(c.input_queue.front(), Some(Command::Reboot { seconds: 10 }))
            });
        assert!(
            queued || dispatched,
            "admin reboot should be queued or dispatched, not dropped"
        );
    }

    /// A non-admin `copyover` is masked exactly like an unknown command and
    /// never forwarded to the engine.
    #[test]
    fn ingame_copyover_masked_for_non_admin() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        spawn_ingame(&mut app, conn, make_character(Vec::new()));

        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "copyover 10".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let out = cursor
            .read(msgs)
            .find(|o| o.connection == conn)
            .expect("expected a response");
        assert_eq!(out.text, "Unknown command. Type 'commands' for a list.\n");
        assert!(!out.prepend_newline, "must match unknown-command framing");

        let engine = app.world().resource::<Messages<EngineCommand>>();
        assert_eq!(engine.get_cursor().read(engine).count(), 0);
    }

    /// An admin `copyover` is accepted (queued), never masked.
    #[test]
    fn ingame_copyover_allowed_for_admin() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        spawn_ingame(&mut app, conn, make_character(vec![Role::Admin]));

        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "copyover 10".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        assert!(
            !cursor
                .read(msgs)
                .any(|o| o.connection == conn && o.text.contains("Unknown command")),
            "admin copyover must not be masked"
        );

        let engine = app.world().resource::<Messages<EngineCommand>>();
        let dispatched = engine
            .get_cursor()
            .read(engine)
            .any(|e| matches!(e.command, Command::Copyover { seconds: 10 }));
        let mut clients = app.world_mut().query::<&Client>();
        let queued = clients
            .iter(app.world())
            .find(|c| c.connection == conn)
            .is_some_and(|c| {
                matches!(
                    c.input_queue.front(),
                    Some(Command::Copyover { seconds: 10 })
                )
            });
        assert!(
            queued || dispatched,
            "admin copyover should be queued or dispatched, not dropped"
        );
    }

    /// A non-admin `gecho` is masked exactly like an unknown command and never
    /// forwarded to the engine.
    #[test]
    fn ingame_gecho_masked_for_non_admin() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        spawn_ingame(&mut app, conn, make_character(Vec::new()));

        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "gecho hello world".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let out = cursor
            .read(msgs)
            .find(|o| o.connection == conn)
            .expect("expected a response");
        assert_eq!(out.text, "Unknown command. Type 'commands' for a list.\n");
        assert!(!out.prepend_newline, "must match unknown-command framing");

        let engine = app.world().resource::<Messages<EngineCommand>>();
        assert_eq!(engine.get_cursor().read(engine).count(), 0);
    }

    /// An admin `sockets` lists every live session by connection id: address,
    /// session state, character, and account. The pre-game session (no
    /// character/account) renders `"-"` placeholders.
    #[test]
    fn ingame_sockets_lists_sessions_for_admin() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        // Spawned first (snapshot order) with the HIGHER id, so the output
        // proves the id sort rather than echoing spawn order.
        let login_conn = app
            .world_mut()
            .spawn(Connection {
                id: 2,
                addr: "127.0.0.1:22222".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        app.world_mut().spawn(Client::new(login_conn));
        let admin_conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:11111".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        spawn_ingame(&mut app, admin_conn, make_character(vec![Role::Admin]));
        let account = app
            .world_mut()
            .spawn(Account {
                testing: false,
                id: GrimId::new(),
                identifier: "spy@xf00.com".into(),
                password_hash: String::new(),
                characters: Vec::new(),
                created_at: Utc::now(),
            })
            .id();
        {
            let mut qs = app.world_mut().query::<&mut Client>();
            let world = app.world_mut();
            let mut client = qs
                .iter_mut(world)
                .find(|c| c.connection == admin_conn)
                .expect("admin client");
            client.account = Some(account);
        }

        app.world_mut().write_message(ConnectionInput {
            connection: admin_conn,
            text: "sockets".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let out = cursor
            .read(msgs)
            .find(|o| o.connection == admin_conn)
            .expect("expected a response");
        assert!(
            out.text.starts_with("Sockets connected (2):\n"),
            "got: {}",
            out.text
        );
        let first = out.text.find("127.0.0.1:11111").expect("admin row");
        let second = out.text.find("127.0.0.1:22222").expect("login row");
        assert!(
            first < second,
            "rows must sort by connection id:\n{}",
            out.text
        );
        // Admin typed `sockets`: the touch system stamped activity this tick,
        // so idle reads `0s`. The login session never sent input and the sweep
        // runs after dispatch, so it still reads `-`.
        assert!(
            out.text
                .contains(" 1  127.0.0.1:11111  InGame  Hero  spy@@xf00.com    0s\n"),
            "got: {}",
            out.text
        );
        assert!(
            out.text
                .contains(" 2  127.0.0.1:22222  Login   -     -                -\n"),
            "got: {}",
            out.text
        );
        // The `@xf00` above must arrive escaped (`@@`): identifiers are emails
        // and always contain `@`, which the transport renderer would otherwise
        // read as colour markup.

        // Answered session-locally: nothing queued for the engine.
        let engine = app.world().resource::<Messages<EngineCommand>>();
        assert_eq!(engine.get_cursor().read(engine).count(), 0);
    }

    /// Same-tick freshness: Alice's `afk` and Bob's `who` queued before one
    /// update — Bob's list must show her marker without waiting a tick. The
    /// per-tick snapshot re-syncs as lines dispatch.
    #[test]
    fn ingame_afk_visible_to_same_tick_who() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let alice_conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:11111".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let bob_conn = app
            .world_mut()
            .spawn(Connection {
                id: 2,
                addr: "127.0.0.1:22222".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        let mut alice = make_character(vec![]);
        alice.name = "Alice".into();
        let mut bob = make_character(vec![]);
        bob.name = "Bob".into();
        spawn_ingame(&mut app, alice_conn, alice);
        spawn_ingame(&mut app, bob_conn, bob);

        app.world_mut().write_message(ConnectionInput {
            connection: alice_conn,
            text: "afk".into(),
        });
        app.world_mut().write_message(ConnectionInput {
            connection: bob_conn,
            text: "who".into(),
        });
        app.update();
        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let out = cursor
            .read(msgs)
            .find(|o| o.connection == bob_conn)
            .expect("expected a who response");
        assert!(
            out.text.contains("Alice (AFK)"),
            "same-tick who must show the marker:\n{}",
            out.text
        );
    }

    /// A non-admin `sockets` is masked exactly like an unknown command and
    /// never forwarded to the engine.
    #[test]
    fn ingame_sockets_masked_for_non_admin() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        spawn_ingame(&mut app, conn, make_character(Vec::new()));

        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "sockets".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let out = cursor
            .read(msgs)
            .find(|o| o.connection == conn)
            .expect("expected a response");
        assert_eq!(out.text, "Unknown command. Type 'commands' for a list.\n");
        assert!(!out.prepend_newline, "must match unknown-command framing");

        let engine = app.world().resource::<Messages<EngineCommand>>();
        assert_eq!(engine.get_cursor().read(engine).count(), 0);
    }

    /// A `sockets` from a session whose character is absent from the character
    /// query fails closed: the admin check cannot confirm admin, so the verb
    /// is masked exactly like an unknown command.
    #[test]
    fn ingame_sockets_masked_when_character_missing() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();
        // No such character entity exists, so `characters.get` fails.
        let mut client = Client::new(conn);
        client.state = ClientState::InGame;
        client.character = Some(Entity::PLACEHOLDER);
        let session = app.world_mut().spawn(client).id();
        let scene = app.world_mut().spawn(InGameScene).id();
        app.world_mut().entity_mut(session).add_child(scene);
        app.world_mut()
            .entity_mut(session)
            .insert(SceneStack(vec![scene]));

        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "sockets".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let out = cursor
            .read(msgs)
            .find(|o| o.connection == conn)
            .expect("expected a response");
        assert_eq!(out.text, "Unknown command. Type 'commands' for a list.\n");
        assert!(!out.prepend_newline, "must match unknown-command framing");

        let engine = app.world().resource::<Messages<EngineCommand>>();
        assert_eq!(engine.get_cursor().read(engine).count(), 0);
    }

    /// `where` lists beings in the actor's area (issue #136): objects share
    /// `Name + InRoom` but carry neither `Character` nor `Creature`, so the
    /// lantern stays out. The header names the area; the actor is included.
    #[test]
    fn ingame_where_lists_beings_not_objects() {
        let mut app = test_app();
        let town = spawn_area(&mut app, "town", "Town");
        let tavern = spawn_room_in(&mut app, town, "tavern", "Tavern");
        let garden = spawn_room_in(&mut app, town, "garden", "Garden");
        let wilds = spawn_area(&mut app, "wilds", "Wilds");
        let clearing = spawn_room_in(&mut app, wilds, "clearing", "Clearing");
        app.world_mut().insert_resource(StartingRoom(tavern));
        let hero_conn = spawn_conn(&mut app, 1, 11111);
        let bob_conn = spawn_conn(&mut app, 2, 22222);
        let stranger_conn = spawn_conn(&mut app, 3, 33333);
        spawn_ingame_in(&mut app, hero_conn, named_character("Hero"), tavern);
        spawn_ingame_in(&mut app, bob_conn, named_character("Bob"), garden);
        spawn_ingame_in(
            &mut app,
            stranger_conn,
            named_character("Stranger"),
            clearing,
        );
        // Seeded mob composition: `Creature` + the shared `Actor` base.
        app.world_mut().spawn((
            GrimName("Goblin".into()),
            Creature,
            Actor {
                race: String::new(),
                level: 1,
                gender: Gender::Neutral,
            },
            InRoom { room: tavern },
        ));
        // The bug: a ground object matches `Name + InRoom` like a being.
        app.world_mut().spawn((
            GrimName("brass lantern".into()),
            Object,
            InRoom { room: tavern },
        ));

        app.world_mut().write_message(ConnectionInput {
            connection: hero_conn,
            text: "where".into(),
        });
        app.update();
        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let out = cursor
            .read(msgs)
            .find(|o| o.connection == hero_conn)
            .expect("expected a where response");
        assert!(
            out.text.starts_with("In your area (Town):\n"),
            "header must name the area:\n{}",
            out.text
        );
        for want in ["Hero in [Tavern]", "Bob in [Garden]", "Goblin in [Tavern]"] {
            assert!(out.text.contains(want), "missing {want}:\n{}", out.text);
        }
        for banned in ["lantern", "Stranger"] {
            assert!(!out.text.contains(banned), "leaked {banned}:\n{}", out.text);
        }
    }

    /// `where` with an unresolvable room fails closed: the empty message, no
    /// header naming nothing, no leak of other-area beings.
    #[test]
    fn ingame_where_without_room_shows_empty() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let conn = spawn_conn(&mut app, 1, 11111);
        // `spawn_ingame` leaves `InRoom` on `PLACEHOLDER`: no room resolves.
        spawn_ingame(&mut app, conn, named_character("Hero"));
        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "where".into(),
        });
        app.update();
        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let out = cursor
            .read(msgs)
            .find(|o| o.connection == conn)
            .expect("expected a where response");
        assert_eq!(out.text, "No one else in this area.\n");
    }

    /// `where` alone in an area lists just the actor under the area header.
    #[test]
    fn ingame_where_alone_lists_self() {
        let mut app = test_app();
        let town = spawn_area(&mut app, "town", "Town");
        let tavern = spawn_room_in(&mut app, town, "tavern", "Tavern");
        app.world_mut().insert_resource(StartingRoom(tavern));
        let conn = spawn_conn(&mut app, 1, 11111);
        spawn_ingame_in(&mut app, conn, named_character("Hero"), tavern);
        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "where".into(),
        });
        app.update();
        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let out = cursor
            .read(msgs)
            .find(|o| o.connection == conn)
            .expect("expected a where response");
        assert_eq!(out.text, "In your area (Town):\n  Hero in [Tavern]\n");
    }

    // ── handle_client_input: InGame with blank line ──
    #[test]
    fn ingame_blank_line_triggers_prompt() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));

        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 1,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id();

        let char_entity = app
            .world_mut()
            .spawn((
                GrimName("Hero".into()),
                InRoom { room },
                Player { connection: conn },
            ))
            .id();

        let mut client = Client::new(conn);
        client.state = ClientState::InGame;
        client.character = Some(char_entity);
        let session = app.world_mut().spawn(client).id();
        let scene = app.world_mut().spawn(InGameScene).id();
        app.world_mut().entity_mut(session).add_child(scene);
        app.world_mut()
            .entity_mut(session)
            .insert(SceneStack(vec![scene]));

        app.world_mut().write_message(ConnectionInput {
            connection: conn,
            text: "".into(),
        });
        app.update();

        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        let outputs: Vec<&ConnectionOutput> = cursor.read(msgs).collect();
        assert!(
            outputs
                .iter()
                .any(|o| o.connection == conn && o.text == " "),
            "Blank line should write a space to trigger prompt"
        );
    }

    // ── finger: online sheet, offline sheet, unknown name ──
    fn finger_conn(app: &mut App, id: usize) -> Entity {
        app.world_mut()
            .spawn(Connection {
                id,
                addr: "127.0.0.1:12345".parse().unwrap(),
                echo_hidden: false,
            })
            .id()
    }

    fn finger_text(app: &App, conn: Entity) -> String {
        let msgs = app.world().resource::<Messages<ConnectionOutput>>();
        let mut cursor = msgs.get_cursor();
        cursor
            .read(msgs)
            .filter(|o| o.connection == conn)
            .map(|o| o.text.clone())
            .collect()
    }

    #[test]
    fn finger_online_answers_live_sheet() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let viewer_conn = finger_conn(&mut app, 1);
        let viewer = spawn_ingame(&mut app, viewer_conn, make_character(Vec::new()));
        app.world_mut().entity_mut(viewer).insert(InRoom { room });

        let mut zara = make_character(Vec::new());
        zara.name = "Zara".into();
        zara.race = "elf".into();
        zara.class = "mage".into();
        zara.level = 3;
        zara.gender = Gender::Female;
        let zara_conn = finger_conn(&mut app, 2);
        let zara_entity = spawn_ingame(&mut app, zara_conn, zara);
        app.world_mut()
            .entity_mut(zara_entity)
            .insert(InRoom { room })
            .insert(Description(vec!["Brave.".into(), "Bold.".into()]));

        app.world_mut().write_message(ConnectionInput {
            connection: viewer_conn,
            text: "finger zara".into(),
        });
        app.update();

        assert_eq!(
            finger_text(&app, viewer_conn),
            "Name: Zara\nLevel: 3\nGender: Female\nRace: elf\nClass: mage\nDescription:\nBrave.\nBold.\n"
        );
    }

    #[test]
    fn finger_offline_answers_stored_sheet() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let viewer_conn = finger_conn(&mut app, 1);
        let viewer = spawn_ingame(&mut app, viewer_conn, make_character(Vec::new()));
        app.world_mut().entity_mut(viewer).insert(InRoom { room });

        let mut wrack = make_character(Vec::new());
        wrack.name = "Wrack".into();
        wrack.race = "human".into();
        wrack.class = "warrior".into();
        wrack.level = 5;
        wrack.gender = Gender::Male;
        let config = app.world().resource::<PersistenceConfig>().clone();
        std::fs::write(
            config.characters_dir().join("Wrack.json"),
            serde_json::to_string(&wrack).unwrap(),
        )
        .unwrap();

        app.world_mut().write_message(ConnectionInput {
            connection: viewer_conn,
            text: "finger Wrack".into(),
        });
        app.update();

        assert_eq!(
            finger_text(&app, viewer_conn),
            "Name: Wrack\nLevel: 5\nGender: Male\nRace: human\nClass: warrior\nDescription:\nA new adventurer.\n"
        );
    }

    #[test]
    fn finger_unknown_name_misses() {
        let mut app = test_app();
        let room = spawn_room(&mut app);
        app.world_mut().insert_resource(StartingRoom(room));
        let viewer_conn = finger_conn(&mut app, 1);
        let viewer = spawn_ingame(&mut app, viewer_conn, make_character(Vec::new()));
        app.world_mut().entity_mut(viewer).insert(InRoom { room });

        app.world_mut().write_message(ConnectionInput {
            connection: viewer_conn,
            text: "finger Nobody".into(),
        });
        app.update();

        assert_eq!(
            finger_text(&app, viewer_conn),
            "You don't know anyone by that name.\n"
        );
    }
}

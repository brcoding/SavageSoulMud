use bevy::log::info;
use bevy::prelude::*;
use grim_actor::{Actor, Character, InRoom, Linkdead, OutputHistory, Player, StoredCharacter};
use grim_core::components::{Account, Client, Name as GrimName};
use grim_core::events::{LinkdeadAnnounce, MoveEvent};
use grim_core::GrimId;
use grim_networking::{Connection, ConnectionClosed};
use grim_world::{Area, Room, RoomLocation};
use std::fs;
use std::path::PathBuf;

/// Where accounts and characters are stored. Defaults to `data/`; a test harness
/// (or an author) inserts a custom directory before adding the plugin to isolate
/// state. `accounts/` and `characters/` live under `dir`.
#[derive(Resource, Clone, Debug)]
pub struct PersistenceConfig {
    pub dir: PathBuf,
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self { dir: "data".into() }
    }
}

impl PersistenceConfig {
    pub fn accounts_dir(&self) -> PathBuf {
        self.dir.join("accounts")
    }
    pub fn characters_dir(&self) -> PathBuf {
        self.dir.join("characters")
    }
}

/// Loads accounts/characters on startup, saves on disconnect.
pub struct PersistencePlugin;

impl Plugin for PersistencePlugin {
    fn build(&self, app: &mut App) {
        // Only inserts the default when the author/harness hasn't set one.
        app.init_resource::<PersistenceConfig>()
            .add_message::<ConnectionClosed>()
            .add_message::<MoveEvent>()
            .add_systems(Startup, load_persisted_data)
            .add_systems(Update, (save_on_disconnect, save_on_move));
    }
}

/// Spawn every account and character found on disk. Missing directories are
/// treated as empty (no-op) rather than errors.
fn load_persisted_data(mut commands: Commands, config: Res<PersistenceConfig>) {
    let accounts_dir = config.accounts_dir();
    let characters_dir = config.characters_dir();
    let _ = fs::create_dir_all(&accounts_dir);
    let _ = fs::create_dir_all(&characters_dir);
    // The single-file blocklist survives restarts with the accounts; a missing
    // file is an empty list.
    commands.insert_resource(crate::BanList::load(&config.dir));
    if let Ok(entries) = fs::read_dir(&accounts_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(account) = serde_json::from_str::<Account>(&data) {
                    commands.spawn(account);
                }
            }
        }
    }

    // Characters are NOT loaded here. They load lazily from disk when their
    // account logs in (see `load_account_characters`) and are despawned on quit,
    // so the world only holds characters that are actually in play.
}

/// Read every character on disk as flat [`StoredCharacter`] DTOs. Missing dir
/// → empty. Corrupt/unparseable files are skipped, like the account load.
pub fn load_all_characters(config: &PersistenceConfig) -> Vec<StoredCharacter> {
    let dir = config.characters_dir();
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(stored) = serde_json::from_str::<StoredCharacter>(&data) {
                    out.push(stored);
                }
            }
        }
    }
    out
}

/// Read every character on disk belonging to `account_id`, as flat
/// [`StoredCharacter`] DTOs (the caller splits them into components when
/// spawning). Used to lazily bring an account's characters into the world at
/// login. Missing dir → empty.
pub fn load_account_characters(
    config: &PersistenceConfig,
    account_id: GrimId,
) -> Vec<StoredCharacter> {
    load_all_characters(config)
        .into_iter()
        .filter(|stored| stored.account_id == account_id)
        .collect()
}

/// Read a single character from disk by its canonical name (`<name>.json`) as a
/// flat [`StoredCharacter`], for login-by-name of a character that isn't
/// currently in the world. Names are stored canonical and callers normalize user
/// input (see `grim_scene::validation::normalize_character_name`) before calling,
/// so an exact filename match works.
pub fn load_character_by_name(config: &PersistenceConfig, name: &str) -> Option<StoredCharacter> {
    let path = config.characters_dir().join(format!("{name}.json"));
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str::<StoredCharacter>(&data).ok()
}

/// On connection close: persist the bound account/character (refreshing the
/// character's `last_room` from its current `InRoom`), then mark the character
/// as Linkdead and despawn the client/connection entities.
#[allow(clippy::too_many_arguments)]
fn save_on_disconnect(
    mut commands: Commands,
    mut closed: MessageReader<ConnectionClosed>,
    clients: Query<(Entity, &Client)>,
    connections: Query<&Connection>,
    accounts: Query<&Account>,
    mut characters: Query<&mut Character>,
    names: Query<&GrimName>,
    actors: Query<&Actor>,
    inroom: Query<&InRoom>,
    rooms: Query<(&Room, &Area)>,
    histories: Query<&OutputHistory>,
    mut announce_linkdead: MessageWriter<LinkdeadAnnounce>,
    players: Query<&Player>,
    pack: grim_object::persist::Carried,
    config: Res<PersistenceConfig>,
) {
    for ev in closed.read() {
        let conn = ev.connection;
        if let Ok(c) = connections.get(conn) {
            info!("Connection closed from {}", c.addr);
        }

        let mut client_entity = None;
        let mut account_entity = None;
        let mut character_entity = None;
        for (e, client) in clients.iter() {
            if client.connection == conn {
                client_entity = Some(e);
                account_entity = client.account;
                character_entity = client.character;
                break;
            }
        }
        let Some(client_e) = client_entity else {
            // No client for this connection; still despawn the connection.
            commands.entity(conn).despawn();
            continue;
        };

        if let Some(acct_e) = account_entity {
            if let Ok(account) = accounts.get(acct_e) {
                let path = config.accounts_dir().join(format!("{}.json", account.id));
                if let Ok(json) = serde_json::to_string_pretty(account) {
                    let _ = fs::write(path, json);
                }
            }
        }
        if let Some(char_e) = character_entity {
            // Save the character to disk from its three components
            // (`Name + Actor + Character`) via the flat `StoredCharacter` DTO —
            // the only disk surface. Requires all three; a partial entity (e.g.
            // a bare `Character` with no `Name`/`Actor`) skips the save.
            if let (Ok(name), Ok(actor)) = (names.get(char_e), actors.get(char_e)) {
                if let Ok(mut character) = characters.get_mut(char_e) {
                    if let Ok(ir) = inroom.get(char_e) {
                        if let Ok((room, area)) = rooms.get(ir.room) {
                            character.last_room = Some(RoomLocation {
                                area: area.friendly_id.clone(),
                                room: room.friendly_id.clone(),
                            });
                        }
                    }
                    let mut stored = StoredCharacter::from_components(name, actor, &character);
                    stored.inventory = grim_object::persist::snapshot_pack(&pack, char_e);
                    let path = config.characters_dir().join(format!("{}.json", name.0));
                    if let Ok(json) = serde_json::to_string_pretty(&stored) {
                        let _ = fs::write(path, json);
                    }
                }
            }
            // Transfer OutputHistory from connection to character before despawn
            if let Ok(history) = histories.get(conn) {
                commands.entity(char_e).insert(history.clone());
            }
            // Player is present only while connected. On a normal close, remove
            // it and mark the character linkdead. Skip only if another session
            // took the character over — its live `Player.connection` differs from
            // the connection being closed, so we must not disturb it.
            let taken_over = players
                .get(char_e)
                .map(|p| p.connection != conn)
                .unwrap_or(false);
            if !taken_over {
                commands.entity(char_e).remove::<Player>();
                commands.entity(char_e).insert(Linkdead);
                if let Ok(name) = names.get(char_e) {
                    info!("Character '{}' went linkdead", name.0);
                    announce_linkdead.write(LinkdeadAnnounce {
                        subject: char_e,
                        name: name.0.clone(),
                        reconnecting: false,
                    });
                }
            }
        }
        commands.entity(client_e).despawn();
        commands.entity(conn).despawn();
    }
}

/// Persist a character to disk each time it walks to a new room. `grim-world`
/// keeps the in-memory `Character.last_room` current on every move; this mirrors
/// that to disk immediately, so the on-disk `last_room` is always up to date.
/// That is what lets a copyover successor (which reloads the world from scratch
/// and reads characters from disk) restore each player exactly where they were —
/// no copyover-time flush needed. (A future WAL/autosave pass will generalise
/// this; see the durable-persistence follow-up.)
fn save_on_move(
    mut moves: MessageReader<MoveEvent>,
    characters: Query<(&GrimName, &Actor, &Character)>,
    pack: grim_object::persist::Carried,
    config: Res<PersistenceConfig>,
) {
    for ev in moves.read() {
        // Only player characters persist; creatures and other movers (which have
        // no `Character`) are ignored.
        let Ok((name, actor, character)) = characters.get(ev.actor) else {
            continue;
        };
        let dir = config.characters_dir();
        let _ = fs::create_dir_all(&dir);
        let path = dir.join(format!("{}.json", name.0));
        let mut stored = StoredCharacter::from_components(name, actor, character);
        stored.inventory = grim_object::persist::snapshot_pack(&pack, ev.actor);
        if let Ok(json) = serde_json::to_string_pretty(&stored) {
            let _ = fs::write(path, json);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use grim_core::components::Gender;
    use grim_core::GrimId;
    use std::sync::{LazyLock, Mutex};

    /// Serialises filesystem-touching tests: all load/save tests use
    /// `data/` paths, so running them in parallel races.
    static FS_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(PersistencePlugin);
        app.add_message::<LinkdeadAnnounce>();
        app
    }

    fn setup_save_dirs() {
        let _ = fs::create_dir_all("data/accounts");
        let _ = fs::create_dir_all("data/characters");
    }

    fn cleanup_save_dirs() {
        let _ = fs::remove_dir_all("data/accounts");
        let _ = fs::remove_dir_all("data/characters");
    }

    /// A flat [`StoredCharacter`] with sensible defaults, for disk fixtures and
    /// spawning. Mirrors the pre-split `Character` field set.
    fn stored(name: &str, account_id: GrimId) -> StoredCharacter {
        StoredCharacter {
            credits: 100,
            id: GrimId::new(),
            name: name.into(),
            account_id,
            created_at: Utc::now(),
            last_room: None,
            roles: Vec::new(),
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

    /// Spawn a live PC entity (`Name + Actor + Character`) from a DTO.
    fn spawn_pc(app: &mut App, s: &StoredCharacter) -> Entity {
        let (name, actor, character) = s.clone().into_components();
        app.world_mut().spawn((name, actor, character)).id()
    }

    fn make_connection(app: &mut App, id: usize) -> Entity {
        let addr: std::net::SocketAddr = ([127, 0, 0, 1], 30000 + id as u16).into();
        app.world_mut()
            .spawn(Connection {
                id,
                addr,
                echo_hidden: false,
            })
            .id()
    }

    // --- account save/load ---
    mod account_save_load {
        use super::*;

        #[test]
        fn test_load_persisted_data_with_valid_account() {
            let _guard = FS_LOCK.lock().unwrap();
            let _ = fs::remove_dir_all("data/accounts");
            let _ = fs::remove_dir_all("data/characters");
            let account_id = GrimId::new();
            let account = Account {
                id: account_id,
                identifier: "testuser".into(),
                password_hash: "hash".into(),
                characters: vec![],
                created_at: Utc::now(),
            };

            let _ = fs::create_dir_all("data/accounts");
            let _ = fs::create_dir_all("data/characters");
            let account_path = format!("data/accounts/{account_id}.json");
            fs::write(&account_path, serde_json::to_string(&account).unwrap()).unwrap();

            let mut app = test_app();
            app.update();

            let mut query = app.world_mut().query::<&Account>();
            let loaded: Vec<&Account> = query.iter(app.world()).collect();
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded[0].identifier, "testuser");
            assert_eq!(loaded[0].id, account_id);

            let _ = fs::remove_dir_all("data/accounts");
            let _ = fs::remove_dir_all("data/characters");
        }
    }

    // --- character save/load (lazy login-time load, skip rules) ---
    mod character_save_load {
        use super::*;

        #[test]
        fn startup_does_not_load_characters_but_helper_reads_them_by_account() {
            // Characters load lazily (on login), not at startup. A unique temp dir so
            // no FS_LOCK is needed.
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("grim-pers-lazy-{}-{}", std::process::id(), n));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("characters")).unwrap();

            let account_id = GrimId::new();
            let mine = stored("TestHero", account_id);
            let other = stored("Stranger", GrimId::new()); // a different account
            for c in [&mine, &other] {
                fs::write(
                    dir.join("characters").join(format!("{}.json", c.name)),
                    serde_json::to_string(c).unwrap(),
                )
                .unwrap();
            }

            let config = PersistenceConfig { dir: dir.clone() };

            // Startup does NOT spawn any character entity.
            let mut app = App::new();
            app.insert_resource(config.clone());
            app.add_plugins(MinimalPlugins)
                .add_plugins(PersistencePlugin);
            app.add_message::<LinkdeadAnnounce>();
            app.update();
            let mut q = app.world_mut().query::<&Character>();
            assert_eq!(
                q.iter(app.world()).len(),
                0,
                "startup must not load characters"
            );

            // The lazy loader reads only this account's characters, by name too.
            let mut mine_only = load_account_characters(&config, account_id);
            assert_eq!(mine_only.len(), 1);
            assert_eq!(mine_only.remove(0).name, "TestHero");
            assert_eq!(
                load_character_by_name(&config, "TestHero").unwrap().name,
                "TestHero"
            );
            assert!(load_character_by_name(&config, "Nobody").is_none());

            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn load_all_characters_reads_every_file_regardless_of_account() {
            // Backs the `wizlist` startup snapshot: every admin on disk must be
            // visible even when their account never logs in. A unique temp dir
            // so no FS_LOCK is needed.
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("grim-pers-all-{}-{}", std::process::id(), n));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("characters")).unwrap();

            let mut admin = stored("Archon", GrimId::new());
            admin.roles.push(grim_actor::Role::Admin);
            let plain = stored("Pleb", GrimId::new());
            for c in [&admin, &plain] {
                fs::write(
                    dir.join("characters").join(format!("{}.json", c.name)),
                    serde_json::to_string(c).unwrap(),
                )
                .unwrap();
            }
            // A non-JSON file and a corrupt blob are skipped, not fatal.
            fs::write(dir.join("characters").join("notes.txt"), "hi").unwrap();
            fs::write(dir.join("characters").join("broken.json"), "{nope").unwrap();

            let config = PersistenceConfig { dir: dir.clone() };
            let mut all = load_all_characters(&config);
            all.sort_by(|a, b| a.name.cmp(&b.name));
            assert_eq!(all.len(), 2);
            assert_eq!(all[0].name, "Archon");
            assert!(all[0].roles.contains(&grim_actor::Role::Admin));
            assert_eq!(all[1].name, "Pleb");

            // Missing dir reads as empty.
            let missing = PersistenceConfig {
                dir: dir.join("nope"),
            };
            assert!(load_all_characters(&missing).is_empty());

            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn test_load_persisted_data_with_both_account_and_character() {
            let _guard = FS_LOCK.lock().unwrap();
            let _ = fs::remove_dir_all("data/accounts");
            let _ = fs::remove_dir_all("data/characters");
            let _ = fs::create_dir_all("data/accounts");
            let _ = fs::create_dir_all("data/characters");

            let account = Account {
                id: GrimId::new(),
                identifier: "dualuser".into(),
                password_hash: "hash".into(),
                characters: vec![],
                created_at: Utc::now(),
            };
            let acct_path = format!("data/accounts/{}.json", account.id);
            fs::write(&acct_path, serde_json::to_string(&account).unwrap()).unwrap();

            let character = stored("DualHero", account.id);
            let char_path = format!("data/characters/{}.json", character.name);
            fs::write(&char_path, serde_json::to_string(&character).unwrap()).unwrap();

            let mut app = test_app();
            app.update();

            let mut acct_query = app.world_mut().query::<&Account>();
            let loaded_accts: Vec<_> = acct_query.iter(app.world()).collect();
            assert_eq!(loaded_accts.len(), 1);
            assert_eq!(loaded_accts[0].identifier, "dualuser");

            // Startup loads the account but NOT its characters (those load on login).
            let mut char_query = app.world_mut().query::<&Character>();
            assert_eq!(char_query.iter(app.world()).len(), 0);
            let config = PersistenceConfig {
                dir: std::path::PathBuf::from("data"),
            };
            let chars = load_account_characters(&config, account.id);
            assert_eq!(chars.len(), 1);
            assert_eq!(chars[0].name, "DualHero");

            let _ = fs::remove_dir_all("data/accounts");
            let _ = fs::remove_dir_all("data/characters");
        }

        #[test]
        fn test_load_persisted_data_skips_non_json_files() {
            let _guard = FS_LOCK.lock().unwrap();
            let _ = fs::remove_dir_all("data/accounts");
            let _ = fs::remove_dir_all("data/characters");
            let _ = fs::create_dir_all("data/characters");

            fs::write("data/characters/readme.txt", "not json").unwrap();

            let mut app = test_app();
            app.update();

            let mut query = app.world_mut().query::<&Character>();
            assert_eq!(query.iter(app.world()).len(), 0);

            let _ = fs::remove_dir_all("data/characters");
        }

        #[test]
        fn test_load_persisted_data_skips_invalid_json() {
            let _guard = FS_LOCK.lock().unwrap();
            let _ = fs::remove_dir_all("data/accounts");
            let _ = fs::remove_dir_all("data/characters");
            let _ = fs::create_dir_all("data/characters");

            fs::write("data/characters/bad.json", "{{{ not valid json }}}").unwrap();

            let mut app = test_app();
            app.update();

            let mut query = app.world_mut().query::<&Character>();
            assert_eq!(query.iter(app.world()).len(), 0);

            let _ = fs::remove_dir_all("data/characters");
        }
    }

    // --- linkdead / save-on-disconnect ---
    mod disconnect_linkdead {
        use super::*;

        #[test]
        fn test_save_on_disconnect_no_matching_client() {
            let _guard = FS_LOCK.lock().unwrap();
            setup_save_dirs();

            let mut app = test_app();
            let conn = make_connection(&mut app, 1);
            app.world_mut()
                .write_message(ConnectionClosed { connection: conn });
            app.update();

            // Connection despawned, no client to save
            assert!(app.world().get_entity(conn).is_err());
            // No files written
            assert!(fs::read_dir("data/accounts")
                .map(|mut e| e.next().is_none())
                .unwrap_or(true));
            assert!(fs::read_dir("data/characters")
                .map(|mut e| e.next().is_none())
                .unwrap_or(true));

            cleanup_save_dirs();
        }

        #[test]
        fn test_save_on_disconnect_account_only() {
            let _guard = FS_LOCK.lock().unwrap();
            setup_save_dirs();

            let mut app = test_app();
            let conn = make_connection(&mut app, 1);

            let account = Account {
                id: GrimId::new(),
                identifier: "acctonly".into(),
                password_hash: "hash".into(),
                characters: vec![],
                created_at: Utc::now(),
            };
            let acct_e = app.world_mut().spawn(account.clone()).id();

            let client = Client {
                account: Some(acct_e),
                character: None, // no character bound
                ..Client::new(conn)
            };
            let client_e = app.world_mut().spawn(client).id();

            app.world_mut()
                .write_message(ConnectionClosed { connection: conn });
            app.update();

            // Account file written
            let acct_path = format!("data/accounts/{}.json", account.id);
            let saved: Account =
                serde_json::from_str(&fs::read_to_string(&acct_path).unwrap()).unwrap();
            assert_eq!(saved.identifier, "acctonly");

            // No character file written (no character bound)
            assert!(fs::read_dir("data/characters")
                .map(|mut e| e.next().is_none())
                .unwrap_or(true));

            // Client + connection despawned
            assert!(app.world().get_entity(client_e).is_err());
            assert!(app.world().get_entity(conn).is_err());
            // Account entity still alive
            assert!(app.world().get_entity(acct_e).is_ok());

            cleanup_save_dirs();
        }

        #[test]
        fn test_save_on_disconnect_character_without_account_entity() {
            let _guard = FS_LOCK.lock().unwrap();
            setup_save_dirs();

            let mut app = test_app();
            let conn = make_connection(&mut app, 1);

            let character = stored("NoAccountHero", GrimId::new()); // orphan — no Account entity
            let char_e = spawn_pc(&mut app, &character);

            let client = Client {
                account: None, // no account entity
                character: Some(char_e),
                ..Client::new(conn)
            };
            let client_e = app.world_mut().spawn(client).id();

            app.world_mut()
                .write_message(ConnectionClosed { connection: conn });
            app.update();

            // Character file written (character_entity is Some)
            let char_path = format!("data/characters/{}.json", character.name);
            let saved: StoredCharacter =
                serde_json::from_str(&fs::read_to_string(&char_path).unwrap()).unwrap();
            assert_eq!(saved.name, "NoAccountHero");

            // Character marked linkdead
            let mut ld = app.world_mut().query::<&Linkdead>();
            assert_eq!(ld.iter(app.world()).len(), 1);

            // Client + connection despawned
            assert!(app.world().get_entity(client_e).is_err());
            assert!(app.world().get_entity(conn).is_err());

            cleanup_save_dirs();
        }

        #[test]
        fn test_save_on_disconnect_full_save_with_linkdead() {
            let _guard = FS_LOCK.lock().unwrap();
            setup_save_dirs();

            let mut app = test_app();
            let conn = make_connection(&mut app, 1);

            let account = Account {
                id: GrimId::new(),
                identifier: "fullsave".into(),
                password_hash: "hash".into(),
                characters: vec![],
                created_at: Utc::now(),
            };
            let acct_e = app.world_mut().spawn(account.clone()).id();

            let character = stored("FullSaveHero", account.id);
            let char_e = spawn_pc(&mut app, &character);

            let client = Client {
                account: Some(acct_e),
                character: Some(char_e),
                ..Client::new(conn)
            };
            let client_e = app.world_mut().spawn(client).id();

            app.world_mut()
                .write_message(ConnectionClosed { connection: conn });
            app.update();

            // Both files written
            let acct_path = format!("data/accounts/{}.json", account.id);
            let saved_acct: Account =
                serde_json::from_str(&fs::read_to_string(&acct_path).unwrap()).unwrap();
            assert_eq!(saved_acct.identifier, "fullsave");

            let char_path = format!("data/characters/{}.json", character.name);
            let saved_char: StoredCharacter =
                serde_json::from_str(&fs::read_to_string(&char_path).unwrap()).unwrap();
            assert_eq!(saved_char.name, "FullSaveHero");

            // Character marked linkdead: Linkdead inserted, Player REMOVED (a
            // linkdead being has no Player under the present-only-while-connected
            // model).
            let mut ld = app
                .world_mut()
                .query::<(&Character, &GrimName, &Linkdead)>();
            let results: Vec<_> = ld.iter(app.world()).collect();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].1 .0, "FullSaveHero");
            let mut players = app.world_mut().query::<&Player>();
            assert_eq!(
                players.iter(app.world()).len(),
                0,
                "linkdead character must not keep a Player"
            );

            // LinkdeadAnnounce emitted
            let messages = app.world().resource::<Messages<LinkdeadAnnounce>>();
            let mut cursor = messages.get_cursor();
            let events: Vec<_> = cursor.read(messages).collect();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].name, "FullSaveHero");
            assert!(!events[0].reconnecting);

            // Client + connection despawned
            assert!(app.world().get_entity(client_e).is_err());
            assert!(app.world().get_entity(conn).is_err());

            cleanup_save_dirs();
        }

        #[test]
        fn test_save_on_disconnect_updates_last_room_from_inroom() {
            let _guard = FS_LOCK.lock().unwrap();
            setup_save_dirs();

            let mut app = test_app();
            let conn = make_connection(&mut app, 1);

            let room_e = app
                .world_mut()
                .spawn((
                    Room {
                        id: GrimId::new(),
                        friendly_id: "test_room".into(),
                        name: "Test Room".into(),
                        description: "".into(),
                        area: Entity::PLACEHOLDER,
                    },
                    Area {
                        id: GrimId::new(),
                        friendly_id: "test_area".into(),
                        name: "Test Area".into(),
                    },
                ))
                .id();
            // Patch room.area to point to itself
            app.world_mut().entity_mut(room_e).insert(Room {
                id: GrimId::new(),
                friendly_id: "test_room".into(),
                name: "Test Room".into(),
                description: "".into(),
                area: room_e,
            });

            let account = Account {
                id: GrimId::new(),
                identifier: "roomtest".into(),
                password_hash: "hash".into(),
                characters: vec![],
                created_at: Utc::now(),
            };
            let acct_e = app.world_mut().spawn(account.clone()).id();

            let character = stored("RoomHero", account.id); // starts with no last_room
            let (name, actor, ch) = character.clone().into_components();
            let char_e = app
                .world_mut()
                .spawn((name, actor, ch, InRoom { room: room_e }))
                .id();

            let client = Client {
                account: Some(acct_e),
                character: Some(char_e),
                ..Client::new(conn)
            };
            let _ = app.world_mut().spawn(client).id();

            app.world_mut()
                .write_message(ConnectionClosed { connection: conn });
            app.update();

            // Character file shows last_room populated from InRoom
            let char_path = format!("data/characters/{}.json", character.name);
            let saved: StoredCharacter =
                serde_json::from_str(&fs::read_to_string(&char_path).unwrap()).unwrap();
            let loc = saved.last_room.expect("expected last_room to be set");
            assert_eq!(loc.area, "test_area");
            assert_eq!(loc.room, "test_room");

            cleanup_save_dirs();
        }

        #[test]
        fn test_save_on_disconnect_transfers_output_history() {
            let _guard = FS_LOCK.lock().unwrap();
            setup_save_dirs();

            let mut app = test_app();
            let conn = make_connection(&mut app, 1);

            // OutputHistory on the connection entity
            let mut history = OutputHistory::with_max(100);
            history.push("line 1");
            history.push("line 2");
            app.world_mut().entity_mut(conn).insert(history.clone());

            let account = Account {
                id: GrimId::new(),
                identifier: "historytest".into(),
                password_hash: "hash".into(),
                characters: vec![],
                created_at: Utc::now(),
            };
            let acct_e = app.world_mut().spawn(account.clone()).id();

            let character = stored("HistoryHero", account.id);
            let char_e = spawn_pc(&mut app, &character);

            let client = Client {
                account: Some(acct_e),
                character: Some(char_e),
                ..Client::new(conn)
            };
            let _ = app.world_mut().spawn(client).id();

            app.world_mut()
                .write_message(ConnectionClosed { connection: conn });
            app.update();

            // Character should now have the OutputHistory
            let mut hist_query = app.world_mut().query::<(&GrimName, &OutputHistory)>();
            let results: Vec<_> = hist_query.iter(app.world()).collect();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].0 .0, "HistoryHero");
            assert_eq!(
                results[0].1.lines.iter().collect::<Vec<_>>(),
                vec!["line 1", "line 2"]
            );

            cleanup_save_dirs();
        }

        #[test]
        fn test_save_on_disconnect_skips_save_on_get_mut_failure() {
            let _guard = FS_LOCK.lock().unwrap();
            setup_save_dirs();

            let mut app = test_app();
            let conn = make_connection(&mut app, 1);

            let account = Account {
                id: GrimId::new(),
                identifier: "errortest".into(),
                password_hash: "hash".into(),
                characters: vec![],
                created_at: Utc::now(),
            };
            let acct_e = app.world_mut().spawn(account.clone()).id();

            // Character entity but NO Character component — characters.get_mut will fail
            let char_e = app.world_mut().spawn(()).id();

            let client = Client {
                account: Some(acct_e),
                character: Some(char_e),
                ..Client::new(conn)
            };
            let client_e = app.world_mut().spawn(client).id();

            app.world_mut()
                .write_message(ConnectionClosed { connection: conn });
            app.update();

            // Account file still written (account save is independent)
            let acct_path = format!("data/accounts/{}.json", account.id);
            let saved: Account =
                serde_json::from_str(&fs::read_to_string(&acct_path).unwrap()).unwrap();
            assert_eq!(saved.identifier, "errortest");

            // No character file written (get_mut failed)
            assert!(fs::read_dir("data/characters")
                .map(|mut e| e.next().is_none())
                .unwrap_or(true));

            // Client + connection still despawned cleanup
            assert!(app.world().get_entity(client_e).is_err());
            assert!(app.world().get_entity(conn).is_err());

            cleanup_save_dirs();
        }

        #[test]
        fn test_save_on_disconnect_skips_linkdead_announce_on_query_failure() {
            let _guard = FS_LOCK.lock().unwrap();
            setup_save_dirs();

            let mut app = test_app();
            let conn = make_connection(&mut app, 1);

            // Character entity but NO Character component — save_on_disconnect
            // still reaches the OutputHistory/Player/Linkdead block (line 123-136)
            // but characters.get(char_e) at line 131 will fail, skipping the
            // LinkdeadAnnounce.
            let char_e = app.world_mut().spawn(()).id();

            let client = Client {
                account: None,
                character: Some(char_e),
                ..Client::new(conn)
            };
            let client_e = app.world_mut().spawn(client).id();

            app.world_mut()
                .write_message(ConnectionClosed { connection: conn });
            app.update();

            // No LinkdeadAnnounce emitted (characters.get failed at line 131)
            let messages = app.world().resource::<Messages<LinkdeadAnnounce>>();
            let mut cursor = messages.get_cursor();
            assert!(cursor.read(messages).next().is_none());

            // Client + connection despawned
            assert!(app.world().get_entity(client_e).is_err());
            assert!(app.world().get_entity(conn).is_err());

            cleanup_save_dirs();
        }
    }

    // --- PersistenceConfig directory redirection ---
    mod config_redirect {
        use super::*;

        /// A non-default `PersistenceConfig` must redirect both load and save, and
        /// nothing may leak to the default `data/` root. No `FS_LOCK`: the temp root
        /// is unique, so this test is isolated from the `data/`-based ones.
        #[test]
        fn configured_directory_redirects_load_and_save() {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("grim-pers-cfg-{}-{}", std::process::id(), n));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("characters")).unwrap();

            // Seed a character file in the configured dir → it must load from there.
            let seeded = stored("CfgHero", GrimId::new());
            fs::write(
                dir.join("characters").join("CfgHero.json"),
                serde_json::to_string(&seeded).unwrap(),
            )
            .unwrap();

            let config = PersistenceConfig { dir: dir.clone() };
            let mut app = App::new();
            app.insert_resource(config.clone());
            app.add_plugins(MinimalPlugins)
                .add_plugins(PersistencePlugin);
            app.add_message::<LinkdeadAnnounce>();
            app.update(); // Startup load (accounts only)

            // The lazy character loader must read from the configured directory.
            assert!(
                load_account_characters(&config, seeded.account_id)
                    .iter()
                    .any(|c| c.name == "CfgHero"),
                "load must read the configured directory"
            );

            // Save: disconnect an account; the file must land in the configured dir.
            let conn = make_connection(&mut app, 1);
            let account = Account {
                id: GrimId::new(),
                identifier: "cfguser".into(),
                password_hash: "hash".into(),
                characters: vec![],
                created_at: Utc::now(),
            };
            let acct_id = account.id;
            let acct_e = app.world_mut().spawn(account).id();
            let client = Client {
                account: Some(acct_e),
                character: None,
                ..Client::new(conn)
            };
            app.world_mut().spawn(client);
            app.world_mut()
                .write_message(ConnectionClosed { connection: conn });
            app.update();

            assert!(
                dir.join("accounts")
                    .join(format!("{acct_id}.json"))
                    .exists(),
                "save must write to the configured directory"
            );
            assert!(
                !std::path::Path::new(&format!("data/accounts/{acct_id}.json")).exists(),
                "nothing may leak to the default data/ root"
            );

            let _ = fs::remove_dir_all(&dir);
        }
    }

    // --- save-on-move ---
    mod save_on_move {
        use super::*;

        /// Walking to a new room writes the character to disk with its updated
        /// `last_room`, so an unexpected restart / copyover resumes it there. Unique
        /// temp root → no `FS_LOCK` needed.
        #[test]
        fn save_on_move_persists_updated_last_room() {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("grim-pers-move-{}-{}", std::process::id(), n));
            let _ = fs::remove_dir_all(&dir);

            let mut app = App::new();
            app.insert_resource(PersistenceConfig { dir: dir.clone() });
            app.add_plugins(MinimalPlugins)
                .add_plugins(PersistencePlugin);
            app.add_message::<LinkdeadAnnounce>();
            app.update(); // Startup load (empty)

            // A character standing in a room, with last_room already refreshed by
            // grim-world's move handler (simulated here by setting it directly).
            let room = app.world_mut().spawn(()).id();
            let mut character = stored("Rover", GrimId::new());
            character.last_room = Some(RoomLocation {
                area: "haven".into(),
                room: "square".into(),
            });
            let (name, actor_c, ch) = character.into_components();
            let actor = app
                .world_mut()
                .spawn((name, actor_c, ch, InRoom { room }))
                .id();

            // Emit the move; save_on_move should write the character to disk.
            app.world_mut().write_message(MoveEvent {
                actor,
                from: room,
                to: room,
                direction: grim_core::cardinal::Cardinal::North,
            });
            app.update();

            let path = dir.join("characters").join("Rover.json");
            assert!(path.exists(), "move must persist the character to disk");
            let saved: StoredCharacter =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            let loc = saved
                .last_room
                .expect("persisted character keeps last_room");
            assert_eq!(loc.area, "haven");
            assert_eq!(loc.room, "square");

            // A non-PC mover (no Character component) is ignored, not an error.
            let creature = app.world_mut().spawn(InRoom { room }).id();
            app.world_mut().write_message(MoveEvent {
                actor: creature,
                from: room,
                to: room,
                direction: grim_core::cardinal::Cardinal::South,
            });
            app.update();

            let _ = fs::remove_dir_all(&dir);
        }
    }
}

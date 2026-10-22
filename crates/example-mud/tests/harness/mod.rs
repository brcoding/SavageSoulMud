//! Headless end-to-end harness for example-mud.
//!
//! Boots the real game stack (`GrimHeadlessPlugins`) with the real world seed,
//! but no transport: input is injected as `ConnectionInput` and every
//! `ConnectionOutput` line is recorded per connection. Each `Mud::new()` gets a
//! fresh temp data directory, so tests start from a clean slate and never touch
//! the repo's `data/`.
//!
//! ```ignore
//! let mut mud = Mud::new();
//! let (alice, banner) = mud.connect();
//! banner.assert_contains("Character name or account email");
//! mud.send(alice, "alice@example.com").assert_contains("create an account");
//! ```

#![allow(dead_code)]

use bevy::prelude::*;
use grim::GrimHeadlessPlugins;
use grim::components::Name as GrimName;
use grim::plugins::PersistenceConfig;
use grim::{
    Actor, Character, CompiledTrigger, ScriptTriggers, StoredCharacter, TriggerKind, compile,
};
use grim::{
    Connection, ConnectionClosed, ConnectionEstablished, ConnectionInput, ConnectionOutput,
    DisconnectRequest, IdleConfig,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Unique suffix per harness instance so concurrent tests get distinct temp dirs.
static INSTANCE: AtomicUsize = AtomicUsize::new(0);

/// A connected session â€” a handle to one simulated telnet user.
#[derive(Clone, Copy)]
pub struct Session {
    pub conn: Entity,
}

/// Output produced in response to a single interaction, with content assertions.
#[must_use]
pub struct Output {
    text: String,
}

impl Output {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn contains(&self, needle: &str) -> bool {
        self.text.contains(needle)
    }

    /// Assert the output contains `needle`; panics with the full transcript.
    pub fn assert_contains(&self, needle: &str) -> &Self {
        assert!(
            self.text.contains(needle),
            "expected output to contain {needle:?}, got:\n{}",
            self.text
        );
        self
    }

    /// Assert the output does NOT contain `needle`.
    pub fn assert_excludes(&self, needle: &str) -> &Self {
        assert!(
            !self.text.contains(needle),
            "expected output to NOT contain {needle:?}, got:\n{}",
            self.text
        );
        self
    }
}

/// Outcome of [`Mud::send_shutdown`]: the session's output plus every
/// `AppExit` / `CopyoverDue` written while the pump ran.
pub struct ShutdownOutcome {
    pub output: Output,
    pub exits: Vec<AppExit>,
    pub copyover_due: usize,
}

pub struct Mud {
    app: App,
    next_conn: usize,
    data_dir: PathBuf,
    areas_dir: PathBuf,
    cursor: bevy::ecs::message::MessageCursor<ConnectionOutput>,
    exit_cursor: bevy::ecs::message::MessageCursor<AppExit>,
    due_cursor: bevy::ecs::message::MessageCursor<grim::events::CopyoverDue>,
    disconnect_cursor: bevy::ecs::message::MessageCursor<DisconnectRequest>,
    buffers: HashMap<Entity, Vec<String>>,
    read_offsets: HashMap<Entity, usize>,
}

impl Mud {
    /// Boot a fresh MUD with an isolated temp data directory and the real seed.
    pub fn new() -> Self {
        Self::with_areas("tests/fixtures/areas")
    }

    pub fn with_savsoul() -> Self {
        Self::with_areas("../../data/areas")
    }
    pub fn with_savage_soul() -> Self {
        let mut mud = Self::with_savsoul();
        example_mud::species::install(&mut mud.app);
        mud.app
            .insert_resource(grim::CharacterBuildMode::SavageSoul);
        mud.app
            .add_plugins(example_mud::savage_soul::SavageSoulPlugin);
        // Command scenarios own fixed encounters; population behavior has its
        // own manual-clock tests rather than wandering during login fixtures.
        mud.app
            .insert_resource(example_mud::population::PopulationEnabled(false));
        mud.app.update();
        mud
    }

    pub fn saved_character(&self, name: &str) -> grim::StoredCharacter {
        serde_json::from_slice(
            &std::fs::read(
                self.data_dir
                    .join("characters")
                    .join(format!("{name}.json")),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn with_areas(areas: &str) -> Self {
        let areas_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(areas);
        let id = INSTANCE.fetch_add(1, Ordering::Relaxed);
        let data_dir = std::env::temp_dir().join(format!("grim-e2e-{}-{}", std::process::id(), id));
        let _ = std::fs::remove_dir_all(&data_dir);

        let mut app = App::new();
        app.insert_resource(example_mud::DisableGrapevine);
        app.insert_resource(PersistenceConfig {
            dir: data_dir.clone(),
        });
        // Manual clock (no TimePlugin) so per-command cooldowns advance
        // deterministically in `pump`.
        app.init_resource::<Time>();
        app.add_plugins(GrimHeadlessPlugins);
        // Area blueprints load from the filesystem; point at the repo's
        // committed data/areas so the harness world matches what ships.
        app.insert_resource(example_mud::seed::AreaBlueprintDir(
            areas_dir.clone(),
        ));
        app.add_systems(Startup, example_mud::seed::seed_world);
        app.add_plugins(example_mud::shop::ShopPlugin);
        app.insert_resource(example_mud::help::HelpDir(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/help"),
        ));
        app.add_plugins(example_mud::help::HelpPlugin);

        // Run startup (seed + persistence load) and grab an output cursor before
        // any connection exists.
        app.update();
        let cursor = app
            .world()
            .resource::<Messages<ConnectionOutput>>()
            .get_cursor();
        let exit_cursor = app.world().resource::<Messages<AppExit>>().get_cursor();
        let due_cursor = app
            .world()
            .resource::<Messages<grim::events::CopyoverDue>>()
            .get_cursor();
        let disconnect_cursor = app
            .world()
            .resource::<Messages<DisconnectRequest>>()
            .get_cursor();

        Self {
            app,
            next_conn: 1,
            areas_dir,
            data_dir,
            cursor,
            exit_cursor,
            due_cursor,
            disconnect_cursor,
            buffers: HashMap::new(),
            read_offsets: HashMap::new(),
        }
    }

    /// Advance one frame, recording any output. Called several times per
    /// interaction so multi-hop dispatch and the command cooldown settle.
    fn tick(&mut self) {
        {
            let mut time = self.app.world_mut().resource_mut::<Time>();
            time.advance_by(Duration::from_secs(1));
        }
        self.app.update();
        let collected: Vec<(Entity, String)> = {
            let msgs = self.app.world().resource::<Messages<ConnectionOutput>>();
            self.cursor
                .read(msgs)
                .map(|o| (o.connection, o.text.clone()))
                .collect()
        };
        for (conn, text) in collected {
            self.buffers.entry(conn).or_default().push(text);
        }
    }

    /// Settle the world over several frames (enough for a queued command's
    /// cooldown to elapse and its output to be produced).
    fn pump(&mut self) {
        for _ in 0..8 {
            self.tick();
        }
    }

    /// Return output for `conn` recorded since it was last read, advancing the
    /// per-connection read cursor. So passive receipt (another player's speech)
    /// and command responses share one position.
    fn collect_new(&mut self, conn: Entity) -> Output {
        let len = self.buffers.get(&conn).map(|b| b.len()).unwrap_or(0);
        let from = self.read_offsets.get(&conn).copied().unwrap_or(0);
        let text = self
            .buffers
            .get(&conn)
            .map(|b| b[from.min(len)..].join(""))
            .unwrap_or_default();
        self.read_offsets.insert(conn, len);
        Output { text }
    }

    /// Open a new connection. Returns the session and the initial output
    /// (login banner + prompt).
    pub fn connect(&mut self) -> (Session, Output) {
        let id = self.next_conn;
        self.next_conn += 1;
        let addr: SocketAddr = format!("127.0.0.1:{}", 40000 + id).parse().unwrap();
        let conn = self
            .app
            .world_mut()
            .spawn(Connection {
                id,
                addr,
                echo_hidden: false,
            })
            .id();
        self.app.world_mut().write_message(ConnectionEstablished {
            connection: conn,
            addr,
        });
        self.pump();
        (Session { conn }, self.collect_new(conn))
    }

    /// Send a line of input as this session and return the resulting output.
    pub fn send(&mut self, session: Session, line: &str) -> Output {
        self.app.world_mut().write_message(ConnectionInput {
            connection: session.conn,
            text: line.to_string(),
        });
        self.pump();
        self.collect_new(session.conn)
    }

    /// Advance the world and return any output this session received passively
    /// (e.g. another player's speech), without sending anything.
    pub fn recv(&mut self, session: Session) -> Output {
        self.pump();
        self.collect_new(session.conn)
    }

    /// Drop the connection (as a socket close would), triggering save-on-disconnect.
    pub fn disconnect(&mut self, session: Session) {
        self.app.world_mut().write_message(ConnectionClosed {
            connection: session.conn,
        });
        self.pump();
    }

    /// Override the idle thresholds (seconds) for this test.
    pub fn set_idle_config(&mut self, config: IdleConfig) {
        self.app.insert_resource(config);
    }

    /// Connections the server asked to sever since the last call (idle
    /// timeouts, quits). The headless harness has no transport, so nothing
    /// acts on these â€” they prove the sweep fired.
    pub fn drain_disconnects(&mut self) -> Vec<Entity> {
        let msgs = self.app.world().resource::<Messages<DisconnectRequest>>();
        self.disconnect_cursor
            .read(msgs)
            .map(|m| m.connection)
            .collect()
    }

    /// Reboot the world in place: rebuild the app on the SAME data directory
    /// and re-run the seed, as a server restart would. Disk state (accounts,
    /// characters, packs) survives; live entities do not. Old sessions are
    /// dead â€” reconnect afterwards.
    pub fn reboot(&mut self) {
        let soul_mode = self
            .app
            .world()
            .get_resource::<grim::CharacterBuildMode>()
            .is_some_and(|m| *m == grim::CharacterBuildMode::SavageSoul);
        let data_dir = self.data_dir.clone();
        let mut app = App::new();
        app.insert_resource(example_mud::DisableGrapevine);
        app.insert_resource(PersistenceConfig {
            dir: data_dir.clone(),
        });
        app.init_resource::<Time>();
        app.add_plugins(GrimHeadlessPlugins);
        app.insert_resource(example_mud::seed::AreaBlueprintDir(
            self.areas_dir.clone(),
        ));
        app.add_systems(Startup, example_mud::seed::seed_world);
        app.add_plugins(example_mud::shop::ShopPlugin);
        app.insert_resource(example_mud::help::HelpDir(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/help"),
        ));
        app.add_plugins(example_mud::help::HelpPlugin);
        if soul_mode {
            example_mud::species::install(&mut app);
            app.insert_resource(grim::CharacterBuildMode::SavageSoul);
            app.add_plugins(example_mud::savage_soul::SavageSoulPlugin);
            app.insert_resource(example_mud::population::PopulationEnabled(false));
        }
        app.update();
        self.app = app;
        self.next_conn = 1;
        // Entity ids recycle in the fresh world: drop every recorded buffer,
        // or a new connection could read a dead one's transcript.
        self.buffers.clear();
        self.read_offsets.clear();
        let cursor = self
            .app
            .world()
            .resource::<Messages<ConnectionOutput>>()
            .get_cursor();
        self.cursor = cursor;
        self.exit_cursor = self
            .app
            .world()
            .resource::<Messages<AppExit>>()
            .get_cursor();
        self.due_cursor = self
            .app
            .world()
            .resource::<Messages<grim::events::CopyoverDue>>()
            .get_cursor();
    }

    /// Send a shutdown-verb line (`shutdown|reboot|copyover N`) and settle,
    /// returning the output plus every exit/handoff request written along the
    /// way. Drained per tick because messages only survive a couple of frames
    /// â€” a drain after the pump would miss a request written mid-pump (and a
    /// `copyover` fires exactly once, so there is no sticky re-fire to catch
    /// up with).
    pub fn send_shutdown(&mut self, session: Session, line: &str) -> ShutdownOutcome {
        self.app.world_mut().write_message(ConnectionInput {
            connection: session.conn,
            text: line.to_string(),
        });
        let mut exits = Vec::new();
        let mut copyover_due = 0;
        for _ in 0..8 {
            self.tick();
            let exit_msgs = self.app.world().resource::<Messages<AppExit>>();
            exits.extend(self.exit_cursor.read(exit_msgs).cloned());
            let due_msgs = self
                .app
                .world()
                .resource::<Messages<grim::events::CopyoverDue>>();
            copyover_due += self.due_cursor.read(due_msgs).count();
        }
        ShutdownOutcome {
            output: self.collect_new(session.conn),
            exits,
            copyover_due,
        }
    }

    /// Names of player characters currently in the world (for assertions about
    /// state). Filtered to entities with a `Character`, so seeded rooms and NPCs
    /// (which also carry a name) are excluded.
    pub fn character_names(&mut self) -> Vec<String> {
        let mut q = self.app.world_mut().query::<(&GrimName, &Character)>();
        let mut names: Vec<String> = q
            .iter(self.app.world())
            .map(|(name, _)| name.0.clone())
            .collect();
        names.sort();
        names
    }

    /// Fetch a flat [`StoredCharacter`] view of the in-world character with this
    /// name, if present. Lets a scenario assert on stored build data
    /// (gender/race/class/level) which, post-split, lives across
    /// `Name + Actor + Character`.
    pub fn character(&mut self, name: &str) -> Option<StoredCharacter> {
        let mut q = self
            .app
            .world_mut()
            .query::<(&GrimName, &Actor, &Character)>();
        q.iter(self.app.world())
            .find(|(n, _, _)| n.0 == name)
            .map(|(n, a, c)| StoredCharacter::from_components(n, a, c))
    }

    /// Mutate the in-world character with this name via a flat [`StoredCharacter`]
    /// view. A test-only escape hatch for state the creation flow can't produce
    /// (admin role, level, race/class) so a scenario can set up a realistic WHO
    /// list. The edit is split back onto the `Actor`/`Character` components.
    pub fn edit_character(&mut self, name: &str, edit: impl FnOnce(&mut StoredCharacter)) {
        let mut q = self
            .app
            .world_mut()
            .query::<(&GrimName, &mut Actor, &mut Character)>();
        let world = self.app.world_mut();
        if let Some((n, mut actor, mut ch)) = q.iter_mut(world).find(|(n, _, _)| n.0 == name) {
            let mut stored = StoredCharacter::from_components(n, &actor, &ch);
            edit(&mut stored);
            let (_, new_actor, new_ch) = stored.into_components();
            *actor = new_actor;
            *ch = new_ch;
        }
    }

    /// Replace a mob's script triggers by mob name. Test-only determinism:
    /// the seed gates greetings on `rand()`, so scenarios pin unconditional
    /// scripts and keep asserting on the same speech text.
    pub fn set_mob_triggers(&mut self, name: &str, scripts: &[(TriggerKind, &str)]) {
        let mut q = self
            .app
            .world_mut()
            .query::<(&GrimName, &mut ScriptTriggers)>();
        let world = self.app.world_mut();
        let (_, mut triggers) = q
            .iter_mut(world)
            .find(|(n, _)| n.0 == name)
            .expect("scripted mob present");
        triggers.0 = scripts
            .iter()
            .map(|(on, src)| CompiledTrigger {
                on: *on,
                bytecode: compile(src).expect("valid script"),
            })
            .collect();
    }

    /// Rewrite a logged-out character's on-disk JSON to look like it was created
    /// before races/classes existed: empty race/class and the default gender,
    /// with id/account/level preserved. Lets a scenario drive the legacy-backfill
    /// login path (menu â†’ picker â†’ world). The character must be saved to disk
    /// (logged out) first.
    pub fn make_character_legacy(&self, name: &str) {
        let path = self
            .data_dir
            .join("characters")
            .join(format!("{name}.json"));
        let data = std::fs::read_to_string(&path).expect("character json on disk");
        let mut ch: StoredCharacter = serde_json::from_str(&data).unwrap();
        ch.gender = grim::components::Gender::Neutral;
        ch.race = String::new();
        ch.class = String::new();
        std::fs::write(&path, serde_json::to_string_pretty(&ch).unwrap()).unwrap();
    }
}

impl Default for Mud {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Mud {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}

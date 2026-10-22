//! Minimal MUD binary used by the copyover integration test (`tests/copyover.rs`).
//!
//! It is the *lightest* real server that can exercise a copyover end-to-end: the
//! full `GrimDefaultPlugins` stack (telnet transport + world + scene +
//! persistence + channel) over a real TCP port, seeded with the example world.
//! Port and data directory come from the environment so the test can bind an
//! unused port and isolate persistence in a temp dir:
//!
//! - `GRIM_TEST_PORT` — TCP port to listen on (default 4000)
//! - `GRIM_TEST_DATA` — persistence root directory (default `data`)
//! - `GRIM_AREAS_DIR` — area blueprint directory (default `data/areas`)
//!
//! On copyover (`SIGUSR2`) this process execs itself again; the successor
//! inherits the same environment, so it lands in the same data dir and adopts
//! the handed-over listener rather than binding the port afresh.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use example_mud::seed::{self, AreaBlueprintDir};
use grim::{GrimDefaultPlugins, PersistenceConfig};

fn main() {
    let port: u16 = std::env::var("GRIM_TEST_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4000);
    let data = std::env::var("GRIM_TEST_DATA").unwrap_or_else(|_| "data".to_string());

    // Each generation records its own PID so the test can signal the *current*
    // server directly across copyovers — mirroring how the real deploy signals
    // the systemd MainPID (which the MAINPID handoff keeps current), rather than
    // relying on process-group signals to reach a reparented successor.
    if let Ok(pidfile) = std::env::var("GRIM_TEST_PIDFILE") {
        let _ = std::fs::write(pidfile, std::process::id().to_string());
    }

    let mut app = App::new();
    // Same 60 Hz headless cap as `src/main.rs` (issue #121): this binary also
    // `app.run()`s a real server.
    app.add_plugins(
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 60.0,
        ))),
    );
    app.add_plugins(LogPlugin {
        filter: "info".into(),
        ..Default::default()
    });
    // Isolate persistence before the plugins init their default config.
    app.insert_resource(PersistenceConfig { dir: data.into() });
    // Area blueprints load from the filesystem; the test points this at the
    // repo's committed `data/areas` (default `data/areas` otherwise).
    if let Ok(areas) = std::env::var("GRIM_AREAS_DIR") {
        app.insert_resource(AreaBlueprintDir(areas.into()));
    }
    app.add_plugins(GrimDefaultPlugins {
        telnet_port: port,
        ..Default::default()
    });
    app.add_systems(Startup, seed::seed_world);
    app.run();
}

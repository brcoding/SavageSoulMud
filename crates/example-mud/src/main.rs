use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use example_mud::seed;
use grim::GrimDefaultPlugins;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--validate-area") {
        let result = args
            .get(2)
            .ok_or("Missing area path".to_string())
            .and_then(|path| std::fs::read_to_string(path).map_err(|e| e.to_string()))
            .and_then(|text| seed::validate_area_json(&text));
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }

    if args.get(1).is_some_and(|arg| arg == "--validate-races") {
        let result = args
            .get(2)
            .ok_or("Missing race catalogue path".to_string())
            .and_then(|path| std::fs::read_to_string(path).map_err(|e| e.to_string()))
            .and_then(|text| example_mud::species::Catalog::parse(&text).map(|_| ()));
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }

    if args.get(1).is_some_and(|arg| arg == "--validate-creator") {
        let result = args
            .get(2)
            .ok_or("Missing creator configuration path".to_string())
            .and_then(|path| std::fs::read_to_string(path).map_err(|e| e.to_string()))
            .and_then(|text| example_mud::creator::Settings::parse(&text).map(|_| ()));
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }

    let mut app = App::new();

    // Headless server: cap the frame loop at 60 Hz. Bare `MinimalPlugins` runs
    // `Update` with no wait, pinning a core at 99% while idle (issue #121) and
    // overflowing the change-detection window (the "has not run for N ticks"
    // WARN). Timed logic reads `Time` deltas, so the rate only sets latency.
    app.add_plugins(
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / 60.0,
        ))),
    );
    app.add_plugins(LogPlugin {
        filter: "info".into(),
        ..Default::default()
    });

    // The full GRIM stack, from one facade crate. Swap `GrimDefaultPlugins` for
    // the individual plugins (all under `grim::plugins`) to omit or replace any.
    app.add_plugins(GrimDefaultPlugins {
        telnet_port: 4000,
        ..Default::default()
    });
    example_mud::species::install(&mut app);
    app.insert_resource(grim::CharacterBuildMode::SavageSoul);
    app.insert_resource(grim::AccessibilityOnboarding(true));

    // Seed the world
    app.add_systems(Startup, seed::seed_world);
    app.add_plugins(example_mud::shop::ShopPlugin);
    app.add_plugins(example_mud::help::HelpPlugin);
    app.add_plugins(example_mud::intermud::IntermudPlugin);
    app.add_plugins(example_mud::savage_soul::SavageSoulPlugin);

    app.run();
}

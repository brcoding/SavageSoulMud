//! Cardinal direction verbs: `north|east|south|west|up|down` (and their
//! single-letter prefixes via the registry's priority matching), plus
//! `northeast|northwest|southeast|southwest` and `ne|nw|se|sw`.
//!
//! Registration lives here — like the warned-countdown verbs in
//! `countdown.rs` — to hold `parser.rs` under its size cap; parse-level
//! tests stay in `parser.rs` with the rest.

use grim_command::CommandRegistry;
use grim_core::cardinal::Cardinal;
use grim_core::events::Command;

/// Register compass directions and diagonal aliases. Called last in `build_registry`, so
/// each new command front-loads above the last and single-character input
/// (`n`/`e`/`s`/`w`/`u`/`d`) reaches movement first.
pub(crate) fn register(r: &mut CommandRegistry<Command>) {
    for (full, short, direction) in [
        ("northeast", "ne", Cardinal::Northeast),
        ("northwest", "nw", Cardinal::Northwest),
        ("southeast", "se", Cardinal::Southeast),
        ("southwest", "sw", Cardinal::Southwest),
    ] {
        for name in [full, short] {
            r.register(name, move |_| Some(Command::Move { direction }));
            r.deprioritize(name);
        }
    }
    r.register("north", |_| {
        Some(Command::Move {
            direction: Cardinal::North,
        })
    });
    r.register("east", |_| {
        Some(Command::Move {
            direction: Cardinal::East,
        })
    });
    r.register("south", |_| {
        Some(Command::Move {
            direction: Cardinal::South,
        })
    });
    r.register("west", |_| {
        Some(Command::Move {
            direction: Cardinal::West,
        })
    });
    r.register("up", |_| {
        Some(Command::Move {
            direction: Cardinal::Up,
        })
    });
    r.register("down", |_| {
        Some(Command::Move {
            direction: Cardinal::Down,
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagonals_resolve_without_changing_single_letter_moves() {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        for (word, direction) in [
            ("n", Cardinal::North),
            ("e", Cardinal::East),
            ("s", Cardinal::South),
            ("w", Cardinal::West),
            ("ne", Cardinal::Northeast),
            ("northeast", Cardinal::Northeast),
            ("nw", Cardinal::Northwest),
            ("northwest", Cardinal::Northwest),
            ("se", Cardinal::Southeast),
            ("southeast", Cardinal::Southeast),
            ("sw", Cardinal::Southwest),
            ("southwest", Cardinal::Southwest),
        ] {
            assert!(
                matches!(registry.resolve(word, ""), Some(Command::Move { direction: actual }) if actual == direction)
            );
        }
    }
}

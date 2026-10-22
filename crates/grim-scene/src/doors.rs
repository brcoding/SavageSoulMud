//! Door verbs: `open <direction>` / `close <direction>`.
//!
//! Both need a direction, so a bare `open` or `close` is unknown (like
//! `get`/`drop`). Registration lives here — factories plus the one
//! `register` entry — to hold `parser.rs` under its size cap; parse-level
//! tests stay in `parser.rs` with the rest.

use grim_command::CommandRegistry;
use grim_core::cardinal::Cardinal;
use grim_core::events::Command;

fn open(rest: &str) -> Option<Command> {
    let target = rest.trim();
    if target.is_empty() { return None; }
    Some(match Cardinal::parse(target) {
        Some(direction) => Command::Open { direction },
        None => Command::SavageSoul { verb: "open_object".into(), arg: target.into() },
    })
}

fn close(rest: &str) -> Option<Command> {
    let target = rest.trim();
    if target.is_empty() { return None; }
    Some(match Cardinal::parse(target) {
        Some(direction) => Command::Close { direction },
        None => Command::SavageSoul { verb: "close_object".into(), arg: target.into() },
    })
}

/// Register both door verbs, parked behind older prefixes so `o` still
/// reaches `ooc` and `c` still reaches `commands`/`config` (exact words
/// always match first).
pub(crate) fn register(r: &mut CommandRegistry<Command>) {
    r.register("open", open);
    r.register("close", close);
    r.deprioritize("open");
    r.deprioritize("close");
}

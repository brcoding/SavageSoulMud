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
    Cardinal::parse(rest.trim()).map(|direction| Command::Open { direction })
}

fn close(rest: &str) -> Option<Command> {
    Cardinal::parse(rest.trim()).map(|direction| Command::Close { direction })
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

use bevy::log::warn;
use grim_command::CommandRegistry;
use grim_core::events::{Command, DescOp};

/// Parse a raw input line into a Command using `registry`.
///
/// Prefix matching resolves by priority (see `grim-command`), so single-letter
/// direction shortcuts (n/e/s/w/u/d) work when north/east/... are registered.
pub fn parse_command(registry: &CommandRegistry<Command>, input: &str) -> Option<Command> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (word, rest) = trimmed.split_once(' ').unwrap_or((trimmed, ""));
    registry.resolve(word, rest.trim())
}

/// Build the command registry and log any contested prefixes once, so a
/// silently-shadowed abbreviation surfaces at startup rather than as a confused
/// player. The scene plugin inserts the result as a Bevy resource.
pub fn command_registry() -> CommandRegistry<Command> {
    let r = build_registry();
    for contest in r.contested_prefixes() {
        warn!(
            "command prefix '{}' resolves to '{}', shadowing {}",
            contest.prefix,
            contest.winner,
            contest.shadowed.join(", ")
        );
    }
    r
}

/// Parse `<target> <message>` for `tell`/`whisper`. Both parts required.
fn parse_tell(rest: &str) -> Option<Command> {
    let (target, text) = rest.split_once(' ')?;
    let (target, text) = (target.trim(), text.trim());
    if target.is_empty() || text.is_empty() {
        return None;
    }
    Some(Command::Tell {
        target: target.to_string(),
        text: text.to_string(),
    })
}

/// Parse `config [key] [value]`: bare lists settings, one word shows one,
/// two words set one. More words are unknown â€” values are single tokens.
fn parse_config(rest: &str) -> Option<Command> {
    let mut words = rest.split_whitespace();
    let key = words.next();
    let value = words.next();
    if words.next().is_some() {
        return None;
    }
    Some(Command::Config {
        key: key.map(str::to_string),
        value: value.map(str::to_string),
    })
}

/// Split `<item> <target>` for `give`/`steal`. Tokens are shell-like: an
/// optional `N*`/`N.` selector prefix plus a bare word or a `"quoted phrase"`.
/// Usually the item is the first token and the being everything after it
/// (`give "brass lantern" bob`), but an `all`-headed item (`all`, `all sword`,
/// `all.sword`) is greedy â€” it runs to the last token so the being stays one
/// word (`give all sword "Grimmok Ironhand"`, quoted when multi-word). Both
/// parts required; an unterminated quote is unknown.
fn split_transfer(rest: &str) -> Option<(String, String)> {
    let rest = rest.trim();
    let tokens = split_tokens(rest)?;
    if tokens.is_empty() {
        return None;
    }
    if is_all_head(&rest[tokens[0].0..tokens[0].1]) {
        if tokens.len() < 2 {
            return None;
        }
        let (item_end, target_start) = (tokens[tokens.len() - 2].1, tokens[tokens.len() - 1].0);
        return Some((
            rest[..item_end].trim().to_string(),
            rest[target_start..].trim().to_string(),
        ));
    }
    let (first_start, first_end) = tokens[0];
    let target = rest[first_end..].trim();
    if target.is_empty() {
        return None;
    }
    Some((rest[first_start..first_end].to_string(), target.to_string()))
}

/// Byte ranges of shell-like tokens in `rest`: `[N*|N.]?(bare|"quoted")`.
/// `None` on an unterminated quote.
fn split_tokens(rest: &str) -> Option<Vec<(usize, usize)>> {
    let bytes = rest.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        // Optional `N*` / `N.` selector prefix stays glued to its token.
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > i && j < bytes.len() && (bytes[j] == b'*' || bytes[j] == b'.') {
            i = j + 1;
        }
        if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            if i >= bytes.len() {
                return None;
            }
            i += 1;
            // Text glued to the closing quote (`"sword"bob`) is malformed:
            // without a delimiter the item/being divide is a guess.
            if i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                return None;
            }
        } else {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
        }
        tokens.push((start, i));
    }
    Some(tokens)
}

/// Whether a split token heads an `all` item: `all` plus a boundary (end,
/// `.`, `*`, or `"`) so `alloy` stays a plain word. Case-insensitive.
fn is_all_head(token: &str) -> bool {
    // Strip a glued `N*` / `N.` prefix first (`2.all` still heads an all).
    let mut head = token;
    if let Some(digits) = head.find(|c: char| !c.is_ascii_digit()) {
        if digits > 0 {
            let after = &head[digits..];
            if let Some(stripped) = after.strip_prefix(|c| c == '*' || c == '.') {
                head = stripped;
            }
        }
    }
    let Some(head3) = head.get(..3) else {
        return false;
    };
    if !head3.eq_ignore_ascii_case("all") {
        return false;
    }
    match head.as_bytes().get(3) {
        None => true,
        Some(b'.' | b'*' | b'"') => true,
        Some(_) => false,
    }
}

#[allow(clippy::too_many_lines)] // reason: flat command-registration list
fn build_registry() -> CommandRegistry<Command> {
    let mut r = CommandRegistry::new();

    // â”€â”€ Social commands â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    r.register("say", |rest| {
        if rest.is_empty() {
            None
        } else {
            Some(Command::Channel {
                channel: "say".to_string(),
                text: rest.to_string(),
            })
        }
    });
    r.register("yell", |rest| {
        if rest.is_empty() {
            None
        } else {
            Some(Command::Channel {
                channel: "yell".to_string(),
                text: rest.to_string(),
            })
        }
    });
    r.register("ooc", |rest| {
        if rest.is_empty() {
            None
        } else {
            Some(Command::Channel {
                channel: "ooc".to_string(),
                text: rest.to_string(),
            })
        }
    });
    // `channel <name> <text>` â€” a generic command for all configured channels.
    // This allows players to use custom channels added via ChannelRegistry.
    r.register("channel", |rest| {
        let rest = rest.trim();
        let (channel, text) = rest.split_once(' ')?;
        let (channel, text) = (channel.trim(), text.trim());
        if channel.is_empty() || text.is_empty() {
            return None;
        }
        Some(Command::Channel {
            channel: channel.to_string(),
            text: text.to_string(),
        })
    });
    // `tell <target> <message>` (alias: `whisper`). Needs both a target and a
    // non-empty message; a bare `tell` or `tell name` is rejected.
    r.register("tell", parse_tell);
    r.register("whisper", parse_tell);
    // `reply <message>` â€” to the last player who whispered you.
    r.register("reply", |rest| {
        let text = rest.trim();
        (!text.is_empty()).then(|| Command::Reply {
            text: text.to_string(),
        })
    });
    // `title <text>` sets the WHO title; a bare `title` clears it. Always parses
    // (empty = clear), so it is a recognized command either way.
    r.register("title", |rest| {
        Some(Command::Title {
            text: rest.trim().to_string(),
        })
    });

    // â”€â”€ Game actions â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    // Reading fixtures uses the same target lookup and description as looking.
    r.register("read", |rest| {
        (!rest.is_empty()).then(|| Command::Look {
            target: Some(rest.to_string()),
        })
    });
    r.deprioritize("read");
    r.register("look", |rest| {
        if rest.is_empty() {
            Some(Command::Look { target: None })
        } else {
            Some(Command::Look {
                target: Some(rest.to_string()),
            })
        }
    });
    // `desc` views your paragraphs; `desc clear` empties them,
    // `desc + <line>` appends one, `desc -` drops the last, `desc edit`
    // opens the line editor. Anything else (including a bare `+`) is unknown.
    r.register("desc", |rest| {
        let rest = rest.trim();
        if rest.is_empty() {
            Some(Command::Desc { op: DescOp::Show })
        } else if rest.eq_ignore_ascii_case("clear") {
            Some(Command::Desc { op: DescOp::Clear })
        } else if rest.eq_ignore_ascii_case("edit") {
            Some(Command::Desc { op: DescOp::Edit })
        } else if rest == "-" {
            Some(Command::Desc { op: DescOp::Remove })
        } else {
            let line = rest.strip_prefix('+')?.trim();
            (!line.is_empty()).then(|| Command::Desc {
                op: DescOp::Add(line.to_string()),
            })
        }
    });
    // `map` â€” bare only (`map <anything>` is unknown, like `sockets`). The `m`
    // prefix is uncontested, so this steals no abbreviation.
    r.register("map", |rest| rest.trim().is_empty().then_some(Command::Map));
    r.register("who", |_| Some(Command::Who));
    r.register("wizlist", |_| Some(Command::Wizlist));
    r.register("where", |_| Some(Command::Where));
    r.register("afk", |_| Some(Command::Afk));
    // `finger <name>` â€” rejected with no argument so a bare `finger` is unknown.
    r.register("finger", |rest| {
        let target = rest.trim();
        (!target.is_empty()).then(|| Command::Finger {
            target: target.to_string(),
        })
    });
    r.register("inventory", |_| Some(Command::Inventory));
    r.register("list", |rest| rest.is_empty().then_some(Command::ListShop));
    r.register("credits", |rest| rest.is_empty().then_some(Command::Credits));
    r.register("buy", |rest| (!rest.is_empty()).then(|| Command::Buy { target: rest.to_string() }));
    r.register("sell", |rest| (!rest.is_empty()).then(|| Command::Sell { target: rest.to_string() }));
    for word in ["list", "credits", "buy", "sell"] { r.deprioritize(word); }
    r.register("equipment", |_| Some(Command::Equipment));
    r.register("inv", |_| Some(Command::Inventory));
    r.register("eq", |_| Some(Command::Equipment));
    r.register("areas", |_| Some(Command::Areas));
    // `config [key] [value]` â€” list settings, cycle one, or set one.
    // Registered BEFORE `commands` so the shared `c`/`co` abbreviations keep
    // reaching the older verb (`con` never matched `commands` â€” `com` vs
    // `con` â€” so it and longer prefixes reach `config`); exact words match.
    r.register("config", parse_config);
    r.register("commands", |_| Some(Command::Commands));
    r.register("help", |rest| Some(Command::Help { topic: rest.to_string() }));
    // `sockets` â€” admin-only, session-local (masked at dispatch, see
    // `handle_ingame`). Takes no argument, so `sockets <anything>` is unknown.
    // Registered before the directions so single-letter/short prefixes still
    // resolve to movement first.
    r.register("sockets", |rest| {
        rest.trim().is_empty().then_some(Command::Sockets)
    });
    // `wiznet [on|off|security|logins]` â€” admin-only, engine-queued + masked
    // (see `handle_wiznet`). Bare lists; one token sets/toggles; more than
    // one word is unknown.
    r.register("wiznet", |rest| {
        let words: Vec<&str> = rest.split_whitespace().collect();
        match words.as_slice() {
            [] => Some(Command::Wiznet { arg: None }),
            [one] => Some(Command::Wiznet {
                arg: Some(one.to_string()),
            }),
            _ => None,
        }
    });
    // `wiz` is now contested (wizlist vs wiznet): keep it resolving to the
    // older verb so no muscle memory breaks (`wiznâ€¦` still reaches wiznet).
    r.prioritize("wizlist");
    r.register("quit", |_| Some(Command::Quit));
    r.register("exit", |_| Some(Command::Quit));
    // `recall` â€” return to the Town Square. Bare only (`recall <anything>`
    // is unknown, like `map`/`sockets`).
    r.register("recall", |rest| {
        rest.trim().is_empty().then_some(Command::Recall)
    });
    // Bare-only factories reject the rest line, and resolution never falls
    // through to the next prefix candidate â€” so a new bare verb must sink
    // below the older verbs sharing its prefixes (`reboot`/`copyover` do the
    // same): short `r`/`re` keep reaching `reply`, only `recâ€¦` reaches recall.
    r.deprioritize("recall");
    // `get <keyword>` / `drop <keyword>` â€” need a target, so a bare `get` or
    // `drop` is unknown. Registered before the admin verbs so the `ge` prefix
    // still resolves to `gecho` (later registrations win prefix ties).
    r.register("get", |rest| {
        let target = rest.trim();
        (!target.is_empty()).then(|| Command::Get {
            target: target.to_string(),
        })
    });
    r.register("drop", |rest| {
        let target = rest.trim();
        (!target.is_empty()).then(|| Command::Drop {
            target: target.to_string(),
        })
    });
    // Door verbs (`open|close <direction>`) live in `doors.rs`
    // (factories + priority); register them here with the rest.
    crate::doors::register(&mut r);
    // `give <item> <target>` / `steal <item> <target>` â€” quote-aware split
    // (`split_transfer`): the item is one token (selector prefixes and
    // `"quoted phrases"` stay glued), the being the rest â€” except an
    // `all`-headed item, which runs to the last token. Alongside get/drop so
    // the `ge` prefix still reaches `gecho`.
    r.register("give", |rest| {
        let (item, target) = split_transfer(rest)?;
        Some(Command::Give { item, target })
    });
    r.register("steal", |rest| {
        let (item, target) = split_transfer(rest)?;
        Some(Command::Steal { item, target })
    });

    // â”€â”€ Admin â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    // Warned-countdown verbs (`shutdown|reboot|copyover`) live in
    // `countdown.rs` (factories + priority); register them here with the rest.
    crate::countdown::register(&mut r);
    // `goto <address>` â€” admin-gated + masked at dispatch (see grim-scene
    // dispatcher). Rejected with no argument so a bare `goto` is unknown.
    r.register("goto", |rest| {
        let target = rest.trim();
        (!target.is_empty()).then(|| Command::Goto {
            target: target.to_string(),
        })
    });
    // `gecho <text>` â€” admin-gated + masked at dispatch. Rejected with no
    // argument so a bare `gecho` is unknown.
    r.register("gecho", |rest| {
        let text = rest.trim();
        (!text.is_empty()).then(|| Command::Gecho {
            text: text.to_string(),
        })
    });
    // `ban list [type]` / `ban add <type> <pattern>` /
    // `ban remove <type> <pattern>` â€” admin-gated + masked at dispatch (see
    // `handle_ingame`). Anything else (bare `ban`, unknown type, missing or
    // extra args) is unknown, like the other admin verbs.
    r.register("ban", crate::ban::parse_ban);
    // â”€â”€ Cardinal directions (last = highest priority for single-char) â”€
    crate::directions::register(&mut r);

    // `goto` and `gecho` share the `g` prefix. `register` front-loads priority,
    // so `gecho` (registered later) would otherwise win the bare `g`
    // abbreviation. `goto` predates `gecho` and `g` has long meant goto, so keep
    // it: promote `goto` above `gecho`. Longer prefixes (`geâ€¦`) still reach
    // `gecho` unambiguously.
    r.prioritize("goto");

    r
}
#[cfg(test)]
mod tests {
    use grim_command::CommandRegistry;
    use grim_core::cardinal::Cardinal;
    use grim_core::events::Command;
    use grim_core::events::{BanKind, BanOp, DescOp};

    use super::{command_registry, parse_command};

    /// Parse against a freshly-built default registry.
    fn parse(input: &str) -> Option<Command> {
        parse_command(&command_registry(), input)
    }

    // â”€â”€ Cardinal directions â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    #[test]
    fn test_north() {
        assert_eq!(
            parse("n"),
            Some(Command::Move {
                direction: Cardinal::North
            })
        );
        assert_eq!(
            parse("north"),
            Some(Command::Move {
                direction: Cardinal::North
            })
        );
    }

    #[test]
    fn test_east() {
        assert_eq!(
            parse("e"),
            Some(Command::Move {
                direction: Cardinal::East
            })
        );
        assert_eq!(
            parse("east"),
            Some(Command::Move {
                direction: Cardinal::East
            })
        );
    }

    #[test]
    fn test_south() {
        assert_eq!(
            parse("s"),
            Some(Command::Move {
                direction: Cardinal::South
            })
        );
        assert_eq!(
            parse("south"),
            Some(Command::Move {
                direction: Cardinal::South
            })
        );
    }

    #[test]
    fn test_west() {
        assert_eq!(
            parse("w"),
            Some(Command::Move {
                direction: Cardinal::West
            })
        );
        assert_eq!(
            parse("west"),
            Some(Command::Move {
                direction: Cardinal::West
            })
        );
    }

    #[test]
    fn test_up() {
        assert_eq!(
            parse("u"),
            Some(Command::Move {
                direction: Cardinal::Up
            })
        );
        assert_eq!(
            parse("up"),
            Some(Command::Move {
                direction: Cardinal::Up
            })
        );
    }

    #[test]
    fn test_down() {
        assert_eq!(
            parse("d"),
            Some(Command::Move {
                direction: Cardinal::Down
            })
        );
        assert_eq!(
            parse("down"),
            Some(Command::Move {
                direction: Cardinal::Down
            })
        );
    }

    // â”€â”€ Social commands â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    #[test]
    fn test_say_with_text() {
        assert_eq!(
            parse("say hello there"),
            Some(Command::Channel {
                channel: "say".to_string(),
                text: "hello there".to_string()
            })
        );
    }

    #[test]
    fn test_say_empty_is_none() {
        assert_eq!(parse("say"), None);
        assert_eq!(parse("say "), None);
    }

    #[test]
    fn test_say_quote_is_not_a_command() {
        // The `'` shorthand is gone: not a registered name, and no command
        // starts with it, so it resolves to nothing.
        assert_eq!(parse("' hello"), None);
        assert_eq!(parse("'"), None);
    }

    #[test]
    fn test_yell_with_text() {
        assert_eq!(
            parse("yell fire"),
            Some(Command::Channel {
                channel: "yell".to_string(),
                text: "fire".to_string()
            })
        );
    }

    #[test]
    fn test_yell_empty_is_none() {
        assert_eq!(parse("yell"), None);
    }

    #[test]
    fn test_ooc_with_text() {
        assert_eq!(
            parse("ooc anyone here?"),
            Some(Command::Channel {
                channel: "ooc".to_string(),
                text: "anyone here?".to_string()
            })
        );
    }

    #[test]
    fn test_ooc_empty_is_none() {
        assert_eq!(parse("ooc"), None);
    }

    #[test]
    fn test_tell_with_target_and_message() {
        assert_eq!(
            parse("tell Wrack hello there"),
            Some(Command::Tell {
                target: "Wrack".to_string(),
                text: "hello there".to_string()
            })
        );
    }

    #[test]
    fn test_whisper_is_alias_for_tell() {
        assert_eq!(
            parse("whisper self hi"),
            Some(Command::Tell {
                target: "self".to_string(),
                text: "hi".to_string()
            })
        );
    }

    #[test]
    fn test_tell_without_message_is_none() {
        assert_eq!(parse("tell"), None);
        assert_eq!(parse("tell Wrack"), None);
        assert_eq!(parse("tell Wrack   "), None);
    }

    // â”€â”€ Title â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    #[test]
    fn test_title_with_text() {
        assert_eq!(
            parse("title the Bold"),
            Some(Command::Title {
                text: "the Bold".to_string()
            })
        );
    }

    #[test]
    fn test_title_bare_clears() {
        // A bare `title` is a recognized command that clears (empty text).
        assert_eq!(
            parse("title"),
            Some(Command::Title {
                text: String::new()
            })
        );
        assert_eq!(
            parse("title   "),
            Some(Command::Title {
                text: String::new()
            })
        );
    }

    // â”€â”€ Look â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    // `l` is not a registered name; it reaches `look` by prefix.
    #[test]
    fn test_look_without_target() {
        assert_eq!(parse("look"), Some(Command::Look { target: None }));
        assert_eq!(parse("l"), Some(Command::Look { target: None }));
    }

    #[test]
    fn test_read_fixture() {
        assert_eq!(
            parse("read sign"),
            Some(Command::Look {
                target: Some("sign".to_string()),
            })
        );
        assert_eq!(parse("READ   sign"), parse("look sign"));
        assert_eq!(parse("read"), None);
    }

    #[test]
    fn test_look_with_target() {
        assert_eq!(
            parse("look statue"),
            Some(Command::Look {
                target: Some("statue".to_string())
            })
        );
        assert_eq!(
            parse("l statue"),
            Some(Command::Look {
                target: Some("statue".to_string())
            })
        );
    }

    // â”€â”€ Informational â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    #[test]
    fn test_who() {
        assert_eq!(parse("who"), Some(Command::Who));
    }

    #[test]
    fn test_afk() {
        assert_eq!(parse("afk"), Some(Command::Afk));
    }

    #[test]
    fn test_wizlist() {
        assert_eq!(parse("wizlist"), Some(Command::Wizlist));
    }

    #[test]
    fn test_where_cmd() {
        assert_eq!(parse("where"), Some(Command::Where));
    }

    #[test]
    fn test_finger() {
        assert_eq!(
            parse("finger wrack"),
            Some(Command::Finger {
                target: "wrack".to_string()
            })
        );
        assert_eq!(parse("finger"), None);
        assert_eq!(parse("finger   "), None);
    }

    #[test]
    fn test_desc() {
        assert_eq!(parse("desc"), Some(Command::Desc { op: DescOp::Show }));
        assert_eq!(
            parse("desc clear"),
            Some(Command::Desc { op: DescOp::Clear })
        );
        assert_eq!(
            parse("desc + Hello there."),
            Some(Command::Desc {
                op: DescOp::Add("Hello there.".to_string())
            })
        );
        assert_eq!(parse("desc -"), Some(Command::Desc { op: DescOp::Remove }));
        assert_eq!(parse("desc edit"), Some(Command::Desc { op: DescOp::Edit }));
        assert_eq!(parse("desc EDIT"), Some(Command::Desc { op: DescOp::Edit }));
        assert_eq!(parse("desc +"), None);
        assert_eq!(parse("desc +   "), None);
        assert_eq!(parse("desc bogus"), None);
    }

    #[test]
    fn test_wiznet() {
        assert_eq!(parse("wiznet"), Some(Command::Wiznet { arg: None }));
        assert_eq!(
            parse("wiz"),
            Some(Command::Wizlist),
            "contested prefix stays with the older verb"
        );
        assert_eq!(
            parse("wizn"),
            Some(Command::Wiznet { arg: None }),
            "unambiguous prefix resolves"
        );
        assert_eq!(
            parse("wiznet security"),
            Some(Command::Wiznet {
                arg: Some("security".into())
            })
        );
        assert_eq!(
            parse("WIZNET OFF"),
            Some(Command::Wiznet {
                arg: Some("OFF".into())
            }),
            "arg case preserved for the handler"
        );
        assert_eq!(parse("wiznet too many"), None);
    }

    #[test]
    fn test_sockets() {
        assert_eq!(parse("sockets"), Some(Command::Sockets));
        // Unambiguous prefix resolves; an argument rejects (bare verb only).
        assert_eq!(parse("sock"), Some(Command::Sockets));
        assert_eq!(parse("sockets foo"), None);
    }

    #[test]
    fn test_areas_cmd() {
        assert_eq!(parse("areas"), Some(Command::Areas));
    }

    #[test]
    fn test_goto_with_target() {
        assert_eq!(
            parse("goto haven:market-square"),
            Some(Command::Goto {
                target: "haven:market-square".to_string()
            })
        );
    }

    #[test]
    fn test_goto_without_target_is_none() {
        assert_eq!(parse("goto"), None);
        assert_eq!(parse("goto   "), None);
    }

    #[test]
    fn test_recall() {
        assert_eq!(parse("recall"), Some(Command::Recall));
        // Unambiguous prefix resolves; an argument rejects (bare verb only).
        assert_eq!(parse("rec"), Some(Command::Recall));
        assert_eq!(parse("recall tavern"), None);
        // The bare-only factory rejects, with no fall-through â€” so recall is
        // deprioritized and short `re` still reaches the older `reply`.
        assert_eq!(
            parse("re hello"),
            Some(Command::Reply {
                text: "hello".into()
            })
        );
    }

    #[test]
    fn test_g_prefix_resolves_to_goto_not_gecho() {
        // `goto` and `gecho` share the `g` prefix; `goto` is prioritized so the
        // bare `g` abbreviation keeps meaning goto (regression guard).
        assert_eq!(
            parse("g haven:market-square"),
            Some(Command::Goto {
                target: "haven:market-square".to_string()
            })
        );
    }

    #[test]
    fn test_ge_prefix_still_resolves_to_gecho() {
        assert_eq!(
            parse("ge hello world"),
            Some(Command::Gecho {
                text: "hello world".to_string()
            })
        );
    }

    #[test]
    fn test_gecho_with_text() {
        assert_eq!(
            parse("gecho server reboot soon"),
            Some(Command::Gecho {
                text: "server reboot soon".to_string()
            })
        );
    }

    #[test]
    fn test_gecho_without_text_is_none() {
        assert_eq!(parse("gecho"), None);
        assert_eq!(parse("gecho   "), None);
    }
    #[test]
    fn test_ban_list() {
        assert_eq!(
            parse("ban list"),
            Some(Command::Ban {
                op: BanOp::List { filter: None }
            })
        );
        assert_eq!(
            parse("ban list IP"),
            Some(Command::Ban {
                op: BanOp::List {
                    filter: Some(BanKind::Ip)
                }
            })
        );
        assert_eq!(
            parse("ban list account"),
            Some(Command::Ban {
                op: BanOp::List {
                    filter: Some(BanKind::Account)
                }
            })
        );
    }

    #[test]
    fn test_ban_add_remove() {
        assert_eq!(
            parse("ban add ip 127.0.*"),
            Some(Command::Ban {
                op: BanOp::Add {
                    kind: BanKind::Ip,
                    pattern: "127.0.*".to_string()
                }
            })
        );
        assert_eq!(
            parse("BAN REMOVE Character Villain"),
            Some(Command::Ban {
                op: BanOp::Remove {
                    kind: BanKind::Character,
                    pattern: "Villain".to_string()
                }
            })
        );
    }

    #[test]
    fn test_ban_malformed_is_none() {
        assert_eq!(parse("ban"), None);
        assert_eq!(parse("ban frobnicate"), None);
        assert_eq!(parse("ban list email"), None);
        assert_eq!(parse("ban list ip extra"), None);
        assert_eq!(parse("ban add ip"), None);
        assert_eq!(parse("ban add email x@y.z"), None);
        assert_eq!(parse("ban add ip 1.2.3.4 extra"), None);
        assert_eq!(parse("ban remove character"), None);
    }

    #[test]
    fn test_commands_and_help() {
        assert_eq!(parse("commands"), Some(Command::Commands));
        assert_eq!(parse("help"), Some(Command::Help { topic: String::new() }));
    }
    #[test]
    fn test_inventory_and_equipment() {
        assert_eq!(parse("inventory"), Some(Command::Inventory));
        assert_eq!(parse("inv"), Some(Command::Inventory));
        assert_eq!(parse("equipment"), Some(Command::Equipment));
        assert_eq!(parse("eq"), Some(Command::Equipment));
        // Single `e` still moves east; `eq` reaches equipment unambiguously.
        assert_eq!(
            parse("e"),
            Some(Command::Move {
                direction: Cardinal::East
            })
        );
        assert_eq!(parse("eq"), Some(Command::Equipment));
    }

    // â”€â”€ Get / drop â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    #[test]
    fn test_get_and_drop_need_targets() {
        assert_eq!(
            parse("get lantern"),
            Some(Command::Get {
                target: "lantern".into()
            })
        );
        assert_eq!(
            parse("drop lantern"),
            Some(Command::Drop {
                target: "lantern".into()
            })
        );
        assert_eq!(parse("get"), None);
        assert_eq!(parse("drop"), None);
    }

    #[test]
    fn test_ge_still_reaches_gecho_and_g_reaches_goto() {
        // `get` registers before the admin verbs, so the `ge` prefix keeps
        // resolving to `gecho` (later registrations win prefix ties).
        assert_eq!(
            parse("gecho hello"),
            Some(Command::Gecho {
                text: "hello".into()
            })
        );
        assert_eq!(
            parse("ge hello"),
            Some(Command::Gecho {
                text: "hello".into()
            })
        );
        assert_eq!(
            parse("g square"),
            Some(Command::Goto {
                target: "square".into()
            })
        );
    }

    // â”€â”€ Give / steal â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    #[test]
    fn test_give_and_steal_split_item_and_target() {
        assert_eq!(
            parse("give lantern bob"),
            Some(Command::Give {
                item: "lantern".into(),
                target: "bob".into()
            })
        );
        assert_eq!(
            parse("steal coin grimmok"),
            Some(Command::Steal {
                item: "coin".into(),
                target: "grimmok".into()
            })
        );
        // Either half missing is unknown.
        assert_eq!(parse("give"), None);
        assert_eq!(parse("give lantern"), None);
        assert_eq!(parse("steal"), None);
        assert_eq!(parse("steal coin"), None);
    }

    #[test]
    fn test_give_split_keeps_quoted_item_together() {
        assert_eq!(
            parse("give \"brass lantern\" bob"),
            Some(Command::Give {
                item: "\"brass lantern\"".into(),
                target: "bob".into()
            })
        );
        // Multi-word beings still work when the item is one word.
        assert_eq!(
            parse("give sword Grimmok Ironhand"),
            Some(Command::Give {
                item: "sword".into(),
                target: "Grimmok Ironhand".into()
            })
        );
        // Selector prefixes stay glued to a quoted phrase.
        assert_eq!(
            parse("give 2.\"brass lantern\" bob"),
            Some(Command::Give {
                item: "2.\"brass lantern\"".into(),
                target: "bob".into()
            })
        );
        // An unterminated quote is unknown, not a half-split guess.
        assert_eq!(parse("give \"brass bob"), None);
    }

    #[test]
    fn test_give_split_all_runs_to_last_token() {
        assert_eq!(
            parse("give all bob"),
            Some(Command::Give {
                item: "all".into(),
                target: "bob".into()
            })
        );
        assert_eq!(
            parse("give all sword bob"),
            Some(Command::Give {
                item: "all sword".into(),
                target: "bob".into()
            })
        );
        assert_eq!(
            parse("give all.sword bob"),
            Some(Command::Give {
                item: "all.sword".into(),
                target: "bob".into()
            })
        );
        assert_eq!(
            parse("give all \"brass lantern\" bob"),
            Some(Command::Give {
                item: "all \"brass lantern\"".into(),
                target: "bob".into()
            })
        );
        // A lone `all` has no being half.
        assert_eq!(parse("give all"), None);
        // `alloy` is a plain word, not an `all` item.
        assert_eq!(
            parse("give alloy bob"),
            Some(Command::Give {
                item: "alloy".into(),
                target: "bob".into()
            })
        );
    }

    #[test]
    fn test_give_split_multibyte_item_is_literal() {
        // Multibyte words must not panic the `all` boundary check, and they
        // split like any other first token.
        assert_eq!(
            parse("give Ã©pÃ©e bob"),
            Some(Command::Give {
                item: "Ã©pÃ©e".into(),
                target: "bob".into()
            })
        );
    }
    // â”€â”€ Quit â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    #[test]
    fn test_give_and_steal_reject_text_glued_to_quote() {
        // Without a delimiter after the closing quote the item/being divide
        // is a guess, so both verbs are unknown â€” never a half-split move.
        assert_eq!(parse("give \"sword\"bob"), None);
        assert_eq!(parse("give \"brass lantern\"x bob"), None);
        assert_eq!(parse("steal \"coin\"bob"), None);
        assert_eq!(parse("steal 2.\"coin\"x bob"), None);
        // Whitespace-delimited quotes still parse on both verbs.
        assert_eq!(
            parse("steal \"brass lantern\" bob"),
            Some(Command::Steal {
                item: "\"brass lantern\"".into(),
                target: "bob".into()
            })
        );
    }

    #[test]
    fn test_quit_and_exit() {
        assert_eq!(parse("quit"), Some(Command::Quit));
        assert_eq!(parse("exit"), Some(Command::Quit));
    }

    // â”€â”€ Shutdown (admin; gating happens at dispatch) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    #[test]
    fn test_shutdown_with_count() {
        assert_eq!(
            parse("shutdown 30"),
            Some(Command::Shutdown { seconds: 30 })
        );
    }

    #[test]
    fn test_shutdown_defaults_to_30() {
        assert_eq!(parse("shutdown"), Some(Command::Shutdown { seconds: 30 }));
        assert_eq!(
            parse("shutdown abc"),
            Some(Command::Shutdown { seconds: 30 })
        );
    }

    // â”€â”€ Reboot / copyover (admin; same countdown shape as shutdown) â”€â”€
    #[test]
    fn test_reboot_with_count() {
        assert_eq!(parse("reboot 10"), Some(Command::Reboot { seconds: 10 }));
    }

    #[test]
    fn test_reboot_defaults_to_30() {
        assert_eq!(parse("reboot"), Some(Command::Reboot { seconds: 30 }));
        assert_eq!(parse("reboot abc"), Some(Command::Reboot { seconds: 30 }));
    }

    #[test]
    fn test_copyover_with_count() {
        assert_eq!(
            parse("copyover 10"),
            Some(Command::Copyover { seconds: 10 })
        );
    }

    #[test]
    fn test_copyover_defaults_to_30() {
        assert_eq!(parse("copyover"), Some(Command::Copyover { seconds: 30 }));
        assert_eq!(
            parse("copyover abc"),
            Some(Command::Copyover { seconds: 30 })
        );
    }

    #[test]
    fn test_new_admin_verbs_do_not_steal_prefixes() {
        // `reboot`/`copyover` are deprioritized: short prefixes still reach
        // the older verbs; only long unambiguous prefixes reach the new ones.
        assert_eq!(
            parse("r hello"),
            Some(Command::Reply {
                text: "hello".into()
            })
        );
        assert_eq!(
            parse("re hello"),
            Some(Command::Reply {
                text: "hello".into()
            })
        );
        assert_eq!(parse("reb 10"), Some(Command::Reboot { seconds: 10 }));
        assert_eq!(parse("c"), Some(Command::Commands));
        assert_eq!(parse("co"), Some(Command::Commands));
        assert_eq!(parse("copy 10"), Some(Command::Copyover { seconds: 10 }));
    }

    #[test]
    fn test_open_close_parse_and_keep_prefixes() {
        // Bare `open`/`close` are unknown (like get/drop); directions parse
        // full or abbreviated. Deprioritized: `o` still reaches `ooc`, `c`
        // still reaches `commands`/`config`, only long prefixes reach these.
        assert_eq!(parse("open"), None);
        assert_eq!(parse("close"), None);
        assert_eq!(
            parse("open east"),
            Some(Command::Open {
                direction: Cardinal::East
            })
        );
        assert_eq!(
            parse("open e"),
            Some(Command::Open {
                direction: Cardinal::East
            })
        );
        assert_eq!(
            parse("close north"),
            Some(Command::Close {
                direction: Cardinal::North
            })
        );
        assert_eq!(parse("open nowhere"), None);
        assert_eq!(
            parse("ooc anyone here?"),
            Some(Command::Channel {
                channel: "ooc".to_string(),
                text: "anyone here?".to_string()
            })
        );
        assert_eq!(parse("c"), Some(Command::Commands));
        assert_eq!(
            parse("op east"),
            Some(Command::Open {
                direction: Cardinal::East
            })
        );
        assert_eq!(
            parse("cl north"),
            Some(Command::Close {
                direction: Cardinal::North
            })
        );
    }

    // â”€â”€ Edge cases â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    #[test]
    fn test_empty_input() {
        assert_eq!(parse(""), None);
    }

    #[test]
    fn test_whitespace_only() {
        assert_eq!(parse("   "), None);
        assert_eq!(parse("\t"), None);
        assert_eq!(parse(" \t "), None);
    }

    #[test]
    fn test_unknown_command() {
        assert_eq!(parse("foobar"), None);
        assert_eq!(parse("xyzzy"), None);
        assert_eq!(parse("123"), None);
    }

    #[test]
    fn test_direction_wins_over_social() {
        // "n" is a prefix of "north", which is registered â€” direction wins
        assert_eq!(
            parse("n hello"),
            Some(Command::Move {
                direction: Cardinal::North
            })
        );
        assert_eq!(
            parse("s"),
            Some(Command::Move {
                direction: Cardinal::South
            })
        );
    }

    #[test]
    fn test_map_bare_and_abbreviation() {
        // The `m` prefix is uncontested: both the word and the abbreviation
        // reach `map`, while anything with arguments is unknown.
        assert_eq!(parse("map"), Some(Command::Map));
        assert_eq!(parse("m"), Some(Command::Map));
        assert_eq!(parse("ma"), Some(Command::Map));
        assert_eq!(parse("map foo"), None);
    }

    #[test]
    fn test_config_list_show_set() {
        assert_eq!(
            parse("config"),
            Some(Command::Config {
                key: None,
                value: None
            })
        );
        assert_eq!(
            parse("config minimap"),
            Some(Command::Config {
                key: Some("minimap".into()),
                value: None
            })
        );
        assert_eq!(
            parse("config minimap off"),
            Some(Command::Config {
                key: Some("minimap".into()),
                value: Some("off".into())
            })
        );
        // Values are single tokens: a third word is unknown.
        assert_eq!(parse("config minimap off now"), None);
    }

    #[test]
    fn test_config_keeps_older_c_prefixes() {
        // `config` registers before `commands`, so the shared `c`/`co`
        // abbreviations still reach the older verb. `con` is not a prefix of
        // `commands` at all (`com` vs `con`) â€” it unambiguously reaches
        // `config`, as does `conf`.
        assert_eq!(parse("c"), Some(Command::Commands));
        assert_eq!(parse("co"), Some(Command::Commands));
        assert_eq!(
            parse("con"),
            Some(Command::Config {
                key: None,
                value: None
            })
        );
        assert_eq!(
            parse("conf"),
            Some(Command::Config {
                key: None,
                value: None
            })
        );
    }

    // â”€â”€ Prefix matching / registration order â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn test_prefix_match_partial() {
        // "no" is prefix of "north" (default registry doesn't include "note")
        assert_eq!(
            parse("no"),
            Some(Command::Move {
                direction: Cardinal::North
            })
        );
        // "so" is prefix of "south"
        assert_eq!(
            parse("so"),
            Some(Command::Move {
                direction: Cardinal::South
            })
        );
        // "ea" is prefix of "east"
        assert_eq!(
            parse("ea"),
            Some(Command::Move {
                direction: Cardinal::East
            })
        );
        // "we" is prefix of "west"
        assert_eq!(
            parse("we"),
            Some(Command::Move {
                direction: Cardinal::West
            })
        );
        // "do" is prefix of "down"
        assert_eq!(
            parse("do"),
            Some(Command::Move {
                direction: Cardinal::Down
            })
        );
    }

    #[test]
    fn test_case_insensitivity() {
        assert_eq!(
            parse("NORTH"),
            Some(Command::Move {
                direction: Cardinal::North
            })
        );
        assert_eq!(
            parse("N"),
            Some(Command::Move {
                direction: Cardinal::North
            })
        );
        assert_eq!(
            parse("NoRtH"),
            Some(Command::Move {
                direction: Cardinal::North
            })
        );
        assert_eq!(
            parse("SAY Hello"),
            Some(Command::Channel {
                channel: "say".to_string(),
                text: "Hello".to_string()
            })
        );
        assert_eq!(parse("LOOK"), Some(Command::Look { target: None }));
    }

    /// Test custom registry with registration-order priority.
    /// Builds its own registry to avoid relying on the global static.
    #[test]
    fn test_custom_registry_order() {
        let mut r = CommandRegistry::new();
        r.register("note", |rest| {
            Some(Command::Channel {
                channel: "say".to_string(),
                text: rest.to_string(),
            })
        });
        r.register("nordic", |rest| {
            Some(Command::Channel {
                channel: "yell".to_string(),
                text: rest.to_string(),
            })
        });
        r.register("north", |_: &str| {
            Some(Command::Move {
                direction: Cardinal::North,
            })
        });

        // "n" is prefix of all three; last registered (north) wins
        assert_eq!(
            r.resolve("n", ""),
            Some(Command::Move {
                direction: Cardinal::North
            })
        );
        // "no" matches note + north; north wins
        assert_eq!(
            r.resolve("no", ""),
            Some(Command::Move {
                direction: Cardinal::North
            })
        );
        // "nor" matches nordic + north; north wins
        assert_eq!(
            r.resolve("nor", ""),
            Some(Command::Move {
                direction: Cardinal::North
            })
        );
        // "nort" matches only north
        assert_eq!(
            r.resolve("nort", ""),
            Some(Command::Move {
                direction: Cardinal::North
            })
        );
        // "nord" matches only nordic
        assert_eq!(
            r.resolve("nord", ""),
            Some(Command::Channel {
                channel: "yell".to_string(),
                text: "".to_string()
            })
        );
        // "not" matches only note
        assert_eq!(
            r.resolve("not", ""),
            Some(Command::Channel {
                channel: "say".to_string(),
                text: "".to_string()
            })
        );
    }
}

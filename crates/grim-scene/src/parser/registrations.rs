//! Command registration groups, called in their original priority order.

use super::*;

pub(super) fn register_speech(r: &mut CommandRegistry<Command>) {
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
}

pub(super) fn register_inspection(r: &mut CommandRegistry<Command>) {
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
}

pub(super) fn register_presence(r: &mut CommandRegistry<Command>) {
    r.register("wizinvis", |rest| {
        match rest.trim().to_ascii_lowercase().as_str() {
            "" => Some(Command::WizInvis { enabled: None }),
            "on" => Some(Command::WizInvis {
                enabled: Some(true),
            }),
            "off" => Some(Command::WizInvis {
                enabled: Some(false),
            }),
            _ => None,
        }
    });
    r.register("testaccount", |rest| {
        let parts: Vec<_> = rest.split_whitespace().collect();
        if parts.len() != 2 {
            return None;
        }
        let enabled = match parts[1].to_ascii_lowercase().as_str() {
            "on" => true,
            "off" => false,
            _ => return None,
        };
        Some(Command::TestAccount {
            target: parts[0].into(),
            enabled,
        })
    });
    // Preserve the established title/tell abbreviations for ordinary players.
    r.deprioritize("testaccount");
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
}

pub(super) fn register_inventory(r: &mut CommandRegistry<Command>) {
    r.register("list", |rest| rest.is_empty().then_some(Command::ListShop));
    r.register("credits", |rest| {
        rest.is_empty().then_some(Command::Credits)
    });
    r.register("buy", |rest| {
        (!rest.is_empty()).then(|| Command::Buy {
            target: rest.to_string(),
        })
    });
    r.register("sell", |rest| {
        (!rest.is_empty()).then(|| Command::Sell {
            target: rest.to_string(),
        })
    });
    for word in ["list", "credits", "buy", "sell"] {
        r.deprioritize(word);
    }
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
    r.register("help", |rest| {
        Some(Command::Help {
            topic: rest.to_string(),
        })
    });
    // `sockets` â€” admin-only, session-local (masked at dispatch, see
    // `handle_ingame`). Takes no argument, so `sockets <anything>` is unknown.
    // Registered before the directions so single-letter/short prefixes still
    // resolve to movement first.
}

pub(super) fn register_session(r: &mut CommandRegistry<Command>) {
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
}

pub(super) fn register_admin(r: &mut CommandRegistry<Command>) {
    // Door verbs (`open|close <direction>`) live in `doors.rs`
    // (factories + priority); register them here with the rest.
    crate::doors::register(r);
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
    crate::countdown::register(r);
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
    crate::directions::register(r);

    // `goto` and `gecho` share the `g` prefix. `register` front-loads priority,
    // so `gecho` (registered later) would otherwise win the bare `g`
    // abbreviation. `goto` predates `gecho` and `g` has long meant goto, so keep
    // it: promote `goto` above `gecho`. Longer prefixes (`geâ€¦`) still reach
    // `gecho` unambiguously.
    r.prioritize("goto");
}

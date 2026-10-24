//! Command vocabulary; reexported by the parent events module.
use super::*;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    WizInvis {
        enabled: Option<bool>,
    },
    TestAccount {
        target: String,
        enabled: bool,
    },
    /// Editable game help topic.
    Help {
        topic: String,
    },
    /// Game-specific Savage Soul character, equipment, and combat commands.
    SavageSoul {
        verb: String,
        arg: String,
    },
    ListShop,
    Buy {
        target: String,
    },
    Sell {
        target: String,
    },
    Credits,
    /// `look` or `look <target>` — the target is a `grim-target` being spec
    /// (`2.goblin` looks at the second; `"two words"` needs every word).
    Look {
        target: Option<String>,
    },
    /// `config` — list registered settings, show one (`config minimap`), or
    /// set one (`config minimap off`). Handled in `grim-actor`.
    Config {
        key: Option<String>,
        value: Option<String>,
    },
    /// `map` — render the area around the actor's room as ASCII (`@` self,
    /// `o` rooms, `--`/`|` exits, `,`/`'` up/down markers).
    Map,
    /// `say <text>` — room-scoped
    Say {
        text: String,
    },
    /// `yell <text>` — area-scoped
    Yell {
        text: String,
    },
    /// `ooc <text>` — global
    Ooc {
        text: String,
    },
    /// `tell <target> <text>` — private message to one player (fuzzy-matched by
    /// name; `self` targets the sender).
    Tell {
        target: String,
        text: String,
    },
    /// `reply <text>` — private message to the last player who whispered you.
    Reply {
        text: String,
    },
    /// `<social> [target]` — a data-driven social (`grin`, `smile`, …,
    /// registered at startup from `grim-social`). `target` is the first word
    /// of the rest of the line, when present; the handler resolves it against
    /// the actor's roommates (`self` targets the sender).
    Social {
        name: String,
        target: Option<String>,
    },
    /// `socials` — list the data-driven socials (registered at startup from
    /// `grim-social`). Answered from the command registry, like `commands`.
    SocialList,
    /// `channel <name> <text>` — unified channel command (data-driven).
    Channel {
        channel: String,
        text: String,
    },
    /// `title <text>` sets the actor's WHO title (max 60 chars); a bare `title`
    /// (empty text) clears it.
    Title {
        text: String,
    },
    /// `desc` views your description paragraphs; `desc clear` empties them,
    /// `desc + <line>` appends one, `desc -` drops the last, `desc edit`
    /// opens the line editor.
    Desc {
        op: DescOp,
    },
    /// Movement via cardinal direction
    Move {
        direction: Cardinal,
    },
    /// `open <direction>` — open the door guarding that exit.
    /// Bare `open` is rejected as unknown (like `get`/`drop`).
    Open {
        direction: Cardinal,
    },
    /// `close <direction>` — close the door guarding that exit.
    Close {
        direction: Cardinal,
    },
    /// `recall` — return to the Town Square (`haven:square`). No-op with a
    /// reply when already there. Available to every player.
    Recall,
    /// `quit` — clean disconnect
    Quit,
    /// `who` — list connected players
    Who,
    /// `wizlist` — list admin-flagged characters online
    Wizlist,
    /// `where` — show who's in your area and their room
    Where,
    /// `finger <name>` — show a character's description, online or off.
    /// Online characters answer from their live description; offline ones
    /// load from disk (custom descriptions do not persist yet, so offline
    /// output is the default description for now).
    Finger {
        target: String,
    },
    /// `sockets` — admin-only, session-local. List every live connection
    /// (id, address, session state, character, account, idle seconds).
    /// Masked as unknown for non-admins, like the other admin verbs.
    Sockets,
    /// `wiznet` / `wiznet on|off|security|logins` — admin-only. List and
    /// toggle the admin-alert (wiznet) categories. Masked as unknown for
    /// non-admins, like the other admin verbs.
    Wiznet {
        arg: Option<String>,
    },
    /// `afk` — flag yourself as away. Auto-set after `IdleConfig::afk_after_secs`
    /// idle; any input line clears it. Shows on `who`; prompt becomes `<AFK>`.
    Afk,
    /// `inventory` — list the short names of carried objects.
    Inventory,
    /// `equipment` — dummy: always reports empty (no item system yet).
    Equipment,
    /// `get <target>` — pick up matches from the room. A `grim-target` item
    /// spec: one match by default, `2.coin` the second, `3*coin` up to three,
    /// `all [words]` every match.
    Get {
        target: String,
    },
    /// `drop <target>` — drop matches from the pack (same selectors as `get`).
    Drop {
        target: String,
    },
    /// `give <item> <target>` — hand matches to a being here. Item-side
    /// selectors as in `get` (`give all sword bob`); the being is one
    /// recipient (`2.bob` for the second).
    Give {
        item: String,
        target: String,
    },
    /// `steal <item> <target>` — take matches from a being's pack here.
    /// Selectors as in `give`. Existence checks only; no skill checks
    /// (example workflow).
    Steal {
        item: String,
        target: String,
    },
    /// `commands` — list all registered commands
    Commands,
    /// `areas` — list every area in the world by its slug.
    Areas,
    /// `goto <address>` — admin-only. Teleport to a room resolved from an
    /// address (an entity id, `<area>:<room>`, or a bare room slug/grim id).
    Goto {
        target: String,
    },
    /// `gecho <text>` — admin-only. Echo a message to every player in the world,
    /// including the sender. Other admins see it attributed (`Name> text`);
    /// everyone else sees the raw text.
    Gecho {
        text: String,
    },
    /// `shutdown [seconds]` — admin-only. Schedules a graceful server shutdown
    /// after a countdown, broadcasting warnings to all connected players.
    Shutdown {
        seconds: u64,
    },
    /// `reboot [seconds]` — admin-only. Like `shutdown`, but the process exits
    /// non-zero at expiry so the service manager restarts it (cold restart:
    /// connections drop, the world reloads from disk).
    Reboot {
        seconds: u64,
    },
    /// `copyover [seconds]` — admin-only. Warns like `shutdown`, then hands the
    /// live listener + player sockets to a successor process (hot restart:
    /// players stay connected). The telnet transport performs the handoff.
    Copyover {
        seconds: u64,
    },
    /// `ban list [type]` — admin-only. List bans, optionally filtered by
    /// `ip` / `account` / `character`.
    /// `ban add <type> <pattern>` — admin-only. Block an IP (exact or
    /// `*`-wildcard prefix like `127.0.*`), an account (identifier or id), or
    /// a character (name; case-insensitive). Kicks every matching session.
    /// `ban remove <type> <pattern>` — admin-only. Lift a ban.
    Ban {
        op: BanOp,
    },
}

use grim_color::{escape_codes, truncate_visible, visible_width};
use grim_text::{table, tr, Align};

/// Room identity ids appended to a room title for admins only.
pub struct RoomDebugIds<'a> {
    /// Boot-local entity id (`Entity::to_bits`).
    pub entity: u64,
    /// Stable grim id. (Today this is the room's `GrimId`; becomes the base62
    /// Grim ID once that lands — see `docs/adr/0001`.)
    pub grim: &'a str,
    /// Human-facing slug (`friendly_id`).
    pub slug: &'a str,
}

/// A room's title line: the name renders `{B…{x` (blue, self-terminated).
/// Admins additionally see the entity id, grim id, and slug for
/// building/debugging — plain, after the reset, so the blue never bleeds
/// into the parenthesized ids: `{BName{x} (entity:… grim:… slug:…)`.
pub fn room_title(name: &str, debug: Option<RoomDebugIds>) -> String {
    match debug {
        Some(d) => format!(
            "{{B{name}{{x (entity:{} grim:{} slug:{})", // tr-bypass: structural title framing + debug ids, no prose
            d.entity, d.grim, d.slug
        ),
        None => format!("{{B{name}{{x"), // tr-bypass: structural title framing, no prose
    }
}

/// Format a room's full description.
///
/// `presence` holds one ready-made listing line per other being in the room
/// (a creature's long description, `"<name> is standing here."` for a
/// player) — each renders on its own line under the exits. `exits` holds the
/// open directions (plain exits plus open doors), `doors` the closed doors.
/// `Exits:` and `Doors:` always render on one line (with `none` when empty).
/// Admins additionally get the hidden directions appended on the same line
/// (`Secret: …`, itself `none` when the room hides nothing); non-admins
/// never see the heading at all. All groups sit on one line, wrapped in
/// blank lines above and below.
pub fn format_room(
    name: &str,
    desc: &str,
    exits: &[String],
    doors: &[String],
    admin_secret: Option<&[String]>,
    presence: &[String],
) -> String {
    let mut out = format!("{}\n{}", name, desc);
    let list = |items: &[String]| {
        if items.is_empty() {
            "none".to_string()
        } else {
            items.join(" ")
        }
    };
    out.push_str(&format!(
        // tr-bypass: one listing line joining caller-built direction names; `none` is the empty-list marker
        "\n\nExits: {}  Doors: {}",
        list(exits),
        list(doors),
    ));
    // The heading is admin-only: `Some` renders it (even as `none`), `None`
    // omits it entirely — non-admins get no hint that secrets exist.
    if let Some(secret) = admin_secret {
        out.push_str(&format!("  Secret: {}", list(secret))); // tr-bypass: admin-only segment joining caller-built direction names
    }
    if !presence.is_empty() {
        out.push_str(&format!("\n\n{}", presence.join("\n"))); // tr-bypass: joining caller-built presence lines
    }
    out.push('\n');
    out
}

/// Minimap canvas width: every stapled row reserves this gutter plus two
/// spaces before the room text.
pub const MINIMAP_WIDTH: usize = 9;

/// Staple minimap rows to the left of room text: each canvas row pads to
/// [`MINIMAP_WIDTH`] plus two spaces, then the corresponding text line. The
/// short side pads with blanks, and padding never leaves trailing spaces — a
/// missing map row emits blanks, a missing text line emits the map row bare.
pub fn staple_minimap(map_rows: &[String], text: &str) -> String {
    staple_minimap_width(map_rows, text, 80)
}

pub fn staple_minimap_width(map_rows: &[String], text: &str, columns: usize) -> String {
    // Wrap before zipping: client-side wrapping would put continuation text
    // back in column zero, visually breaking the next minimap row.
    let text_rows = wrap_room_column(text, columns.clamp(20, 1000) - MINIMAP_WIDTH - 2);
    let height = map_rows.len().max(text_rows.len());
    let mut out = String::new();
    for i in 0..height {
        let map = map_rows.get(i).map(String::as_str).unwrap_or("");
        let line = text_rows.get(i).map(String::as_str).unwrap_or("");
        if line.is_empty() {
            out.push_str(map);
        } else if map.is_empty() {
            out.push_str(&format!("{:MINIMAP_WIDTH$}  {line}", ""));
        } else {
            // Pad on *visible* columns: colour markup rides along in map rows
            // and byte-count padding would silently stop firing on them.
            let pad = MINIMAP_WIDTH.saturating_sub(visible_width(map));
            out.push_str(map);
            out.push_str(&" ".repeat(pad + 2));
            out.push_str(line);
        }
        if i + 1 < height {
            out.push('\n');
        }
    }
    out
}

/// Wrap the room column using rendered width, preserving explicit newlines
/// and complete colour tokens. Long unbroken names/URLs must fit too.
fn wrap_room_column(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for paragraph in text.split('\n') {
        if visible_width(paragraph) <= width {
            rows.push(paragraph.to_string());
            continue;
        }
        let mut line = String::new();
        for mut word in paragraph.split_whitespace() {
            if !line.is_empty() && visible_width(&line) + 1 + visible_width(word) > width {
                rows.push(std::mem::take(&mut line));
            }
            while visible_width(word) > width {
                let chunk = truncate_visible(word, width);
                word = word.get(chunk.len()..).unwrap_or(""); // Token-safe UTF-8 prefix.
                rows.push(chunk);
            }
            if !word.is_empty() {
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
            }
        }
        rows.push(line);
    }
    rows
}

/// Format a look at a specific entity.
pub fn format_entity(name: &str, desc: &str) -> String {
    format!("{}\n{}\n", name, desc)
}

pub fn format_say(speaker: &str, text: &str) -> String {
    tr!("social.say.third_party", speaker = speaker, text = text)
}

/// Format a yell message broadcast to an area.
///
/// `text` is escaped: these still build their string with `format!` rather than
/// going through the catalog, so the escaping `tr` performs does not apply and
/// has to be done here. Once these become channel configuration they inherit it.
pub fn format_yell(speaker: &str, text: &str) -> String {
    format!("{} yells, '{}'\n", speaker, escape_codes(text))
}

/// Format an OOC message broadcast globally.
///
/// See [`format_yell`] for why `text` is escaped here.
pub fn format_ooc(speaker: &str, text: &str) -> String {
    format!("[OOC] {}: {}\n", speaker, escape_codes(text))
}

/// Format a channel message based on the catalog key from the channel config.
/// The catalog key determines which template to use (e.g., "channel.say.third_party").
pub fn format_channel_message(catalog_key: &str, speaker: &str, text: &str) -> String {
    // Construct the full catalog key from the channel key
    // For example: "channel.say.third_party"
    let full_key = format!("{}.third_party", catalog_key);
    tr!(full_key.as_str(), speaker = speaker, text = text)
}

/// Render an admin `gecho`. `sender: Some(name)` attributes it (`Name> text`)
/// for other admins; `None` yields the raw text (sender + non-admins). Text is
/// escaped so a broadcast can't inject colour codes.
pub fn format_gecho(sender: Option<&str>, text: &str) -> String {
    match sender {
        Some(name) => format!("{}> {}\n", name, escape_codes(text)),
        None => format!("{}\n", escape_codes(text)),
    }
}

/// Format a movement broadcast.
pub fn format_move(actor: &str, direction: &str, leaving: bool) -> String {
    if leaving {
        format!("{} leaves {}.\n", actor, direction)
    } else {
        format!("{} arrives.\n", actor)
    }
}

/// Format the where list (same-area beings with room names, actor included).
#[allow(dead_code)]
pub fn format_where_list(area: &str, entries: &[(String, String)]) -> String {
    if entries.is_empty() {
        "No one else in this area.\n".into()
    } else {
        let mut out = format!("In your area ({}):\n", area);
        for (name, room) in entries {
            out.push_str(&format!("  {} in [{}]\n", name, room));
        }
        out
    }
}

/// Format the `areas` list: each area's slug and display name, already sorted.
#[allow(dead_code)]
pub fn format_areas_list(entries: &[(String, String)]) -> String {
    if entries.is_empty() {
        return "No areas exist.\n".into();
    }
    let mut out = format!("Areas ({}):\n", entries.len());
    for (slug, name) in entries {
        out.push_str(&format!("  {} — {}\n", slug, name));
    }
    out
}

/// One live connection's `sockets` row inputs, already resolved to
/// display-ready strings by the caller (`handle_ingame`).
pub struct SocketRow {
    /// Transport-side connection id (`Connection::id`).
    pub id: usize,
    /// Remote address (`Connection::addr`).
    pub addr: String,
    /// Session-state label (e.g. `InGame`, `Login`).
    pub state: String,
    /// Character name, or `"-"` while the session has none.
    pub character: String,
    /// Account identifier, or `"-"` while the session has none.
    pub account: String,
    /// Seconds since the session's last input, or `None` when never seen.
    pub idle_secs: Option<u64>,
}
/// Render the full `sockets` list as an aligned table (`table` in grim-text).
/// Cells are escaped first, so connection data (notably `@`-bearing account
/// identifiers) can never read as colour markup; widths measure visible
/// columns, so the escaped `@@` pairs don't over-pad.
pub fn format_sockets_list(rows: &[SocketRow]) -> String {
    if rows.is_empty() {
        return tr!("sockets.empty");
    }
    let count = rows.len().to_string();
    let mut out = tr!("sockets.header", total = count);
    let headers = vec![
        "ID".to_string(),
        "IP".to_string(),
        "State".to_string(),
        "Name".to_string(),
        "Account".to_string(),
        "Idle".to_string(),
    ];
    let body: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            vec![
                escape_codes(&row.id.to_string()),
                escape_codes(&row.addr),
                escape_codes(&row.state),
                escape_codes(&row.character),
                escape_codes(&row.account),
                escape_codes(
                    &row.idle_secs
                        .map(|s| format!("{s}s"))
                        .unwrap_or_else(|| "-".to_string()),
                ),
            ]
        })
        .collect();
    out.push_str(&table(
        headers,
        body,
        vec![
            Align::Right,
            Align::Left,
            Align::Left,
            Align::Left,
            Align::Left,
            Align::Right,
        ],
    ));
    out
}

/// Format the command list: a grid of resolvable keywords, no help text.
/// Explanations live behind a future `help <command>`; this only shows what
/// the parser accepts (statics plus data-driven names like socials), packed
/// row-major into 80 columns.
pub fn format_commands(names: &[String]) -> String {
    "Available commands:\n".to_string() + &grim_text::column_grid(names, 80)
}

/// Choose one nonblank line per login. Runtime reads let operators edit the
/// list without restarting; the legacy MOTD remains a safe fallback.
pub fn format_motd() -> String {
    std::fs::read_to_string("assets/motdlist.txt")
        .ok()
        .and_then(|list| random_motd(&list))
        .unwrap_or_else(|| {
            std::fs::read_to_string("assets/motd.txt")
                .unwrap_or_else(|_| include_str!("../../../assets/motd.txt").to_string())
        })
}

/// MOTD for immediate reconnects, which do not pause for Enter.
pub fn format_motd_message() -> String {
    let text = format_motd();
    text.trim_end()
        .strip_suffix("(Press Enter to Continue)")
        .unwrap_or(text.trim_end())
        .trim_end()
        .to_string()
}

fn random_motd(list: &str) -> Option<String> {
    let entries: Vec<_> = list
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if entries.is_empty() {
        return None;
    }
    let entry = entries[rand::random_range(0..entries.len())];
    Some(format!("{entry}\n\n(Press Enter to Continue)\n"))
}

/// One row of a creation menu: a display `name`, an identifying `slug` (matched
/// as a prefix), and an optional one-line `description`.
pub struct MenuItem<'a> {
    pub name: &'a str,
    pub slug: &'a str,
    pub description: &'a str,
}

/// Render a numbered creation menu: a `title`, one `N. Name - description` line
/// per item (the `- description` omitted when empty), and a trailing `prompt`.
pub fn format_selection_menu(title: &str, items: &[MenuItem], prompt: &str) -> String {
    let mut out = format!("[ {title} ]\n");
    for (i, item) in items.iter().enumerate() {
        if item.description.is_empty() {
            out.push_str(&format!("{}. {}\n", i + 1, item.name));
        } else {
            out.push_str(&format!(
                "{}. {} - {}\n",
                i + 1,
                item.name,
                item.description
            ));
        }
    }
    out.push('\n');
    out.push_str(prompt);
    out
}

/// Resolve a line of menu input against `items`, returning the 0-based position
/// of the chosen row. Accepts either a 1-based index or a case-insensitive
/// prefix of a row's name or slug (first match wins). Returns `None` on blank,
/// out-of-range, or unmatched input, so the caller re-prompts without advancing.
pub fn parse_menu_choice(input: &str, items: &[MenuItem]) -> Option<usize> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if let Ok(idx) = input.parse::<usize>() {
        // `then` (lazy) not `then_some`: `idx - 1` would underflow for idx == 0.
        return (idx >= 1 && idx <= items.len()).then(|| idx - 1);
    }
    let lower = input.to_lowercase();
    items.iter().position(|item| {
        item.name.to_lowercase().starts_with(&lower) || item.slug.to_lowercase().starts_with(&lower)
    })
}

/// Format a linkdead announce.
pub fn format_linkdead(name: &str, reconnecting: bool) -> String {
    if reconnecting {
        format!("{} has reconnected.\n", name)
    } else {
        format!("{} has gone linkdead.\n", name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_world::MapConfig;

    // ── format_areas_list ────────────────────────────────────────

    #[test]
    fn areas_list_empty() {
        assert_eq!(format_areas_list(&[]), "No areas exist.\n");
    }

    #[test]
    fn areas_list_lists_slug_and_name() {
        let entries = vec![
            ("haven".to_string(), "Haven".to_string()),
            ("swamp".to_string(), "Southern Swamp".to_string()),
        ];
        let got = format_areas_list(&entries);
        assert!(got.starts_with("Areas (2):\n"));
        assert!(got.contains("  haven — Haven\n"));
        assert!(got.contains("  swamp — Southern Swamp\n"));
    }

    // ── room_title ───────────────────────────────────────────────

    #[test]
    fn room_title_plain_without_debug() {
        assert_eq!(room_title("Town Square", None), "{BTown Square{x");
    }

    #[test]
    fn room_title_shows_ids_for_admin() {
        let got = room_title(
            "Town Square",
            Some(RoomDebugIds {
                entity: 42,
                grim: "abc-123",
                slug: "town-square",
            }),
        );
        assert_eq!(
            got,
            "{BTown Square{x (entity:42 grim:abc-123 slug:town-square)"
        );
    }

    // ── format_room ──────────────────────────────────────────────

    #[test]
    fn room_with_exits_and_presence() {
        let exits = vec!["north".into(), "east".into()];
        let presence = vec![
            "Grimmok Ironhand stands here, hammering metal.".into(),
            "Alice is standing here.".into(),
        ];
        let got = format_room("The Tavern", "A warm room.", &exits, &[], None, &presence);
        assert_eq!(
            got,
            "The Tavern\nA warm room.\n\nExits: north east  Doors: none\n\nGrimmok Ironhand stands here, hammering metal.\nAlice is standing here.\n"
        );
        assert!(!got.contains("Also here:"));
    }

    #[test]
    fn room_with_closed_doors_and_secret() {
        let exits = vec!["north".into(), "south".into()];
        let doors = vec!["east".into()];
        let secret = vec!["down".into()];
        let got = format_room(
            "The Tavern",
            "A warm room.",
            &exits,
            &doors,
            Some(&secret),
            &[],
        );
        assert_eq!(
            got,
            "The Tavern\nA warm room.\n\nExits: north south  Doors: east  Secret: down\n"
        );
    }

    #[test]
    fn room_with_only_doors() {
        let got = format_room("Cell", "Dark.", &[], &["north".into()], None, &[]);
        assert_eq!(got, "Cell\nDark.\n\nExits: none  Doors: north\n");
    }

    #[test]
    fn room_no_exits_no_doors_shows_none() {
        let got = format_room(
            "Void",
            "Empty.",
            &[],
            &[],
            Some(&[]),
            &["Guard is here.".into()],
        );
        assert!(got.starts_with("Void\nEmpty.\n\nExits: none  Doors: none  Secret: none\n"));
        assert!(got.contains("\nGuard is here.\n"));
    }

    #[test]
    fn room_no_occupants() {
        let exits = vec!["south".into()];
        let got = format_room("Cell", "Dark.", &exits, &[], None, &[]);
        assert_eq!(got, "Cell\nDark.\n\nExits: south  Doors: none\n");
    }

    #[test]
    fn room_empty_both() {
        let got = format_room("Empty", "Nothing.", &[], &[], None, &[]);
        assert_eq!(got, "Empty\nNothing.\n\nExits: none  Doors: none\n");
    }

    // ── staple_minimap ───────────────────────────────────────────

    #[test]
    fn long_descriptions_stay_beside_every_minimap_row() {
        let map = vec![
            "#--#--#".into(),
            "|  |  |".into(),
            "#--@@--#".into(),
            "|  |  |".into(),
        ];
        let description = "A large wall fashioned of overturned cars, road signs, garbage, and twisted metal parts to the south forms a gate. To the north a few buildings can be seen down a broken asphalt road.";
        let output = staple_minimap(
            &map,
            &format!("Royal Oak Wall Gate\n{description}\n\nExits: north west"),
        );
        for (i, row) in output.lines().enumerate() {
            assert!(visible_width(row) <= 80, "{row}");
            if let Some(map_row) = map.get(i) {
                assert!(row.starts_with(map_row));
            }
        }
        let column = output
            .lines()
            .map(|row| row.get(11..).unwrap_or(""))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(column.contains("metal parts to the south forms a gate."));
        assert!(column.contains("Exits: north west"));
    }

    #[test]
    fn room_column_wrap_keeps_unicode_and_colour_tokens() {
        let word = format!("{{R{}{{x", "é".repeat(150));
        let rows = wrap_room_column(&word, 69);
        assert!(rows.iter().all(|row| visible_width(row) <= 69));
        assert_eq!(rows.concat(), word);
        assert_eq!(
            wrap_room_column("Title\n\nShort.", 69),
            vec!["Title", "", "Short."]
        );
    }

    #[test]
    fn minimap_gutter_matches_canvas_width() {
        // The gutter must equal the canvas it staples: a width change on
        // either side without the other silently misaligns rows.
        assert_eq!(MINIMAP_WIDTH, MapConfig::MINIMAP.width);
    }

    #[test]
    fn staple_zips_map_and_text() {
        let map = vec!["    @".into(), "    |".into()];
        assert_eq!(
            staple_minimap(&map, "Hall\nCold."),
            "    @      Hall\n    |      Cold."
        );
    }

    #[test]
    fn staple_pads_short_sides_without_trailing_spaces() {
        // Long text: blank gutter, no trailing blanks on the map-only side.
        let got = staple_minimap(&["@".into()], "One\nTwo\nThree");
        assert_eq!(got, "@          One\n           Two\n           Three");
        // Long map: bare map rows, never blank-padded.
        let got = staple_minimap(&["@".into(), "|".into()], "One");
        assert_eq!(got, "@          One\n|");
        // Trailing newline survives with no trailing spaces.
        assert_eq!(staple_minimap(&[], "One\n"), "           One\n");
    }

    #[test]
    fn staple_pads_coloured_rows_on_visible_width() {
        // Colour markup rides along in map rows: the gutter must measure
        // visible columns, not bytes, or room text drifts left.
        let got = staple_minimap(&["    {R@@{x".into()], "Hall");
        assert_eq!(got, "    {R@@{x      Hall");
        assert_eq!(visible_width("    {R@@{x"), 5);
    }

    // ── format_entity ────────────────────────────────────────────

    #[test]
    fn entity_basic() {
        assert_eq!(
            format_entity("Sword", "A rusty blade."),
            "Sword\nA rusty blade.\n"
        );
    }

    #[test]
    fn entity_empty_desc() {
        assert_eq!(format_entity("Nothing", ""), "Nothing\n\n");
    }

    // ── format_say ───────────────────────────────────────────────

    #[test]
    fn say_basic() {
        assert_eq!(
            format_say("Alice", "hello there"),
            "@xf0fAlice says @r'@x909hello there@r'\n"
        );
    }
    #[test]
    fn say_empty_text() {
        assert_eq!(format_say("Bob", ""), "@xf0fBob says @r'@x909@r'\n");
    }

    /// `say {RHELLO` used to turn the room's text red. Spoken text is data.
    #[test]
    fn say_does_not_let_speech_inject_colour() {
        let out = format_say("Alice", "{RHELLO");
        let rendered = grim_color::ansi(&grim_color::convert_16color(&out));
        assert!(
            rendered.contains("{RHELLO"),
            "spoken markup must render literally: {rendered:?}"
        );
    }

    /// `yell` and `ooc` build their string with `format!`, not the catalog, so
    /// they need escaping of their own — verify they got it.
    #[test]
    fn yell_and_ooc_do_not_let_speech_inject_colour() {
        for out in [
            format_yell("Alice", "{RHELLO"),
            format_ooc("Alice", "{RHELLO"),
        ] {
            let rendered = grim_color::ansi(&grim_color::convert_16color(&out));
            assert!(
                rendered.contains("{RHELLO"),
                "spoken markup must render literally: {rendered:?}"
            );
            assert!(
                !rendered.contains('\x1b'),
                "no colour emitted: {rendered:?}"
            );
        }
    }

    /// Names cannot currently carry markup — `validate_character_name` allows
    /// only alphanumerics, spaces, hyphens and apostrophes. This guards the
    /// formatter anyway, so a future relaxation of that rule fails loudly here
    /// rather than quietly becoming an injection.
    #[test]
    fn say_does_not_let_a_name_inject_colour() {
        let out = format_say("@xf00Alice", "hi");
        let rendered = grim_color::ansi(&grim_color::convert_16color(&out));
        assert!(
            rendered.contains("@xf00Alice"),
            "name markup must render literally: {rendered:?}"
        );
    }

    // ── format_yell ──────────────────────────────────────────────

    #[test]
    fn yell_basic() {
        assert_eq!(
            format_yell("Guard", "intruder"),
            "Guard yells, 'intruder'\n"
        );
    }

    #[test]
    fn yell_empty_text() {
        assert_eq!(format_yell("Echo", ""), "Echo yells, ''\n");
    }

    // ── format_ooc ───────────────────────────────────────────────

    #[test]
    fn ooc_basic() {
        assert_eq!(
            format_ooc("Alice", "anyone at the tavern?"),
            "[OOC] Alice: anyone at the tavern?\n"
        );
    }

    #[test]
    fn ooc_empty_text() {
        assert_eq!(format_ooc("Bob", ""), "[OOC] Bob: \n");
    }

    // ── format_move ──────────────────────────────────────────────

    #[test]
    fn move_leaving() {
        assert_eq!(format_move("Alice", "north", true), "Alice leaves north.\n");
    }

    #[test]
    fn move_arriving() {
        assert_eq!(format_move("Bob", "east", false), "Bob arrives.\n");
    }

    #[test]
    fn sockets_empty_list() {
        assert_eq!(format_sockets_list(&[]), "No connections.\n");
    }

    #[test]
    fn sockets_rows_render_as_table_with_idle() {
        let rows = [
            SocketRow {
                id: 1,
                addr: "127.0.0.1:11111".into(),
                state: "InGame".into(),
                character: "Hero".into(),
                account: "spy@xf00.com".into(),
                idle_secs: Some(12),
            },
            SocketRow {
                id: 2,
                addr: "127.0.0.1:22222".into(),
                state: "Login".into(),
                character: "-".into(),
                account: "-".into(),
                idle_secs: None,
            },
        ];
        let got = format_sockets_list(&rows);
        let want = concat!(
            "Sockets connected (2):\n",
            "ID  IP               State   Name  Account       Idle\n",
            "--  ---------------  ------  ----  ------------  ----\n",
            " 1  127.0.0.1:11111  InGame  Hero  spy@@xf00.com   12s\n",
            " 2  127.0.0.1:22222  Login   -     -                -\n",
        );
        assert_eq!(got, want);
    }

    // ── format_where_list ────────────────────────────────────────

    #[test]
    fn where_empty() {
        assert_eq!(
            format_where_list("Town", &[]),
            "No one else in this area.\n"
        );
    }

    #[test]
    fn where_populated() {
        let entries = vec![
            ("Alice".into(), "Tavern".into()),
            ("Bob".into(), "Garden".into()),
        ];
        let got = format_where_list("Town", &entries);
        assert!(got.starts_with("In your area (Town):\n"));
        assert!(got.contains("  Alice in [Tavern]\n"));
        assert!(got.contains("  Bob in [Garden]\n"));
    }

    // ── format_commands ──────────────────────────────────────────

    #[test]
    fn commands_lists_keywords_in_a_grid() {
        let got = format_commands(&["who".into(), "grin".into(), "say".into(), "look".into()]);
        // Sorted row-major in one row at this width.
        assert_eq!(got, "Available commands:\ngrin    look    say     who\n");
        assert!(!got.contains("—"), "no help text in the grid:\n{got}");
    }

    #[test]
    fn commands_grid_empty_is_header_only() {
        assert_eq!(format_commands(&[]), "Available commands:\n");
    }
    #[test]
    fn motd_list_uses_one_nonblank_line() {
        assert!(random_motd("\u{feff} \r\n \n").is_none());
        assert_eq!(
            random_motd("\u{feff}Only entry\r\n\r\n"),
            Some("Only entry\n\n(Press Enter to Continue)\n".to_string())
        );
        for _ in 0..16 {
            let message = random_motd("First\r\n\nSecond\n").unwrap();
            assert!(message.starts_with("First\n") || message.starts_with("Second\n"));
            assert!(!(message.contains("First") && message.contains("Second")));
        }
    }

    #[test]
    fn motd_non_empty() {
        let got = format_motd();
        assert!(!got.is_empty());
    }

    // ── format_linkdead ──────────────────────────────────────────

    #[test]
    fn linkdead_gone() {
        assert_eq!(
            format_linkdead("Alice", false),
            "Alice has gone linkdead.\n"
        );
    }

    #[test]
    fn linkdead_reconnected() {
        assert_eq!(format_linkdead("Bob", true), "Bob has reconnected.\n");
    }

    // ── format_selection_menu ────────────────────────────────────

    fn items() -> Vec<MenuItem<'static>> {
        vec![
            MenuItem {
                name: "Warrior",
                slug: "warrior",
                description: "Hits things.",
            },
            MenuItem {
                name: "Mage",
                slug: "mage",
                description: "",
            },
        ]
    }

    #[test]
    fn selection_menu_numbers_and_prompts() {
        let got = format_selection_menu("Class", &items(), "Pick: ");
        assert!(got.starts_with("[ Class ]\n"));
        assert!(got.contains("1. Warrior - Hits things.\n"));
        // Empty description omits the dash.
        assert!(got.contains("2. Mage\n"));
        assert!(got.ends_with("Pick: "));
    }

    // ── parse_menu_choice ────────────────────────────────────────

    #[test]
    fn parse_choice_by_index() {
        assert_eq!(parse_menu_choice("1", &items()), Some(0));
        assert_eq!(parse_menu_choice("2", &items()), Some(1));
    }

    #[test]
    fn parse_choice_index_out_of_range_or_zero() {
        assert_eq!(parse_menu_choice("0", &items()), None);
        assert_eq!(parse_menu_choice("3", &items()), None);
    }

    #[test]
    fn parse_choice_by_name_prefix_case_insensitive() {
        assert_eq!(parse_menu_choice("war", &items()), Some(0));
        assert_eq!(parse_menu_choice("MAGE", &items()), Some(1));
    }

    #[test]
    fn parse_choice_by_slug_prefix() {
        assert_eq!(parse_menu_choice("warr", &items()), Some(0));
    }

    #[test]
    fn parse_choice_blank_or_unmatched_is_none() {
        assert_eq!(parse_menu_choice("", &items()), None);
        assert_eq!(parse_menu_choice("   ", &items()), None);
        assert_eq!(parse_menu_choice("cleric", &items()), None);
    }
}

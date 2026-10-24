//! Session-local list renderers: the MUD-style `who` list plus the `where`
//! and `areas` lists. Factored out of [`crate::command::handle_ingame`] to
//! hold that dispatcher under the line budget — the same one-file-per-list
//! shape as `sockets.rs`, `finger.rs`, and `wizlist.rs`.

use std::cmp::Ordering;
use std::collections::HashSet;

use crate::formatter;
use crate::params::{PlayerChars, RoomResolver, SessionRes};
use crate::sockets::ClientSnapshot;
use bevy::prelude::*;
use chrono::{DateTime, Utc};
use grim_actor::Linkdead;
use grim_core::components::Gender;

/// The WHO ordering keys for one online character.
pub(crate) struct WhoKey {
    pub(crate) is_admin: bool,
    pub(crate) level: u32,
    pub(crate) connected_at: DateTime<Utc>,
    /// Lower-cased name for a case-insensitive tiebreak.
    pub(crate) sort_name: String,
}

/// One online character's WHO data: the ordering [`WhoKey`] plus the
/// fully-computed [`WhoRow`] to render.
pub(crate) struct WhoData<'a> {
    pub(crate) key: WhoKey,
    pub(crate) row: WhoRow<'a>,
}

/// WHO ordering: admins first, alphabetical by name; then everyone else by
/// level DESC, connect-time ASC (oldest connection first), name ASC.
pub(crate) fn who_order(a: &WhoKey, b: &WhoKey) -> Ordering {
    match (a.is_admin, b.is_admin) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (true, true) => a.sort_name.cmp(&b.sort_name),
        (false, false) => b
            .level
            .cmp(&a.level)
            .then_with(|| a.connected_at.cmp(&b.connected_at))
            .then_with(|| a.sort_name.cmp(&b.sort_name)),
    }
}

/// Map a [`Gender`] to its single-character WHO code.
pub(crate) fn gender_char(gender: Gender) -> &'static str {
    match gender {
        Gender::Male => "M",
        Gender::Female => "F",
        Gender::Neutral => "N",
    }
}

// ─── WHO list (MUD-style column grid) ────────────────────────────────

/// Column widths for the WHO stat block `LLL G RRRRR CCC GGGGG`.
const WHO_LEVEL_W: usize = 3;
const WHO_GENDER_W: usize = 1;
const WHO_RACE_W: usize = 5;
const WHO_CLASS_W: usize = 3;
const WHO_GUILD_W: usize = 5;

/// Colour reset appended after any admin-supplied WHO value (a `restrings`
/// override or a `title`), so its colour can't bleed into the next column, the
/// name, or the following row. Engine-computed columns (level number, `M`/`F`/`N`,
/// registry abbrevs) carry no colour and are left untouched.
const WHO_RESET: &str = "{x";

/// Left-justify `s` into exactly `width` *visible* columns: pad with trailing
/// spaces, or truncate to fit. Colour-aware — markup (`{X`/`@xRGB`) occupies no
/// columns and is never split mid-token — so a coloured restring override stays
/// aligned with the rest of the grid.
fn pad_left(s: &str, width: usize) -> String {
    let truncated = grim_color::truncate_visible(s, width);
    let pad = width.saturating_sub(grim_color::visible_width(&truncated));
    format!("{truncated}{}", " ".repeat(pad))
}

/// Right-justify `s` into exactly `width` *visible* columns: pad with leading
/// spaces, or truncate to fit. Colour-aware (see [`pad_left`]). Used for the
/// right-aligned level column.
fn pad_right(s: &str, width: usize) -> String {
    let truncated = grim_color::truncate_visible(s, width);
    let pad = width.saturating_sub(grim_color::visible_width(&truncated));
    format!("{}{truncated}", " ".repeat(pad))
}

/// One character's WHO row inputs. The caller computes each column's default
/// text (level → `IMM`/number, gender → `M`/`F`/`N`, race/class → registry
/// abbrev, guild → blank); this renderer applies any `restrings` override and
/// the fixed-width grid.
pub(crate) struct WhoRow<'a> {
    /// Level column default: `"IMM"` for admins, else the level number.
    pub level: String,
    /// Gender column default: `"M"` / `"F"` / `"N"`.
    pub gender: String,
    /// Race abbrev (blank for legacy/unknown slug).
    pub race: String,
    /// Class abbrev (blank for legacy/unknown slug).
    pub class: String,
    /// Guild (blank — no guild system yet).
    pub guild: String,
    /// Real character name — NEVER restrung.
    pub name: String,
    /// Optional title, shown after the name.
    pub title: Option<String>,
    /// Persisted display overrides (`who_level`/`who_gender`/`who_race`/
    /// `who_class`/`who_guild` per column, `who` for the whole stat block).
    pub restrings: &'a std::collections::HashMap<String, String>,
    /// Whether to append the `(Linkdead)` marker.
    pub linkdead: bool,
    /// Whether to append the `(AFK)` marker.
    pub afk: bool,
    /// Whether to append the `(offline)` marker (wizlist snapshot rows).
    pub offline: bool,
}

impl WhoRow<'_> {
    /// The `LLL G RRRRR CCC GGGGG` stat block: a `who` restring replaces it
    /// verbatim (followed by a [`WHO_RESET`] colour reset); otherwise each column
    /// is its restring override (if any, also reset) or its computed default,
    /// padded/truncated to the column width.
    fn stat_block(&self) -> String {
        if let Some(whole) = self.restrings.get("who") {
            return format!("{whole}{WHO_RESET}");
        }
        // Each column is padded/truncated to its width. A restring override may
        // carry colour, so it gets a trailing reset; the engine default doesn't.
        let col = |key: &str, default: &str, width: usize, left: bool| -> String {
            let (text, restrung) = match self.restrings.get(key) {
                Some(v) => (v.as_str(), true),
                None => (default, false),
            };
            let padded = if left {
                pad_left(text, width)
            } else {
                pad_right(text, width)
            };
            if restrung {
                format!("{padded}{WHO_RESET}")
            } else {
                padded
            }
        };
        format!(
            "{} {} {} {} {}",
            col("who_level", &self.level, WHO_LEVEL_W, false),
            col("who_gender", &self.gender, WHO_GENDER_W, true),
            col("who_race", &self.race, WHO_RACE_W, true),
            col("who_class", &self.class, WHO_CLASS_W, true),
            col("who_guild", &self.guild, WHO_GUILD_W, true),
        )
    }
}
/// Render one WHO line: the stat block, the real name, an optional title, and
/// trailing `(AFK)` / `(Linkdead)` / `(offline)` markers.
pub(crate) fn format_who_row(row: &WhoRow) -> String {
    let mut line = format!("{} {}", row.stat_block(), row.name);
    if let Some(title) = &row.title {
        if !title.is_empty() {
            line.push(' ');
            line.push_str(title);
            line.push_str(WHO_RESET);
        }
    }
    if row.afk {
        line.push_str(" (AFK)");
    }
    if row.linkdead {
        line.push_str(" (Linkdead)");
    }
    if row.offline {
        line.push_str(" (offline)");
    }
    line
}

/// Render the full WHO list from already-sorted rows.
pub(crate) fn format_who_list(rows: &[WhoRow]) -> String {
    if rows.is_empty() {
        return "No players online.\n".into();
    }
    let mut out = format!("Players online ({}):\n", rows.len());
    for row in rows {
        out.push_str(&format_who_row(row));
        out.push('\n');
    }
    out
}

/// Character entities behind AFK-flagged sessions, resolved from the per-tick
/// snapshot (the dispatcher's `&mut Client` borrow forbids a second query).
pub(crate) fn afk_chars(snapshot: &[ClientSnapshot]) -> HashSet<Entity> {
    snapshot
        .iter()
        .filter(|s| s.afk)
        .filter_map(|s| s.character)
        .collect()
}

/// The MUD-style `who` list. Each online character renders as
/// `LLL G RRRRR CCC GGGGG Name Title` (admins show `IMM` for level; restrings
/// override columns — see [`WhoRow`]). Sort: admins first, alphabetical; then
/// everyone else by level DESC, connect-time ASC, name ASC. Linkdead
/// characters (no live session) are excluded — they stay visible in rooms via
/// `look`, not here. AFK characters carry an `(AFK)` marker.
pub(crate) fn format_who(
    player_chars: &PlayerChars,
    linkdead: &Query<&Linkdead>,
    afk_chars: &HashSet<Entity>,
    res: &SessionRes,
    viewer: Option<Entity>,
) -> String {
    let mut data = collect_who_data(player_chars, linkdead, afk_chars, res, viewer);
    // Linkdead bodies stay seeable in rooms (`look`); the online list is for
    // live sessions only. `wizlist` keeps its own marked rows via the shared
    // collector — this filter is `who`-only.
    data.retain(|d| !d.row.linkdead);
    data.sort_by(|a, b| who_order(&a.key, &b.key));
    let rows: Vec<WhoRow> = data.into_iter().map(|d| d.row).collect();
    format_who_list(&rows)
}

/// One [`WhoData`] per online character: the ordering [`WhoKey`] plus the
/// fully-computed [`WhoRow`]. Shared by [`format_who`] and [`format_wizlist`]
pub(crate) fn collect_who_data<'a>(
    player_chars: &'a PlayerChars<'_, '_>,
    linkdead: &Query<&Linkdead>,
    afk_chars: &HashSet<Entity>,
    res: &SessionRes,
    viewer: Option<Entity>,
) -> Vec<WhoData<'a>> {
    player_chars
        .iter()
        .filter_map(|(e, n, _, actor, character, _, connected)| {
            let ch = character?;
            let observer = viewer
                .and_then(|v| player_chars.get(v).ok())
                .and_then(|(_, _, _, _, c, _, _)| c);
            if !ch.visible_to(observer, viewer == Some(e)) {
                return None;
            }
            // Race/level/gender live on the shared `Actor` base now.
            let actor = actor?;
            let is_admin = ch.is_admin();
            let race_abbrev = res
                .races
                .get(&actor.race)
                .map(|r| r.abbrev.clone())
                .unwrap_or_default();
            let class_abbrev = res
                .classes
                .get(&ch.class)
                .map(|c| c.abbrev.clone())
                .unwrap_or_default();
            let level_text = if is_admin {
                "IMM".to_string()
            } else {
                actor.level.to_string()
            };
            Some(WhoData {
                key: WhoKey {
                    is_admin,
                    level: actor.level,
                    // Fall back to creation time if (impossibly) unstamped, so
                    // the tiebreak stays deterministic rather than panicking.
                    connected_at: connected.map_or(ch.created_at, |c| c.0),
                    sort_name: n.0.to_lowercase(),
                },
                row: WhoRow {
                    level: level_text,
                    gender: gender_char(actor.gender).to_string(),
                    race: race_abbrev,
                    class: class_abbrev,
                    guild: String::new(),
                    name: n.0.clone(),
                    title: ch.title.clone(),
                    restrings: &ch.restrings,
                    linkdead: linkdead.get(e).is_ok(),
                    afk: afk_chars.contains(&e),
                    offline: false,
                },
            })
        })
        .collect()
}

/// The `where` list: beings in the actor's current area, by room. Only
/// characters and creatures show up — objects share `Name + InRoom` but carry
/// neither marker. The actor themself is included.
pub(crate) fn format_where(
    char_entity: Entity,
    player_chars: &PlayerChars,
    rooms: &RoomResolver,
) -> String {
    let (area, area_name) = player_chars
        .get(char_entity)
        .ok()
        .and_then(|(_, _, ir, _, _, _, _)| {
            let (_, r, _) = rooms.rooms.get(ir.room).ok()?;
            let name = rooms.areas.get(r.area).ok()?.name.clone();
            Some((r.area, name))
        })
        .unzip();
    let mut entries: Vec<(String, String)> = Vec::new();
    if let (Some(area), Some(name)) = (area, area_name) {
        for (e, n, ir, _, character, creature, _) in player_chars.iter() {
            let observer = player_chars
                .get(char_entity)
                .ok()
                .and_then(|(_, _, _, _, c, _, _)| c);
            if character.is_some_and(|c| !c.visible_to(observer, e == char_entity)) {
                continue;
            }
            if character.is_none() && creature.is_none() {
                continue;
            }
            if let Ok((_, r, rn)) = rooms.rooms.get(ir.room) {
                if r.area == area {
                    entries.push((n.0.clone(), rn.0.clone()));
                }
            }
        }
        entries.sort_by(|a, b| a.1.cmp(&b.1));
        return formatter::format_where_list(&name, &entries);
    }
    formatter::format_where_list("", &entries)
}

/// The sorted, deduped `areas` list of `(friendly_id, name)`.
pub(crate) fn format_areas(rooms: &RoomResolver) -> String {
    let mut entries: Vec<(String, String)> = rooms
        .areas
        .iter()
        .map(|a| (a.friendly_id.clone(), a.name.clone()))
        .collect();
    entries.sort();
    entries.dedup();
    formatter::format_areas_list(&entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wizlist::format_wizlist_list;
    use chrono::{TimeZone, Utc};
    use std::cmp::Ordering;
    use std::collections::HashMap;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    fn key(is_admin: bool, level: u32, connected: i64, name: &str) -> WhoKey {
        WhoKey {
            is_admin,
            level,
            connected_at: at(connected),
            sort_name: name.to_lowercase(),
        }
    }

    #[test]
    fn gender_char_maps_each_variant() {
        assert_eq!(gender_char(Gender::Male), "M");
        assert_eq!(gender_char(Gender::Female), "F");
        assert_eq!(gender_char(Gender::Neutral), "N");
    }

    #[test]
    fn admins_sort_before_non_admins() {
        // A level-1 admin outranks a level-99 player.
        assert_eq!(
            who_order(&key(true, 1, 100, "Zed"), &key(false, 99, 1, "Aaa")),
            Ordering::Less
        );
        assert_eq!(
            who_order(&key(false, 99, 1, "Aaa"), &key(true, 1, 100, "Zed")),
            Ordering::Greater
        );
    }

    #[test]
    fn admins_sort_alphabetically_case_insensitive() {
        assert_eq!(
            who_order(&key(true, 5, 1, "bob"), &key(true, 5, 1, "Alice")),
            Ordering::Greater
        );
    }

    #[test]
    fn non_admins_sort_by_level_desc_then_connect_then_name() {
        // Higher level first.
        assert_eq!(
            who_order(&key(false, 10, 5, "Bob"), &key(false, 5, 1, "Al")),
            Ordering::Less
        );
        // Same level → oldest connection (smaller timestamp) first.
        assert_eq!(
            who_order(&key(false, 10, 1, "Zed"), &key(false, 10, 9, "Al")),
            Ordering::Less
        );
        // Same level + same connect time → name ascending.
        assert_eq!(
            who_order(&key(false, 10, 5, "Al"), &key(false, 10, 5, "Bob")),
            Ordering::Less
        );
    }

    #[test]
    fn full_ordering_admins_then_level_then_connect() {
        let mut keys = [
            key(false, 10, 30, "Carol"),
            key(true, 1, 99, "Zara"),
            key(false, 10, 10, "Bob"),
            key(true, 1, 1, "Alice"),
            key(false, 5, 5, "Dave"),
        ];
        keys.sort_by(who_order);
        let order: Vec<&str> = keys.iter().map(|k| k.sort_name.as_str()).collect();
        // Admins alpha (alice, zara), then level-10 by connect (bob<carol), then
        // the level-5 player.
        assert_eq!(order, vec!["alice", "zara", "bob", "carol", "dave"]);
    }

    // ── WHO column grid ──────────────────────────────────────────

    /// A `WhoRow` builder with sensible defaults for terse tests. `restrings`
    /// is threaded in by reference so the borrow outlives the row.
    fn who_row<'a>(
        level: &str,
        gender: &str,
        race: &str,
        class: &str,
        name: &str,
        restrings: &'a HashMap<String, String>,
    ) -> WhoRow<'a> {
        WhoRow {
            level: level.into(),
            gender: gender.into(),
            race: race.into(),
            class: class.into(),
            guild: String::new(),
            name: name.into(),
            title: None,
            restrings,
            linkdead: false,
            afk: false,
            offline: false,
        }
    }

    #[test]
    fn pad_left_pads_and_truncates() {
        assert_eq!(pad_left("Hi", 5), "Hi   ");
        assert_eq!(pad_left("Human", 5), "Human");
        assert_eq!(pad_left("Halfling", 5), "Halfl");
        assert_eq!(pad_left("", 3), "   ");
    }

    #[test]
    fn pad_right_pads_and_truncates() {
        assert_eq!(pad_right("5", 3), "  5");
        assert_eq!(pad_right("100", 3), "100");
        assert_eq!(pad_right("IMM", 3), "IMM");
        assert_eq!(pad_right("1000", 3), "100");
    }

    #[test]
    fn who_empty_list() {
        assert_eq!(format_who_list(&[]), "No players online.\n");
    }

    #[test]
    fn wizlist_empty_list() {
        assert_eq!(format_wizlist_list(&[]), "No wizards found.\n");
    }

    #[test]
    fn wizlist_marks_offline_rows() {
        let rs = HashMap::new();
        let mut row = who_row("IMM", "M", "Human", "War", "Gandalf", &rs);
        row.offline = true;
        let got = format_wizlist_list(&[row]);
        assert!(got.starts_with("Wizards (1):\n"), "header:\n{got}");
        assert!(got.contains("Gandalf (offline)\n"), "row:\n{got}");
    }

    #[test]
    fn who_marks_afk_rows() {
        let rs = HashMap::new();
        let mut row = who_row("5", "M", "Human", "War", "Bob", &rs);
        row.afk = true;
        let got = format_who_list(&[row]);
        assert!(got.contains("Bob (AFK)\n"), "row:\n{got}");
    }

    /// Column geometry check: the stat block is always 21 chars, so the name
    /// begins at index 22 on every non-`who`-override row. Keeps the hand-typed
    /// exact-string expectations below honest.
    #[test]
    fn who_row_name_column_is_fixed() {
        let rs = HashMap::new();
        let row = who_row("5", "M", "Human", "War", "Alice", &rs);
        let line = format_who_row(&row);
        assert_eq!(&line[..22], "  5 M Human War       ");
        assert_eq!(&line[22..], "Alice");
    }

    #[test]
    fn who_row_numeric_level_and_grid() {
        let rs = HashMap::new();
        let row = who_row("5", "M", "Human", "War", "Alice", &rs);
        // LLL(right) G RRRRR(left) CCC(left) GGGGG(left) Name
        assert_eq!(format_who_row(&row), "  5 M Human War       Alice");
    }

    #[test]
    fn who_row_imm_level_for_admin() {
        let rs = HashMap::new();
        let row = who_row("IMM", "F", "Elf", "Mag", "Boss", &rs);
        assert_eq!(format_who_row(&row), "IMM F Elf   Mag       Boss");
    }

    #[test]
    fn who_row_neutral_gender_and_title() {
        let rs = HashMap::new();
        let mut row = who_row("3", "N", "Dwarf", "Cle", "Nn", &rs);
        row.title = Some("the Grey".into());
        // Title may carry admin colour → trailing {x reset so it can't bleed.
        assert_eq!(format_who_row(&row), "  3 N Dwarf Cle       Nn the Grey{x");
    }

    #[test]
    fn who_row_legacy_blank_race_and_class() {
        let rs = HashMap::new();
        let row = who_row("1", "N", "", "", "Old", &rs);
        // Blank race (5) + blank class (3) + blank guild (5) → all spaces.
        assert_eq!(format_who_row(&row), "  1 N                 Old");
    }

    #[test]
    fn who_row_race_abbrev_truncated_to_five() {
        let rs = HashMap::new();
        let row = who_row("1", "M", "TooLongRace", "Warrior", "X", &rs);
        // race truncates to 5, class truncates to 3.
        assert_eq!(format_who_row(&row), "  1 M TooLo War       X");
    }

    #[test]
    fn who_row_linkdead_marker_appended() {
        let rs = HashMap::new();
        let mut row = who_row("2", "M", "Human", "War", "Gone", &rs);
        row.linkdead = true;
        assert!(format_who_row(&row).ends_with("Gone (Linkdead)"));
    }

    #[test]
    fn who_restring_overrides_each_column() {
        let mut rs = HashMap::new();
        rs.insert("who_level".into(), "GOD".into());
        rs.insert("who_gender".into(), "X".into());
        rs.insert("who_race".into(), "Deity".into());
        rs.insert("who_class".into(), "Sun".into());
        rs.insert("who_guild".into(), "Elite".into());
        let row = who_row("5", "M", "Human", "War", "Zeus", &rs);
        // Each restrung column gets a trailing {x reset (padded first, then reset).
        assert_eq!(format_who_row(&row), "GOD{x X{x Deity{x Sun{x Elite{x Zeus");
    }

    #[test]
    fn who_restring_overrides_are_padded_and_truncated() {
        let mut rs = HashMap::new();
        rs.insert("who_race".into(), "VeryLongRace".into());
        rs.insert("who_gender".into(), "MF".into());
        let row = who_row("5", "M", "Human", "War", "Q", &rs);
        // race truncated to 5, gender truncated to 1, grid preserved; the two
        // restrung columns get {x, the engine defaults don't.
        assert_eq!(format_who_row(&row), "  5 M{x VeryL{x War       Q");
    }

    #[test]
    fn who_coloured_restring_keeps_column_alignment() {
        // A coloured override occupies the same *visible* width as the plain
        // default (markup is zero-width), so the grid does not shift.
        let plain_rs = HashMap::new();
        let plain = format_who_row(&who_row("5", "M", "Human", "War", "Q", &plain_rs));
        let mut rs = HashMap::new();
        rs.insert("who_class".into(), "{RWiz".into()); // visible "Wiz" (3) == WHO_CLASS_W
        let coloured = format_who_row(&who_row("5", "M", "Human", "War", "Q", &rs));
        assert_eq!(
            grim_color::visible_width(&coloured),
            grim_color::visible_width(&plain),
            "a coloured override must not shift the visible grid"
        );
        assert!(
            coloured.contains("{RWiz{x"),
            "colour preserved with a trailing reset: {coloured:?}"
        );
    }

    #[test]
    fn who_coloured_restring_truncates_without_splitting_a_token() {
        // An over-long coloured override trims to the column's visible width and
        // never cuts the `{R` token in half.
        let mut rs = HashMap::new();
        rs.insert("who_race".into(), "{RVeryLongRace".into()); // visible width 5 cap
        let row = who_row("5", "M", "Human", "War", "Q", &rs);
        let line = format_who_row(&row);
        assert!(
            line.contains("{RVeryL{x"),
            "kept {{R + 5 visible chars: {line:?}"
        );
        assert!(!line.contains("{RVeryLo"), "must stop at 5 visible chars");
    }

    #[test]
    fn who_full_block_override_rendered_verbatim() {
        let mut rs = HashMap::new();
        rs.insert("who".into(), "[ the Almighty ]".into());
        let mut row = who_row("5", "M", "Human", "War", "God", &rs);
        row.title = Some("of Olympus".into());
        // Whole stat block replaced verbatim (+reset), then ` Name Title` (+reset).
        assert_eq!(format_who_row(&row), "[ the Almighty ]{x God of Olympus{x");
    }

    #[test]
    fn who_list_headers_and_joins_rows() {
        let rs = HashMap::new();
        let rows = vec![
            who_row("5", "M", "Human", "War", "Alice", &rs),
            who_row("3", "F", "Elf", "Mag", "Bob", &rs),
        ];
        let got = format_who_list(&rows);
        assert!(got.starts_with("Players online (2):\n"));
        assert!(got.contains("  5 M Human War       Alice\n"));
        assert!(got.contains("  3 F Elf   Mag       Bob\n"));
    }
}

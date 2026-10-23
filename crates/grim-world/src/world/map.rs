//! ASCII area-map rendering: topology snapshot in, canvas rows out.
//!
//! Being-free: rooms are [`Entity`] keys, direction deltas are fixed, and the
//! viewing actor is just `center` (rendered `@`). Both the `map` verb
//! (`grim-actor`) and the look minimap (`grim-scene`) share this function with
//! different [`MapConfig`]s.
//!
//! Geometry: room `(gx, gy)` → cell `(cx + gx * 3, cy + gy * 2)` with `center`
//! at canvas middle `(width / 2, height / 2)`. `E`/`W` gaps are `--`, `N`/`S`
//! gaps `|`; `Up` marks `,` at `(x - 1, y - 1)`, `Down` marks `'` at
//! `(x + 1, y + 1)`. Layout is a BFS in fixed `Cardinal` order
//! (N, E, S, W, Up, Down, NE, NW, SE, SW) — never `HashMap` iteration — so repeated calls
//! render byte-identical canvases; the first path to claim a cell wins and
//! rooms past `±range` are neither placed nor traversed. Draw order is rooms,
//! then connectors (gap cells for every exit link, so an off-canvas neighbour
//! still leaves a visible stub), then up/down markers onto empty cells;
//! everything clips to the canvas. Returned rows keep leading spaces and are
//! trimmed of trailing spaces.
//! Diagonals move one grid cell along each axis and draw two `/` or `\` gap glyphs.
//! Conflicting links which would point at a different room are omitted. Incoming
//! one-way connections to center never imply a return exit the actor cannot use.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::Entity;
use grim_core::cardinal::Cardinal;

/// Canvas + range limits for [`render_map`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapConfig {
    /// Canvas width in characters.
    pub width: usize,
    /// Canvas height in rows.
    pub height: usize,
    /// Max room-grid distance east/west of center.
    pub range_h: i32,
    /// Max room-grid distance north/south of center.
    pub range_v: i32,
}

impl MapConfig {
    /// The `map` command canvas: 80x20, 10 rooms horizontal, 5 vertical.
    pub const MAP: Self = Self {
        width: 80,
        height: 20,
        range_h: 10,
        range_v: 5,
    };
    /// The look minimap canvas: 9x7, 5 rooms horizontal, 5 vertical.
    pub const MINIMAP: Self = Self {
        width: 9,
        height: 7,
        range_h: 5,
        range_v: 5,
    };
}

/// Fixed neighbour order: `Cardinal` declaration order, so BFS discovery is
/// deterministic regardless of how the snapshot `HashMap`s iterate.
const DIRS: [Cardinal; 10] = [
    Cardinal::North,
    Cardinal::East,
    Cardinal::South,
    Cardinal::West,
    Cardinal::Up,
    Cardinal::Down,
    Cardinal::Northeast,
    Cardinal::Northwest,
    Cardinal::Southeast,
    Cardinal::Southwest,
];

/// Grid delta for a direction; `Up`/`Down` stay on their room's cell (they
/// render as markers, not travel).
fn delta(dir: Cardinal) -> (i32, i32) {
    match dir {
        Cardinal::North => (0, -1),
        Cardinal::East => (1, 0),
        Cardinal::South => (0, 1),
        Cardinal::West => (-1, 0),
        Cardinal::Northeast => (1, -1),
        Cardinal::Northwest => (-1, -1),
        Cardinal::Southeast => (1, 1),
        Cardinal::Southwest => (-1, 1),
        Cardinal::Up | Cardinal::Down => (0, 0),
    }
}

/// Write `glyph` at `(x, y)` unless out of bounds or the cell is taken —
/// rooms land first on blank canvas, so connectors and markers can never
/// overwrite one (several gap/marker geometries provably cannot collide with
/// a room cell at all; the guard is belt-and-braces).
fn put(canvas: &mut [Vec<char>], x: i32, y: i32, glyph: char) {
    if x < 0 || y < 0 {
        return;
    }
    let (x, y) = (x as usize, y as usize);
    if y < canvas.len() && x < canvas[y].len() && canvas[y][x] == ' ' {
        canvas[y][x] = glyph;
    }
}

/// Render the area around `center` as one [`String`] per canvas row.
/// `exits` is the live topology (room → its direction links); rooms absent
/// from the snapshot are dead ends. A room with no snapshot entry still
/// renders as a lone `@`. Glyphs carry transport-independent `{`-family colour
/// markup (`@` `{R`, `o` `{g`, links `{w`, up `,` `{Y`, down `'` `{y`), each
/// reset with `{x` so colour never bleeds into neighbouring text. The `@`
/// glyph is escaped (`@@`) — a bare `@` is a markup introducer.
pub fn render_map(
    center: Entity,
    exits: &HashMap<Entity, HashMap<Cardinal, Entity>>,
    cfg: &MapConfig,
) -> Vec<String> {
    // ── Layout: BFS claiming grid cells ──
    // `seen` marks every discovered room (so a later path can never move it);
    // `placed`/`cells` hold only rooms that won a cell. A room whose cell is
    // taken or lies past `±range` stays visited but unplaced: a deterministic
    // dead end. (`Up`/`Down` links skip BFS entirely — markers only.)
    let mut seen: HashSet<Entity> = HashSet::from([center]);
    let mut placed: HashMap<Entity, (i32, i32)> = HashMap::from([(center, (0, 0))]);
    let mut cells: HashMap<(i32, i32), Entity> = HashMap::from([((0, 0), center)]);
    let mut queue: VecDeque<(Entity, i32, i32)> = VecDeque::from([(center, 0, 0)]);
    while let Some((room, gx, gy)) = queue.pop_front() {
        let links = match exits.get(&room) {
            Some(links) => links,
            None => continue,
        };
        for dir in DIRS {
            // `Up`/`Down` never travel: they render as markers in the draw
            // phase. Skipping them here keeps their targets out of `seen`, so
            // a room that is *also* cardinal-reachable still places (and an
            // up-only room stays invisible, markers aside, with no cycle risk
            // — unplaced rooms never enqueue).
            if matches!(dir, Cardinal::Up | Cardinal::Down) {
                continue;
            }
            let Some(next) = links.get(&dir) else {
                continue;
            };
            if !seen.insert(*next) {
                continue;
            }
            let (dx, dy) = delta(dir);
            let (nx, ny) = (gx + dx, gy + dy);
            if nx.abs() > cfg.range_h || ny.abs() > cfg.range_v {
                continue;
            }
            if cells.contains_key(&(nx, ny)) {
                continue;
            }
            placed.insert(*next, (nx, ny));
            cells.insert((nx, ny), *next);
            queue.push_back((*next, nx, ny));
        }
    }

    // ── Draw ──
    let cx = cfg.width as i32 / 2;
    let cy = cfg.height as i32 / 2;
    let mut canvas = vec![vec![' '; cfg.width]; cfg.height];
    for (room, (gx, gy)) in &placed {
        let glyph = if *room == center { '@' } else { '#' };
        put(&mut canvas, cx + gx * 3, cy + gy * 2, glyph);
    }
    for (room, (gx, gy)) in &placed {
        let links = match exits.get(room) {
            Some(links) => links,
            None => continue,
        };
        // Only draw links toward their actual target. A conflicting loop can
        // place that target elsewhere, or put an unrelated room in this cell.
        // Incoming one-way links must not suggest a usable exit from `center`.
        let visible: HashMap<_, _> = links.iter().filter(|(dir, target)| {
            if matches!(dir, Cardinal::Up | Cardinal::Down) { return true; }
            let (dx, dy) = delta(**dir);
            let expected = (gx + dx, gy + dy);
            if placed.get(*target).is_some_and(|position| *position != expected) {
                return false;
            }
            if cells.get(&expected).is_some_and(|occupant| occupant != *target) {
                return false;
            }
            if expected == (0, 0) && *room != center {
                return exits.get(&center).and_then(|out| out.get(&dir.opposite())) == Some(room);
            }
            true
        }).map(|(dir, target)| (*dir, *target)).collect();
        let links = &visible;
        let (x, y) = (cx + gx * 3, cy + gy * 2);
        if links.contains_key(&Cardinal::North) {
            put(&mut canvas, x, y - 1, '|');
        }
        if links.contains_key(&Cardinal::South) {
            put(&mut canvas, x, y + 1, '|');
        }
        if links.contains_key(&Cardinal::East) {
            put(&mut canvas, x + 1, y, '-');
            put(&mut canvas, x + 2, y, '-');
        }
        if links.contains_key(&Cardinal::West) {
            put(&mut canvas, x - 1, y, '-');
            put(&mut canvas, x - 2, y, '-');
        }
        for (dir, dx, dy, glyph) in [
            (Cardinal::Northeast, 1, -1, '/'),
            (Cardinal::Northwest, -1, -1, '\\'),
            (Cardinal::Southeast, 1, 1, '\\'),
            (Cardinal::Southwest, -1, 1, '/'),
        ] {
            if links.contains_key(&dir) {
                put(&mut canvas, x + dx, y + dy, glyph);
                put(&mut canvas, x + 2 * dx, y + dy, glyph);
            }
        }
        if links.contains_key(&Cardinal::Up) {
            put(&mut canvas, x - 1, y - 1, ',');
        }
        if links.contains_key(&Cardinal::Down) {
            put(&mut canvas, x + 1, y + 1, '\'');
        }
    }
    canvas
        .into_iter()
        .map(|row| {
            // Worst-case glyph expansion is 6 bytes (`@` → `{R@@{x`).
            let mut out = String::with_capacity(row.len() * 6);
            for glyph in row {
                match glyph {
                    '@' => out.push_str("{R@@{x"),
                    '#' => out.push_str("{go{x"),
                    '-' => out.push_str("{w-{x"),
                    '|' => out.push_str("{w|{x"),
                    '/' => out.push_str("{w/{x"),
                    '\\' => out.push_str("{w\\{x"),
                    ',' => out.push_str("{Y,{x"),
                    '\'' => out.push_str("{y'{x"),
                    _ => out.push(glyph),
                }
            }
            out.trim_end().to_owned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "{R@@{x";
    const RM: &str = "{go{x";
    const HL: &str = "{w-{x";
    const VL: &str = "{w|{x";
    const UP: &str = "{Y,{x";
    const DN: &str = "{y'{x";

    fn e(n: u64) -> Entity {
        Entity::from_bits(n)
    }

    fn link(
        exits: &mut HashMap<Entity, HashMap<Cardinal, Entity>>,
        from: Entity,
        dir: Cardinal,
        to: Entity,
    ) {
        exits.entry(from).or_default().insert(dir, to);
    }

    #[test]
    fn lone_room_centers_self() {
        let rows = render_map(e(1), &HashMap::new(), &MapConfig::MAP);
        assert_eq!(rows[10], format!("{:40}{ME}", ""));
        assert!(rows
            .iter()
            .enumerate()
            .all(|(i, r)| i == 10 || r.is_empty()));
    }

    #[test]
    fn cardinal_exits_draw_gaps() {
        let center = e(1);
        let mut exits = HashMap::new();
        link(&mut exits, center, Cardinal::North, e(2));
        link(&mut exits, center, Cardinal::East, e(3));
        link(&mut exits, center, Cardinal::South, e(4));
        link(&mut exits, center, Cardinal::West, e(5));
        let rows = render_map(center, &exits, &MapConfig::MAP);
        let pad = " ".repeat(37);
        assert_eq!(rows[10], format!("{pad}{RM}{HL}{HL}{ME}{HL}{HL}{RM}"));
        assert_eq!(rows[9], format!("{:40}{VL}", ""));
        assert_eq!(rows[8], format!("{:40}{RM}", ""));
        assert_eq!(rows[11], format!("{:40}{VL}", ""));
        assert_eq!(rows[12], format!("{:40}{RM}", ""));
    }

    #[test]
    fn overlapping_target_does_not_invent_an_east_exit() {
        let mut exits = HashMap::new();
        link(&mut exits, e(1), Cardinal::North, e(2));
        link(&mut exits, e(1), Cardinal::Northeast, e(3));
        link(&mut exits, e(3), Cardinal::South, e(4));
        // The east neighbor points west to a different room which loses
        // the center's cell. Its connector used to look like @--#.
        link(&mut exits, e(4), Cardinal::West, e(5));
        let rows = render_map(e(1), &exits, &MapConfig::MINIMAP);
        assert_eq!(rows[3], format!("    {ME}  {RM}"));
    }

    #[test]
    fn incoming_one_way_link_does_not_invent_a_return_exit() {
        let mut exits = HashMap::new();
        link(&mut exits, e(1), Cardinal::Northeast, e(2));
        link(&mut exits, e(2), Cardinal::South, e(3));
        link(&mut exits, e(3), Cardinal::West, e(1));
        let rows = render_map(e(1), &exits, &MapConfig::MINIMAP);
        assert_eq!(rows[3], format!("    {ME}  {RM}"));
    }

    #[test]
    fn diagonal_exits_place_rooms_and_connectors() {
        for (direction, dx, dy, glyph) in [
            (Cardinal::Northeast, 1, -1, '/'),
            (Cardinal::Northwest, -1, -1, '\\'),
            (Cardinal::Southeast, 1, 1, '\\'),
            (Cardinal::Southwest, -1, 1, '/'),
        ] {
            let mut exits = HashMap::new();
            link(&mut exits, e(1), direction, e(2));
            link(&mut exits, e(2), direction.opposite(), e(1));
            let rows = render_map(e(1), &exits, &MapConfig::MAP);
            assert_eq!(rows[(10 + 2 * dy) as usize], format!("{}{RM}", " ".repeat((40 + 3 * dx) as usize)));
            let connector = format!("{{w{glyph}{{x");
            assert_eq!(rows[(10 + dy) as usize].matches(connector.as_str()).count(), 2);
            assert_eq!(rows, render_map(e(1), &exits, &MapConfig::MAP));
        }
    }

    #[test]
    fn up_down_markers_offset_from_room() {
        let center = e(1);
        let mut exits = HashMap::new();
        link(&mut exits, center, Cardinal::North, e(2));
        link(&mut exits, center, Cardinal::Up, e(3));
        link(&mut exits, center, Cardinal::Down, e(4));
        let rows = render_map(center, &exits, &MapConfig::MAP);
        // `,` sits up-one/left-one beside the north `|`; `'` down-one/right-one.
        assert_eq!(rows[9], format!("{:39}{UP}{VL}", ""));
        assert_eq!(rows[11], format!("{:41}{DN}", ""));
        // Up/down targets share the room's cell, so they never place.
        assert_eq!(rows.iter().filter(|r| r.contains('o')).count(), 1);
    }

    #[test]
    fn loop_terminates_with_first_claim_winning() {
        // Diamond: center -E-> a -S-> b, center -S-> c, b -W-> c, c -N-> center.
        // BFS (N,E,S,W) claims c from center before b's path reaches it.
        let (center, a, b, c) = (e(1), e(2), e(3), e(4));
        let mut exits = HashMap::new();
        link(&mut exits, center, Cardinal::East, a);
        link(&mut exits, center, Cardinal::South, c);
        link(&mut exits, a, Cardinal::South, b);
        link(&mut exits, b, Cardinal::West, c);
        link(&mut exits, c, Cardinal::North, center);
        let rows = render_map(center, &exits, &MapConfig::MAP);
        let pad = " ".repeat(40);
        assert_eq!(rows[10], format!("{pad}{ME}{HL}{HL}{RM}"));
        assert_eq!(rows[11], format!("{pad}{VL}  {VL}"));
        assert_eq!(rows[12], format!("{pad}{RM}{HL}{HL}{RM}"));
    }

    #[test]
    fn second_path_to_taken_cell_loses() {
        // d wins (1,-1) via center -N-> c -E-> d before a -N-> b gets there.
        let (center, a, b, c, d) = (e(1), e(2), e(3), e(4), e(5));
        let mut exits = HashMap::new();
        link(&mut exits, center, Cardinal::North, c);
        link(&mut exits, center, Cardinal::East, a);
        link(&mut exits, c, Cardinal::East, d);
        link(&mut exits, a, Cardinal::North, b);
        let rows = render_map(center, &exits, &MapConfig::MAP);
        assert_eq!(rows[8], format!("{:40}{RM}{HL}{HL}{RM}", ""));
        // center + a + c + d place; the loser leaves no glyph behind.
        assert_eq!(
            rows.iter()
                .flat_map(|r| r.chars())
                .filter(|&g| g == 'o')
                .count(),
            3
        );
    }

    #[test]
    fn off_canvas_rooms_leave_connector_stubs() {
        let cfg = MapConfig {
            width: 7,
            height: 5,
            range_h: 5,
            range_v: 5,
        };
        let (center, a, b) = (e(1), e(2), e(3));
        let mut exits = HashMap::new();
        link(&mut exits, center, Cardinal::East, a);
        link(&mut exits, a, Cardinal::East, b);
        let rows = render_map(center, &exits, &cfg);
        assert_eq!(rows.len(), 5);
        // a's cell (x=6) fits; b's (x=9) clips with its gaps, leaving the
        // in-bounds `--` stub from center as the visible trace.
        assert_eq!(rows[2], format!("   {ME}{HL}{HL}{RM}"));
    }

    #[test]
    fn repeated_calls_are_byte_identical() {
        let center = e(1);
        let mut exits = HashMap::new();
        link(&mut exits, center, Cardinal::East, e(2));
        link(&mut exits, center, Cardinal::South, e(3));
        link(&mut exits, e(2), Cardinal::South, e(4));
        let first = render_map(center, &exits, &MapConfig::MAP);
        assert_eq!(first, render_map(center, &exits, &MapConfig::MAP));
    }

    #[test]
    fn up_link_does_not_hide_cardinal_reachable_room() {
        // X is up-linked from center but also walkable via center -E-> a -N-> X:
        // the marker-only Up link must not poison BFS, so X still places.
        let (center, a, x) = (e(1), e(2), e(3));
        let mut exits = HashMap::new();
        link(&mut exits, center, Cardinal::Up, x);
        link(&mut exits, center, Cardinal::East, a);
        link(&mut exits, a, Cardinal::North, x);
        let rows = render_map(center, &exits, &MapConfig::MAP);
        assert_eq!(rows[10], format!("{:40}{ME}{HL}{HL}{RM}", ""));
        assert_eq!(rows[8], format!("{:43}{RM}", ""));
        assert_eq!(rows[9], format!("{:39}{UP}   {VL}", ""));
    }
    #[test]
    fn minimap_canvas_renders_neighbors() {
        // The real P2 canvas shares the P1 code path: lock its geometry now.
        let center = e(1);
        let mut exits = HashMap::new();
        link(&mut exits, center, Cardinal::East, e(2));
        link(&mut exits, center, Cardinal::North, e(3));
        let rows = render_map(center, &exits, &MapConfig::MINIMAP);
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[3], format!("    {ME}{HL}{HL}{RM}"));
        assert_eq!(rows[2], format!("    {VL}"));
        assert_eq!(rows[1], format!("    {RM}"));
    }

    #[test]
    fn every_glyph_colour_is_reset() {
        // Each coloured glyph must close with `{x`, or map colours bleed into
        // neighbouring text (the minimap is stapled left of room prose).
        let center = e(1);
        let mut exits = HashMap::new();
        link(&mut exits, center, Cardinal::North, e(2));
        link(&mut exits, center, Cardinal::East, e(3));
        link(&mut exits, center, Cardinal::South, e(4));
        link(&mut exits, center, Cardinal::West, e(5));
        link(&mut exits, center, Cardinal::Up, e(6));
        link(&mut exits, center, Cardinal::Down, e(7));
        let text = render_map(center, &exits, &MapConfig::MAP).join("\n");
        let openers = ["{R", "{g", "{w", "{Y", "{y"]
            .iter()
            .map(|op| text.matches(op).count())
            .sum::<usize>();
        assert!(openers > 0, "map must carry colour");
        assert_eq!(text.matches("{x").count(), openers);
    }
}

//! Room-look rendering: the room title + exits + presence block (`emit_look_room`)
//! and single-entity descriptions (`emit_look_entity`). Factored out of
//! `output.rs` (file-length cap) — the same one-concern-per-file shape as
//! `recall_output.rs` and `item_output.rs`.

use std::collections::HashMap;

use bevy::prelude::*;
use grim_actor::{Character, Linkdead};
use grim_config::ConfigRegistry;
use grim_core::components::{Client, Description, Name as GrimName};
use grim_core::events::{LookEntity, LookRoom};
use grim_networking::ConnectionOutput;
use grim_text::tr;
use grim_world::{render_map, Doors, Exits, MapConfig, Room};

use crate::formatter;
use crate::output::{find_conn, Occupants};

/// The room-links query backing `emit_look_room`: the exit map plus the
/// optional door map on the same room entity. One query (not two) keeps
/// `format_output` at Bevy's 16-parameter ceiling.
pub(crate) type RoomLinks<'w, 's> = Query<'w, 's, (Entity, &'static Exits, Option<&'static Doors>)>;

/// Split a room's exits into the open directions, the closed-door
/// directions, and the closed-secret directions. All three lists sort in
/// `Cardinal` display order (north, east, south, west, up, down, diagonals) via the
/// derived `Ord`. An *open* hidden door lists under Exits like any open
/// door; only a *closed* secret hides in `secret` — and only for admins
/// (players get an empty secret list, so no heading renders). Discovery of
/// a closed secret is by guessing `open <dir>` or walking through, never
/// by listing.
fn partition_exits(
    exits: &Exits,
    doors: Option<&Doors>,
    admin: bool,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut open = Vec::new();
    let mut closed = Vec::new();
    let mut secret = Vec::new();
    for dir in exits.exits.keys() {
        let door = doors.and_then(|d| d.doors.get(dir));
        let is_closed = door.is_some_and(|d| !d.open);
        if door.is_some_and(|d| d.hidden) && is_closed {
            if admin {
                secret.push(*dir);
            }
            continue;
        }
        (if is_closed { &mut closed } else { &mut open }).push(*dir);
    }
    open.sort();
    closed.sort();
    secret.sort();
    let names =
        |dirs: Vec<grim_core::cardinal::Cardinal>| dirs.iter().map(colored_direction).collect();
    (names(open), names(closed), names(secret))
}

/// Room exit directions use the original Savage Soul green palette. Reset
/// each name so the colour never bleeds into labels or the next listing.
fn colored_direction(dir: &grim_core::cardinal::Cardinal) -> String {
    format!("{{g{dir}{{x")
}

/// Colour only the NPC's name when its authored room line contains it.
/// ASCII case folding preserves UTF-8 byte boundaries and existing colour escapes.
fn creature_line(name: &str, line: &str) -> String {
    let escaped = grim_color::escape_codes(name);
    if !escaped.is_empty() {
        let folded = line.to_ascii_lowercase();
        for (start, _) in folded.match_indices(&escaped.to_ascii_lowercase()) {
            let end = start + escaped.len();
            let (Some(before), Some(name), Some(after)) = (line.get(..start), line.get(start..end), line.get(end..)) else { continue; };
            // A rat's name must not highlight the middle of "scratches".
            if before.chars().next_back().is_some_and(char::is_alphanumeric)
                || after.chars().next().is_some_and(char::is_alphanumeric) { continue; }
            let prefix = before.to_ascii_lowercase();
            let article = ["a ", "an ", "the "].iter().any(|a| prefix.starts_with(a));
            if article && before.chars().count() <= 50
                && before.chars().all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '\'')) {
                return format!("{{G{before}{name}{{x{after}");
            }
            return format!("{before}{{G{name}{{x{after}");
        }
    }
    format!("{{G{line}{{x")
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_look_room(
    ev: &LookRoom,
    rooms: &Query<(Entity, &Room, &GrimName)>,
    room_occupants: &Occupants,
    room_exits: &RoomLinks,
    characters: &Query<&Character>,
    clients: &Query<&Client>,
    terminals: &Query<&grim_networking::TerminalSize>,
    linkdead_chars: &Query<&Linkdead>,
    config_registry: &ConfigRegistry,
    visibility: &grim_actor::visibility::Visibility,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    let Ok((_, room, name)) = rooms.get(ev.room) else {
        return;
    };
    if !visibility.can_see(ev.target,ev.room) {
        outputs.write(ConnectionOutput::new(find_conn(ev.target,room_occupants),"It is too dark to see. Bring a light or use night vision.\n"));
        return;
    }
    // Admins see the room's ids in the title plus the hidden exits under
    // `Secret:`; players see neither.
    let is_admin = characters
        .get(ev.target)
        .map(|c| c.is_admin())
        .unwrap_or(false);
    let (exits, doors, secret) = room_exits
        .get(ev.room)
        .ok()
        .map(|(_, e, d)| partition_exits(e, d, is_admin))
        .unwrap_or_default();
    let presence = collect_presence(
        ev.room,
        &room.id.to_string(),
        ev.target,
        room_occupants,
        characters,
        clients,
        linkdead_chars,
    );
    let screenreader = characters.get(ev.target).is_ok_and(|c| c.config.get("screenreader").is_some_and(|v| v == "on"));
    let grim = room.id.to_string();
    let title = formatter::room_title(
        &name.0,
        (is_admin && !screenreader).then_some(formatter::RoomDebugIds {
            entity: ev.room.to_bits(),
            grim: &grim,
            slug: &room.friendly_id,
        }),
    );
    let conn = find_conn(ev.target, room_occupants);
    let mut body = formatter::format_room(
        &title,
        &room.description,
        &exits,
        &doors,
        is_admin.then_some(secret.as_slice()),
        &presence,
    );
    if screenreader {
        // Preserve descriptions, exits and presence; avoid empty headings and
        // debug noise that otherwise dominate spoken room navigation.
        body = body.replace("  Doors: none", "").replace("  Secret: none", "").replace("\n\n", "\n");
    }
    // Minimap: the same renderer as `map` on the small canvas, stapled left
    // when the looker's resolved `minimap` setting is on. Actors without a
    // `Character` and unseeded registries (unit tests) keep the map, matching
    // the pre-toggle behavior.
    let minimap_on = !screenreader && characters
        .get(ev.target)
        .map(|c| {
            config_registry
                .resolve(&c.config, "minimap")
                .unwrap_or("on")
                == "on"
        })
        .unwrap_or(true);
    let text = if minimap_on {
        let mut snapshot = HashMap::new();
        for (room_entity, links, doors) in room_exits.iter() {
            // Secrets never draw: strip hidden exits from the map snapshot so
            // neither the minimap nor `map` leaks them (admin view included —
            // admins get the `Secret:` suffix in text instead).
            let mut visible = links.exits.clone();
            if let Some(d) = doors {
                visible.retain(|dir, _| !d.doors.get(dir).is_some_and(|door| door.hidden));
            }
            snapshot.insert(room_entity, visible);
        }
        let map_rows = render_map(ev.room, &snapshot, &MapConfig::MINIMAP);
        let width = terminals.get(conn).map(|size| usize::from(size.columns)).unwrap_or(80);
        formatter::staple_minimap_width(&map_rows, &body, width)
    } else {
        body
    };
    outputs.write(ConnectionOutput {
        echo: None,
        ..ConnectionOutput::new(conn, text)
    });
}

/// Presence lines, one per other being — plus one per ground object — in the
/// room: player characters (standing line, position-driven once positions
/// exist) sort above creatures (their long room line, or a plain fallback),
/// with objects (their long room line, or the short name) last. Each group
/// sorts by name so the listing is deterministic. Objects share the presence
/// block with no blank line between the groups. Factored out of
/// [`emit_look_room`] for the line budget.
#[allow(clippy::too_many_arguments)]
fn collect_presence(
    room: Entity,
    room_id: &str,
    target: Entity,
    room_occupants: &Occupants,
    characters: &Query<&Character>,
    clients: &Query<&Client>,
    linkdead_chars: &Query<&Linkdead>,
) -> Vec<String> {
    let mut players: Vec<(String, String)> = Vec::new();
    let mut creatures: Vec<(String, String)> = Vec::new();
    let mut objects: Vec<(String, String)> = Vec::new();
    // Membership funnels through the shared helper (mapped from the rich
    // tuple); the per-kind rendering below stays local.
    for e in grim_actor::in_room(
        room,
        room_occupants.iter().map(|(e, ir, ..)| (e, ir.room)),
        Some(target),
    ) {
        let Ok((_, _, player, occ_name, room_line, is_object)) = room_occupants.get(e) else {
            continue;
        };
        if let Ok(character)=characters.get(e) {
            if character.game_state.get("posture_room").and_then(|v|v.as_str())==Some(room_id) {
                if let Some(posture)=character.game_state.get("posture_display").and_then(|v|v.as_str()) {
                    let suffix=if linkdead_chars.get(e).is_ok(){" (linkdead)"}else if player.and_then(|p|clients.iter().find(|c|c.connection==p.connection)).is_some_and(|c|c.afk){" (AFK)"}else{""};
                    players.push((occ_name.0.clone(),format!("{} is {}{}.",grim_color::escape_codes(&occ_name.0),grim_color::escape_codes(posture),suffix)));
                    continue;
                }
            }
            // Linkdead dominates (a linkdead character has no session, so it
            // can never also be AFK); otherwise an AFK session reads marked.
            // Literal keys (not a variable): the tr-coverage check resolves
            // every catalog reference statically.
            if linkdead_chars.get(e).is_ok() {
                players.push((
                    occ_name.0.clone(),
                    tr!(
                        "room.presence.standing_linkdead",
                        name = occ_name.0.as_str()
                    ),
                ));
            } else {
                let afk = player
                    .and_then(|p| clients.iter().find(|c| c.connection == p.connection))
                    .is_some_and(|c| c.afk);
                if afk {
                    players.push((
                        occ_name.0.clone(),
                        tr!("room.presence.standing_afk", name = occ_name.0.as_str()),
                    ));
                } else {
                    players.push((
                        occ_name.0.clone(),
                        tr!("room.presence.standing", name = occ_name.0.as_str()),
                    ));
                }
            }
        } else if is_object.is_some() {
            // Ground only: carried objects have no `InRoom`, so they never
            // reach this loop.
            match room_line.filter(|l| !l.0.is_empty()) {
                Some(line) => objects.push((occ_name.0.clone(), line.0.clone())),
                None => objects.push((occ_name.0.clone(), occ_name.0.clone())),
            }
        } else if let Some(line) = room_line.filter(|l| !l.0.is_empty()) {
            creatures.push((occ_name.0.clone(), line.0.clone()));
        } else {
            creatures.push((
                occ_name.0.clone(),
                tr!("room.presence.here", name = occ_name.0.as_str()),
            ));
        }
    }
    players.sort();
    creatures.sort();
    objects.sort();
    players
        .into_iter()
        .chain(creatures.into_iter().map(|(name, line)| {
            let rendered = creature_line(&name, &line);
            (name, rendered)
        }))
        .chain(objects.into_iter().map(|(name, line)| (name, format!("{{m{line}{{x"))))
        .map(|(_, line)| line)
        .collect()
}

pub(crate) fn emit_look_entity(
    ev: &LookEntity,
    room_occupants: &Occupants,
    names: &Query<&GrimName>,
    descriptions: &Query<(Option<&Description>, Option<&grim_core::components::ItemCondition>, Option<&grim_object::Object>)>,
    visibility: &grim_actor::visibility::Visibility,
    outputs: &mut MessageWriter<ConnectionOutput>,
) {
    if ev.subject!=ev.target && room_occupants.get(ev.subject).is_ok() && room_occupants.get(ev.target).is_ok_and(|(_,r,..)|!visibility.can_see(ev.target,r.room)) {
        outputs.write(ConnectionOutput::new(find_conn(ev.target,room_occupants),"It is too dark to see that.\n"));
        return;
    }
    let Ok(subj_name) = names.get(ev.subject) else {
        return;
    };
    // Entries hold no newlines themselves; each warrants one between them.
    let mut desc = descriptions
        .get(ev.subject)
        .map(|(d,_,_)| d.map(|d|d.0.join("\n")).unwrap_or_default())
        .unwrap_or_default();
    if let Ok((_, condition, Some(_)))=descriptions.get(ev.subject) {
        let label=condition.cloned().unwrap_or_default();
        if !desc.is_empty(){desc.push('\n');}
        desc.push_str(&format!("Condition: {}.",label.label()));
    }
    let conn = find_conn(ev.target, room_occupants);
    outputs.write(ConnectionOutput {
        echo: None,
        ..ConnectionOutput::new(conn, formatter::format_entity(&subj_name.0, &desc))
    });
}

//! presence implementation, split from the parent subsystem.

use super::*;

/// Presence lines, one per other being — plus one per ground object — in the
/// room: player characters (standing line, position-driven once positions
/// exist) sort above creatures (their long room line, or a plain fallback),
/// with objects (their long room line, or the short name) last. Each group
/// sorts by name so the listing is deterministic. Objects share the presence
/// block with no blank line between the groups. Factored out of
/// [`emit_look_room`] for the line budget.
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_presence(
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
        if !room_occupants.sees(target, e) {
            continue;
        }
        if let Ok(character) = characters.get(e) {
            if character
                .game_state
                .get("posture_room")
                .and_then(|v| v.as_str())
                == Some(room_id)
            {
                if let Some(posture) = character
                    .game_state
                    .get("posture_display")
                    .and_then(|v| v.as_str())
                {
                    let suffix = if linkdead_chars.get(e).is_ok() {
                        " (linkdead)"
                    } else if player
                        .and_then(|p| clients.iter().find(|c| c.connection == p.connection))
                        .is_some_and(|c| c.afk)
                    {
                        " (AFK)"
                    } else {
                        ""
                    };
                    players.push((
                        occ_name.0.clone(),
                        format!(
                            "{} is {}{}.",
                            grim_color::escape_codes(&occ_name.0),
                            grim_color::escape_codes(posture),
                            suffix
                        ),
                    ));
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
    ordered_presence(players, creatures, objects)
}

pub(super) fn ordered_presence(
    mut players: Vec<(String, String)>,
    mut creatures: Vec<(String, String)>,
    mut objects: Vec<(String, String)>,
) -> Vec<String> {
    players.sort();
    creatures.sort();
    objects.sort();
    players
        .into_iter()
        .chain(creatures.into_iter().map(|(name, line)| {
            let rendered = creature_line(&name, &line);
            (name, rendered)
        }))
        .chain(
            objects
                .into_iter()
                .map(|(name, line)| (name, format!("{{m{line}{{x"))),
        )
        .map(|(_, line)| line)
        .collect()
}

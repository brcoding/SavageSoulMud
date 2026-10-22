//! The recovered south exit is guarded independently of the physical door.
use bevy::prelude::*;
use grim::components::{Keywords, Name as GrimName};
use grim::events::InfoMessage;
use grim::{AttemptWalk, Cardinal, CarriedBy, Character, Object, Room};

/// Preserve natural `open gate` as well as the engine's `open south` syntax.
pub(crate) fn object_door(world: &mut World, actor: Entity, verb: &str, arg: &str) -> bool {
    if !matches!(
        arg.trim().to_ascii_lowercase().as_str(),
        "gate" | "city gate" | "south gate" | "north gate" | "the gate"
    ) {
        return false;
    }
    let room = world
        .get::<grim::InRoom>(actor)
        .and_then(|ir| world.get::<Room>(ir.room));
    let direction = match room.map(|r| r.friendly_id.as_str()) {
        Some("romainstwallgate") => Cardinal::South,
        Some("rosouthroad01") => Cardinal::North,
        _ => return false,
    };
    world.write_message(grim::events::EngineCommand {
        client: actor,
        command: if verb == "open_object" {
            grim::events::Command::Open { direction }
        } else {
            grim::events::Command::Close { direction }
        },
    });
    true
}

pub(crate) fn guard(
    mut walk: On<AttemptWalk>,
    rooms: Query<&Room>,
    characters: Query<&Character>,
    objects: Query<(&GrimName, Option<&Keywords>, &CarriedBy), With<Object>>,
    mut replies: MessageWriter<InfoMessage>,
) {
    if walk.direction != Cardinal::South
        || !rooms
            .get(walk.room)
            .is_ok_and(|r| r.friendly_id == "romainstwallgate")
    {
        return;
    }
    let qualified = characters
        .get(walk.actor)
        .ok()
        .and_then(|c| c.game_state.get("quests"))
        .and_then(|v| v.as_array())
        .is_some_and(|quests| {
            quests
                .iter()
                .filter_map(|v| v.as_str())
                .any(|q| matches!(q, "Escape from Royal Oak" | "Savior of Royal Oak"))
        });
    let message = if !qualified {
        "The gate refuses to open. Perhaps you can help the people of Royal Oak by dealing with the wraith problem."
    } else if !objects.iter().any(|(name, keys, held)| {
        held.carrier == walk.actor
            && (name.0.eq_ignore_ascii_case("soulboy")
                || keys.is_some_and(|k| k.0.iter().any(|s| s.eq_ignore_ascii_case("soulboy"))))
    }) {
        "The gate scans your wrist for a Soulboy and does not find one. The gate refuses to open."
    } else {
        replies.write(InfoMessage {
            target: walk.actor,
            text: "Your Soulboy beeps, causing the gate to slide open.".into(),
        });
        return;
    };
    walk.event_mut().denied = true;
    replies.write(InfoMessage {
        target: walk.actor,
        text: message.into(),
    });
}

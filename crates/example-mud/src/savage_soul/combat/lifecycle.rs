//! lifecycle implementation, split from the parent subsystem.

use super::*;

pub(in crate::savage_soul) fn begin_battle(
    world: &mut World,
    actor: Entity,
    target: &str,
    force_trample: bool,
    lethal: bool,
) -> Result<(), String> {
    if world
        .get::<Character>(actor)
        .is_some_and(|c| c.game_state.contains_key("posture_display"))
    {
        return Err("Stand up before attacking.".into());
    }
    if world
        .resource::<Battles>()
        .0
        .iter()
        .any(|b| b.player == actor)
    {
        return Err("You are already attacking. Use stop to end combat.".into());
    }
    let room = world
        .get::<InRoom>(actor)
        .ok_or("You are not in a room.")?
        .room;
    let spec =
        grim::parse_target(target, grim::ParseOptions::BEING).ok_or("Use attack <creature>.")?;
    let candidates: Vec<_> = world
        .query_filtered::<(
            Entity,
            &GrimName,
            &InRoom,
            Option<&grim::components::Keywords>,
        ), With<Creature>>()
        .iter(world)
        .filter(|(_, _, r, _)| r.room == room)
        .map(|(e, n, _, k)| (e, n.0.clone(), k.map(|k| k.0.clone()).unwrap_or_default()))
        .collect();
    let matches = grim::query(
        &spec,
        candidates
            .iter()
            .map(|(e, n, k)| (*e, n.as_str(), k.as_slice())),
    );
    let npc = *matches.first().ok_or("That creature is not here.")?;
    if world.get::<crate::medical::Contained>(npc).is_some() {
        return Err("The table's containment field prevents you from attacking it.".into());
    }
    if let Some(result) = crate::royal_oak::attack(world, actor, npc) {
        return result;
    }
    if world.get::<SoulData>(npc).unwrap().is_incapacitated()
        && (!lethal || world.get::<SoulData>(npc).unwrap().injury.is_dead())
    {
        return Err("That creature is already incapacitated.".into());
    }
    if world.resource::<Battles>().0.iter().any(|b| b.npc == npc) {
        return Err("That creature is already in a duel.".into());
    }
    let trample = world
        .get::<NpcCombatBehavior>(npc)
        .is_some_and(|b| b.can_be_trampled)
        && world
            .get::<SoulData>(actor)
            .zip(world.get::<SoulData>(npc))
            .is_some_and(|(a, d)| !a.prone && a.size > d.size + 2);
    if force_trample && !trample {
        return Err(
            "Trampling requires a grounded target more than two sizes smaller than you.".into(),
        );
    }
    let now = world.resource::<Time>().elapsed_secs_f64();
    world.resource_mut::<Battles>().0.push(Battle {
        player: actor,
        npc,
        next_round: now,
        rounds: 0,
        trample,
        lethal,
    });
    let name = world.get::<GrimName>(npc).unwrap().0.clone();
    world
        .get_mut::<Character>(actor)
        .unwrap()
        .game_state
        .insert("combat_opponent".into(), serde_json::json!(name));
    reply(
        world,
        actor,
        format!(
            "Combat begins: {name}{}.",
            if trample { " (trample)" } else { "" }
        ),
    );
    Ok(())
}

pub(in crate::savage_soul) fn combat_end(world: &mut World, actor: Entity, text: &str, xp: u32) {
    if let Some(mut c) = world.get_mut::<Character>(actor) {
        c.game_state.remove("combat_opponent");
    }
    reply(world, actor, format!("{text} XP +{xp}."));
}

pub(in crate::savage_soul) fn retreat_npc(world: &mut World, player: Entity, npc: Entity) {
    if let Some(mut c) = world.get_mut::<Character>(player) {
        c.game_state.remove("combat_opponent");
    }
    let name = world
        .get::<GrimName>(npc)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| "creature".into());
    let Some(room) = world.get::<InRoom>(npc).map(|r| r.room) else {
        return;
    };
    let mut exits: Vec<_> = world
        .get::<grim::Exits>(room)
        .map(|e| e.exits.iter().map(|(d, r)| (*d, *r)).collect())
        .unwrap_or_default();
    exits.retain(|(d, to)| {
        world.get::<grim::Room>(*to).is_some()
            && !world
                .get::<grim::Doors>(room)
                .and_then(|doors| doors.doors.get(d))
                .is_some_and(|door| !door.open)
    });
    exits.sort_by_key(|(d, _)| d.to_string());
    if let Some((direction, to)) = exits.first() {
        world.entity_mut(npc).insert(InRoom { room: *to });
        reply(
            world,
            player,
            format!("The {name} flees {direction}. Combat ends. XP +0."),
        );
    } else {
        reply(
            world,
            player,
            format!("The {name} backs away. Combat ends. XP +0."),
        );
    }
}

pub(in crate::savage_soul) fn battle_finished(pc: &SoulData, npc: &SoulData, lethal: bool) -> bool {
    hard_incapacitated(pc)
        || if lethal {
            npc.injury.is_dead()
        } else {
            hard_incapacitated(npc)
        }
}

pub(in crate::savage_soul) fn hard_incapacitated(soul: &SoulData) -> bool {
    soul.injury.is_dead() || soul.unconscious || soul.comatose || soul.is_helpless()
}

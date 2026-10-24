//! spawning implementation, split from the parent subsystem.

use super::*;

/// Stamp one NPC blueprint into `room`: the being bundle plus its compiled
/// script triggers. A trigger that fails to compile is logged and skipped —
/// the mob still spawns, triggerless for that moment.
pub(crate) fn spawn_npc(
    commands: &mut Commands,
    area_slug: &str,
    npc: &NpcBlueprint,
    room: Entity,
) -> Entity {
    // Compile each trigger once now: a typo fails loudly at startup (logged,
    // trigger skipped) instead of on a player's move.
    let mut compiled = Vec::with_capacity(npc.triggers.len());
    for def in &npc.triggers {
        match compile(&def.script) {
            Ok(bytecode) => compiled.push(CompiledTrigger {
                on: def.on,
                bytecode,
            }),
            Err(error) => error!(
                "area '{area_slug}' npc '{}': skipping trigger that does not compile: {error}",
                npc.name
            ),
        }
    }
    let mut mob = commands.spawn((
        Creature,
        // Seeded mobs carry the shared `Actor` base with sensible
        // defaults (no race/build data in blueprints yet): empty race,
        // level 1, neutral gender.
        Actor {
            race: npc.race.clone(),
            level: npc.level,
            gender: match npc.gender.as_str() {
                "male" => Gender::Male,
                "female" => Gender::Female,
                _ => Gender::Neutral,
            },
        },
        GrimName(npc.name.clone()),
        Description(npc.description.clone()),
        Keywords(npc.keywords.clone()),
        RoomDescription(npc.room_description.clone()),
        InRoom { room },
    ));
    if let Some(shop) = &npc.shop {
        mob.insert(shop.clone());
    }
    if let Some(soul) = &npc.soul {
        mob.insert(soul.clone());
    }
    if let Some(behavior) = &npc.combat_behavior {
        mob.insert(behavior.clone());
    }
    if !compiled.is_empty() {
        info!(
            "area '{area_slug}' npc '{}': {} script trigger(s) loaded",
            npc.name,
            compiled.len()
        );
        mob.insert(ScriptTriggers(compiled));
    }
    mob.id()
}

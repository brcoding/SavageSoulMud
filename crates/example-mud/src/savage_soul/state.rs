//! state implementation, split from the parent subsystem.

use super::*;

pub(super) fn hydrate(world: &mut World, mut next_equipment: Local<f64>) {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let reconcile = now >= *next_equipment;
    if reconcile {
        *next_equipment = now + 1.0;
    }
    let people: Vec<_> = world
        .query_filtered::<(Entity, &Character), Without<SoulData>>()
        .iter(world)
        .map(|(e, c)| (e, c.game_state.get(STATE).cloned()))
        .collect();
    for (e, saved) in people {
        let soul = match saved {
            Some(v) => match serde_json::from_value(v) {
                Ok(s) => s,
                Err(error) => {
                    error!("Invalid Savage Soul state for {e:?}: {error}");
                    continue;
                }
            },
            None => SoulData::default(),
        };
        if let Some(appearance) = soul
            .species
            .as_ref()
            .and_then(|s| s.template.as_ref())
            .and_then(|t| t.appearance.clone())
        {
            world
                .entity_mut(e)
                .insert(grim::components::Description(vec![appearance]));
        }
        world.entity_mut(e).insert(soul.clone());
        world
            .get_mut::<Character>(e)
            .unwrap()
            .game_state
            .insert(STATE.into(), serde_json::to_value(&soul).unwrap());
        let mut character = world.get_mut::<Character>(e).unwrap();
        character
            .restrings
            .entry("who_class".into())
            .or_insert("Soul".into());
        character.game_state.insert(
            "equipment_display".into(),
            serde_json::Value::String(equipment_display(&soul)),
        );
        if soul.is_incapacitated() {
            character.game_state.insert(
                "action_restriction".into(),
                serde_json::Value::String("You cannot act while incapacitated.".into()),
            );
        }
    }
    let mobs: Vec<_> = world
        .query_filtered::<Entity, (With<Creature>, Without<SoulData>)>()
        .iter(world)
        .collect();
    for e in mobs {
        world.entity_mut(e).insert(SoulData {
            total_xp: 0,
            ..Default::default()
        });
    }
    crate::species::sync_visibility(world);
    let live: Vec<_> = world
        .query::<(Entity, &SoulData, &Character)>()
        .iter(world)
        .map(|(e, s, _)| (e, s.clone()))
        .collect();
    for (entity, soul) in live {
        let opponent = world
            .resource::<Battles>()
            .0
            .iter()
            .find(|b| b.player == entity)
            .and_then(|b| world.get::<GrimName>(b.npc))
            .map(|n| n.0.clone());
        if let Some(mut character) = world.get_mut::<Character>(entity) {
            character.game_state.insert(
                "prompt_maxmana".into(),
                serde_json::json!(soul.derived().mana_max),
            );
            if let Some(name) = opponent {
                character
                    .game_state
                    .insert("combat_opponent".into(), serde_json::json!(name));
            } else {
                character.game_state.remove("combat_opponent");
            }
        }

        if !reconcile {
            continue;
        }
        reconcile_equipment(world, entity, soul);
    }
}

/// Save the complete character snapshot before publishing a character edit.
pub(super) fn commit(
    world: &mut World,
    actor: Entity,
    mut soul: SoulData,
    credits: Option<u64>,
) -> Result<(), String> {
    if soul.injury == rules::InjuryCondition::CriticallyInjured
        && soul.critical_since.is_none()
        && !soul.stabilized
    {
        soul.critical_since = Some(unix_now());
    }
    if let Some(character) = world.get::<Character>(actor) {
        let mut character = character.clone();
        character.game_state.insert(
            STATE.into(),
            serde_json::to_value(&soul).map_err(|e| e.to_string())?,
        );
        character.game_state.insert(
            "equipment_display".into(),
            serde_json::Value::String(equipment_display(&soul)),
        );
        if !soul.can_move() {
            character.game_state.insert(
                "action_restriction".into(),
                serde_json::Value::String(
                    "You cannot act while incapacitated or helpless from your load.".into(),
                ),
            );
        } else {
            character.game_state.remove("action_restriction");
        }
        character.game_state.insert(
            "action_restriction_allow_drop".into(),
            serde_json::Value::Bool(
                soul.is_helpless()
                    && !soul.stunned
                    && !soul.unconscious
                    && !soul.comatose
                    && !soul.injury.is_dead(),
            ),
        );
        if let Some(balance) = credits {
            character.credits = balance;
        }
        let mut stored = StoredCharacter::from_components(
            world.get::<GrimName>(actor).ok_or("Missing name")?,
            world.get::<Actor>(actor).ok_or("Missing actor")?,
            &character,
        );
        stored.inventory = crate::shop::held(world, actor)
            .into_iter()
            .map(|(_, o)| o)
            .collect();
        stored.inventory.sort_by(|a, b| a.name.cmp(&b.name));
        crate::shop::persist(world, &stored)
            .map_err(|e| format!("Could not save character: {e}. No edit applied."))?;
        world.entity_mut(actor).insert(character);
    }
    if world.get::<Creature>(actor).is_some() && soul.injury.is_dead() {
        leave_npc_remains(world, actor, soul.injury);
        return Ok(());
    }
    world.entity_mut(actor).insert(soul);
    Ok(())
}

/// Death ends the NPC's role. Reuse its entity as an ordinary persisted object;
/// possessions fall beside it because the engine has no container inventory.
pub(super) fn leave_npc_remains(world: &mut World, entity: Entity, injury: rules::InjuryCondition) {
    use grim::components::{Description, Keywords, RoomDescription};
    if let Some(room) = world.get::<InRoom>(entity).map(|r| r.room) {
        let items: Vec<_> = world
            .query_filtered::<(Entity, &CarriedBy), With<Object>>()
            .iter(world)
            .filter(|(_, held)| held.carrier == entity)
            .map(|(e, _)| e)
            .collect();
        for item in items {
            world
                .entity_mut(item)
                .remove::<CarriedBy>()
                .insert(InRoom { room });
        }
    }
    if injury == rules::InjuryCondition::Obliterated {
        world.despawn(entity);
        return;
    }
    let name = world
        .get::<GrimName>(entity)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| "creature".into());
    let reference = if name.chars().next().is_some_and(char::is_lowercase)
        && !["a ", "an ", "the "].iter().any(|a| name.starts_with(a))
    {
        format!("the {name}")
    } else {
        name.clone()
    };
    let kind = if injury == rules::InjuryCondition::Destroyed {
        "remains"
    } else {
        "corpse"
    };
    let condition = crate::conditions::fresh(world);
    let mut keywords = world
        .get::<Keywords>(entity)
        .map(|k| k.0.clone())
        .unwrap_or_default();
    keywords.extend([
        "corpse".into(),
        "body".into(),
        "remains".into(),
        name.clone(),
    ]);
    keywords.sort();
    keywords.dedup();
    world
        .entity_mut(entity)
        .remove::<(
            Creature,
            Actor,
            SoulData,
            NpcCombatBehavior,
            crate::shop::Merchant,
            grim::ScriptTriggers,
        )>()
        .insert((
            Object,
            condition,
            GrimName(format!("{kind} of {reference}")),
            Keywords(keywords),
            Description(vec![format!("The {kind} of {reference}.")]),
            RoomDescription(format!("The {kind} of {reference} lies here.")),
        ));
}

/// Build changes invalidate pending recovery and read-only trainer requests.
pub(crate) fn after_rebuild(world: &mut World, actor: Entity) {
    if let Some(mut resting) = world.get_resource_mut::<Resting>() {
        resting.0.retain(|(e, _, _)| *e != actor);
    }
    world
        .entity_mut(actor)
        .remove::<crate::trainer::Conversation>();
    crate::trainer_ai::cancel(world, actor);
    crate::species::sync_visibility(world);
}

fn reconcile_equipment(world: &mut World, entity: Entity, mut soul: SoulData) {
    let pack = crate::shop::held(world, entity);
    let owns = |name: &str| pack.iter().any(|(_, o)| o.name == name);
    let mut changed = false;
    let weight: f64 = pack
        .iter()
        .filter_map(|(_, o)| {
            world
                .resource::<EquipmentCatalog>()
                .0
                .iter()
                .find(|g| g.name.eq_ignore_ascii_case(&o.name))
                .map(|g| g.weight_kg)
        })
        .sum();
    if (soul.load_kg - weight).abs() > 0.0001 {
        soul.load_kg = weight;
        changed = true;
    }
    if !soul.weapon.is_empty() && !owns(&soul.weapon) {
        soul.weapon.clear();
        changed = true;
    }
    if !soul.armor.is_empty() && !owns(&soul.armor) {
        soul.armor.clear();
        soul.armor_bonus = 0;
        soul.sharp_armor_bonus = 0;
        soul.armor_coverage = 0;
        changed = true;
    }
    if changed {
        if let Err(error) = commit(world, entity, soul, None) {
            error!("Equipment reconciliation: {error}");
        }
    }
}

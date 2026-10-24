//! transaction implementation, split from the parent subsystem.

use super::*;

pub(super) fn keep_condition(next: &mut SoulData, live: &SoulData) {
    next.injury = live.injury;
    next.stunned = live.stunned;
    next.unconscious = live.unconscious;
    next.comatose = live.comatose;
    next.prone = live.prone;
    next.winded = live.winded;
    next.fatigued = live.fatigued;
    next.critical_since = live.critical_since;
    next.critical_checks = live.critical_checks;
    next.stabilized = live.stabilized;
    next.defense_mode = live.defense_mode.clone();
    next.mana = live.mana.max(0).min(next.derived().mana_max);
}

pub(super) fn equip(
    next: &mut SoulData,
    live: &SoulData,
    inventory: &[grim::StoredObject],
    catalog: &crate::savage_soul::EquipmentCatalog,
) -> Vec<String> {
    next.weapon.clear();
    next.armor.clear();
    next.armor_bonus = 0;
    next.sharp_armor_bonus = 0;
    next.armor_coverage = 0;
    let mut removed = Vec::new();
    for (name, kind) in [(&live.weapon, "weapon"), (&live.armor, "armor")] {
        if name.is_empty() {
            continue;
        }
        let gear = catalog.0.iter().find(|g| {
            g.name == *name
                && g.kind == kind
                && inventory.iter().any(|o| o.name == *name)
                && next.size >= g.min_size
                && (kind != "armor" || next.size == g.min_size)
        });
        if let Some(g) = gear {
            if kind == "weapon" {
                next.weapon = name.clone();
            } else {
                next.armor = name.clone();
                next.armor_bonus = g.armor_bonus;
                next.sharp_armor_bonus = g.sharp_armor_bonus;
                next.armor_coverage = g.armor_coverage;
            }
        } else {
            removed.push(format!("{name} will be unequipped and kept in your pack."));
        }
    }
    next.load_kg = inventory
        .iter()
        .filter_map(|o| {
            catalog
                .0
                .iter()
                .find(|g| g.name == o.name)
                .map(|g| g.weight_kg)
        })
        .sum();
    removed
}

pub(super) fn backup(world: &World, stored: &StoredCharacter) -> Result<(), String> {
    let characters = world
        .resource::<grim::plugins::PersistenceConfig>()
        .characters_dir();
    let dir = characters
        .parent()
        .ok_or("Missing save directory.")?
        .join("rebuild-backups");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create rebuild backup: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let path = dir.join(format!("{}-{}.json", stored.id, GrimId::new()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("Cannot create rebuild backup: {e}"))?;
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(stored).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("Cannot write rebuild backup: {e}"))
}

pub(super) fn body_parts(
    world: &mut World,
    actor: Entity,
    required: &BTreeMap<String, usize>,
) -> Result<(Vec<Entity>, String), String> {
    if required.is_empty() {
        return Ok((Vec::new(), "Body parts: none required.".into()));
    }
    let catalog = crate::crafting::load(world)?;
    let mut pack = crate::shop::held(world, actor);
    pack.sort_by_key(|(e, _)| e.to_bits());
    let mut selected = Vec::new();
    let mut lines = Vec::new();
    for (id, count) in required {
        let name = &catalog
            .items
            .get(id)
            .ok_or_else(|| format!("Unknown body part {id}; ask a wizard to repair creator.json."))?
            .name;
        let found: Vec<_> = pack
            .iter()
            .filter(|(e, o)| {
                !selected.contains(e)
                    && !crate::crafting::expired(o)
                    && (if o.keywords.iter().any(|k| k.starts_with("craft:")) {
                        o.keywords.iter().any(|k| k == &format!("craft:{id}"))
                    } else {
                        o.name.eq_ignore_ascii_case(name)
                    })
            })
            .map(|(e, _)| *e)
            .collect();
        lines.push(format!(
            "{count} {name} (carrying {}, missing {})",
            found.len(),
            count.saturating_sub(found.len())
        ));
        selected.extend(found.into_iter().take(*count));
    }
    Ok((selected, format!("Body parts: {}.", lines.join(", "))))
}

pub(super) fn apply(
    world: &mut World,
    actor: Entity,
    draft: &Draft,
    settings: &Settings,
) -> Result<String, String> {
    if !settings.med_bay(world, actor) {
        return Err("The DNA/chassis rewrite can only be performed in a configured med bay. Go to Royal Oak Med Room.".into());
    }
    if crate::savage_soul::in_combat(world, actor) {
        return Err("Finish combat before the procedure.".into());
    }
    draft.check(world, actor, settings)?;
    let live = world
        .get::<SoulData>(actor)
        .cloned()
        .ok_or("Character unavailable.")?;
    if live.is_incapacitated() {
        return Err(
            "You must be conscious, able to act and free of stun before the procedure.".into(),
        );
    }
    let before = capture(world, actor)?;
    if before.credits < draft.procedure_credits {
        return Err(format!(
            "The procedure costs {} credits; you have {}. Nothing has been charged.",
            draft.procedure_credits, before.credits
        ));
    }
    let (consumed, parts_text) = body_parts(world, actor, &draft.materials)?;
    if consumed.len() != draft.materials.values().sum::<usize>() {
        return Err(format!(
            "Missing required body parts. {parts_text} Nothing has been charged."
        ));
    }
    let inventory = crate::shop::held(world, actor)
        .into_iter()
        .filter(|(e, _)| !consumed.contains(e))
        .map(|(_, o)| o)
        .collect::<Vec<_>>();
    let mut soul = draft.soul.clone();
    keep_condition(&mut soul, &live);
    let unequipped = equip(
        &mut soul,
        &live,
        &inventory,
        world.resource::<crate::savage_soul::EquipmentCatalog>(),
    );
    if soul.available_xp() < 0 {
        return Err("The procedure would leave a negative XP balance.".into());
    }
    let after = rewritten_snapshot(&before, draft, &soul, inventory)?;
    backup(world, &before)?;
    crate::shop::persist(world, &after).map_err(|e| {
        format!("Could not save the procedure: {e}. Your character, XP and credits are unchanged.")
    })?;
    let appearance = soul
        .species
        .as_ref()
        .and_then(|s| s.template.as_ref())
        .and_then(|t| t.appearance.clone())
        .unwrap_or_else(|| "An adventurer of the scorched earth.".into());
    for item in consumed {
        world.despawn(item);
    }
    let (_, new_actor, new_character) = after.into_components();
    world.entity_mut(actor).insert((
        new_actor,
        new_character,
        soul.clone(),
        Description(vec![appearance]),
    ));
    crate::savage_soul::after_rebuild(world, actor);
    Ok(format!("The med-bay scanner completes your DNA/chassis rewrite. Race: {}.\nPaid {} credits and {} XP for the race change. Earned XP remains {}; unspent XP: {}. Injuries, mana already spent, belongings apart from required body parts, and quest progress were preserved.\n{}{}",draft.race,draft.procedure_credits,draft.race_fee_xp,soul.total_xp,soul.available_xp(),parts_text,if unequipped.is_empty(){String::new()}else{format!("\n{}",unequipped.join("\n"))}))
}

fn rewritten_snapshot(
    before: &StoredCharacter,
    draft: &Draft,
    soul: &SoulData,
    inventory: Vec<grim::StoredObject>,
) -> Result<StoredCharacter, String> {
    let mut after = before.clone();
    after.race = draft.race.clone();
    after.gender = draft.gender;
    after.class = "savage-soul".into();
    after.credits -= draft.procedure_credits;
    after.inventory = inventory;
    after.game_state.remove(DRAFT);
    after.game_state.remove("combat_opponent");
    after.game_state.insert(
        "savage_soul".into(),
        serde_json::to_value(soul).map_err(|e| e.to_string())?,
    );
    after
        .game_state
        .insert("prompt_maxmana".into(), json!(soul.derived().mana_max));
    after.game_state.insert(
        "equipment_display".into(),
        json!(format!(
            "Wielding: {}\nWearing: {}\n",
            if soul.weapon.is_empty() {
                "unarmed"
            } else {
                &soul.weapon
            },
            if soul.armor.is_empty() {
                "none"
            } else {
                &soul.armor
            }
        )),
    );
    for key in ["who", "who_race", "who_gender"] {
        after.restrings.remove(key);
    }
    after.restrings.insert("who_class".into(), "Soul".into());
    if !soul.can_move() {
        after.game_state.insert(
            "action_restriction".into(),
            json!("You cannot act while incapacitated or helpless from your load."),
        );
    } else {
        after.game_state.remove("action_restriction");
    }
    after.game_state.insert(
        "action_restriction_allow_drop".into(),
        json!(
            soul.is_helpless()
                && !soul.stunned
                && !soul.unconscious
                && !soul.comatose
                && !soul.injury.is_dead()
        ),
    );
    Ok(after)
}

//! display implementation, split from the parent subsystem.

use super::*;

pub(in crate::savage_soul) fn score(soul: &SoulData, credits: u64) -> String {
    let d = soul.derived();
    let mut out = format!(
        "Savage Soul\nXP: {} total, {} unspent | Credits: {}\nSize {} | Mass {} | {}{}{}\n",
        soul.total_xp,
        soul.available_xp(),
        credits,
        soul.size,
        soul.mass,
        soul.injury.label(),
        if soul.stunned { ", stunned" } else { "" },
        if soul.unconscious {
            ", unconscious"
        } else {
            ""
        }
    );
    for key in rules::ATTRIBUTES {
        out.push_str(&format!(
            "  {}: {} (level {})\n",
            key.replace('_', " "),
            soul.attribute(key),
            soul.attribute_levels.get(key).unwrap_or(&0)
        ));
    }
    out.push_str(&format!("  Faith: {} (not trainable)\nStrength {} | Passive Defense {} | Dodge {} | Quickness {}\nMana {}/{} | Base DR {} | Sharp DR {} | Armor DR {} | Armor Sharp DR {}\nWielding: {} | Wearing: {} | Defense: {}\nPeculiarities: {:?}\nCommands: train <attribute>, pecs, learn <peculiarity>, wield <item>, wear <armor>, attack <target>, stop, defend auto|none, task <attribute> <difficulty>.",soul.base_attribute("faith"),d.strength,d.passive_defense,d.dodge,d.quickness,soul.mana,d.mana_max,d.base_dr,d.base_sharp_dr,d.armor_dr,d.armor_sharp_dr,if soul.weapon.is_empty(){"unarmed"}else{&soul.weapon},if soul.armor.is_empty(){"none"}else{&soul.armor},soul.defense_mode,soul.peculiarities));
    if let Some(species) = &soul.species {
        if let Some(t) = &species.template {
            out.push_str(&format!(
                "\nRace: {} (v{}); {} C available, {} NC available. Use race for traits.",
                t.name,
                t.version,
                soul.available_combat_levels(),
                species.spare_nc()
            ));
        }
    }
    out.push_str(&format!(
        "\nKnown equipment load: {:.2} kg ({})",
        soul.load_kg,
        soul.load_label()
    ));
    out
}

pub(in crate::savage_soul) fn equipment_display(soul: &SoulData) -> String {
    format!(
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
    )
}

/// Route by the requested source before invoking a specialized quest handler.
pub(in crate::savage_soul) fn get_from(
    world: &mut World,
    actor: Entity,
    arg: &str,
) -> Result<(), String> {
    if let Some(result) = crate::morlock::get_from(world, actor, arg) {
        return result;
    }
    let lower = arg.to_ascii_lowercase();
    let at = lower
        .rfind(" from ")
        .ok_or("Use get <item> from <container>.")?;
    let container = arg
        .get(at + 6..)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("Name a container.")?;
    if crate::desk::is_desk(container) {
        return crate::desk::interact(world, actor, "get_from", arg);
    }
    let room = world
        .get::<InRoom>(actor)
        .ok_or("You are not in a room.")?
        .room;
    let spec =
        grim::parse_target(container, grim::ParseOptions::BEING).ok_or("Name one container.")?;
    let objects: Vec<_> = world
        .query_filtered::<(
            Entity,
            &GrimName,
            Option<&grim::components::Keywords>,
            Option<&InRoom>,
            Option<&CarriedBy>,
        ), With<Object>>()
        .iter(world)
        .filter(|(_, _, _, r, h)| {
            r.is_some_and(|r| r.room == room) || h.is_some_and(|h| h.carrier == actor)
        })
        .map(|(e, n, k, _, _)| (e, n.0.clone(), k.map(|k| k.0.clone()).unwrap_or_default()))
        .collect();
    let found = grim::query(
        &spec,
        objects
            .iter()
            .map(|(e, n, k)| (*e, n.as_str(), k.as_slice())),
    );
    let (_, name, keywords) = objects
        .iter()
        .find(|(e, _, _)| Some(e) == found.first())
        .ok_or("You don't see that container here.")?;
    if keywords.iter().any(|k| k == "corpse")
        && (name.starts_with("corpse of ") || name.starts_with("remains of "))
    {
        reply(world, actor, "The corpse is empty.");
    } else {
        reply(world, actor, format!("{name} is not a container."));
    }
    Ok(())
}

pub(in crate::savage_soul) fn wizard_command(
    world: &mut World,
    actor: Entity,
    verb: &str,
    arg: &str,
) -> Result<(), String> {
    if !world.get::<Character>(actor).unwrap().is_admin() {
        return Err("That command requires wizard privileges.".into());
    }
    let (name, amount) = if verb == "awardxp" {
        let (n, a) = arg
            .rsplit_once(' ')
            .ok_or("Use awardxp <character> <amount>.")?;
        (
            n,
            Some(a.parse::<i64>().map_err(|_| "XP must be an integer.")?),
        )
    } else {
        (arg, None)
    };
    let target = if name.is_empty() || name.eq_ignore_ascii_case("self") {
        actor
    } else {
        world
            .query::<(Entity, &GrimName, &SoulData)>()
            .iter(world)
            .find(|(_, n, _)| n.0.eq_ignore_ascii_case(name))
            .map(|(e, _, _)| e)
            .ok_or("That character is not loaded.")?
    };
    let mut soul = world.get::<SoulData>(target).unwrap().clone();
    if let Some(xp) = amount {
        if !(1..=1000000).contains(&xp) {
            return Err("Award between 1 and 1,000,000 XP.".into());
        }
        soul.total_xp = soul.total_xp.checked_add(xp).ok_or("XP overflow")?;
    } else {
        soul.reset_conditions();
        if let Some(mut character) = world.get_mut::<Character>(target) {
            if let Some(state) = character.game_state.get_mut("royal_oak_desk") {
                state["venom_next"] = serde_json::Value::Null;
            }
        }
        world
            .resource_mut::<Battles>()
            .0
            .retain(|b| b.player != target && b.npc != target);
    }
    commit(world, target, soul, None)?;
    if verb == "soulheal" && world.get::<Creature>(target).is_some() {
        let name = world
            .get::<GrimName>(target)
            .map(|n| n.0.clone())
            .unwrap_or_else(|| "A creature".into());
        world
            .entity_mut(target)
            .insert(grim::components::RoomDescription(format!(
                "{name} is here."
            )));
    }
    reply(
        world,
        actor,
        if verb == "awardxp" {
            "XP awarded and saved."
        } else {
            "Conditions restored and saved."
        },
    );
    Ok(())
}

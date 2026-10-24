//! runtime implementation, split from the parent subsystem.

use super::*;

pub(crate) fn allows_food(soul: &SoulData, diet: &str) -> bool {
    soul.trait_level("carnivore") == 0 || diet == "meat"
}

pub(crate) fn command(world: &mut World, actor: Entity, arg: &str) {
    let text = if arg.trim().is_empty() {
        world.get::<SoulData>(actor).and_then(|s|s.species.as_ref()).map(|s|s.summary()).unwrap_or_else(||"Your existing character keeps its original build. No new species template has been applied.".into())
    } else {
        let key = arg.trim().to_ascii_lowercase();
        let catalog = world.resource::<Catalog>();
        if key == "list" || key == "all" {
            catalog
                .0
                .iter()
                .map(|t| {
                    let s = t.build().unwrap();
                    format!(
                        "{}: {} unspent starting XP. {}",
                        t.slug,
                        s.available_xp(),
                        t.description
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            catalog
                .0
                .iter()
                .find(|t| t.slug == key || t.name.eq_ignore_ascii_case(&key))
                .map(|t| t.build().unwrap().species.unwrap().summary())
                .unwrap_or_else(|| "Unknown race. Use races to list choices.".into())
        }
    };
    world.write_message(grim::events::InfoMessage {
        target: actor,
        text,
    });
}

/// Repay the capacity a disadvantage granted before removing its restriction.
/// This uses spare capacity first, buying only a deficit; total XP is unchanged.
pub(crate) fn buy_off(soul: &mut SoulData, name: &str) -> Result<String, String> {
    let name = name.trim().to_ascii_lowercase();
    let original = soul
        .species
        .as_ref()
        .ok_or("Your legacy build has no template disadvantages.")?;
    let level = original
        .template
        .as_ref()
        .and_then(|t| t.disadvantages.get(&name))
        .copied()
        .ok_or("That is not one of your template disadvantages.")?;
    if original.bought_off.contains_key(&name) {
        return Err("That disadvantage has already been bought off.".into());
    }
    let (c, nc) = cost(&name, level, true)?;
    let mut next = soul.clone();
    let species = next.species.as_mut().unwrap();
    species.bonus_c -= c;
    species.bonus_nc -= nc;
    species.converted_c = ((species.cost_nc - species.bonus_nc).max(0) + 1) / 2;
    let xp = (-next.available_combat_levels()).max(0) * 100;
    if xp > next.available_xp() {
        return Err(format!(
            "You need {xp} unspent XP (or more unused capacity) to repay this disadvantage."
        ));
    }
    next.peculiarity_xp += xp;
    let species = next.species.as_mut().unwrap();
    species.bought_off.insert(name.clone(), level);
    species.traits.remove(&name);
    next.mana = next.mana.min(next.derived().mana_max);
    *soul = next;
    Ok(format!("Bought off {name}; repaid {c} C and {nc} NC using spare capacity and {xp} XP. The restriction is removed."))
}

/// Reconstruct transient vision/light components after login or copyover.
pub(crate) fn sync_visibility(world: &mut World) {
    let actors: Vec<_> = world
        .query::<(
            Entity,
            &SoulData,
            Option<&grim::InRoom>,
            Option<&grim::DarkVision>,
        )>()
        .iter(world)
        .map(|(e, s, r, v)| {
            (
                e,
                s.trait_level("enhanced vision") > 0,
                r.map(|r| r.room),
                v.is_some(),
            )
        })
        .collect();
    for (e, night, _, old) in &actors {
        if night != old {
            if *night {
                world.entity_mut(*e).insert(grim::DarkVision);
            } else {
                world.entity_mut(*e).remove::<grim::DarkVision>();
            }
        }
    }
    let items: Vec<_> = world
        .query_filtered::<(
            Entity,
            &grim::components::Keywords,
            Option<&grim::LightSource>,
        ), With<grim::Object>>()
        .iter(world)
        .map(|(e, k, l)| (e, k.0.iter().any(|k| k == "light:active"), l.is_some()))
        .collect();
    for (e, lit, old) in items {
        if lit != old {
            if lit {
                world.entity_mut(e).insert(grim::LightSource);
            } else {
                world.entity_mut(e).remove::<grim::LightSource>();
            }
        }
    }
    let dark: std::collections::HashSet<_> = world
        .query_filtered::<Entity, With<grim::RoomDarkness>>()
        .iter(world)
        .collect();
    let lights:Vec<_>=world.query_filtered::<(Option<&grim::InRoom>,Option<&grim::CarriedBy>),With<grim::LightSource>>().iter(world)
        .map(|(r,c)|(r.map(|r|r.room),c.map(|c|c.carrier))).collect();
    let lit: std::collections::HashSet<_> = lights
        .into_iter()
        .filter_map(|(r, c)| {
            r.or_else(|| c.and_then(|e| world.get::<grim::InRoom>(e).map(|r| r.room)))
        })
        .collect();
    for (e, night, room, _) in actors {
        let penalty = if !night && room.is_some_and(|r| dark.contains(&r) && !lit.contains(&r)) {
            3
        } else {
            0
        };
        let mut soul = world.get_mut::<SoulData>(e).unwrap();
        if soul.vision_penalty != penalty {
            soul.vision_penalty = penalty;
        }
    }
}

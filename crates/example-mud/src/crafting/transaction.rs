//! transaction implementation, split from the parent subsystem.

use super::*;

pub(super) fn plan_recipe(
    world: &mut World,
    actor: Entity,
    catalog: &Catalog,
    r: &Recipe,
    pack: &[(Entity, StoredObject)],
    verb: &str,
    target: Option<Entity>,
) -> Result<(HashSet<Entity>, Vec<StoredObject>), String> {
    let room = world.get::<InRoom>(actor).ok_or("You are nowhere.")?.room;
    let fixtures: Vec<(String, Vec<String>)> = world
        .query_filtered::<(&GrimName, Option<&Keywords>, &InRoom), Without<Actor>>()
        .iter(world)
        .filter(|(_, _, p)| p.room == room)
        .map(|(n, k, _)| (n.0.clone(), k.map(|k| k.0.clone()).unwrap_or_default()))
        .collect();
    for tool in &r.tools {
        let available = pack
            .iter()
            .any(|(_, i)| !expired(i) && (matches_item(catalog, i, tool) || named(i, tool)))
            || fixtures.iter().any(|(name, keys)| {
                normal(name) == normal(tool)
                    || keys
                        .iter()
                        .any(|k| k == &format!("craft:{tool}") || normal(k) == normal(tool))
            });
        if !available {
            return Err(format!("You need access to {}.", display(catalog, tool)));
        }
    }
    let mut removed = HashSet::new();
    for (id, amount) in &r.ingredients {
        if *amount == 0 || *amount > 1000 {
            return Err("That recipe has an invalid quantity.".into());
        }
        let selected: Vec<Entity> = pack
            .iter()
            .filter(|(e, i)| {
                !removed.contains(e)
                    && !expired(i)
                    && (verb != "reduce" || target.is_some_and(|t| t == *e))
                    && (matches_item(catalog, i, id)
                        || (verb == "reduce"
                            && target.is_some_and(|target| target == *e)
                            && !i.keywords.iter().any(|k| k.starts_with("craft:"))
                            && r.aliases.iter().any(|a| named(i, a))))
            })
            .map(|(e, _)| *e)
            .take(*amount)
            .collect();
        if selected.len() != *amount {
            return Err(format!("You need {amount} {}.", display(catalog, id)));
        }
        removed.extend(selected);
    }
    let mut outputs = Vec::new();
    for (id, amount) in &r.products {
        if *amount == 0 || *amount > 1000 || outputs.len() + amount > 1000 {
            return Err("That recipe has an invalid output quantity.".into());
        }
        let item = catalog
            .items
            .get(id)
            .ok_or("That recipe has a missing product; no items changed.")?;
        for _ in 0..*amount {
            let mut item = item.clone();
            item.keywords.push(format!("craft:{id}"));
            outputs.push(item);
        }
    }
    Ok((removed, outputs))
}

pub(super) fn finish_recipe(
    world: &mut World,
    actor: Entity,
    catalog: &Catalog,
    r: &Recipe,
    pack: &[(Entity, StoredObject)],
    plan: (HashSet<Entity>, Vec<StoredObject>),
    verb: &str,
) -> Result<(), String> {
    let (removed, outputs) = plan;
    let mut character = world
        .get::<Character>(actor)
        .ok_or("Character data is unavailable.")?
        .clone();
    character.game_state.insert(
        "savage_soul".into(),
        serde_json::to_value(world.get::<crate::savage_rules::SoulData>(actor).unwrap())
            .map_err(|_| "Could not save character conditions.")?,
    );
    let mut saved = StoredCharacter::from_components(
        world
            .get::<GrimName>(actor)
            .ok_or("Missing character name.")?,
        world.get::<Actor>(actor).ok_or("Missing character data.")?,
        &character,
    );
    saved.inventory = pack
        .iter()
        .filter(|(e, _)| !removed.contains(e))
        .map(|(_, o)| o.clone())
        .chain(outputs.iter().cloned())
        .collect();
    crate::shop::persist(world, &saved)
        .map_err(|_| "Could not save crafting. Your items have not changed.")?;
    world.entity_mut(actor).insert(character);
    for entity in removed {
        world.despawn(entity);
    }
    for item in outputs {
        world.spawn((
            Object,
            GrimName(item.name),
            Description(item.description),
            Keywords(item.keywords),
            RoomDescription(item.room_description),
            item.condition.unwrap_or_default(),
            CarriedBy { carrier: actor },
        ));
    }
    say(
        world,
        actor,
        format!(
            "You {} {}. Produced: {}.",
            if verb == "reduce" { "reduce" } else { "craft" },
            r.name,
            r.products
                .iter()
                .map(|(id, n)| format!("{n} {}", display(catalog, id)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
}

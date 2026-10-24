//! Recovered recipes using ordinary objects and complete-pack transactions.
use bevy::prelude::*;
use grim::components::{Description, Keywords, Name as GrimName, RoomDescription};
use grim::events::InfoMessage;
use grim::{Actor, CarriedBy, Character, InRoom, Object, StoredCharacter, StoredObject};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
mod transaction;
use transaction::{finish_recipe, plan_recipe};

#[derive(Deserialize, Default)]
pub(crate) struct Catalog {
    #[serde(default)]
    pub items: BTreeMap<String, StoredObject>,
    #[serde(default)]
    recipes: Vec<Recipe>,
    #[serde(default)]
    reductions: Vec<Recipe>,
    #[serde(default)]
    pub meals: BTreeMap<String, Meal>,
}
#[derive(Deserialize)]
pub(crate) struct Meal {
    pub verb: String,
    pub message: String,
    #[serde(default)]
    pub diet: String,
}
#[derive(Deserialize)]
struct Recipe {
    id: String,
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    ingredients: BTreeMap<String, usize>,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    products: BTreeMap<String, usize>,
    #[serde(default)]
    unavailable: Option<String>,
}
pub(crate) fn load(world: &World) -> Result<Catalog, String> {
    let path = world
        .get_resource::<crate::seed::AreaBlueprintDir>()
        .map(|d| {
            d.0.parent()
                .unwrap_or(&d.0)
                .join("savage-soul/crafting.json")
        })
        .unwrap_or_else(|| "data/savage-soul/crafting.json".into());
    let bytes = std::fs::read(path).map_err(|_| "Crafting recipes are unavailable.".to_string())?;
    serde_json::from_slice(&bytes)
        .map_err(|_| "The crafting catalog needs repair; no items changed.".into())
}
fn normal(s: &str) -> String {
    s.trim().to_ascii_lowercase().replace('-', " ")
}
fn tagged(item: &StoredObject, id: &str) -> bool {
    item.keywords.iter().any(|k| k == &format!("craft:{id}"))
}
fn named(item: &StoredObject, name: &str) -> bool {
    normal(&item.name) == normal(name) || item.keywords.iter().any(|k| normal(k) == normal(name))
}
pub(crate) fn matches_item(catalog: &Catalog, item: &StoredObject, id: &str) -> bool {
    if item.keywords.iter().any(|k| k.starts_with("craft:")) {
        return tagged(item, id);
    }
    catalog
        .items
        .get(id)
        .is_some_and(|base| item.name.eq_ignore_ascii_case(&base.name))
        || named(item, &id.replace('-', " "))
}
pub(crate) fn expired(item: &StoredObject) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    item.condition
        .as_ref()
        .and_then(|c| c.decay.as_ref())
        .is_some_and(|d| d.expires_at <= now)
}
fn display(catalog: &Catalog, id: &str) -> String {
    catalog
        .items
        .get(id)
        .map(|i| i.name.clone())
        .unwrap_or_else(|| id.replace('-', " "))
}
fn details(catalog: &Catalog, r: &Recipe) -> String {
    let list = |values: &BTreeMap<String, usize>| {
        values
            .iter()
            .map(|(id, n)| format!("{n} {}", display(catalog, id)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{}\nIngredients: {}\nTools: {}\nProduces: {}{}",
        r.name,
        list(&r.ingredients),
        if r.tools.is_empty() {
            "none".into()
        } else {
            r.tools
                .iter()
                .map(|id| display(catalog, id))
                .collect::<Vec<_>>()
                .join(", ")
        },
        list(&r.products),
        r.unavailable
            .as_ref()
            .map(|reason| format!("\nUnavailable: {reason}"))
            .unwrap_or_default()
    )
}
pub(crate) fn command(
    world: &mut World,
    actor: Entity,
    verb: &str,
    arg: &str,
) -> Result<(), String> {
    let catalog = load(world)?;
    let pack = crate::shop::held(world, actor);
    if verb == "resources" {
        let text = catalog
            .items
            .iter()
            .filter_map(|(id, item)| {
                let n = pack
                    .iter()
                    .filter(|(_, o)| matches_item(&catalog, o, id))
                    .count();
                (n > 0).then(|| format!("{n} {}", item.name))
            })
            .collect::<Vec<_>>()
            .join("\n");
        return say(
            world,
            actor,
            if text.is_empty() {
                "You carry no catalogued crafting supplies.".into()
            } else {
                format!("Crafting supplies:\n{text}")
            },
        );
    }
    let entries = if verb == "reduce" || verb == "reductions" {
        &catalog.reductions
    } else {
        &catalog.recipes
    };
    let listing = verb == "recipes" || verb == "reductions";
    if listing || arg.trim().is_empty() {
        let text = recipe_listing(&catalog, entries, verb, arg)?;
        return say(world, actor, text);
    }
    let soul = world
        .get::<crate::savage_rules::SoulData>(actor)
        .ok_or("Your character record is unavailable.")?;
    if soul.is_incapacitated() {
        return Err("You cannot craft in your current condition.".into());
    }
    if crate::savage_soul::in_combat(world, actor) {
        return Err("Finish combat before crafting or reducing items.".into());
    }
    let spec = grim::parse_target(arg, grim::ParseOptions::BEING)
        .ok_or("Name a recipe or carried item.")?;
    let targets = grim::query(
        &spec,
        pack.iter()
            .map(|(e, o)| (*e, o.name.as_str(), o.keywords.as_slice())),
    );
    let target = targets
        .first()
        .and_then(|entity| pack.iter().find(|(e, _)| e == entity));
    let r = if verb == "reduce" {
        let (_, item) = target.ok_or("You are not carrying that item.")?;
        if item.name.eq_ignore_ascii_case(&soul.weapon)
            || item.name.eq_ignore_ascii_case(&soul.armor)
        {
            return Err("Remove or unwield that item before reducing it.".into());
        }
        entries
            .iter()
            .find(|r| {
                r.ingredients
                    .keys()
                    .any(|id| matches_item(&catalog, item, id))
                    || (!item.keywords.iter().any(|k| k.starts_with("craft:"))
                        && r.aliases.iter().any(|a| named(item, a)))
            })
            .ok_or("That item has no recovered reduction recipe.")?
    } else {
        entries
            .iter()
            .find(|r| {
                normal(&r.name) == normal(arg)
                    || normal(&r.id) == normal(arg)
                    || r.aliases.iter().any(|a| normal(a) == normal(arg))
            })
            .ok_or("Unknown recipe. Use recipes to see available recipes.")?
    };
    if let Some(reason) = &r.unavailable {
        return Err(format!("{} is unavailable: {reason}", r.name));
    }
    if r.ingredients.is_empty() || r.products.is_empty() {
        return Err("That recipe is incomplete; no items changed.".into());
    }
    let (removed, outputs) = plan_recipe(
        world,
        actor,
        &catalog,
        r,
        &pack,
        verb,
        target.map(|(e, _)| *e),
    )?;
    finish_recipe(world, actor, &catalog, r, &pack, (removed, outputs), verb)
}
fn say(world: &mut World, actor: Entity, text: String) -> Result<(), String> {
    world.write_message(InfoMessage {
        target: actor,
        text,
    });
    Ok(())
}

fn recipe_listing(
    catalog: &Catalog,
    entries: &[Recipe],
    verb: &str,
    arg: &str,
) -> Result<String, String> {
    let text = if arg.trim().is_empty() {
        format!(
            "{}:\n{}\nUse {} <name> for requirements.",
            if verb == "reductions" {
                "Reductions"
            } else {
                "Recipes"
            },
            entries
                .iter()
                .map(|r| format!(
                    "{}{}",
                    r.name,
                    if r.unavailable.is_some() {
                        " (unavailable)"
                    } else {
                        ""
                    }
                ))
                .collect::<Vec<_>>()
                .join("\n"),
            if verb == "reductions" {
                "reductions"
            } else {
                "recipes"
            }
        )
    } else {
        let r = entries
            .iter()
            .find(|r| {
                normal(&r.name) == normal(arg)
                    || normal(&r.id) == normal(arg)
                    || r.aliases.iter().any(|a| normal(a) == normal(arg))
            })
            .ok_or("No recipe by that name. Use recipes or reductions.")?;
        details(catalog, r)
    };
    Ok(text)
}

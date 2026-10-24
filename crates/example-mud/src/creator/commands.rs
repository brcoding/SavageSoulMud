//! commands implementation, split from the parent subsystem.

use super::*;

pub(super) fn show(world: &mut World, actor: Entity, d: &Draft) -> Result<String, String> {
    let live = world
        .get::<SoulData>(actor)
        .cloned()
        .ok_or("Character unavailable.")?;
    let before = capture(world, actor)?;
    let mut preview = d.soul.clone();
    keep_condition(&mut preview, &live);
    let gear = equip(
        &mut preview,
        &live,
        &before.inventory,
        world.resource::<crate::savage_soul::EquipmentCatalog>(),
    );
    let values = rules::ATTRIBUTES
        .iter()
        .map(|a| format!("{} {}", a.replace('_', " "), d.soul.base_attribute(a)))
        .collect::<Vec<_>>();
    let mut text=format!("Character draft: {} / {:?}\nEarned XP {} (kept); live unspent {} -> draft unspent {}.\nRace-change fee: {} XP. Previous committed race-change spending: {} XP.\nProcedure: {} credits (you have {}). Species capacity purchases: {} XP.\n{}\n{} C available. Nothing has been charged.\n",d.race,d.gender,d.soul.total_xp,live.available_xp(),d.soul.available_xp(),d.race_fee_xp,live.rebuild_xp,d.procedure_credits,before.credits,d.soul.peculiarity_xp,values.join(" | "),d.soul.available_combat_levels());
    let (_, parts) = body_parts(world, actor, &d.materials)?;
    text.push_str(&format!("{parts}\n"));
    if !gear.is_empty() {
        text.push_str(&format!("{}\n", gear.join("\n")));
    }
    text.push_str("Use creator race <name>, creator train <attribute>, creator learn <name>, creator gender <gender>.\ncreator confirm pays for and applies this draft; creator cancel spends nothing. Use help creator for all options.");
    Ok(text)
}

pub(super) fn reply(world: &mut World, actor: Entity, text: String) {
    world.write_message(grim::events::InfoMessage {
        target: actor,
        text,
    });
}

pub(crate) fn command(world: &mut World, actor: Entity, arg: &str) -> Result<(), String> {
    let arg = arg.trim();
    let (op, rest) = arg.split_once(' ').unwrap_or((arg, ""));
    let op = op.to_ascii_lowercase();
    let rest = rest.trim();
    if op == "help" {
        reply(world, actor, HELP.into());
        return Ok(());
    }
    if op == "cancel" {
        save_draft(world, actor, None)?;
        reply(
            world,
            actor,
            "Draft discarded. No XP or credits were spent.".into(),
        );
        return Ok(());
    }
    let settings = Settings::load(world)?;
    if !settings.med_bay(world, actor) {
        return Err("Use the character creator in a med bay. Royal Oak Med Room can rewrite DNA and reconfigure synthetic bodies. Nothing has been charged.".into());
    }
    let c = world
        .get::<Character>(actor)
        .cloned()
        .ok_or("Character unavailable.")?;
    let a = world
        .get::<Actor>(actor)
        .cloned()
        .ok_or("Character unavailable.")?;
    let live = world
        .get::<SoulData>(actor)
        .cloned()
        .ok_or("Character unavailable.")?;
    if op == "races" || (op == "race" && rest.is_empty()) {
        let lines = available_races(world, &c, &a, &live, &settings);
        reply(world, actor, lines);
        return Ok(());
    }
    let existing = c.game_state.get(DRAFT).cloned();
    if op == "confirm" && existing.is_none() {
        return Err("Open creator and review a draft before confirming.".into());
    }
    let mut d = match existing.filter(|_| op != "reset") {
        Some(value) => serde_json::from_value(value)
            .map_err(|_| "Draft data is invalid. Use creator reset to start again.")?,
        None => {
            let base = template(world.resource::<Catalog>(), &a.race)
                .or_else(|_| template(world.resource::<Catalog>(), "human"))?;
            Draft::new(c.id, &a, &live, &base, &settings)?
        }
    };
    if op != "reset" {
        d.check(world, actor, &settings)?;
    }
    let mut result = String::new();
    match op.as_str() {
        "" | "show" | "stats" | "reset" => {}
        "race" => {
            let selected = template(world.resource::<Catalog>(), rest)?;
            let gender = d.gender;
            d = Draft::new(c.id, &a, &live, &selected, &settings)?;
            d.gender = gender;
            result = "Race selected; uncommitted training has been reset.\n".into();
        }
        "gender" => {
            d.gender = match rest.to_ascii_lowercase().as_str() {
                "male" | "m" => Gender::Male,
                "female" | "f" => Gender::Female,
                "neutral" | "n" => Gender::Neutral,
                _ => return Err("Use creator gender male, female, or neutral.".into()),
            }
        }
        "train" => result = d.soul.train(rest)? + "\n",
        "learn" => result = d.soul.learn(rest)? + "\n",
        "buyoff" => result = crate::species::buy_off(&mut d.soul, rest)? + "\n",
        "confirm" => {
            if !rest.is_empty() {
                return Err("Use exactly creator confirm after reviewing the draft.".into());
            }
            let text = apply(world, actor, &d, &settings)?;
            reply(world, actor, text);
            return Ok(());
        }
        _ => return Err(HELP.into()),
    }
    if d.soul.available_xp() < 0 {
        return Err("The draft cannot have negative XP.".into());
    }
    save_draft(world, actor, Some(&d))?;
    let text = show(world, actor, &d)?;
    reply(world, actor, result + &text);
    Ok(())
}

fn available_races(
    world: &World,
    c: &Character,
    a: &Actor,
    live: &SoulData,
    settings: &Settings,
) -> String {
    let templates = world.resource::<Catalog>().0.clone();
    let lines = templates
        .iter()
        .enumerate()
        .map(|(i, t)| match Draft::new(c.id, a, live, t, settings) {
            Ok(d) => format!(
                "{}. {} [{}]: {} XP change fee, {} XP left for training. {}",
                i + 1,
                t.name,
                t.slug,
                d.race_fee_xp,
                d.soul.available_xp(),
                t.description
            ),
            Err(e) => format!("{}. {} [{}]: {e}", i + 1, t.name, t.slug),
        })
        .collect::<Vec<_>>()
        .join("\n");
    lines
}

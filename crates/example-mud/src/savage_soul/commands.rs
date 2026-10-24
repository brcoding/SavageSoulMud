//! commands implementation, split from the parent subsystem.

use super::*;
mod display;
pub(super) use display::{equipment_display, get_from, score, wizard_command};

pub(super) fn handle_commands(world: &mut World, mut cursor: Local<MessageCursor<EngineCommand>>) {
    let inputs: Vec<_> = cursor
        .read(world.resource::<Messages<EngineCommand>>())
        .filter_map(|m| match &m.command {
            Command::SavageSoul { verb, arg }
                if ![
                    "screenreader",
                    "combatlog",
                    "prompt",
                    "intermud",
                    "i3",
                    "imud",
                ]
                .contains(&verb.as_str()) =>
            {
                Some((m.client, verb.clone(), arg.clone()))
            }
            _ => None,
        })
        .collect();
    for (actor, verb, arg) in inputs {
        if world.get::<Player>(actor).is_none() {
            continue;
        }
        if let Err(e) = run_command(world, actor, &verb, &arg) {
            reply(world, actor, e);
        }
    }
}

pub(super) fn run_command(
    world: &mut World,
    actor: Entity,
    verb: &str,
    arg: &str,
) -> Result<(), String> {
    let mut soul = world
        .get::<SoulData>(actor)
        .ok_or("Your Savage Soul record could not be loaded.")?
        .clone();
    let credits = world.get::<Character>(actor).unwrap().credits;
    if soul.vision_penalty > 0 && ["inspect", "read_note", "disarm", "hack"].contains(&verb) {
        return Err("It is too dark for that. Bring a light or use night vision.".into());
    }

    if dispatch_immediate(world, actor, verb, arg, &soul, credits)? {
        return Ok(());
    }
    if soul.is_incapacitated() {
        return Err("You cannot act in your current condition. A wizard can use soulheal.".into());
    }
    if [
        "train", "learn", "buyoff", "wield", "wear", "remove", "exchange", "rest",
    ]
    .contains(&verb)
        && world
            .resource::<Battles>()
            .0
            .iter()
            .any(|b| b.player == actor)
    {
        return Err("Finish combat before changing your build or equipment.".into());
    }
    if ["open_object", "close_object"].contains(&verb)
        && crate::gate::object_door(world, actor, verb, arg)
    {
        return Ok(());
    }
    let message = match verb {
        "put" | "place" => return crate::medical::place(world, actor, arg),
        "sit" | "lie" | "lay" => return crate::medical::rest(world, actor, verb, arg),
        "stand" => return crate::medical::stand(world, actor, arg),
        "get_from" => return get_from(world, actor, arg),
        "disarm" | "open_object" | "close_object" => {
            return crate::desk::interact(world, actor, verb, arg)
        }
        "drink" | "eat" => return crate::consumables::consume(world, actor, verb, arg),
        "train" => soul.train(arg)?,
        "learn" => soul.learn(arg)?,
        "buyoff" => crate::species::buy_off(&mut soul, arg)?,
        "defend" => {
            if !["auto", "dodge", "none"].contains(&arg) {
                return Err("Use defend auto, defend dodge, or defend none.".into());
            }
            soul.defense_mode = arg.into();
            format!("Defense set to {arg}. Active defense discards a Ready token when available.")
        }
        "wield" | "wear" => equip_item(world, actor, verb, arg, &mut soul)?,
        "remove" => {
            if ["weapon", "all"].contains(&arg) {
                soul.weapon.clear();
            }
            if ["armor", "all"].contains(&arg) {
                soul.armor.clear();
                soul.armor_bonus = 0;
                soul.sharp_armor_bonus = 0;
                soul.armor_coverage = 0;
            }
            if !["weapon", "armor", "all"].contains(&arg) {
                return Err("Use remove weapon, remove armor, or remove all.".into());
            }
            "Equipment removed.".into()
        }
        "task" => return attempt_task(world, actor, arg, &soul),
        "attack" | "kill" | "trample" | "stomp" => {
            return begin_battle(
                world,
                actor,
                arg,
                ["trample", "stomp"].contains(&verb),
                verb == "kill",
            )
        }
        "rest" => {
            if !soul.winded {
                return Err("You are not winded. Rest does not heal wounds.".into());
            }
            if world
                .resource::<Resting>()
                .0
                .iter()
                .any(|(e, _, _)| *e == actor)
            {
                return Err("You are already resting.".into());
            }
            let room = world.get::<InRoom>(actor).unwrap().room;
            let end = world.resource::<Time>().elapsed_secs_f64() + 5.0;
            world.resource_mut::<Resting>().0.push((actor, room, end));
            reply(world,actor,"You rest for one five-second round, then attempt Will Power at DF 10 to recover from being winded.");
            return Ok(());
        }
        "exchange" => return exchange_xp(world, actor, arg, soul, credits),
        _ => return Err("Unknown Savage Soul command.".into()),
    };
    commit(world, actor, soul, None)?;
    reply(world, actor, message);
    Ok(())
}

fn equip_item(
    world: &mut World,
    actor: Entity,
    verb: &str,
    arg: &str,
    soul: &mut SoulData,
) -> Result<String, String> {
    let spec = grim::parse_target(arg, grim::ParseOptions::BEING)
        .ok_or("Name an item you are carrying.")?;
    let pack = crate::shop::held(world, actor);
    let matches = grim::query(
        &spec,
        pack.iter()
            .map(|(e, o)| (*e, o.name.as_str(), o.keywords.as_slice())),
    );
    let item = &pack
        .iter()
        .find(|(e, _)| Some(e) == matches.first())
        .ok_or("You are not carrying that item.")?
        .1;
    let gear = world
        .resource::<EquipmentCatalog>()
        .0
        .iter()
        .find(|g| g.name.eq_ignore_ascii_case(&item.name))
        .ok_or("That item has no Savage Soul equipment definition.")?;
    if soul.size < gear.min_size || (gear.kind == "armor" && soul.size != gear.min_size) {
        return Err(format!(
            "{} requires Size {}{}.",
            gear.name,
            gear.min_size,
            if gear.kind == "weapon" {
                " or greater"
            } else {
                " exactly"
            }
        ));
    }
    Ok(if verb == "wield" && gear.kind == "weapon" {
        soul.weapon = item.name.clone();
        format!("You wield {}.", item.name)
    } else if verb == "wear" && gear.kind == "armor" {
        soul.armor = item.name.clone();
        soul.armor_bonus = gear.armor_bonus;
        soul.sharp_armor_bonus = gear.sharp_armor_bonus;
        soul.armor_coverage = gear.armor_coverage;
        format!("You wear {}.", item.name)
    } else {
        return Err("Use wield for a weapon or wear for armor.".into());
    })
}

fn attempt_task(
    world: &mut World,
    actor: Entity,
    arg: &str,
    soul: &SoulData,
) -> Result<(), String> {
    let (name, df) = arg
        .rsplit_once(' ')
        .ok_or("Use task <attribute> <difficulty>.")?;
    let name = rules::normalize_attribute(name);
    if !rules::ATTRIBUTES.contains(&name.as_str()) && name != "faith" {
        return Err("Unknown task attribute.".into());
    }
    let df: i32 = df.parse().map_err(|_| "Difficulty must be an integer.")?;
    if !(1..=100).contains(&df) {
        return Err("Difficulty must be between 1 and 100.".into());
    }
    let bonus = soul.attribute(&name);
    let result = rules::resolve_task(bonus, df, false, || world.resource_mut::<Dice>().roll());
    reply(
        world,
        actor,
        format!(
            "{name} attempt: total {} versus DF {df}; {} margin {}{}.",
            result.total,
            if result.success { "success" } else { "failure" },
            result.margin,
            if result.critical_failure {
                " (critical failure)"
            } else {
                ""
            }
        ),
    );
    Ok(())
}

fn exchange_xp(
    world: &mut World,
    actor: Entity,
    arg: &str,
    mut soul: SoulData,
    credits: u64,
) -> Result<(), String> {
    let xp: i64 = arg
        .parse()
        .map_err(|_| "Use exchange <unspent XP>; 1 XP grants 100 credits.")?;
    if xp <= 0
        || xp > soul.available_xp()
        || soul.attribute_levels.values().any(|l| *l > 0)
        || !soul.peculiarities.is_empty()
        || soul
            .species
            .as_ref()
            .is_some_and(|s| s.cost_c > 0 || s.cost_nc > 0)
    {
        return Err("Exchange is only available before buying attributes or Peculiarities, and cannot exceed unspent XP.".into());
    }
    let balance = credits
        .checked_add((xp as u64).checked_mul(100).ok_or("Credit overflow")?)
        .ok_or("Credit overflow")?;
    soul.total_xp -= xp;
    commit(world, actor, soul, Some(balance))?;
    reply(
        world,
        actor,
        format!("Exchanged {xp} XP. You have {balance} credits."),
    );
    Ok(())
}

fn recover_stun(world: &mut World, actor: Entity, mut soul: SoulData) -> Result<(), String> {
    if world
        .resource::<Battles>()
        .0
        .iter()
        .any(|b| b.player == actor)
    {
        return Err(
            "In combat you automatically recover from stun on your next activation.".into(),
        );
    }
    if soul.stunned && !soul.unconscious && !soul.comatose && !soul.injury.is_dead() {
        soul.stunned = false;
        commit(world, actor, soul, None)?;
        reply(
            world,
            actor,
            "You use your activation to recover from stun.",
        );
        return Ok(());
    }
    Err("You are not able to recover from stun right now.".into())
}

fn dispatch_immediate(
    world: &mut World,
    actor: Entity,
    verb: &str,
    arg: &str,
    soul: &SoulData,
    credits: u64,
) -> Result<bool, String> {
    match verb {
        "zebedee" | "i2" => {
            crate::zebedee::command(world, actor, arg);
            return Ok(true);
        }
        "protocols" => return (crate::client_protocols::command(world, actor, arg)).map(|_| true),
        "grapevine" | "gv" => return (crate::grapevine::command(world, actor, arg)).map(|_| true),
        "ask" | "talk" => return (crate::royal_oak::ask(world, actor, arg)).map(|_| true),
        "press" => return (crate::morlock::press(world, actor, arg)).map(|_| true),
        "hack" => return (crate::hack::command(world, actor, arg)).map(|_| true),
        "craft" | "reduce" | "recipes" | "reductions" | "resources" => {
            return (crate::crafting::command(world, actor, verb, arg)).map(|_| true)
        }
        "spawns" => {
            let text = crate::population::status(world, actor, arg)?;
            reply(world, actor, text);
            return Ok(true);
        }
        "activate" => return (crate::royal_oak::activate(world, actor, arg)).map(|_| true),
        "inspect" | "read_note" => {
            return (crate::desk::interact(world, actor, verb, arg)).map(|_| true)
        }
        "creator" | "rebuild" | "respec" => {
            return (crate::creator::command(world, actor, arg)).map(|_| true)
        }
        "race" => {
            crate::species::command(world, actor, arg);
            return Ok(true);
        }
        "races" => {
            crate::species::command(
                world,
                actor,
                if arg.trim().is_empty() { "list" } else { arg },
            );
            return Ok(true);
        }
        "score" | "stats" => {
            reply(world, actor, score(soul, credits));
            return Ok(true);
        }
        "pecs" => {
            reply(world,actor,format!("Mechanically supported Peculiarities: {}.\nUse learn <name>; XP-earned Combat levels are used first. Other rulebook Peculiarities are not enabled yet.",rules::supported_peculiarities().join(", ")));
            return Ok(true);
        }
        "train" if arg.is_empty() => {
            reply(world,actor,format!("Unspent XP: {}. Trainable attributes: {}.\nUse train <attribute> to buy its next level. Faith cannot be trained.",soul.available_xp(),rules::ATTRIBUTES.join(", ")));
            return Ok(true);
        }
        "stop" => {
            world
                .resource_mut::<Battles>()
                .0
                .retain(|b| b.player != actor);
            combat_end(world, actor, "You stop attacking.", 0);
            return Ok(true);
        }
        "soulheal" | "awardxp" => return (wizard_command(world, actor, verb, arg)).map(|_| true),
        "recover" => return (recover_stun(world, actor, soul.clone())).map(|_| true),
        _ => {}
    }
    Ok(false)
}

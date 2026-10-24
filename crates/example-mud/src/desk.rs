//! Royal Oak quest desk. Progress and loot are saved together per character.
use crate::savage_rules::{self as rules, InjuryCondition, SoulData};
use bevy::prelude::*;
use grim::components::{Description, Keywords, Name as GrimName, RoomDescription};
use grim::{Actor, CarriedBy, Character, InRoom, Object, Room, StoredCharacter, StoredObject};
use serde::{Deserialize, Serialize};

const KEY: &str = "royal_oak_desk";
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    open: bool,
    safe: bool,
    dagger_taken: bool,
    note_taken: bool,
    venom_next: Option<u64>,
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn reply(world: &mut World, actor: Entity, text: impl Into<String>) {
    world.write_message(grim::events::InfoMessage {
        target: actor,
        text: text.into(),
    });
}
fn state(world: &World, actor: Entity) -> Result<State, String> {
    world
        .get::<Character>(actor)
        .ok_or("Character is missing.")?
        .game_state
        .get(KEY)
        .map(|v| {
            serde_json::from_value(v.clone()).map_err(|_| "Your desk progress needs repair.".into())
        })
        .unwrap_or_else(|| Ok(State::default()))
}
fn here(world: &World, actor: Entity) -> bool {
    world
        .get::<InRoom>(actor)
        .and_then(|p| world.get::<Room>(p.room))
        .is_some_and(|r| r.friendly_id == "robarngrillfl104")
}
pub(crate) fn is_desk(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "desk" | "wooden desk" | "drawer" | "the desk" | "the drawer"
    )
}
fn dagger() -> StoredObject {
    StoredObject { condition: None,name:"mist dagger".into(),keywords:vec!["dagger".into(),"mist".into(),"mist dagger".into(),"weapon".into()],description:vec!["The handle of the dagger seems very simple and plain. The blade is made up of a smokey mist that seems to be part of another reality.".into()],room_description:"A mist-bladed dagger lies here.".into()}
}
fn note() -> StoredObject {
    StoredObject {
        condition: None,
        name: "dagger note".into(),
        keywords: vec![
            "note".into(),
            "paper".into(),
            "slip".into(),
            "dagger note".into(),
        ],
        description: vec![include_str!("../../../data/quests/royal-oak/daggernote.txt").into()],
        room_description: "A small note lies here.".into(),
    }
}

/// Write quest state, Soul state and complete pack in one atomic snapshot.
fn save(
    world: &mut World,
    actor: Entity,
    state: &State,
    soul: &SoulData,
    add: &[StoredObject],
) -> Result<(), String> {
    let mut character = world
        .get::<Character>(actor)
        .ok_or("Missing character.")?
        .clone();
    character.game_state.insert(
        KEY.into(),
        serde_json::to_value(state).map_err(|e| e.to_string())?,
    );
    let mut soul = soul.clone();
    if soul.injury == InjuryCondition::CriticallyInjured && soul.critical_since.is_none() {
        soul.critical_since = Some(now());
    }
    character.game_state.insert(
        "savage_soul".into(),
        serde_json::to_value(&soul).map_err(|e| e.to_string())?,
    );
    if !soul.can_move() {
        character.game_state.insert(
            "action_restriction".into(),
            serde_json::Value::String("You cannot act in your current condition.".into()),
        );
    }
    let mut stored = StoredCharacter::from_components(
        world.get::<GrimName>(actor).ok_or("Missing name.")?,
        world.get::<Actor>(actor).ok_or("Missing actor.")?,
        &character,
    );
    stored.inventory = crate::shop::held(world, actor)
        .into_iter()
        .map(|(_, o)| o)
        .chain(add.iter().cloned())
        .collect();
    stored.inventory.sort_by(|a, b| a.name.cmp(&b.name));
    crate::shop::persist(world, &stored)
        .map_err(|e| format!("Could not save desk interaction: {e}. No change applied."))?;
    world.entity_mut(actor).insert((character, soul));
    for o in add {
        world.spawn((
            Object,
            GrimName(o.name.clone()),
            Description(o.description.clone()),
            Keywords(o.keywords.clone()),
            RoomDescription(o.room_description.clone()),
            o.condition.clone().unwrap_or_default(),
            CarriedBy { carrier: actor },
        ));
    }
    Ok(())
}

pub(crate) fn interact(
    world: &mut World,
    actor: Entity,
    verb: &str,
    arg: &str,
) -> Result<(), String> {
    if verb == "read_note" {
        let owns = crate::shop::held(world, actor)
            .iter()
            .any(|(_, o)| o.name == "dagger note");
        if !owns {
            world.write_message(grim::events::EngineCommand {
                client: actor,
                command: grim::events::Command::Look {
                    target: Some(arg.into()),
                },
            });
            return Ok(());
        }
        reply(world, actor, note().description.join("\n"));
        return Ok(());
    }
    if !here(world, actor) {
        if verb == "inspect" {
            world.write_message(grim::events::EngineCommand {
                client: actor,
                command: grim::events::Command::Look {
                    target: Some(arg.into()),
                },
            });
            return Ok(());
        }
        return Err("There is no trapped desk here.".into());
    }
    let mut state = state(world, actor)?;
    let mut soul = world
        .get::<SoulData>(actor)
        .ok_or("Missing Savage Soul record.")?
        .clone();
    if verb == "inspect" {
        inspect_desk(world, actor, arg, &state)?;
        return Ok(());
    }
    if soul.is_incapacitated() {
        return Err("You cannot manipulate the desk in your current condition.".into());
    }
    let mut additions = Vec::new();
    let message = match verb {
        "disarm" => {
            if !is_desk(arg) {
                return Err("Disarm what? Try disarm drawer.".into());
            }
            if state.safe {
                return Err("The desk's trap is already disarmed or spent.".into());
            }
            state.safe = true;
            "You successfully disarm the poison needle trap!".to_string()
        }
        "open_object" | "close_object" => {
            if !is_desk(arg) {
                return Err("That object cannot be opened or closed here.".into());
            }
            let opening = verb == "open_object";
            if state.open == opening {
                return Err(format!(
                    "The desk is already {}.",
                    if opening { "open" } else { "closed" }
                ));
            }
            state.open = opening;
            if opening && !state.safe {
                state.safe = true;
                state.venom_next = Some(now());
                let effect = venom_check(&mut state, &mut soul, || rand::random_range(1..=10));
                format!("You open the desk. You are pricked by a poison needle trap!\n{effect}")
            } else {
                format!(
                    "You {} the desk.{}",
                    if opening { "open" } else { "close" },
                    if opening { " Try look in desk." } else { "" }
                )
            }
        }
        "get_from" => take_from_desk(arg, &mut state, &mut additions)?,
        _ => return Err("Unknown desk interaction.".into()),
    };
    save(world, actor, &state, &soul, &additions)?;
    reply(world, actor, message);
    Ok(())
}

fn venom_check(state: &mut State, soul: &mut SoulData, dice: impl FnMut() -> i32) -> String {
    if soul.trait_level("resistant to poison") >= 2 {
        state.venom_next = None;
        return "Your poison resistance makes the venom harmless.".into();
    }

    let result = rules::resolve_task(soul.attribute("body"), 10, false, dice);
    if result.critical_failure {
        soul.injury = InjuryCondition::Dead;
        state.venom_next = None;
        return "Critical failure resisting the venom. You die.".into();
    }
    if result.success {
        state.venom_next = None;
        return "Your Body attempt resists the venom; it has no further effect.".into();
    }
    rules::apply_wound(soul, 2 * result.margin);
    state.venom_next = if soul.injury.is_dead() {
        None
    } else {
        Some(now() + 5)
    };
    format!("You fail Body versus DF 10. Venom inflicts wound level {}; you are {}. It will test your Body again in five seconds.",2*result.margin,soul.injury.label())
}
pub(crate) fn tick_venom(world: &mut World) {
    let actors: Vec<_> = world
        .query::<(Entity, &Character, &SoulData)>()
        .iter(world)
        .filter(|(_, c, _)| c.game_state.contains_key(KEY))
        .map(|(e, _, s)| (e, s.clone()))
        .collect();
    for (actor, mut soul) in actors {
        let Ok(mut state) = state(world, actor) else {
            continue;
        };
        if !state.venom_next.is_some_and(|t| t <= now()) {
            continue;
        }
        if soul.injury.is_dead() {
            state.venom_next = None;
        } else {
            let message = venom_check(&mut state, &mut soul, || rand::random_range(1..=10));
            if let Err(error) = save(world, actor, &state, &soul, &[]) {
                error!("Venom save failed: {error}");
                continue;
            }
            reply(world, actor, message);
            continue;
        }
        if let Err(error) = save(world, actor, &state, &soul, &[]) {
            error!("Venom save failed: {error}");
        }
    }
}

fn take_from_desk(
    arg: &str,
    state: &mut State,
    additions: &mut Vec<StoredObject>,
) -> Result<String, String> {
    let lower = arg.to_ascii_lowercase();
    let at = lower.rfind(" from ").ok_or("Use get <item> from desk.")?;
    let target = arg.get(..at).ok_or("Invalid item target.")?;
    let container = arg.get(at + 6..).ok_or("Invalid container target.")?;
    if !is_desk(container) {
        return Err("That container is not supported here.".into());
    }
    if !state.open {
        return Err("The desk is closed. Disarm drawer, then open desk.".into());
    }
    let items = [dagger(), note()];
    let mut available = Vec::new();
    if !state.dagger_taken {
        available.push((0, &items[0]));
    }
    if !state.note_taken {
        available.push((1, &items[1]));
    }
    let spec =
        grim::parse_target(target, grim::ParseOptions::ITEM).ok_or("Use get <item> from desk.")?;
    let selected = grim::query(
        &spec,
        available.iter().map(|(id, o)| {
            (
                Entity::from_raw_u32(*id as u32).unwrap(),
                o.name.as_str(),
                o.keywords.as_slice(),
            )
        }),
    );
    if selected.is_empty() {
        return Err("There is no matching item left in the desk.".into());
    }
    for entity in selected {
        let id = available
            .iter()
            .find(|(id, _)| Entity::from_raw_u32(*id as u32) == Some(entity))
            .unwrap()
            .0;
        if id == 0 {
            state.dagger_taken = true;
        } else {
            state.note_taken = true;
        }
        additions.push(items[id].clone());
    }
    Ok(format!(
        "You take {} from the desk.",
        additions
            .iter()
            .map(|o| o.name.as_str())
            .collect::<Vec<_>>()
            .join(" and ")
    ))
}

fn inspect_desk(world: &mut World, actor: Entity, arg: &str, state: &State) -> Result<(), String> {
    let target = arg.trim().strip_prefix("in ").unwrap_or(arg);
    if !is_desk(target) {
        return Err("Look in what? Try look in desk.".into());
    }
    let mut text=format!("Wooden Desk\nThe plain wooden desk has been shoved a few inches from the wall. A drawer runs along its front.\nThe desk is {}. The needle trap is {}.",if state.open{"open"}else{"closed"},if state.safe{"disarmed or spent"}else{"armed"});
    if state.open {
        let mut contents = Vec::new();
        if !state.dagger_taken {
            contents.push("a mist-bladed dagger");
        }
        if !state.note_taken {
            contents.push("a note");
        }
        text.push_str(&format!(
            "\nInside: {}.\nUse get <item> from desk.",
            if contents.is_empty() {
                "nothing".into()
            } else {
                contents.join(", ")
            }
        ));
    } else {
        text.push_str("\nDisarm drawer, then open desk to inspect its contents.");
    }
    reply(world, actor, text);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn venom_resists_wounds_and_kills_on_critical_failure() {
        let mut state = State::default();
        let mut soul = SoulData::default();
        venom_check(&mut state, &mut soul, || 10);
        assert!(state.venom_next.is_none());
        venom_check(&mut state, &mut soul, || 2);
        assert!(state.venom_next.is_some());
        assert_ne!(soul.injury, InjuryCondition::Uninjured);
        venom_check(&mut state, &mut soul, || 1);
        assert_eq!(soul.injury, InjuryCondition::Dead);
    }
}

//! Recovered Royal Oak interactions: mist dagger quest, Sue's Soulboy and Tully.
use crate::savage_rules::SoulData;
use bevy::prelude::*;
use grim::components::{Description, Keywords, Name as GrimName, RoomDescription};
use grim::events::InfoMessage;
use grim::{
    Actor, CarriedBy, Character, Creature, InRoom, Object, Player, Room, StoredCharacter,
    StoredObject,
};
use std::collections::HashMap;
mod dialogue;
pub(crate) use dialogue::ask;

const QUEST: &str = "Savior of Royal Oak";
#[derive(Resource, Default)]
pub(crate) struct Progress {
    visits: HashMap<Entity, Entity>,
    respawn: Vec<Respawn>,
}
struct Respawn {
    at: f64,
    room: Entity,
    actor: Actor,
    name: String,
    description: Vec<String>,
    keywords: Vec<String>,
    room_description: String,
}
fn reply(world: &mut World, actor: Entity, text: impl Into<String>) {
    world.write_message(InfoMessage {
        target: actor,
        text: text.into(),
    });
}

fn key() -> StoredObject {
    StoredObject {
        condition: None,
        name: "metal gate key".into(),
        description: vec!["The key to the Royal Oak Village Gate.".into()],
        keywords: vec![
            "key".into(),
            "gate key".into(),
            "ROGateKey".into(),
            "metal".into(),
        ],
        room_description: "A metal gate key lies here.".into(),
    }
}
fn soulboy() -> StoredObject {
    StoredObject{ condition: None,name:"Soulboy".into(),description:vec!["A personal data pad that displays your character status. Use activate soulboy to view your Savage Soul record and quests.".into()],keywords:vec!["soulboy".into(),"data pad".into(),"pad".into()],room_description:"A Soulboy lies here.".into()}
}

fn save(
    world: &mut World,
    actor: Entity,
    mut character: Character,
    soul: SoulData,
    remove: &[Entity],
    add: &[StoredObject],
) -> Result<(), String> {
    character.game_state.insert(
        "savage_soul".into(),
        serde_json::to_value(&soul).map_err(|e| e.to_string())?,
    );
    character.game_state.insert(
        "equipment_display".into(),
        serde_json::Value::String(format!(
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
    let mut stored = StoredCharacter::from_components(
        world.get::<GrimName>(actor).ok_or("Missing name.")?,
        world.get::<Actor>(actor).ok_or("Missing actor.")?,
        &character,
    );
    stored.inventory = crate::shop::held(world, actor)
        .into_iter()
        .filter(|(e, _)| !remove.contains(e))
        .map(|(_, o)| o)
        .chain(add.iter().cloned())
        .collect();
    stored.inventory.sort_by(|a, b| a.name.cmp(&b.name));
    crate::shop::persist(world, &stored).map_err(|e| {
        format!("Could not save quest progress: {e}. No reward or item change applied.")
    })?;
    world.entity_mut(actor).insert((character, soul));
    for entity in remove {
        world.despawn(*entity);
    }
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

/// `None` lets ordinary NPC combat proceed. This encounter never enters a duel.
pub(crate) fn attack(world: &mut World, actor: Entity, npc: Entity) -> Option<Result<(), String>> {
    if !world
        .get::<GrimName>(npc)
        .is_some_and(|n| n.0.eq_ignore_ascii_case("archwraith"))
    {
        return None;
    }
    let in_marsh = world
        .get::<InRoom>(npc)
        .and_then(|r| world.get::<Room>(r.room))
        .is_some_and(|r| r.friendly_id == "zombiemarsh07");
    if !in_marsh {
        return None;
    }
    Some(defeat(world, actor, npc))
}
fn defeat(world: &mut World, actor: Entity, npc: Entity) -> Result<(), String> {
    let mut character = world
        .get::<Character>(actor)
        .ok_or("Missing character.")?
        .clone();
    let mut quests: Vec<String> = character
        .game_state
        .get("quests")
        .map(|v| serde_json::from_value(v.clone()).map_err(|_| "Your quest record needs repair."))
        .transpose()?
        .unwrap_or_default();
    if quests.iter().any(|q| q == QUEST) {
        return Err(
            "You have already banished the Archwraith and completed Savior of Royal Oak.".into(),
        );
    }
    let mut soul = world
        .get::<SoulData>(actor)
        .ok_or("Missing Savage Soul record.")?
        .clone();
    let pack = crate::shop::held(world, actor);
    let dagger = pack
        .iter()
        .find(|(_, o)| o.name == "mist dagger")
        .map(|(e, _)| *e);
    let Some(dagger) = dagger else {
        return Err("Your ordinary weapons cannot harm the Archwraith. Find the mist dagger described in the inn's note.".into());
    };
    if soul.weapon != "mist dagger" {
        return Err("Sensing the danger of the dagger in your possession, the wraith keeps its distance. Wield dagger before attacking.".into());
    }
    let points = match character.game_state.get("quest_points") {
        None => 0,
        Some(v) => v.as_u64().ok_or("Your quest-point record needs repair.")?,
    };
    character.game_state.insert(
        "quest_points".into(),
        serde_json::json!(points.checked_add(10).ok_or("Quest-point overflow.")?),
    );
    quests.push(QUEST.into());
    character
        .game_state
        .insert("quests".into(), serde_json::json!(quests));
    soul.total_xp = soul.total_xp.checked_add(2000).ok_or("XP overflow.")?;
    soul.weapon.clear();
    let additions = if pack.iter().any(|(_, o)| {
        o.keywords
            .iter()
            .any(|k| k.eq_ignore_ascii_case("ROGateKey"))
    }) {
        Vec::new()
    } else {
        vec![key()]
    };
    let room = world
        .get::<InRoom>(npc)
        .ok_or("The wraith is no longer here.")?
        .room;
    let template = Respawn {
        at: world.resource::<Time>().elapsed_secs_f64() + 60.0,
        room,
        actor: world
            .get::<Actor>(npc)
            .ok_or("Missing wraith data.")?
            .clone(),
        name: world.get::<GrimName>(npc).unwrap().0.clone(),
        description: world
            .get::<Description>(npc)
            .map(|d| d.0.clone())
            .unwrap_or_default(),
        keywords: world
            .get::<Keywords>(npc)
            .map(|k| k.0.clone())
            .unwrap_or_default(),
        room_description: world
            .get::<RoomDescription>(npc)
            .map(|d| d.0.clone())
            .unwrap_or_default(),
    };
    save(world, actor, character, soul, &[dagger], &additions)?;
    let managed = world.get::<crate::population::Member>(npc).is_some();
    world.despawn(npc);
    if !managed {
        world.resource_mut::<Progress>().respawn.push(template);
    }
    if crate::preferences::detail(world, actor) == crate::preferences::Detail::Brief {
        reply(world,actor,"Archwraith defeated. Savior of Royal Oak complete. XP +2000. Quest points +10. Gate key received; mist dagger consumed.");
    } else {
        reply(world,actor,"With the sound of ripping paper, the mist dagger plunges into the wraith, tearing through dimensional walls. With a pop, the wraith disappears along with your dagger.\nYou have completed Savior of Royal Oak! You receive 10 quest points and 2000 XP. Your gate key and quest progress are saved. Return to Royal Oak's south gate with your Soulboy.");
    }
    Ok(())
}

pub(crate) fn activate(world: &mut World, actor: Entity, arg: &str) -> Result<(), String> {
    if !arg.trim().eq_ignore_ascii_case("soulboy") {
        return Err("Use activate soulboy.".into());
    }
    if !crate::shop::held(world, actor)
        .iter()
        .any(|(_, o)| o.name.eq_ignore_ascii_case("soulboy"))
    {
        return Err("You need a Soulboy. Visit Nurse Sue in the medical center hub.".into());
    }
    let character = world.get::<Character>(actor).unwrap();
    let quests = character
        .game_state
        .get("quests")
        .map(|v| v.to_string())
        .unwrap_or_else(|| "none".into());
    let points = character
        .game_state
        .get("quest_points")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    reply(world,actor,format!("Soulboy\nQuests: {quests}\nQuest points: {points}\nUse score, train, pecs and help for your character record."));
    world.write_message(grim::events::EngineCommand {
        client: actor,
        command: grim::events::Command::SavageSoul {
            verb: "score".into(),
            arg: String::new(),
        },
    });
    Ok(())
}

pub(crate) fn tick(world: &mut World) {
    let mut progress = world.remove_resource::<Progress>().unwrap_or_default();
    let now = world.resource::<Time>().elapsed_secs_f64();
    let mut pending = Vec::new();
    for template in progress.respawn.drain(..) {
        if now < template.at {
            pending.push(template);
            continue;
        }
        if world.get::<Room>(template.room).is_some() {
            world.spawn((
                Creature,
                template.actor,
                GrimName(template.name),
                Description(template.description),
                Keywords(template.keywords),
                RoomDescription(template.room_description),
                InRoom {
                    room: template.room,
                },
                SoulData {
                    total_xp: 0,
                    ..Default::default()
                },
            ));
        }
    }
    progress.respawn = pending;
    let visits: Vec<_> = world
        .query_filtered::<(Entity, &InRoom), With<Player>>()
        .iter(world)
        .map(|(e, r)| (e, r.room))
        .collect();
    progress
        .visits
        .retain(|e, _| visits.iter().any(|(v, _)| v == e));
    for (actor, room) in visits {
        if progress.visits.insert(actor, room) == Some(room) {
            continue;
        }
        if !world
            .get::<Room>(room)
            .is_some_and(|r| r.friendly_id == "medical-center-hub")
        {
            continue;
        }
        let nurse = world
            .query_filtered::<(&GrimName, &InRoom, &SoulData), With<Creature>>()
            .iter(world)
            .any(|(n, r, s)| {
                r.room == room && n.0.eq_ignore_ascii_case("Sue Rodgers") && !s.is_incapacitated()
            });
        if !nurse {
            continue;
        }
        let Some(soul) = world.get::<SoulData>(actor).cloned() else {
            continue;
        };
        if soul.is_incapacitated() {
            continue;
        }
        if crate::shop::held(world, actor)
            .iter()
            .any(|(_, o)| o.name.eq_ignore_ascii_case("soulboy"))
        {
            continue;
        }
        let Some(character) = world.get::<Character>(actor).cloned() else {
            continue;
        };
        match save(world,actor,character,soul,&[],&[soulboy()]) {
            Ok(())=>reply(world,actor,"Nurse Sue says, 'You shouldn't be running around without a Soulboy. I have one for you here.'\nSue gives you a Soulboy. Use activate soulboy."),
            Err(error)=>reply(world,actor,error),
        }
    }
    world.insert_resource(progress);
}

#[cfg(test)]
mod trainer_tests {
    use super::*;
    #[test]
    fn conversational_questions_keep_personal_context_and_require_local_npc() {
        let mut world = World::new();
        world.init_resource::<Messages<InfoMessage>>();
        let room = world.spawn_empty().id();
        let actor = world.spawn((SoulData::default(), InRoom { room })).id();
        let npc = world
            .spawn((
                Creature,
                GrimName("tully".into()),
                Keywords(vec!["old man tully".into()]),
                InRoom { room },
            ))
            .id();
        ask(&mut world, actor, "tully how much does melee attack cost?").unwrap();
        assert_eq!(
            world
                .get::<crate::trainer::Conversation>(actor)
                .unwrap()
                .attribute,
            "melee_attack"
        );
        ask(&mut world, actor, "tully can I afford it?").unwrap();
        ask(&mut world, actor, "to old man tully how do I level up?").unwrap();
        let messages = world.resource::<Messages<InfoMessage>>();
        let mut cursor = messages.get_cursor();
        let texts: Vec<_> = cursor
            .read(messages)
            .map(|message| message.text.clone())
            .collect();
        assert!(texts[1].contains("Your base melee attack is"));
        assert!(texts[1].contains("You can afford it"));
        assert!(texts[2].contains("no automatic class-level"));
        assert_eq!(world.get::<SoulData>(actor).unwrap().available_xp(), 600);
        let elsewhere = world.spawn_empty().id();
        world.entity_mut(npc).insert(InRoom { room: elsewhere });
        assert!(ask(&mut world, actor, "tully for help").is_err());
    }
}

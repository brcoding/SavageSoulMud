//! snapshot implementation, split from the parent subsystem.

use super::*;

pub(super) fn visible_exits(world: &World, location: Entity) -> BTreeMap<String, Value> {
    world
        .get::<Exits>(location)
        .into_iter()
        .flat_map(|e| e.exits.iter())
        .filter(|(dir, _)| {
            !world
                .get::<Doors>(location)
                .and_then(|doors| doors.doors.get(dir))
                .is_some_and(|door| door.hidden)
        })
        .filter_map(|(dir, target)| {
            world.get::<Room>(*target).map(|room| {
                (
                    short_direction(&dir.to_string()).into(),
                    json!(mapper_id(world, room.id.as_str()).unwrap_or(0)),
                )
            })
        })
        .collect()
}

pub(super) fn short_direction(dir: &str) -> &str {
    match dir {
        "north" => "n",
        "east" => "e",
        "south" => "s",
        "west" => "w",
        "northeast" => "ne",
        "northwest" => "nw",
        "southeast" => "se",
        "southwest" => "sw",
        "up" => "u",
        "down" => "d",
        _ => dir,
    }
}

pub(super) fn snapshot(world: &World, actor: Entity) -> BTreeMap<String, Value> {
    let Some(character) = world.get::<Character>(actor) else {
        return BTreeMap::new();
    };
    let Some(soul) = world.get::<crate::savage_rules::SoulData>(actor) else {
        return BTreeMap::new();
    };
    let derived = soul.derived();
    let mut data = BTreeMap::new();
    data.insert("Char.Vitals".into(),json!({"injury":soul.injury.label(),"mana":soul.mana,"maxmana":derived.mana_max,"xp":soul.total_xp,"unspentxp":soul.available_xp(),"credits":character.credits,"winded":soul.winded,"fatigued":soul.fatigued,"stunned":soul.stunned}));
    data.insert("Char.Status".into(),json!({"name":world.get::<GrimName>(actor).map(|n|n.0.as_str()).unwrap_or(""),"race":world.get::<Actor>(actor).map(|a|a.race.as_str()).unwrap_or(""),"system":"Savage Soul","build":"XP-based"}));
    let attributes: BTreeMap<_, _> = crate::savage_rules::ATTRIBUTES
        .into_iter()
        .chain(["faith"])
        .map(|name| {
            (
                name,
                json!({"base":soul.base_attribute(name),"effective":soul.attribute(name)}),
            )
        })
        .collect();
    data.insert("Char.Stats".into(),json!({"attributes":attributes,"strength":derived.strength,"passive_defense":derived.passive_defense,"dodge":derived.dodge,"quickness":derived.quickness}));
    if let Some(room_entity) = world.get::<InRoom>(actor).map(|ir| ir.room) {
        if let Some(room) = world.get::<Room>(room_entity) {
            if soul.vision_penalty > 0 {
                data.insert(
                    "Room.Info".into(),
                    json!({"num":-1,"name":"Darkness","area":"","exits":{}}),
                );
            } else {
                data.insert("Room.Info".into(),json!({"num":mapper_id(world,room.id.as_str()).map(i64::from).unwrap_or(-1),"grim_id":room.id.as_str(),"name":room.name,"area":world.get::<Area>(room.area).map(|a|a.name.as_str()).unwrap_or(""),"exits":visible_exits(world,room_entity)}));
            }
        }
    }
    data
}

pub(super) fn variables(data: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    let mut vars = BTreeMap::new();
    for (name, key) in [
        ("INJURY", "injury"),
        ("MANA", "mana"),
        ("MANA_MAX", "maxmana"),
        ("EXPERIENCE", "xp"),
        ("EXPERIENCE_UNSPENT", "unspentxp"),
        ("CREDITS", "credits"),
        ("WINDED", "winded"),
        ("FATIGUED", "fatigued"),
        ("STUNNED", "stunned"),
    ] {
        if let Some(value) = data.get("Char.Vitals").and_then(|v| v.get(key)) {
            vars.insert(name.into(), value.clone());
        }
    }
    for (name, key) in [
        ("ROOM_VNUM", "num"),
        ("ROOM_NAME", "name"),
        ("AREA_NAME", "area"),
        ("ROOM_EXITS", "exits"),
    ] {
        if let Some(value) = data.get("Room.Info").and_then(|v| v.get(key)) {
            vars.insert(name.into(), value.clone());
        }
    }
    if let Some(name) = data.get("Char.Status").and_then(|v| v.get("name")) {
        vars.insert("CHARACTER_NAME".into(), name.clone());
    }
    vars
}

pub(super) fn inventory(world: &mut World, actor: Entity) -> Vec<Value> {
    world
        .query_filtered::<(Entity, &CarriedBy, &GrimName), With<Object>>()
        .iter(world)
        .filter(|(_, held, _)| held.carrier == actor)
        .map(|(entity, _, name)| json!({"id":entity.to_bits().to_string(),"name":name.0}))
        .collect()
}

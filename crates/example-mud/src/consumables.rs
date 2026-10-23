//! Ordinary supplies: consume contents, save the complete inventory first.
use bevy::prelude::*;
use grim::components::{Description, Keywords, Name as GrimName, RoomDescription};
use grim::events::InfoMessage;
use grim::{Actor, CarriedBy, Character, Object, StoredCharacter, StoredObject};

pub(crate) fn consume(world:&mut World, actor:Entity, verb:&str, target:&str)->Result<(),String> {
    let spec=grim::parse_target(target,grim::ParseOptions::BEING)
        .ok_or_else(||format!("Use {verb} <item> you are carrying."))?;
    let pack=crate::shop::held(world,actor);
    let matches=grim::query(&spec,pack.iter().map(|(e,o)|(*e,o.name.as_str(),o.keywords.as_slice())));
    let (entity,item)=pack.iter().find(|(e,_)|Some(e)==matches.first()).ok_or("You are not carrying that item.")?;
    let catalog=crate::crafting::load(world).ok();
    if verb=="eat" && world.get::<crate::savage_rules::SoulData>(actor).is_some_and(|s|s.trait_level("carnivore")>0) {
        let meat=item.keywords.iter().any(|k|k=="food:meat") || catalog.as_ref().is_some_and(|c|c.meals.iter().any(|(id,m)|m.verb=="eat"&&m.diet=="meat"&&item.keywords.iter().any(|k|k==&format!("craft:{id}"))));
        if !crate::species::allows_food(world.get::<crate::savage_rules::SoulData>(actor).unwrap(),if meat{"meat"}else{"mixed"}) {return Err("Your Carnivore trait allows only meat. This food is not marked as a meat-only meal.".into());}
    }
    let catalog_meal=catalog.as_ref().and_then(|c|c.meals.iter().find(|(id,m)|m.verb==verb&&item.keywords.iter().any(|k|k==&format!("craft:{id}"))).map(|(_,m)|m.message.clone()));
    let (replacement,message)=if let Some(message)=catalog_meal {(None,message)} else { match (verb,item.name.to_ascii_lowercase().as_str()) {
        ("drink","water bottle")=>(Some(StoredObject { condition: None,
            name:"empty water bottle".into(),
            description:vec!["An empty drinking-water bottle. The water has been consumed.".into()],
            keywords:vec!["empty".into(),"water".into(),"bottle".into(),"empty bottle".into()],
            room_description:"An empty water bottle lies here.".into(),
        }),"You drink the water. You are left with an empty bottle.".to_string()),
        ("drink","empty water bottle")=>return Err("The bottle is empty.".into()),
        ("eat","ration pack")=>(None,"You open the ration pack and eat the preserved food.".to_string()),
        _=>return Err(format!("You cannot {verb} {}.",item.name)),
    }};
    let mut character=world.get::<Character>(actor).ok_or("Character data is missing.")?.clone();
    let timestamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    character.game_state.insert(if verb=="drink"{"last_drink_at"}else{"last_meal_at"}.into(),serde_json::Value::from(timestamp));
    let mut saved=StoredCharacter::from_components(world.get::<GrimName>(actor).ok_or("Character name is missing.")?,world.get::<Actor>(actor).ok_or("Character data is missing.")?,&character);
    saved.inventory=pack.iter().filter(|(e,_)|e!=entity).map(|(_,o)|o.clone()).chain(replacement.iter().cloned()).collect();
    saved.inventory.sort_by(|a,b|a.name.cmp(&b.name));
    crate::shop::persist(world,&saved).map_err(|e|format!("Could not save consumption: {e}. Your item has not changed."))?;
    world.entity_mut(actor).insert(character);
    world.despawn(*entity);
    if let Some(item)=replacement {
        world.spawn((Object,GrimName(item.name),Description(item.description),Keywords(item.keywords),RoomDescription(item.room_description),item.condition.unwrap_or_default(),CarriedBy{carrier:actor}));
    }
    world.write_message(InfoMessage{target:actor,text:message.into()});
    Ok(())
}

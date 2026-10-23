//! Corpse regeneration and living-patient treatment are separate operations.
use std::collections::HashMap;
use bevy::prelude::*;
use grim::{Actor,Area,Character,Creature,InRoom,Object,Player,Room,StoredCharacter};
use grim::components::{Description,Keywords,Name as GrimName,RoomDescription};
use crate::savage_rules::SoulData;

#[derive(Resource,Default)]
struct Prototypes(HashMap<String,crate::seed::NpcBlueprint>);
#[derive(Component)]
pub(crate) struct Contained {until:f64}
pub(crate) fn register(app:&mut App){app.init_resource::<Prototypes>();app.add_observer(hold_posture);}
fn hold_posture(mut walk:On<grim::AttemptWalk>,characters:Query<&Character>,rooms:Query<&Room>,mut info:MessageWriter<grim::events::InfoMessage>) {
    let seated=characters.get(walk.actor).ok().zip(rooms.get(walk.room).ok()).is_some_and(|(c,r)|c.game_state.contains_key("posture_display")&&c.game_state.get("posture_room").and_then(|v|v.as_str())==Some(r.id.to_string().as_str()));
    if seated {walk.event_mut().denied=true;info.write(grim::events::InfoMessage{target:walk.actor,text:"Stand up first.".into()});}
}
pub(crate) fn remember(world:&mut World,template:&crate::seed::NpcBlueprint) {
    world.init_resource::<Prototypes>();
    world.resource_mut::<Prototypes>().0.entry(template.name.to_ascii_lowercase()).or_insert_with(||template.clone());
}
fn table(surface:&str)->bool {matches!(surface,"table"|"medical table"|"metal table"|"surface"|"the table"|"the medical table")}
fn patient(world:&mut World,actor:Entity)->Result<(Entity,SoulData,Character),String> {
    let room=world.get::<InRoom>(actor).ok_or("You are not in a room.")?.room;
    let here=world.get::<Room>(room).is_some_and(|r|r.friendly_id=="start"&&world.get::<Area>(r.area).is_some_and(|a|a.friendly_id=="savsoul"));
    let present=world.query_filtered::<(&GrimName,&InRoom),Without<Object>>().iter(world).any(|(n,r)|r.room==room&&n.0.eq_ignore_ascii_case("medical table"));
    if !here||!present{return Err("The medical table is in Royal Oak Med Room.".into());}
    if crate::savage_soul::in_combat(world,actor){return Err("Finish combat before using the medical table.".into());}
    let soul=world.get::<SoulData>(actor).cloned().ok_or("Your character record is unavailable.")?;
    if soul.is_incapacitated(){return Err("You cannot use the table in your current condition.".into());}
    if world.query_filtered::<&InRoom,With<Contained>>().iter(world).any(|r|r.room==room){return Err("The table is still processing another patient.".into());}
    let character=world.get::<Character>(actor).cloned().ok_or("Missing character.")?;
    Ok((room,soul,character))
}
fn save(world:&mut World,actor:Entity,mut character:Character,soul:SoulData,remove:Option<Entity>)->Result<(),String> {
    character.game_state.insert("savage_soul".into(),serde_json::to_value(&soul).map_err(|e|e.to_string())?);
    let mut saved=StoredCharacter::from_components(world.get::<GrimName>(actor).ok_or("Missing name.")?,world.get::<Actor>(actor).ok_or("Missing character.")?,&character);
    saved.inventory=crate::shop::held(world,actor).into_iter().filter(|(e,_)|Some(*e)!=remove).map(|(_,o)|o).collect();
    saved.inventory.sort_by(|a,b|a.name.cmp(&b.name));
    crate::shop::persist(world,&saved).map_err(|e|format!("Could not save table treatment: {e}. Nothing has changed."))?;
    world.entity_mut(actor).insert((character,soul));if let Some(item)=remove{world.despawn(item);}Ok(())
}
fn announce(world:&mut World,room:Entity,text:String) {
    let viewers:Vec<_>=world.query_filtered::<(Entity,&InRoom),With<Player>>().iter(world).filter(|(_,r)|r.room==room).map(|(e,_)|e).collect();
    for target in viewers{world.write_message(grim::events::InfoMessage{target,text:text.clone()});}
}
pub(crate) fn place(world:&mut World,actor:Entity,arg:&str)->Result<(),String> {
    let lower=arg.to_ascii_lowercase();let at=lower.rfind(" on ").ok_or("Use put corpse on table.")?;
    let target=arg.get(..at).map(str::trim).ok_or("Name a corpse you are carrying.")?;
    if !table(lower.get(at+4..).unwrap_or("").trim()){return Err("You can place a corpse on the medical table.".into());}
    let (room,soul,character)=patient(world,actor)?;
    let spec=grim::parse_target(target,grim::ParseOptions::BEING).ok_or("Place one corpse on the table.")?;
    let pack=crate::shop::held(world,actor);
    let matches=grim::query(&spec,pack.iter().map(|(e,o)|(*e,o.name.as_str(),o.keywords.as_slice())));
    let (corpse,item)=pack.iter().find(|(e,_)|Some(e)==matches.first()).ok_or("You are not carrying that corpse.")?;
    if !item.name.starts_with("corpse of ")||!item.keywords.iter().any(|k|k=="corpse"){return Err("Only a corpse can be regenerated on the table.".into());}
    let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    if item.condition.as_ref().and_then(|c|c.decay.as_ref()).is_some_and(|d|now>=d.expires_at){return Err("That corpse has decayed too far to use.".into());}
    // Current generated corpses are NPC-only. Match their display identity to
    // a species prototype; this never looks up or revives a player save.
    let reference=item.name.strip_prefix("corpse of ").unwrap().to_string();
    let name=reference.strip_prefix("the ").or_else(||reference.strip_prefix("an ")).or_else(||reference.strip_prefix("a ")).unwrap_or(&reference).to_string();
    let template=world.get_resource::<Prototypes>().and_then(|p|p.0.get(&reference.to_ascii_lowercase()).or_else(||p.0.get(&name.to_ascii_lowercase())).cloned());
    save(world,actor,character,soul,Some(*corpse))?;
    let creature=if let Some(template)=template {
        let e={let mut commands=world.commands();crate::seed::spawn_npc(&mut commands,"medical",&template,room)};world.flush();e
    }else{
        world.spawn((Creature,Actor{race:name.clone(),level:1,gender:grim::Gender::Neutral},GrimName(name.clone()),Description(vec![format!("A regenerated {name}, held in the table's containment field.")]),Keywords(vec![name.clone()]),InRoom{room},SoulData{total_xp:0,..Default::default()})).id()
    };
    let mut restored=world.get::<SoulData>(creature).cloned().unwrap_or_default();restored.reset_conditions();restored.total_xp=0;
    let until=world.resource::<Time>().elapsed_secs_f64()+3.;
    world.entity_mut(creature).remove::<(grim::ScriptTriggers,crate::shop::Merchant,crate::savage_soul::NpcCombatBehavior,crate::population::Member,crate::population::Candidate)>()
        .insert((restored,Contained{until},RoomDescription(format!("{reference} is alive on the medical table, held in a containment field."))));
    announce(world,room,format!("The table regenerates {reference}. A containment field surrounds the creature as a scanner sweeps over it."));Ok(())
}
pub(crate) fn rest(world:&mut World,actor:Entity,verb:&str,arg:&str)->Result<(),String> {
    let lower=arg.trim().to_ascii_lowercase();let surface=lower.strip_prefix("down ").unwrap_or(&lower);let surface=surface.strip_prefix("on ").unwrap_or(surface);
    if !table(surface){return Err(format!("Use {verb} on table."));}
    let (_,mut soul,mut character)=patient(world,actor)?;
    let repair=soul.needs_repair();
    soul.reset_conditions();character.game_state.remove("action_restriction");character.game_state.remove("action_restriction_allow_drop");
    let position=if verb=="sit"{"sitting"}else{"lying"};
    soul.prone=position=="lying";
    let room=world.get::<InRoom>(actor).unwrap().room;
    let room_id=world.get::<Room>(room).unwrap().id.to_string();
    character.game_state.insert("posture".into(),serde_json::json!(position));
    character.game_state.insert("posture_room".into(),serde_json::json!(room_id));
    character.game_state.insert("posture_display".into(),serde_json::json!(format!("{position} on the medical table")));
    if let Some(state)=character.game_state.get_mut("royal_oak_desk").and_then(|v|v.as_object_mut()){state.insert("venom_next".into(),serde_json::Value::Null);}
    save(world,actor,character,soul,None)?;
    let posture=if verb=="sit"{"sit"}else{"lie"};
    world.write_message(grim::events::InfoMessage{target:actor,text:if repair {format!("You {posture} on the medical table. It switches to mechanical repair mode; diagnostic arms repair your damage and service your systems.")}else{format!("You {posture} on the medical table. A warm light restores your wounds, fatigue and mana.")}});Ok(())
}
fn clear_posture(character:&mut Character) {for key in ["posture","posture_room","posture_display"]{character.game_state.remove(key);}}
pub(crate) fn stand(world:&mut World,actor:Entity,arg:&str)->Result<(),String> {
    if !arg.trim().is_empty()&&!arg.trim().eq_ignore_ascii_case("up"){return Err("Use stand or stand up.".into());}
    if crate::savage_soul::in_combat(world,actor){return Err("Finish combat before standing up.".into());}
    let mut soul=world.get::<SoulData>(actor).cloned().ok_or("Your character record is unavailable.")?;
    if soul.is_incapacitated(){return Err("You cannot stand in your current condition.".into());}
    let mut character=world.get::<Character>(actor).cloned().ok_or("Missing character.")?;
    soul.prone=false;clear_posture(&mut character);
    save(world,actor,character,soul,None)?;
    world.write_message(grim::events::InfoMessage{target:actor,text:"You stand up.".into()});Ok(())
}
pub(crate) fn tick(world:&mut World) {
    // Teleports must not leave someone seated on a table in a different room.
    let moved:Vec<_>=world.query::<(Entity,&Character,&InRoom,&SoulData)>().iter(world).filter(|(_,c,r,_)|c.game_state.get("posture_room").and_then(|v|v.as_str()).is_some_and(|id|world.get::<Room>(r.room).is_none_or(|room|room.id.to_string()!=id))).map(|(e,c,_,s)|(e,c.clone(),s.clone())).collect();
    for (actor,mut character,mut soul) in moved {
        if character.game_state.get("posture").and_then(|v|v.as_str())==Some("lying"){soul.prone=false;}
        clear_posture(&mut character);
        if let Err(error)=save(world,actor,character,soul,None){warn!("Posture cleanup: {error}");}
    }
    let now=world.resource::<Time>().elapsed_secs_f64();
    let done:Vec<_>=world.query::<(Entity,&Contained,&InRoom,&GrimName)>().iter(world).filter(|(_,c,_,_)|now>=c.until).map(|(e,_,r,n)|(e,r.room,n.0.clone())).collect();
    for (entity,room,name) in done {
        let label=if name.chars().next().is_some_and(char::is_lowercase)&&!["the ","a ","an "].iter().any(|a|name.starts_with(a)){format!("the {name}")}else{name};
        announce(world,room,format!("The table announces, \"Threat detected.\" A flash of light vaporizes {label}."));world.despawn(entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corpse_regenerates_without_healing_donor_then_vaporizes_and_sitting_heals() {
        let mut w=World::new();w.init_resource::<Time>();w.init_resource::<Messages<grim::events::InfoMessage>>();
        let dir=std::env::temp_dir().join(format!("grim-medical-{}",rand::random::<u64>()));w.insert_resource(grim::plugins::PersistenceConfig{dir:dir.clone()});
        let area=w.spawn(Area{id:grim::GrimId::new(),friendly_id:"savsoul".into(),name:"Savsoul".into()}).id();
        let room=w.spawn(Room{id:grim::GrimId::new(),friendly_id:"start".into(),name:"Royal Oak Med Room".into(),description:String::new(),area}).id();w.spawn((GrimName("medical table".into()),InRoom{room}));
        let stored:StoredCharacter=serde_json::from_value(serde_json::json!({"id":grim::GrimId::new(),"account_id":grim::GrimId::new(),"name":"Patient","created_at":"2026-10-17T00:00:00Z"})).unwrap();
        let connection=w.spawn_empty().id();let actor=w.spawn(stored.into_components()).insert((Player{connection},InRoom{room},SoulData{injury:crate::savage_rules::InjuryCondition::Injured,winded:true,fatigued:true,mana:0,..Default::default()})).id();
        w.spawn((Object,GrimName("corpse of the rat".into()),Keywords(vec!["corpse".into()]),grim::CarriedBy{carrier:actor}));
        place(&mut w,actor,"corpse on table").unwrap();
        assert_eq!(w.get::<SoulData>(actor).unwrap().injury,crate::savage_rules::InjuryCondition::Injured);assert!(w.get::<SoulData>(actor).unwrap().winded);assert_eq!(w.get::<SoulData>(actor).unwrap().mana,0);
        let creature=w.query_filtered::<Entity,With<Contained>>().single(&w).unwrap();assert_eq!(w.get::<GrimName>(creature).unwrap().0,"rat");
        w.resource_mut::<Time>().advance_by(std::time::Duration::from_secs(2));tick(&mut w);assert!(w.get_entity(creature).is_ok());
        w.resource_mut::<Time>().advance_by(std::time::Duration::from_secs(1));tick(&mut w);assert!(w.get_entity(creature).is_err());
        rest(&mut w,actor,"sit","on table").unwrap();let soul=w.get::<SoulData>(actor).unwrap();assert_eq!(soul.injury,crate::savage_rules::InjuryCondition::Uninjured);assert!(!soul.winded&&!soul.fatigued);assert_eq!(soul.mana,soul.derived().mana_max);
        assert_eq!(w.get::<Character>(actor).unwrap().game_state["posture"],"sitting");
        rest(&mut w,actor,"lie","down on table").unwrap();
        assert!(w.get::<SoulData>(actor).unwrap().prone);
        stand(&mut w,actor,"").unwrap();
        assert!(!w.get::<SoulData>(actor).unwrap().prone);assert!(!w.get::<Character>(actor).unwrap().game_state.contains_key("posture"));
        let saved:StoredCharacter=serde_json::from_slice(&std::fs::read(dir.join("characters/Patient.json")).unwrap()).unwrap();assert!(saved.inventory.is_empty());assert_eq!(saved.credits,1000);assert_eq!(saved.game_state["savage_soul"]["total_xp"],600);std::fs::remove_dir_all(dir).unwrap();
    }
}

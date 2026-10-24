//! The authored Morlock optical machine, with one-time character rewards.
use std::collections::HashMap;
use bevy::prelude::*;
use grim::{Actor, Character, InRoom, Room, Player, StoredCharacter, StoredObject, Object, CarriedBy};
use grim::components::{Description, Keywords, Name as GrimName, RoomDescription};
use crate::savage_rules::SoulData;
const QUEST:&str="Master of the Morlock Machine";
const INITIAL:[u8;4]=[6,3,2,0];
const SOLUTION:[u8;4]=[0,2,4,6];
const DIRECTIONS:[&str;8]=["north","northeast","east","southeast","south","southwest","west","northwest"];
#[derive(Clone)]
struct Machine {lenses:[u8;4], closes:Option<f64>, warned:bool, loot:bool, panel:Option<Entity>,hidden:bool,origin:Option<Entity>}
impl Default for Machine {fn default()->Self{Self{lenses:INITIAL,closes:None,warned:false,loot:false,panel:None,hidden:false,origin:None}}}
#[derive(Resource,Default)]
pub(crate) struct Progress(HashMap<Entity,Machine>);
fn reply(world:&mut World,actor:Entity,text:impl Into<String>){world.write_message(grim::events::InfoMessage{target:actor,text:text.into()});}
fn here(world:&World,actor:Entity)->Option<Entity>{let room=world.get::<InRoom>(actor)?.room;world.get::<Room>(room).filter(|r|r.friendly_id=="morlockcavemachine01").map(|_|room)}
fn lens_text(lenses:[u8;4])->String{format!("The buttons rotate four lenses.\n{}",lenses.iter().enumerate().map(|(i,n)|format!("Lens {}: {}.",i+1,DIRECTIONS[*n as usize])).collect::<Vec<_>>().join("\n"))}
fn announce(world:&mut World,room:Entity,text:&str,hidden:bool,origin:Option<Entity>){let players:Vec<_>=world.query_filtered::<(Entity,&InRoom),With<Player>>().iter(world).filter(|(_,r)|r.room==room).map(|(e,_)|e).collect();for player in players{if !hidden||origin==Some(player)||world.get::<Character>(player).is_some_and(Character::is_admin){reply(world,player,text);}}}
fn sync(world:&mut World,room:Entity,state:&Machine){let fixtures:Vec<_>=world.query::<(Entity,&InRoom,&Keywords)>().iter(world).filter(|(_,r,k)|r.room==room&&k.0.iter().any(|s|s=="lenses")).map(|(e,_,_)|e).collect();for e in fixtures{world.entity_mut(e).insert(Description(vec![lens_text(state.lenses)]));}}
fn suit()->StoredObject{StoredObject{condition:Some(Default::default()),name:"nbc suit".into(),keywords:vec!["suit".into(),"nbc".into(),"overalls".into(),"cloth".into()],description:vec!["The NBC suit (Nuclear, Biological, Chemical) is designed to handle most types of radiation and other sources of contamination. It also provides the ability to fight, run and allow high mobility while you are wearing it. The suit is white with a Shelter 1 logo embossed on the front.".into()],room_description:"A white NBC suit lies here.".into()}}
/// Save the complete character and pack before publishing rewards or loot.
fn save(world:&mut World,actor:Entity,mut character:Character,soul:SoulData,add:Option<StoredObject>)->Result<(),String>{
 character.game_state.insert("savage_soul".into(),serde_json::to_value(&soul).map_err(|e|e.to_string())?);
 let mut stored=StoredCharacter::from_components(world.get::<GrimName>(actor).ok_or("Missing name.")?,world.get::<Actor>(actor).ok_or("Missing actor.")?,&character);
 stored.inventory=crate::shop::held(world,actor).into_iter().map(|(_,o)|o).chain(add.iter().cloned()).collect();
 crate::shop::persist(world,&stored).map_err(|e|format!("Could not save machine progress: {e}. No reward or loot change applied."))?;
 world.entity_mut(actor).insert((character,soul));
 if let Some(o)=add{world.spawn((Object,GrimName(o.name),Description(o.description),Keywords(o.keywords),RoomDescription(o.room_description),o.condition.unwrap_or_default(),CarriedBy{carrier:actor}));}
 Ok(())
}
pub(crate) fn press(world:&mut World,actor:Entity,arg:&str)->Result<(),String>{
 let room=here(world,actor).ok_or("There is no optical machine here.")?;
 if world.get::<SoulData>(actor).is_none_or(|s|s.is_incapacitated()){return Err("You cannot operate the machine in your current condition.".into());}
 if crate::savage_soul::in_combat(world,actor){return Err("Finish the fight before adjusting the machine.".into());}
 tick(world);
 let arg=arg.trim().to_ascii_lowercase();let arg=arg.strip_prefix("the ").unwrap_or(&arg);let arg=arg.strip_suffix(" button").unwrap_or(arg);
 let mut state=world.resource::<Progress>().0.get(&room).cloned().unwrap_or_default();
 let changes:&[(usize,u8)]=match arg{"red"=>&[(0,1),(1,3)],"blue"=>&[(2,3)],"green"=>&[(2,3),(3,1)],"yellow"=>&[(3,5),(0,3)],_=>return Err("Press red button, blue button, green button or yellow button.".into())};
 for (lens,increment) in changes{state.lenses[*lens]=(state.lenses[*lens]+increment)%8;}
 let solved=state.lenses==SOLUTION;
 let mut awarded=false;
 if solved{
  let mut character=world.get::<Character>(actor).ok_or("Missing character.")?.clone();
  let mut quests:Vec<String>=character.game_state.get("quests").map(|v|serde_json::from_value(v.clone()).map_err(|_|"Your quest record needs repair.")).transpose()?.unwrap_or_default();
  if !quests.iter().any(|q|q==QUEST){
   let mut soul=world.get::<SoulData>(actor).ok_or("Missing Savage Soul record.")?.clone();
   soul.total_xp=soul.total_xp.checked_add(100).ok_or("XP overflow.")?;
   let points=character.game_state.get("quest_points").map(|v|v.as_u64().ok_or("Your quest-point record needs repair.")).transpose()?.unwrap_or(0);
   character.game_state.insert("quest_points".into(),serde_json::json!(points.checked_add(10).ok_or("Quest-point overflow.")?));
   quests.push(QUEST.into());character.game_state.insert("quests".into(),serde_json::json!(quests));
   save(world,actor,character,soul,None)?;awarded=true;
  }
  if state.panel.is_none(){
   state.panel=Some(world.spawn((GrimName("cave panel".into()),Description(vec!["A small opening in the cave wall behind the panel. Use get suit from panel to take the white NBC suit.".into()]),Keywords(vec!["panel".into(),"opening".into(),"cave panel".into()]),RoomDescription("A small panel stands open in the cave wall.".into()),InRoom{room})).id());state.loot=true;
  }
  state.origin=Some(actor);
  state.hidden=world.get::<Character>(actor).is_some_and(Character::is_hidden);
  state.closes=Some(world.resource::<Time>().elapsed_secs_f64()+120.);state.warned=false;
 }
 sync(world,room,&state);world.resource_mut::<Progress>().0.insert(room,state.clone());
 reply(world,actor,format!("The button clicks.\n{}",lens_text(state.lenses)));
 if solved{announce(world,room,"The beams converge on the cardinal points. A small panel opens in the cave wall.",state.hidden,state.origin);if awarded{reply(world,actor,"Master of the Morlock Machine complete. XP +100. Quest points +10. Saved.");}}
 Ok(())
}
pub(crate) fn get_from(world:&mut World,actor:Entity,arg:&str)->Option<Result<(),String>>{
 let lower=arg.trim().to_ascii_lowercase();let (target,container)=lower.split_once(" from ")?;
 if !matches!(container.trim(),"panel"|"the panel"|"cave panel"|"opening"){return None;}
 Some(take(world,actor,target.trim()))
}
fn take(world:&mut World,actor:Entity,target:&str)->Result<(),String>{
 let room=here(world,actor).ok_or("There is no cave panel here.")?;tick(world);
 let mut state=world.resource::<Progress>().0.get(&room).cloned().unwrap_or_default();
 if state.panel.is_none(){return Err("The panel is closed. Align the lenses to open it.".into());}
 if !matches!(target,"all"|"suit"|"nbc suit"|"white suit"|"white nbc suit"|"overalls"){return Err("The panel holds a white NBC suit.".into());}
 if !state.loot{reply(world,actor,"The panel is empty.");return Ok(());}
 let character=world.get::<Character>(actor).ok_or("Missing character.")?.clone();let soul=world.get::<SoulData>(actor).ok_or("Missing Savage Soul record.")?.clone();
 save(world,actor,character,soul,Some(suit()))?;
 state.loot=false;
 if let Some(panel)=state.panel{world.entity_mut(panel).insert(Description(vec!["The small opening behind the panel is empty.".into()]));}
 world.resource_mut::<Progress>().0.insert(room,state);reply(world,actor,"You take the white NBC suit from the panel.");Ok(())
}
pub(crate) fn tick(world:&mut World){
 let now=world.resource::<Time>().elapsed_secs_f64();let states:Vec<_>=world.resource::<Progress>().0.iter().map(|(r,s)|(*r,s.clone())).collect();
 for (room,mut state) in states{if let Some(deadline)=state.closes{
  if now>=deadline{if let Some(panel)=state.panel{world.despawn(panel);}let hidden=state.hidden;let origin=state.origin;state=Machine::default();sync(world,room,&state);announce(world,room,"The panel silently closes and the lenses rearrange themselves.",hidden,origin);}
  else if now>=deadline-20.&&!state.warned{state.warned=true;announce(world,room,"The panel slowly begins to slide back into place.",state.hidden,state.origin);}
  world.resource_mut::<Progress>().0.insert(room,state);
 }}
}

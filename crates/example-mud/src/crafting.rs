//! Recovered recipes using ordinary objects and complete-pack transactions.
use std::collections::{BTreeMap, HashSet};
use bevy::prelude::*;
use grim::{Actor, Character, CarriedBy, InRoom, Object, StoredCharacter, StoredObject};
use grim::components::{Description, Keywords, Name as GrimName, RoomDescription};
use grim::events::InfoMessage;
use serde::Deserialize;

#[derive(Deserialize, Default)]
pub(crate) struct Catalog {
    #[serde(default)] pub items: BTreeMap<String, StoredObject>,
    #[serde(default)] recipes: Vec<Recipe>,
    #[serde(default)] reductions: Vec<Recipe>,
    #[serde(default)] pub meals: BTreeMap<String, Meal>,
}
#[derive(Deserialize)]
pub(crate) struct Meal {pub verb:String, pub message:String, #[serde(default)] pub diet:String}
#[derive(Deserialize)]
struct Recipe {
    id:String, name:String,
    #[serde(default)] aliases:Vec<String>,
    #[serde(default)] ingredients:BTreeMap<String,usize>,
    #[serde(default)] tools:Vec<String>,
    #[serde(default)] products:BTreeMap<String,usize>,
    #[serde(default)] unavailable:Option<String>,
}
pub(crate) fn load(world:&World)->Result<Catalog,String> {
    let path=world.get_resource::<crate::seed::AreaBlueprintDir>().map(|d|d.0.parent().unwrap_or(&d.0).join("savage-soul/crafting.json")).unwrap_or_else(||"data/savage-soul/crafting.json".into());
    let bytes=std::fs::read(path).map_err(|_|"Crafting recipes are unavailable.".to_string())?;
    serde_json::from_slice(&bytes).map_err(|_|"The crafting catalog needs repair; no items changed.".into())
}
fn normal(s:&str)->String {s.trim().to_ascii_lowercase().replace('-'," ")}
fn tagged(item:&StoredObject,id:&str)->bool {item.keywords.iter().any(|k|k==&format!("craft:{id}"))}
fn named(item:&StoredObject,name:&str)->bool {
    normal(&item.name)==normal(name)||item.keywords.iter().any(|k|normal(k)==normal(name))
}
pub(crate) fn matches_item(catalog:&Catalog,item:&StoredObject,id:&str)->bool {
    if item.keywords.iter().any(|k|k.starts_with("craft:")){return tagged(item,id);}
    catalog.items.get(id).is_some_and(|base|item.name.eq_ignore_ascii_case(&base.name))||named(item,&id.replace('-'," "))
}
pub(crate) fn expired(item:&StoredObject)->bool {
    let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    item.condition.as_ref().and_then(|c|c.decay.as_ref()).is_some_and(|d|d.expires_at<=now)
}
fn display(catalog:&Catalog,id:&str)->String {catalog.items.get(id).map(|i|i.name.clone()).unwrap_or_else(||id.replace('-'," "))}
fn details(catalog:&Catalog,r:&Recipe)->String {
    let list=|values:&BTreeMap<String,usize>|values.iter().map(|(id,n)|format!("{n} {}",display(catalog,id))).collect::<Vec<_>>().join(", ");
    format!("{}\nIngredients: {}\nTools: {}\nProduces: {}{}",r.name,list(&r.ingredients),if r.tools.is_empty(){"none".into()}else{r.tools.iter().map(|id|display(catalog,id)).collect::<Vec<_>>().join(", ")},list(&r.products),r.unavailable.as_ref().map(|reason|format!("\nUnavailable: {reason}")).unwrap_or_default())
}
pub(crate) fn command(world:&mut World,actor:Entity,verb:&str,arg:&str)->Result<(),String> {
    let catalog=load(world)?;
    let pack=crate::shop::held(world,actor);
    if verb=="resources" {
        let text=catalog.items.iter().filter_map(|(id,item)|{let n=pack.iter().filter(|(_,o)|matches_item(&catalog,o,id)).count();(n>0).then(||format!("{n} {}",item.name))}).collect::<Vec<_>>().join("\n");
        return say(world,actor,if text.is_empty(){"You carry no catalogued crafting supplies.".into()}else{format!("Crafting supplies:\n{text}")});
    }
    let entries=if verb=="reduce"||verb=="reductions"{&catalog.reductions}else{&catalog.recipes};
    let listing=verb=="recipes"||verb=="reductions";
    if listing||arg.trim().is_empty() {
        let text=if arg.trim().is_empty(){format!("{}:\n{}\nUse {} <name> for requirements.",if verb=="reductions"{"Reductions"}else{"Recipes"},entries.iter().map(|r|format!("{}{}",r.name,if r.unavailable.is_some(){" (unavailable)"}else{""})).collect::<Vec<_>>().join("\n"),if verb=="reductions"{"reductions"}else{"recipes"})}else{
            let r=entries.iter().find(|r|normal(&r.name)==normal(arg)||normal(&r.id)==normal(arg)||r.aliases.iter().any(|a|normal(a)==normal(arg))).ok_or("No recipe by that name. Use recipes or reductions.")?;
            details(&catalog,r)
        };
        return say(world,actor,text);
    }
    let soul=world.get::<crate::savage_rules::SoulData>(actor).ok_or("Your character record is unavailable.")?;
    if soul.is_incapacitated(){return Err("You cannot craft in your current condition.".into());}
    if crate::savage_soul::in_combat(world,actor){return Err("Finish combat before crafting or reducing items.".into());}
    let spec=grim::parse_target(arg,grim::ParseOptions::BEING).ok_or("Name a recipe or carried item.")?;
    let targets=grim::query(&spec,pack.iter().map(|(e,o)|(*e,o.name.as_str(),o.keywords.as_slice())));
    let target=targets.first().and_then(|entity|pack.iter().find(|(e,_)|e==entity));
    let r=if verb=="reduce" {
        let (_,item)=target.ok_or("You are not carrying that item.")?;
        if item.name.eq_ignore_ascii_case(&soul.weapon)||item.name.eq_ignore_ascii_case(&soul.armor){return Err("Remove or unwield that item before reducing it.".into());}
        entries.iter().find(|r|r.ingredients.keys().any(|id|matches_item(&catalog,item,id))||(!item.keywords.iter().any(|k|k.starts_with("craft:"))&&r.aliases.iter().any(|a|named(item,a)))).ok_or("That item has no recovered reduction recipe.")?
    }else{
        entries.iter().find(|r|normal(&r.name)==normal(arg)||normal(&r.id)==normal(arg)||r.aliases.iter().any(|a|normal(a)==normal(arg))).ok_or("Unknown recipe. Use recipes to see available recipes.")?
    };
    if let Some(reason)=&r.unavailable{return Err(format!("{} is unavailable: {reason}",r.name));}
    if r.ingredients.is_empty()||r.products.is_empty(){return Err("That recipe is incomplete; no items changed.".into());}
    let room=world.get::<InRoom>(actor).ok_or("You are nowhere.")?.room;
    let fixtures:Vec<(String,Vec<String>)>=world.query_filtered::<(&GrimName,Option<&Keywords>,&InRoom),Without<Actor>>().iter(world).filter(|(_,_,p)|p.room==room).map(|(n,k,_)|(n.0.clone(),k.map(|k|k.0.clone()).unwrap_or_default())).collect();
    for tool in &r.tools {
        let available=pack.iter().any(|(_,i)|!expired(i)&&(matches_item(&catalog,i,tool)||named(i,tool)))||fixtures.iter().any(|(name,keys)|normal(name)==normal(tool)||keys.iter().any(|k|k==&format!("craft:{tool}")||normal(k)==normal(tool)));
        if !available{return Err(format!("You need access to {}.",display(&catalog,tool)));}
    }
    let mut removed=HashSet::new();
    for (id,amount) in &r.ingredients {
        if *amount==0||*amount>1000{return Err("That recipe has an invalid quantity.".into());}
        let selected:Vec<Entity>=pack.iter().filter(|(e,i)|!removed.contains(e)&&!expired(i)&&(verb!="reduce"||target.is_some_and(|(t,_)|t==e))&&(matches_item(&catalog,i,id)||(verb=="reduce"&&target.is_some_and(|(target,_)|target==e)&&!i.keywords.iter().any(|k|k.starts_with("craft:"))&&r.aliases.iter().any(|a|named(i,a))))).map(|(e,_)|*e).take(*amount).collect();
        if selected.len()!=*amount{return Err(format!("You need {amount} {}.",display(&catalog,id)));}
        removed.extend(selected);
    }
    let mut outputs=Vec::new();
    for (id,amount) in &r.products {
        if *amount==0||*amount>1000||outputs.len()+amount>1000{return Err("That recipe has an invalid output quantity.".into());}
        let item=catalog.items.get(id).ok_or("That recipe has a missing product; no items changed.")?;
        for _ in 0..*amount {let mut item=item.clone();item.keywords.push(format!("craft:{id}"));outputs.push(item);}
    }
    let mut character=world.get::<Character>(actor).ok_or("Character data is unavailable.")?.clone();
    character.game_state.insert("savage_soul".into(),serde_json::to_value(world.get::<crate::savage_rules::SoulData>(actor).unwrap()).map_err(|_|"Could not save character conditions.")?);
    let mut saved=StoredCharacter::from_components(world.get::<GrimName>(actor).ok_or("Missing character name.")?,world.get::<Actor>(actor).ok_or("Missing character data.")?,&character);
    saved.inventory=pack.iter().filter(|(e,_)|!removed.contains(e)).map(|(_,o)|o.clone()).chain(outputs.iter().cloned()).collect();
    crate::shop::persist(world,&saved).map_err(|_|"Could not save crafting. Your items have not changed.")?;
    world.entity_mut(actor).insert(character);
    for entity in removed {world.despawn(entity);}
    for item in outputs {world.spawn((Object,GrimName(item.name),Description(item.description),Keywords(item.keywords),RoomDescription(item.room_description),item.condition.unwrap_or_default(),CarriedBy{carrier:actor}));}
    say(world,actor,format!("You {} {}. Produced: {}.",if verb=="reduce"{"reduce"}else{"craft"},r.name,r.products.iter().map(|(id,n)|format!("{n} {}",display(&catalog,id))).collect::<Vec<_>>().join(", ")))
}
fn say(world:&mut World,actor:Entity,text:String)->Result<(),String>{world.write_message(InfoMessage{target:actor,text});Ok(())}

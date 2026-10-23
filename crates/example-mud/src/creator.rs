//! Med-bay character rebuilding: persisted drafts, one atomic paid commit.
use bevy::prelude::*;
use grim::{Actor, Area, Character, Gender, GrimId, InRoom, Room, StoredCharacter};
use grim::components::{Description, Name as GrimName};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use crate::{savage_rules::{self as rules,SoulData}, species::{Catalog,Template}};

const DRAFT:&str="character_creator";
const HELP:&str="Med-bay character creator\ncreator races             Available races and costs at your current XP\ncreator race <name>       Select race; restarts draft training\ncreator gender male|female|neutral\ncreator train <attribute> Buy one draft training level\ncreator learn <name>      Buy one draft Peculiarity level\ncreator buyoff <name>     Repay a draft disadvantage\ncreator show              Review your draft and procedure costs\ncreator confirm           Pay and apply the reviewed build\ncreator reset             Discard draft purchases and start again\ncreator cancel            Discard the draft; spend nothing\nYour live character does not change until confirmation.";

#[derive(Clone,Debug,Deserialize,Serialize)]
#[serde(deny_unknown_fields)]
pub struct MedBay {pub area:String,pub room:String}
#[derive(Clone,Debug,Deserialize,Serialize)]
#[serde(default,deny_unknown_fields)]
pub struct Settings {
    pub procedure_credits:u64,
    pub race_change_xp:i64,
    pub race_change_xp_overrides:BTreeMap<String,i64>,
    pub med_bays:Vec<MedBay>,
    pub race_change_materials:BTreeMap<String,BTreeMap<String,usize>>,
}
fn default_materials()->BTreeMap<String,BTreeMap<String,usize>> {
    let body=BTreeMap::from([("sheet-of-steel".into(),6),("steel-rod".into(),4),("rivet".into(),12)]);
    BTreeMap::from([("android".into(),body.clone()),("replicant".into(),body)])
}
impl Default for Settings {
    fn default()->Self {Self{procedure_credits:10000,race_change_xp:400,race_change_xp_overrides:BTreeMap::from([("android".into(),800),("replicant".into(),800)]),med_bays:vec![MedBay{area:"savsoul".into(),room:"start".into()}],race_change_materials:default_materials()}}
}
impl Settings {
    pub fn parse(text:&str)->Result<Self,String> {
        let s:Self=serde_json::from_str(text).map_err(|e|e.to_string())?;
        if s.race_change_xp<0||s.race_change_xp_overrides.values().any(|n|*n<0){return Err("Race-change XP costs cannot be negative.".into())}
        if s.med_bays.is_empty()||s.med_bays.iter().any(|b|b.area.is_empty()||b.room.is_empty()){return Err("Configure at least one med bay with an area and room slug.".into())}
        if s.race_change_materials.iter().any(|(race,parts)|race.is_empty()||parts.iter().any(|(id,n)|id.is_empty()||*n==0||*n>10000)){return Err("Body parts require a race, item ID and quantity from 1 to 10000.".into())}
        Ok(s)
    }
    fn load(world:&World)->Result<Self,String> {
        let path=world.get_resource::<crate::seed::AreaBlueprintDir>().map(|d|d.0.parent().unwrap_or(&d.0).join("savage-soul/creator.json")).unwrap_or_else(||"data/savage-soul/creator.json".into());
        match std::fs::read_to_string(path) {Ok(text)=>Self::parse(&text),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(Self::default()),Err(e)=>Err(format!("Could not read creator settings: {e}"))}
    }
    fn fee(&self,old:&str,new:&str)->i64 {if old==new{0}else{*self.race_change_xp_overrides.get(new).unwrap_or(&self.race_change_xp)}}
    fn materials(&self,old:&str,new:&str)->BTreeMap<String,usize> {if old==new{BTreeMap::new()}else{self.race_change_materials.get(new).cloned().unwrap_or_default()}}
    fn med_bay(&self,world:&World,actor:Entity)->bool {
        world.get::<InRoom>(actor).and_then(|p|world.get::<Room>(p.room)).and_then(|r|world.get::<Area>(r.area).map(|a|(a,r))).is_some_and(|(a,r)|self.med_bays.iter().any(|b|b.area==a.friendly_id&&b.room==r.friendly_id))
    }
}

#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Draft {
    version:u32,
    owner:GrimId,
    original_race:String,
    original_gender:Gender,
    baseline:Value,
    race:String,
    gender:Gender,
    soul:SoulData,
    race_fee_xp:i64,
    procedure_credits:u64,
    materials:BTreeMap<String,usize>,
}
fn fingerprint(soul:&SoulData)->Value {
    // Economic/build fields only: ongoing injuries, mana and equipment are kept
    // live, and copied afresh on commit instead of being restored from a draft.
    json!({"xp":soul.total_xp,"attributes":soul.attribute_levels,"peculiarities":soul.peculiarities,"peculiarity_xp":soul.peculiarity_xp,"fees":soul.rebuild_xp,"modifiers":soul.attribute_modifiers,"size":soul.size,"mass":soul.mass,"species":soul.species})
}
impl Draft {
    fn new(owner:GrimId,original:&Actor,live:&SoulData,template:&Template,settings:&Settings)->Result<Self,String> {
        if live.rebuild_xp<0{return Err("Your recorded race-change spending is invalid; ask a wizard.".into())}
        let fee=settings.fee(&original.race,&template.slug);
        let mut soul=template.build_at_xp(live.total_xp)?;
        soul.rebuild_xp=live.rebuild_xp.checked_add(fee).ok_or("Race-change cost overflow.")?;
        if soul.available_xp()<0{return Err(format!("That build needs {} more XP after the {fee} XP race-change fee and required species traits. Your current character is unchanged.",-soul.available_xp()))}
        Ok(Self{version:1,owner,original_race:original.race.clone(),original_gender:original.gender,baseline:fingerprint(live),race:template.slug.clone(),gender:original.gender,soul,race_fee_xp:fee,procedure_credits:settings.procedure_credits,materials:settings.materials(&original.race,&template.slug)})
    }
    fn check(&self,world:&World,actor:Entity,settings:&Settings)->Result<(),String> {
        let c=world.get::<Character>(actor).ok_or("Character is unavailable.")?;
        let a=world.get::<Actor>(actor).ok_or("Character is unavailable.")?;
        let s=world.get::<SoulData>(actor).ok_or("Character is unavailable.")?;
        if self.version!=1||self.owner!=c.id||self.original_race!=a.race||self.original_gender!=a.gender||self.baseline!=fingerprint(s) {
            return Err("Your XP or live build changed since this draft began. Use creator reset to start a fresh draft; nothing has been charged.".into())
        }
        if self.procedure_credits!=settings.procedure_credits||self.race_fee_xp!=settings.fee(&a.race,&self.race)||self.materials!=settings.materials(&a.race,&self.race){return Err("The procedure price changed. Use creator reset to get a new quote; nothing has been charged.".into())}
        if self.soul.available_xp()<0{return Err("The draft exceeds your available XP.".into())}
        Ok(())
    }
}
fn template(catalog:&Catalog,name:&str)->Result<Template,String> {
    let key=name.trim().to_ascii_lowercase();
    if let Ok(index)=key.parse::<usize>() {return catalog.0.get(index.wrapping_sub(1)).cloned().ok_or("Choose a listed race number.".into())}
    if let Some(t)=catalog.0.iter().find(|t|t.slug==key||t.name.to_ascii_lowercase()==key){return Ok(t.clone())}
    let matches:Vec<_>=catalog.0.iter().filter(|t|t.slug.starts_with(&key)||t.name.to_ascii_lowercase().starts_with(&key)).collect();
    if matches.len()==1 {Ok(matches[0].clone())}else{Err("Choose an unambiguous race name or number from creator races.".into())}
}
fn capture(world:&mut World,actor:Entity)->Result<StoredCharacter,String> {
    let pack=crate::shop::held(world,actor);
    let mut stored=StoredCharacter::from_components(world.get::<GrimName>(actor).ok_or("Missing character name.")?,world.get::<Actor>(actor).ok_or("Missing actor.")?,world.get::<Character>(actor).ok_or("Missing character.")?);
    if let Some(soul)=world.get::<SoulData>(actor){stored.game_state.insert("savage_soul".into(),serde_json::to_value(soul).map_err(|e|e.to_string())?);}
    stored.inventory=pack.into_iter().map(|(_,o)|o).collect();stored.inventory.sort_by(|a,b|a.name.cmp(&b.name));
    Ok(stored)
}
fn save_draft(world:&mut World,actor:Entity,draft:Option<&Draft>)->Result<(),String> {
    let mut stored=capture(world,actor)?;
    if let Some(d)=draft{stored.game_state.insert(DRAFT.into(),serde_json::to_value(d).map_err(|e|e.to_string())?);}else{stored.game_state.remove(DRAFT);}
    crate::shop::persist(world,&stored).map_err(|e|format!("Could not save draft: {e}. Nothing was changed."))?;
    world.get_mut::<Character>(actor).unwrap().game_state=stored.game_state;
    Ok(())
}
fn keep_condition(next:&mut SoulData,live:&SoulData) {
    next.injury=live.injury;next.stunned=live.stunned;next.unconscious=live.unconscious;next.comatose=live.comatose;
    next.prone=live.prone;next.winded=live.winded;next.fatigued=live.fatigued;
    next.critical_since=live.critical_since;next.critical_checks=live.critical_checks;next.stabilized=live.stabilized;
    next.defense_mode=live.defense_mode.clone();next.mana=live.mana.max(0).min(next.derived().mana_max);
}
fn equip(next:&mut SoulData,live:&SoulData,inventory:&[grim::StoredObject],catalog:&crate::savage_soul::EquipmentCatalog)->Vec<String> {
    next.weapon.clear();next.armor.clear();next.armor_bonus=0;next.sharp_armor_bonus=0;next.armor_coverage=0;
    let mut removed=Vec::new();
    for (name,kind) in [(&live.weapon,"weapon"),(&live.armor,"armor")] {
        if name.is_empty(){continue;}
        let gear=catalog.0.iter().find(|g|g.name==*name&&g.kind==kind&&inventory.iter().any(|o|o.name==*name)&&next.size>=g.min_size&&(kind!="armor"||next.size==g.min_size));
        if let Some(g)=gear {if kind=="weapon" {next.weapon=name.clone();}else{next.armor=name.clone();next.armor_bonus=g.armor_bonus;next.sharp_armor_bonus=g.sharp_armor_bonus;next.armor_coverage=g.armor_coverage;}}
        else{removed.push(format!("{name} will be unequipped and kept in your pack."));}
    }
    next.load_kg=inventory.iter().filter_map(|o|catalog.0.iter().find(|g|g.name==o.name).map(|g|g.weight_kg)).sum();
    removed
}
fn backup(world:&World,stored:&StoredCharacter)->Result<(),String> {
    let characters=world.resource::<grim::plugins::PersistenceConfig>().characters_dir();
    let dir=characters.parent().ok_or("Missing save directory.")?.join("rebuild-backups");
    std::fs::create_dir_all(&dir).map_err(|e|format!("Cannot create rebuild backup: {e}"))?;
    #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(&dir,std::fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?;}
    let path=dir.join(format!("{}-{}.json",stored.id,GrimId::new()));
    let mut options=std::fs::OpenOptions::new();options.write(true).create_new(true);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
    let mut file=options.open(path).map_err(|e|format!("Cannot create rebuild backup: {e}"))?;
    use std::io::Write;
    let bytes=serde_json::to_vec_pretty(stored).map_err(|e|e.to_string())?;
    file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|e|format!("Cannot write rebuild backup: {e}"))
}
// Select individual carried instances once; a single item cannot satisfy two requirements.
fn body_parts(world:&mut World,actor:Entity,required:&BTreeMap<String,usize>)->Result<(Vec<Entity>,String),String> {
    if required.is_empty(){return Ok((Vec::new(),"Body parts: none required.".into()))}
    let catalog=crate::crafting::load(world)?;
    let mut pack=crate::shop::held(world,actor);pack.sort_by_key(|(e,_)|e.to_bits());
    let mut selected=Vec::new();let mut lines=Vec::new();
    for (id,count) in required {
        let name=&catalog.items.get(id).ok_or_else(||format!("Unknown body part {id}; ask a wizard to repair creator.json."))?.name;
        let found:Vec<_>=pack.iter().filter(|(e,o)|!selected.contains(e)&&!crate::crafting::expired(o)&&(if o.keywords.iter().any(|k|k.starts_with("craft:")){o.keywords.iter().any(|k|k==&format!("craft:{id}"))}else{o.name.eq_ignore_ascii_case(name)})).map(|(e,_)|*e).collect();
        lines.push(format!("{count} {name} (carrying {}, missing {})",found.len(),count.saturating_sub(found.len())));
        selected.extend(found.into_iter().take(*count));
    }
    Ok((selected,format!("Body parts: {}.",lines.join(", "))))
}
fn apply(world:&mut World,actor:Entity,draft:&Draft,settings:&Settings)->Result<String,String> {
    if !settings.med_bay(world,actor){return Err("The DNA/chassis rewrite can only be performed in a configured med bay. Go to Royal Oak Med Room.".into())}
    if crate::savage_soul::in_combat(world,actor){return Err("Finish combat before the procedure.".into())}
    draft.check(world,actor,settings)?;
    let live=world.get::<SoulData>(actor).cloned().ok_or("Character unavailable.")?;
    if live.is_incapacitated(){return Err("You must be conscious, able to act and free of stun before the procedure.".into())}
    let before=capture(world,actor)?;
    if before.credits<draft.procedure_credits{return Err(format!("The procedure costs {} credits; you have {}. Nothing has been charged.",draft.procedure_credits,before.credits))}
    let (consumed,parts_text)=body_parts(world,actor,&draft.materials)?;
    if consumed.len()!=draft.materials.values().sum::<usize>() {return Err(format!("Missing required body parts. {parts_text} Nothing has been charged."))}
    let inventory=crate::shop::held(world,actor).into_iter().filter(|(e,_)|!consumed.contains(e)).map(|(_,o)|o).collect::<Vec<_>>();
    let mut soul=draft.soul.clone();keep_condition(&mut soul,&live);
    let unequipped=equip(&mut soul,&live,&inventory,world.resource::<crate::savage_soul::EquipmentCatalog>());
    if soul.available_xp()<0{return Err("The procedure would leave a negative XP balance.".into())}
    let mut after=before.clone();after.race=draft.race.clone();after.gender=draft.gender;after.class="savage-soul".into();
    after.credits-=draft.procedure_credits;after.inventory=inventory;
    after.game_state.remove(DRAFT);after.game_state.remove("combat_opponent");
    after.game_state.insert("savage_soul".into(),serde_json::to_value(&soul).map_err(|e|e.to_string())?);
    after.game_state.insert("prompt_maxmana".into(),json!(soul.derived().mana_max));
    after.game_state.insert("equipment_display".into(),json!(format!("Wielding: {}\nWearing: {}\n",if soul.weapon.is_empty(){"unarmed"}else{&soul.weapon},if soul.armor.is_empty(){"none"}else{&soul.armor})));
    for key in ["who","who_race","who_gender"] {after.restrings.remove(key);}after.restrings.insert("who_class".into(),"Soul".into());
    if !soul.can_move(){after.game_state.insert("action_restriction".into(),json!("You cannot act while incapacitated or helpless from your load."));}else{after.game_state.remove("action_restriction");}
    after.game_state.insert("action_restriction_allow_drop".into(),json!(soul.is_helpless()&&!soul.stunned&&!soul.unconscious&&!soul.comatose&&!soul.injury.is_dead()));
    backup(world,&before)?;
    crate::shop::persist(world,&after).map_err(|e|format!("Could not save the procedure: {e}. Your character, XP and credits are unchanged."))?;
    let appearance=soul.species.as_ref().and_then(|s|s.template.as_ref()).and_then(|t|t.appearance.clone()).unwrap_or_else(||"An adventurer of the scorched earth.".into());
    for item in consumed {world.despawn(item);}
    let (_,new_actor,new_character)=after.into_components();
    world.entity_mut(actor).insert((new_actor,new_character,soul.clone(),Description(vec![appearance])));
    crate::savage_soul::after_rebuild(world,actor);
    Ok(format!("The med-bay scanner completes your DNA/chassis rewrite. Race: {}.\nPaid {} credits and {} XP for the race change. Earned XP remains {}; unspent XP: {}. Injuries, mana already spent, belongings apart from required body parts, and quest progress were preserved.\n{}{}",draft.race,draft.procedure_credits,draft.race_fee_xp,soul.total_xp,soul.available_xp(),parts_text,if unequipped.is_empty(){String::new()}else{format!("\n{}",unequipped.join("\n"))}))
}
fn show(world:&mut World,actor:Entity,d:&Draft)->Result<String,String> {
    let live=world.get::<SoulData>(actor).cloned().ok_or("Character unavailable.")?;
    let before=capture(world,actor)?;
    let mut preview=d.soul.clone();keep_condition(&mut preview,&live);
    let gear=equip(&mut preview,&live,&before.inventory,world.resource::<crate::savage_soul::EquipmentCatalog>());
    let values=rules::ATTRIBUTES.iter().map(|a|format!("{} {}",a.replace('_'," "),d.soul.base_attribute(a))).collect::<Vec<_>>();
    let mut text=format!("Character draft: {} / {:?}\nEarned XP {} (kept); live unspent {} -> draft unspent {}.\nRace-change fee: {} XP. Previous committed race-change spending: {} XP.\nProcedure: {} credits (you have {}). Species capacity purchases: {} XP.\n{}\n{} C available. Nothing has been charged.\n",d.race,d.gender,d.soul.total_xp,live.available_xp(),d.soul.available_xp(),d.race_fee_xp,live.rebuild_xp,d.procedure_credits,before.credits,d.soul.peculiarity_xp,values.join(" | "),d.soul.available_combat_levels());
    let (_,parts)=body_parts(world,actor,&d.materials)?;text.push_str(&format!("{parts}\n"));
    if !gear.is_empty(){text.push_str(&format!("{}\n",gear.join("\n")));}
    text.push_str("Use creator race <name>, creator train <attribute>, creator learn <name>, creator gender <gender>.\ncreator confirm pays for and applies this draft; creator cancel spends nothing. Use help creator for all options.");
    Ok(text)
}
fn reply(world:&mut World,actor:Entity,text:String){world.write_message(grim::events::InfoMessage{target:actor,text});}
pub(crate) fn command(world:&mut World,actor:Entity,arg:&str)->Result<(),String> {
    let arg=arg.trim();let (op,rest)=arg.split_once(' ').unwrap_or((arg,""));let op=op.to_ascii_lowercase();let rest=rest.trim();
    if op=="help" {reply(world,actor,HELP.into());return Ok(());}
    if op=="cancel" {save_draft(world,actor,None)?;reply(world,actor,"Draft discarded. No XP or credits were spent.".into());return Ok(());}
    let settings=Settings::load(world)?;
    if !settings.med_bay(world,actor){return Err("Use the character creator in a med bay. Royal Oak Med Room can rewrite DNA and reconfigure synthetic bodies. Nothing has been charged.".into())}
    let c=world.get::<Character>(actor).cloned().ok_or("Character unavailable.")?;
    let a=world.get::<Actor>(actor).cloned().ok_or("Character unavailable.")?;
    let live=world.get::<SoulData>(actor).cloned().ok_or("Character unavailable.")?;
    if op=="races" || (op=="race"&&rest.is_empty()) {
        let templates=world.resource::<Catalog>().0.clone();
        let lines=templates.iter().enumerate().map(|(i,t)|match Draft::new(c.id,&a,&live,t,&settings) {Ok(d)=>format!("{}. {} [{}]: {} XP change fee, {} XP left for training. {}",i+1,t.name,t.slug,d.race_fee_xp,d.soul.available_xp(),t.description),Err(e)=>format!("{}. {} [{}]: {e}",i+1,t.name,t.slug)}).collect::<Vec<_>>().join("\n");
        reply(world,actor,lines);return Ok(());
    }
    let existing=c.game_state.get(DRAFT).cloned();
    if op=="confirm"&&existing.is_none(){return Err("Open creator and review a draft before confirming.".into())}
    let mut d=if op=="reset"||existing.is_none() {
        let base=template(world.resource::<Catalog>(),&a.race).or_else(|_|template(world.resource::<Catalog>(),"human"))?;
        Draft::new(c.id,&a,&live,&base,&settings)?
    }else{serde_json::from_value(existing.unwrap()).map_err(|_|"Draft data is invalid. Use creator reset to start again.")?};
    if op!="reset" {d.check(world,actor,&settings)?;}
    let mut result=String::new();
    match op.as_str() {
        ""|"show"|"stats"|"reset"=>{},
        "race"=>{let selected=template(world.resource::<Catalog>(),rest)?;let gender=d.gender;d=Draft::new(c.id,&a,&live,&selected,&settings)?;d.gender=gender;result="Race selected; uncommitted training has been reset.\n".into();},
        "gender"=>d.gender=match rest.to_ascii_lowercase().as_str(){"male"|"m"=>Gender::Male,"female"|"f"=>Gender::Female,"neutral"|"n"=>Gender::Neutral,_=>return Err("Use creator gender male, female, or neutral.".into())},
        "train"=>result=d.soul.train(rest)?+"\n",
        "learn"=>result=d.soul.learn(rest)?+"\n",
        "buyoff"=>result=crate::species::buy_off(&mut d.soul,rest)?+"\n",
        "confirm"=>{if !rest.is_empty(){return Err("Use exactly creator confirm after reviewing the draft.".into())}let text=apply(world,actor,&d,&settings)?;reply(world,actor,text);return Ok(());},
        _=>return Err(HELP.into()),
    }
    if d.soul.available_xp()<0{return Err("The draft cannot have negative XP.".into())}
    save_draft(world,actor,Some(&d))?;
    let text=show(world,actor,&d)?;reply(world,actor,result+&text);Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    fn fixture()->(World,Entity,std::path::PathBuf) {
        let root=std::env::temp_dir().join(format!("creator-{}",GrimId::new()));
        let mut world=World::new();
        world.insert_resource(grim::plugins::PersistenceConfig{dir:root.clone()});
        world.insert_resource(crate::savage_soul::EquipmentCatalog::default());
        world.insert_resource(Messages::<grim::events::InfoMessage>::default());
        let data=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        world.insert_resource(crate::seed::AreaBlueprintDir(data.join("areas")));
        let catalog=Catalog::parse(include_str!("../../../data/savage-soul/races.json")).unwrap();
        let soul=template(&catalog,"human").unwrap().build_at_xp(4660).unwrap();
        world.insert_resource(catalog);
        let area=world.spawn(Area{id:GrimId::new(),friendly_id:"savsoul".into(),name:"Savsoul".into()}).id();
        let room=world.spawn(Room{id:GrimId::new(),friendly_id:"start".into(),name:"Med bay".into(),description:String::new(),area}).id();
        let c=Character{id:GrimId::new(),account_id:GrimId::new(),credits:20000,created_at:chrono::Utc::now(),last_room:None,roles:vec![grim::Role::Admin],class:"savage-soul".into(),title:Some("Builder".into()),restrings:HashMap::new(),config:HashMap::from([("screenreader".into(),"on".into())]),game_state:HashMap::from([("quest".into(),json!("done")),("prompt_template".into(),json!("custom"))])};
        let actor=world.spawn((c,Actor{race:"human".into(),level:1,gender:Gender::Neutral},soul,GrimName("Testcreator".into()),InRoom{room})).id();
        (world,actor,root)
    }
    fn draft(world:&World,actor:Entity,race:&str)->Draft {
        Draft::new(world.get::<Character>(actor).unwrap().id,world.get::<Actor>(actor).unwrap(),world.get::<SoulData>(actor).unwrap(),&template(world.resource::<Catalog>(),race).unwrap(),&Settings::default()).unwrap()
    }
    fn supplies(world:&mut World,actor:Entity)->Vec<Entity> {
        let catalog=crate::crafting::load(world).unwrap();let mut items=Vec::new();
        for (id,count) in &default_materials()["android"] {
            let item=&catalog.items[id];
            for _ in 0..*count {items.push(world.spawn((grim::Object,GrimName(item.name.clone()),grim::components::Keywords(vec![format!("craft:{id}")]),grim::CarriedBy{carrier:actor})).id());}
        }
        items
    }
    #[test]
    fn budgets_refund_training_keep_earned_xp_and_never_refund_fees() {
        let (mut w,e,root)=fixture();
        {let mut live=w.get_mut::<SoulData>(e).unwrap();live.train("body").unwrap();live.learn("quickness").unwrap();live.rebuild_xp=400;}
        let live=w.get::<SoulData>(e).unwrap();let d=draft(&w,e,"human");
        assert_eq!(d.soul.total_xp,4660);assert_eq!(d.race_fee_xp,0);assert_eq!(d.soul.rebuild_xp,400);assert_eq!(d.soul.available_xp(),4260);assert!(d.soul.attribute_levels.is_empty());assert!(d.soul.peculiarities.is_empty());
        assert!(d.soul.available_xp()>live.available_xp());
        let a=draft(&w,e,"android");assert_eq!(a.soul.rebuild_xp,1200);assert_eq!(a.soul.available_xp(),3460);assert_eq!(a.procedure_credits,10000);assert_eq!(a.materials.values().sum::<usize>(),22);
        let mut poor=SoulData::default();poor.total_xp=0;
        assert!(Draft::new(w.get::<Character>(e).unwrap().id,w.get::<Actor>(e).unwrap(),&poor,&template(w.resource::<Catalog>(),"android").unwrap(),&Settings::default()).is_err());
        let _=std::fs::remove_dir_all(root);
    }
    #[test]
    fn confirm_checks_parts_and_saves_costs_pack_and_preserved_state_together() {
        let (mut w,e,root)=fixture();
        {let mut s=w.get_mut::<SoulData>(e).unwrap();s.injury=rules::InjuryCondition::BarelyInjured;s.mana=2;s.fatigued=true;}
        let d=draft(&w,e,"android");let original=fingerprint(w.get::<SoulData>(e).unwrap());
        assert!(apply(&mut w,e,&d,&Settings::default()).unwrap_err().contains("Missing required body parts"));
        assert_eq!(w.get::<Character>(e).unwrap().credits,20000);assert_eq!(fingerprint(w.get::<SoulData>(e).unwrap()),original);
        let material=supplies(&mut w,e);let keepsake=w.spawn((grim::Object,GrimName("keepsake".into()),grim::CarriedBy{carrier:e})).id();
        save_draft(&mut w,e,Some(&d)).unwrap();
        assert_eq!(w.get::<Character>(e).unwrap().credits,20000);
        apply(&mut w,e,&d,&Settings::default()).unwrap();
        let c=w.get::<Character>(e).unwrap();assert_eq!(c.credits,10000);assert!(c.is_admin());assert_eq!(c.game_state["quest"],json!("done"));assert_eq!(c.game_state["prompt_template"],json!("custom"));assert_eq!(c.config["screenreader"],"on");assert!(!c.game_state.contains_key(DRAFT));
        assert!(material.iter().all(|id|w.get_entity(*id).is_err()));assert!(w.get_entity(keepsake).is_ok());
        let s=w.get::<SoulData>(e).unwrap();assert_eq!(s.total_xp,4660);assert_eq!(s.available_xp(),3860);assert_eq!(s.mana,2);assert!(s.fatigued);assert_eq!(s.injury,rules::InjuryCondition::BarelyInjured);
        let stored:StoredCharacter=serde_json::from_slice(&std::fs::read(root.join("characters/Testcreator.json")).unwrap()).unwrap();assert_eq!(stored.inventory.len(),1);assert_eq!(stored.inventory[0].name,"keepsake");assert_eq!(stored.race,"android");assert_eq!(stored.credits,10000);
        assert_eq!(std::fs::read_dir(root.join("rebuild-backups")).unwrap().count(),1);
        assert!(apply(&mut w,e,&d,&Settings::default()).is_err());
        let _=std::fs::remove_dir_all(root);
    }
    #[test]
    fn stale_quote_wrong_room_and_failed_save_never_consume_anything() {
        let (mut w,e,root)=fixture();let d=draft(&w,e,"android");let items=supplies(&mut w,e);
        let mut price=Settings::default();price.race_change_materials.get_mut("android").unwrap().insert("rivet".into(),13);
        assert!(d.check(&w,e,&price).is_err());
        w.get_mut::<SoulData>(e).unwrap().total_xp+=5;assert!(d.check(&w,e,&Settings::default()).is_err());w.get_mut::<SoulData>(e).unwrap().total_xp-=5;
        let room=w.get::<InRoom>(e).unwrap().room;w.get_mut::<Room>(room).unwrap().friendly_id="outside".into();assert!(apply(&mut w,e,&d,&Settings::default()).is_err());w.get_mut::<Room>(room).unwrap().friendly_id="start".into();
        std::fs::create_dir_all(root.join("characters/Testcreator.json")).unwrap();
        assert!(apply(&mut w,e,&d,&Settings::default()).unwrap_err().contains("Could not save"));
        assert_eq!(w.get::<Character>(e).unwrap().credits,20000);assert_eq!(w.get::<Actor>(e).unwrap().race,"human");assert_eq!(w.get::<SoulData>(e).unwrap().rebuild_xp,0);assert!(items.iter().all(|id|w.get_entity(*id).is_ok()));
        let _=std::fs::remove_dir_all(root);
    }
}

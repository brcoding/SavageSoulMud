//! Savage Soul game integration. See migration/SAVAGE-SOUL-IMPLEMENTATION.md.
use bevy::ecs::message::MessageCursor;
use bevy::prelude::*;
use grim::components::Name as GrimName;
use grim::events::{Command, EngineCommand, InfoMessage};
use grim::{Actor, CarriedBy, Character, Creature, InRoom, Object, Player, StoredCharacter};
use serde::Deserialize;
use crate::preferences::{self,Detail};
use crate::savage_rules::{self as rules, AttackProfile, DamageType, SoulData};

const STATE: &str = "savage_soul";
#[derive(Resource, Default)]
pub(crate) struct Battles(Vec<Battle>);
impl Battles {pub(crate) fn contains(&self,entity:Entity)->bool{self.0.iter().any(|b|b.player==entity||b.npc==entity)}}
struct Battle { player: Entity, npc: Entity, next_round: f64, rounds:u32, trample:bool, lethal:bool }
#[derive(Resource,Default)]
struct Resting(Vec<(Entity,Entity,f64)>);
#[derive(Resource)]
struct Dice;
impl Dice {
    fn roll(&mut self)->i32 {
        rand::random_range(1..=10)
    }
}
#[derive(Clone, Deserialize)]
pub struct Equipment {
    pub name:String,
    pub keywords:Vec<String>,
    pub price:u64,
    pub sell_price:u64,
    pub weight_kg:f64,
    pub kind:String,
    #[serde(default)] pub damage_modifier:i32,
    #[serde(default)] pub attack_bonus:i32,
    #[serde(default)] pub sharp:bool,
    #[serde(default)] pub armor_bonus:i32,
    #[serde(default)] pub sharp_armor_bonus:i32,
    #[serde(default)] pub armor_coverage:i32,
    #[serde(default)] pub min_size:i32,
}
#[derive(Resource, Default)]
pub struct EquipmentCatalog(pub Vec<Equipment>);
/// Authored NPC morale, separate from Savage Soul combat mathematics.
#[derive(Component, Clone, Deserialize, Default)]
#[serde(default)]
pub struct NpcCombatBehavior {
    pub can_be_trampled: bool,
    pub xp_reward: u32,
    pub flee_when_injured: bool,
    pub flee_after_rounds: Option<u32>,
}
pub struct SavageSoulPlugin;
impl Plugin for SavageSoulPlugin {
    fn build(&self, app:&mut App) {
        let dir=app.world().get_resource::<crate::seed::AreaBlueprintDir>().map(|d|d.0.parent().unwrap_or(&d.0).join("savage-soul/items.json")).unwrap_or_else(|| "data/savage-soul/items.json".into());
        let catalog=std::fs::read(&dir).ok().and_then(|b|serde_json::from_slice(&b).ok()).unwrap_or_default();
        preferences::register(app);
        crate::zebedee::register(app);
        crate::grapevine::register(app);
        crate::client_protocols::register(app);
        crate::conditions::register(app);
        crate::population::register(app);
        crate::medical::register(app);
        crate::trainer_ai::register(app);
        app.init_resource::<crate::morlock::Progress>();
        app.add_observer(crate::gate::guard);
        app.insert_resource(Dice).insert_resource(EquipmentCatalog(catalog)).init_resource::<Battles>().init_resource::<Resting>().init_resource::<crate::royal_oak::Progress>()
            .add_systems(Update,(hydrate,handle_commands,tick_battles,tick_conditions,crate::desk::tick_venom,crate::royal_oak::tick,crate::conditions::tick,crate::population::tick,crate::medical::tick,crate::morlock::tick).chain());
    }
}
fn reply(world:&mut World,actor:Entity,text:impl Into<String>) {world.write_message(InfoMessage{target:actor,text:text.into()});}
pub(crate) fn in_combat(world:&World,entity:Entity)->bool {
    world.get_resource::<Battles>().is_some_and(|b|b.contains(entity))
}
fn hydrate(world:&mut World) {
    let people:Vec<_>=world.query_filtered::<(Entity,&Character),Without<SoulData>>().iter(world)
        .map(|(e,c)|(e,c.game_state.get(STATE).cloned())).collect();
    for (e,saved) in people {
        let soul=match saved {Some(v)=>match serde_json::from_value(v) {Ok(s)=>s,Err(error)=>{error!("Invalid Savage Soul state for {e:?}: {error}");continue;}},None=>SoulData::default()};
        if let Some(appearance)=soul.species.as_ref().and_then(|s|s.template.as_ref()).and_then(|t|t.appearance.clone()) {
            world.entity_mut(e).insert(grim::components::Description(vec![appearance]));
        }
        world.entity_mut(e).insert(soul.clone());
        world.get_mut::<Character>(e).unwrap().game_state.insert(STATE.into(),serde_json::to_value(&soul).unwrap());
        let mut character=world.get_mut::<Character>(e).unwrap();
        character.restrings.entry("who_class".into()).or_insert("Soul".into());
        character.game_state.insert("equipment_display".into(),serde_json::Value::String(equipment_display(&soul)));
        if soul.is_incapacitated(){character.game_state.insert("action_restriction".into(),serde_json::Value::String("You cannot act while incapacitated.".into()));}
    }
    let mobs:Vec<_>=world.query_filtered::<Entity,(With<Creature>,Without<SoulData>)>().iter(world).collect();
    for e in mobs {world.entity_mut(e).insert(SoulData{total_xp:0,..Default::default()});}
    crate::species::sync_visibility(world);
    let live: Vec<_> = world.query::<(Entity,&SoulData,&Character)>().iter(world).map(|(e,s,_)|(e,s.clone())).collect();
    for (entity,mut soul) in live {
        let opponent=world.resource::<Battles>().0.iter().find(|b|b.player==entity).and_then(|b|world.get::<GrimName>(b.npc)).map(|n|n.0.clone());
        if let Some(mut character)=world.get_mut::<Character>(entity) {
            character.game_state.insert("prompt_maxmana".into(),serde_json::json!(soul.derived().mana_max));
            if let Some(name)=opponent {character.game_state.insert("combat_opponent".into(),serde_json::json!(name));}
            else {character.game_state.remove("combat_opponent");}
        }

        let pack=crate::shop::held(world,entity);
        let owns=|name:&str|pack.iter().any(|(_,o)|o.name==name);
        let mut changed=false;
        let weight:f64=pack.iter().filter_map(|(_,o)|world.resource::<EquipmentCatalog>().0.iter().find(|g|g.name.eq_ignore_ascii_case(&o.name)).map(|g|g.weight_kg)).sum();
        if (soul.load_kg-weight).abs()>0.0001 {soul.load_kg=weight;changed=true;}
        if !soul.weapon.is_empty()&&!owns(&soul.weapon){soul.weapon.clear();changed=true;}
        if !soul.armor.is_empty()&&!owns(&soul.armor){soul.armor.clear();soul.armor_bonus=0;soul.sharp_armor_bonus=0;soul.armor_coverage=0;changed=true;}
        if changed {if let Err(error)=commit(world,entity,soul,None){error!("Equipment reconciliation: {error}");}}
    }
}
/// Save the complete character snapshot before publishing a character edit.
fn commit(world:&mut World,actor:Entity,mut soul:SoulData,credits:Option<u64>)->Result<(),String> {
    if soul.injury==rules::InjuryCondition::CriticallyInjured && soul.critical_since.is_none() && !soul.stabilized {soul.critical_since=Some(unix_now());}
    if let Some(character)=world.get::<Character>(actor) {
        let mut character=character.clone();
        character.game_state.insert(STATE.into(),serde_json::to_value(&soul).map_err(|e|e.to_string())?);
        character.game_state.insert("equipment_display".into(),serde_json::Value::String(equipment_display(&soul)));
        if !soul.can_move(){character.game_state.insert("action_restriction".into(),serde_json::Value::String("You cannot act while incapacitated or helpless from your load.".into()));}
        else {character.game_state.remove("action_restriction");}
        character.game_state.insert("action_restriction_allow_drop".into(),serde_json::Value::Bool(soul.is_helpless()&&!soul.stunned&&!soul.unconscious&&!soul.comatose&&!soul.injury.is_dead()));
        if let Some(balance)=credits {character.credits=balance;}
        let mut stored=StoredCharacter::from_components(world.get::<GrimName>(actor).ok_or("Missing name")?,world.get::<Actor>(actor).ok_or("Missing actor")?,&character);
        stored.inventory=crate::shop::held(world,actor).into_iter().map(|(_,o)|o).collect();
        stored.inventory.sort_by(|a,b|a.name.cmp(&b.name));
        crate::shop::persist(world,&stored).map_err(|e|format!("Could not save character: {e}. No edit applied."))?;
        world.entity_mut(actor).insert(character);
    }
    if world.get::<Creature>(actor).is_some() && soul.injury.is_dead() {
        leave_npc_remains(world,actor,soul.injury);
        return Ok(());
    }
    world.entity_mut(actor).insert(soul);
    Ok(())
}
/// Death ends the NPC's role. Reuse its entity as an ordinary persisted object;
/// possessions fall beside it because the engine has no container inventory.
fn leave_npc_remains(world:&mut World,entity:Entity,injury:rules::InjuryCondition) {
    use grim::components::{Description,Keywords,RoomDescription};
    if let Some(room)=world.get::<InRoom>(entity).map(|r|r.room) {
        let items:Vec<_>=world.query_filtered::<(Entity,&CarriedBy),With<Object>>().iter(world).filter(|(_,held)|held.carrier==entity).map(|(e,_)|e).collect();
        for item in items {world.entity_mut(item).remove::<CarriedBy>().insert(InRoom{room});}
    }
    if injury==rules::InjuryCondition::Obliterated {world.despawn(entity);return;}
    let name=world.get::<GrimName>(entity).map(|n|n.0.clone()).unwrap_or_else(||"creature".into());
    let reference=if name.chars().next().is_some_and(char::is_lowercase)&&!["a ","an ","the "].iter().any(|a|name.starts_with(a)){format!("the {name}")}else{name.clone()};
    let kind=if injury==rules::InjuryCondition::Destroyed{"remains"}else{"corpse"};
    let condition=crate::conditions::fresh(world);
    let mut keywords=world.get::<Keywords>(entity).map(|k|k.0.clone()).unwrap_or_default();
    keywords.extend(["corpse".into(),"body".into(),"remains".into(),name.clone()]);
    keywords.sort();keywords.dedup();
    world.entity_mut(entity)
        .remove::<(Creature,Actor,SoulData,NpcCombatBehavior,crate::shop::Merchant,grim::ScriptTriggers)>()
        .insert((Object,condition,GrimName(format!("{kind} of {reference}")),Keywords(keywords),
            Description(vec![format!("The {kind} of {reference}.")]),
            RoomDescription(format!("The {kind} of {reference} lies here."))));
}
fn score(soul:&SoulData,credits:u64)->String {
    let d=soul.derived();
    let mut out=format!("Savage Soul\nXP: {} total, {} unspent | Credits: {}\nSize {} | Mass {} | {}{}{}\n",soul.total_xp,soul.available_xp(),credits,soul.size,soul.mass,soul.injury.label(),if soul.stunned {", stunned"}else{""},if soul.unconscious {", unconscious"}else{""});
    for key in rules::ATTRIBUTES {out.push_str(&format!("  {}: {} (level {})\n",key.replace('_'," "),soul.attribute(key),soul.attribute_levels.get(key).unwrap_or(&0)));}
    out.push_str(&format!("  Faith: {} (not trainable)\nStrength {} | Passive Defense {} | Dodge {} | Quickness {}\nMana {}/{} | Base DR {} | Sharp DR {} | Armor DR {} | Armor Sharp DR {}\nWielding: {} | Wearing: {} | Defense: {}\nPeculiarities: {:?}\nCommands: train <attribute>, pecs, learn <peculiarity>, wield <item>, wear <armor>, attack <target>, stop, defend auto|none, task <attribute> <difficulty>.",soul.base_attribute("faith"),d.strength,d.passive_defense,d.dodge,d.quickness,soul.mana,d.mana_max,d.base_dr,d.base_sharp_dr,d.armor_dr,d.armor_sharp_dr,if soul.weapon.is_empty(){"unarmed"}else{&soul.weapon},if soul.armor.is_empty(){"none"}else{&soul.armor},soul.defense_mode,soul.peculiarities));
    if let Some(species)=&soul.species {if let Some(t)=&species.template {out.push_str(&format!("\nRace: {} (v{}); {} C available, {} NC available. Use race for traits.",t.name,t.version,soul.available_combat_levels(),species.spare_nc()));}}
    out.push_str(&format!("\nKnown equipment load: {:.2} kg ({})",soul.load_kg,soul.load_label()));
    out
}
fn equipment_display(soul:&SoulData)->String {format!("Wielding: {}\nWearing: {}\n",if soul.weapon.is_empty(){"unarmed"}else{&soul.weapon},if soul.armor.is_empty(){"none"}else{&soul.armor})}
fn handle_commands(world:&mut World,mut cursor:Local<MessageCursor<EngineCommand>>) {
    let inputs:Vec<_>=cursor.read(world.resource::<Messages<EngineCommand>>()).filter_map(|m|match &m.command {Command::SavageSoul{verb,arg} if !["screenreader","combatlog","prompt","intermud","i3","imud"].contains(&verb.as_str())=>Some((m.client,verb.clone(),arg.clone())),_=>None}).collect();
    for (actor,verb,arg) in inputs {
        if world.get::<Player>(actor).is_none(){continue;}
        if let Err(e)=run_command(world,actor,&verb,&arg) {reply(world,actor,e);}
    }
}
fn run_command(world:&mut World,actor:Entity,verb:&str,arg:&str)->Result<(),String> {
    let mut soul=world.get::<SoulData>(actor).ok_or("Your Savage Soul record could not be loaded.")?.clone();
    let credits=world.get::<Character>(actor).unwrap().credits;
    if soul.vision_penalty>0 && ["inspect","read_note","disarm","hack"].contains(&verb) {return Err("It is too dark for that. Bring a light or use night vision.".into());}

    match verb {
        "zebedee"|"i2"=>{crate::zebedee::command(world,actor,arg);return Ok(());},
        "protocols"=>return crate::client_protocols::command(world,actor,arg),
        "grapevine"|"gv"=>return crate::grapevine::command(world,actor,arg),
        "ask"|"talk"=>return crate::royal_oak::ask(world,actor,arg),
        "press"=>return crate::morlock::press(world,actor,arg),
        "hack"=>return crate::hack::command(world,actor,arg),
        "craft"|"reduce"|"recipes"|"reductions"|"resources"=>return crate::crafting::command(world,actor,verb,arg),
        "spawns"=>{let text=crate::population::status(world,actor,arg)?;reply(world,actor,text);return Ok(());},
        "activate"=>return crate::royal_oak::activate(world,actor,arg),
        "inspect"|"read_note"=>return crate::desk::interact(world,actor,verb,arg),
        "creator"|"rebuild"|"respec"=>return crate::creator::command(world,actor,arg),
        "race"=>{crate::species::command(world,actor,arg);return Ok(());},
        "races"=>{crate::species::command(world,actor,if arg.trim().is_empty(){"list"}else{arg});return Ok(());},
        "score"|"stats"=>{reply(world,actor,score(&soul,credits));return Ok(());},
        "pecs"=>{reply(world,actor,format!("Mechanically supported Peculiarities: {}.\nUse learn <name>; XP-earned Combat levels are used first. Other rulebook Peculiarities are not enabled yet.",rules::supported_peculiarities().join(", ")));return Ok(());},
        "train" if arg.is_empty()=>{reply(world,actor,format!("Unspent XP: {}. Trainable attributes: {}.\nUse train <attribute> to buy its next level. Faith cannot be trained.",soul.available_xp(),rules::ATTRIBUTES.join(", ")));return Ok(());},
        "stop"=>{world.resource_mut::<Battles>().0.retain(|b|b.player!=actor);combat_end(world,actor,"You stop attacking.",0);return Ok(());},
        "soulheal"|"awardxp"=>return wizard_command(world,actor,verb,arg),
        "recover"=>{if world.resource::<Battles>().0.iter().any(|b|b.player==actor){return Err("In combat you automatically recover from stun on your next activation.".into());}if soul.stunned&&!soul.unconscious&&!soul.comatose&&!soul.injury.is_dead(){soul.stunned=false;commit(world,actor,soul,None)?;reply(world,actor,"You use your activation to recover from stun.");return Ok(());}return Err("You are not able to recover from stun right now.".into());},
        _=>{}
    }
    if soul.is_incapacitated(){return Err("You cannot act in your current condition. A wizard can use soulheal.".into());}
    if ["train","learn","buyoff","wield","wear","remove","exchange","rest"].contains(&verb) && world.resource::<Battles>().0.iter().any(|b|b.player==actor) {return Err("Finish combat before changing your build or equipment.".into());}
    if ["open_object","close_object"].contains(&verb) && crate::gate::object_door(world,actor,verb,arg){return Ok(());}
    let message=match verb {
        "put"|"place"=>return crate::medical::place(world,actor,arg),
        "sit"|"lie"|"lay"=>return crate::medical::rest(world,actor,verb,arg),
        "stand"=>return crate::medical::stand(world,actor,arg),
        "get_from"=>return get_from(world,actor,arg),
        "disarm"|"open_object"|"close_object"=>return crate::desk::interact(world,actor,verb,arg),
        "drink"|"eat"=>return crate::consumables::consume(world,actor,verb,arg),
        "train"=>soul.train(arg)?,
        "learn"=>soul.learn(arg)?,
        "buyoff"=>crate::species::buy_off(&mut soul,arg)?,
        "defend"=>{if !["auto","dodge","none"].contains(&arg){return Err("Use defend auto, defend dodge, or defend none.".into());}soul.defense_mode=arg.into();format!("Defense set to {arg}. Active defense discards a Ready token when available.")},
        "wield"|"wear"=>{
            let spec=grim::parse_target(arg,grim::ParseOptions::BEING).ok_or("Name an item you are carrying.")?;
            let pack=crate::shop::held(world,actor);
            let matches=grim::query(&spec,pack.iter().map(|(e,o)|(*e,o.name.as_str(),o.keywords.as_slice())));
            let item=&pack.iter().find(|(e,_)|Some(e)==matches.first()).ok_or("You are not carrying that item.")?.1;
            let gear=world.resource::<EquipmentCatalog>().0.iter().find(|g|g.name.eq_ignore_ascii_case(&item.name)).ok_or("That item has no Savage Soul equipment definition.")?;
            if soul.size<gear.min_size || (gear.kind=="armor" && soul.size!=gear.min_size){return Err(format!("{} requires Size {}{}.",gear.name,gear.min_size,if gear.kind=="weapon"{" or greater"}else{" exactly"}));}
            if verb=="wield" && gear.kind=="weapon" {soul.weapon=item.name.clone();format!("You wield {}.",item.name)}
            else if verb=="wear" && gear.kind=="armor" {soul.armor=item.name.clone();soul.armor_bonus=gear.armor_bonus;soul.sharp_armor_bonus=gear.sharp_armor_bonus;soul.armor_coverage=gear.armor_coverage;format!("You wear {}.",item.name)}
            else {return Err("Use wield for a weapon or wear for armor.".into());}
        },
        "remove"=>{if ["weapon","all"].contains(&arg) {soul.weapon.clear();} if ["armor","all"].contains(&arg){soul.armor.clear();soul.armor_bonus=0;soul.sharp_armor_bonus=0;soul.armor_coverage=0;}if !["weapon","armor","all"].contains(&arg){return Err("Use remove weapon, remove armor, or remove all.".into());}"Equipment removed.".into()},
        "task"=>{let (name,df)=arg.rsplit_once(' ').ok_or("Use task <attribute> <difficulty>.")?;let name=rules::normalize_attribute(name);if !rules::ATTRIBUTES.contains(&name.as_str()) && name!="faith" {return Err("Unknown task attribute.".into());}let df:i32=df.parse().map_err(|_|"Difficulty must be an integer.")?;if !(1..=100).contains(&df){return Err("Difficulty must be between 1 and 100.".into());}let bonus=soul.attribute(&name);let result=rules::resolve_task(bonus,df,false,||world.resource_mut::<Dice>().roll());reply(world,actor,format!("{name} attempt: total {} versus DF {df}; {} margin {}{}.",result.total,if result.success{"success"}else{"failure"},result.margin,if result.critical_failure{" (critical failure)"}else{""}));return Ok(());},
        "attack"|"kill"|"trample"|"stomp"=>return begin_battle(world,actor,arg,["trample","stomp"].contains(&verb),verb=="kill"),
        "rest"=>{if !soul.winded {return Err("You are not winded. Rest does not heal wounds.".into());}if world.resource::<Resting>().0.iter().any(|(e,_,_)|*e==actor){return Err("You are already resting.".into());}let room=world.get::<InRoom>(actor).unwrap().room;let end=world.resource::<Time>().elapsed_secs_f64()+5.0;world.resource_mut::<Resting>().0.push((actor,room,end));reply(world,actor,"You rest for one five-second round, then attempt Will Power at DF 10 to recover from being winded.");return Ok(());},
        "exchange"=>{let xp:i64=arg.parse().map_err(|_|"Use exchange <unspent XP>; 1 XP grants 100 credits.")?;if xp<=0 || xp>soul.available_xp() || soul.attribute_levels.values().any(|l|*l>0) || !soul.peculiarities.is_empty() || soul.species.as_ref().is_some_and(|s|s.cost_c>0||s.cost_nc>0){return Err("Exchange is only available before buying attributes or Peculiarities, and cannot exceed unspent XP.".into());}let balance=credits.checked_add((xp as u64).checked_mul(100).ok_or("Credit overflow")?).ok_or("Credit overflow")?;soul.total_xp-=xp;commit(world,actor,soul,Some(balance))?;reply(world,actor,format!("Exchanged {xp} XP. You have {balance} credits."));return Ok(());},
        _=>return Err("Unknown Savage Soul command.".into())
    };
    commit(world,actor,soul,None)?;reply(world,actor,message);Ok(())
}
/// Route by the requested source before invoking a specialized quest handler.
fn get_from(world:&mut World,actor:Entity,arg:&str)->Result<(),String> {
    if let Some(result)=crate::morlock::get_from(world,actor,arg){return result;}
    let lower=arg.to_ascii_lowercase();
    let at=lower.rfind(" from ").ok_or("Use get <item> from <container>.")?;
    let container=arg.get(at+6..).map(str::trim).filter(|s|!s.is_empty()).ok_or("Name a container.")?;
    if crate::desk::is_desk(container) {return crate::desk::interact(world,actor,"get_from",arg);}
    let room=world.get::<InRoom>(actor).ok_or("You are not in a room.")?.room;
    let spec=grim::parse_target(container,grim::ParseOptions::BEING).ok_or("Name one container.")?;
    let objects:Vec<_>=world.query_filtered::<(Entity,&GrimName,Option<&grim::components::Keywords>,Option<&InRoom>,Option<&CarriedBy>),With<Object>>().iter(world)
        .filter(|(_,_,_,r,h)|r.is_some_and(|r|r.room==room)||h.is_some_and(|h|h.carrier==actor))
        .map(|(e,n,k,_,_)|(e,n.0.clone(),k.map(|k|k.0.clone()).unwrap_or_default())).collect();
    let found=grim::query(&spec,objects.iter().map(|(e,n,k)|(*e,n.as_str(),k.as_slice())));
    let (_,name,keywords)=objects.iter().find(|(e,_,_)|Some(e)==found.first()).ok_or("You don't see that container here.")?;
    if keywords.iter().any(|k|k=="corpse")&&(name.starts_with("corpse of ")||name.starts_with("remains of ")) {
        reply(world,actor,"The corpse is empty.");
    } else {reply(world,actor,format!("{name} is not a container."));}
    Ok(())
}
fn wizard_command(world:&mut World,actor:Entity,verb:&str,arg:&str)->Result<(),String> {
    if !world.get::<Character>(actor).unwrap().is_admin(){return Err("That command requires wizard privileges.".into());}
    let (name,amount)=if verb=="awardxp" {let (n,a)=arg.rsplit_once(' ').ok_or("Use awardxp <character> <amount>.")?;(n,Some(a.parse::<i64>().map_err(|_|"XP must be an integer.")?))}else{(arg,None)};
    let target=if name.is_empty()||name.eq_ignore_ascii_case("self"){actor}else{world.query::<(Entity,&GrimName,&SoulData)>().iter(world).find(|(_,n,_)|n.0.eq_ignore_ascii_case(name)).map(|(e,_,_)|e).ok_or("That character is not loaded.")?};
    let mut soul=world.get::<SoulData>(target).unwrap().clone();
    if let Some(xp)=amount {if !(1..=1000000).contains(&xp){return Err("Award between 1 and 1,000,000 XP.".into());}soul.total_xp=soul.total_xp.checked_add(xp).ok_or("XP overflow")?;}else{soul.reset_conditions();if let Some(mut character)=world.get_mut::<Character>(target){if let Some(state)=character.game_state.get_mut("royal_oak_desk"){state["venom_next"]=serde_json::Value::Null;}}world.resource_mut::<Battles>().0.retain(|b|b.player!=target&&b.npc!=target);}
    commit(world,target,soul,None)?;
    if verb=="soulheal"&&world.get::<Creature>(target).is_some(){
        let name=world.get::<GrimName>(target).map(|n|n.0.clone()).unwrap_or_else(||"A creature".into());
        world.entity_mut(target).insert(grim::components::RoomDescription(format!("{name} is here.")));
    }
    reply(world,actor,if verb=="awardxp"{"XP awarded and saved."}else{"Conditions restored and saved."});Ok(())
}
fn begin_battle(world:&mut World,actor:Entity,target:&str,force_trample:bool,lethal:bool)->Result<(),String> {
    if world.get::<Character>(actor).is_some_and(|c|c.game_state.contains_key("posture_display")){return Err("Stand up before attacking.".into());}
    if world.resource::<Battles>().0.iter().any(|b|b.player==actor){return Err("You are already attacking. Use stop to end combat.".into());}
    let room=world.get::<InRoom>(actor).ok_or("You are not in a room.")?.room;
    let spec=grim::parse_target(target,grim::ParseOptions::BEING).ok_or("Use attack <creature>.")?;
    let candidates:Vec<_>=world.query_filtered::<(Entity,&GrimName,&InRoom,Option<&grim::components::Keywords>),With<Creature>>().iter(world).filter(|(_,_,r,_)|r.room==room).map(|(e,n,_,k)|(e,n.0.clone(),k.map(|k|k.0.clone()).unwrap_or_default())).collect();
    let matches=grim::query(&spec,candidates.iter().map(|(e,n,k)|(*e,n.as_str(),k.as_slice())));
    let npc=*matches.first().ok_or("That creature is not here.")?;
    if world.get::<crate::medical::Contained>(npc).is_some(){return Err("The table's containment field prevents you from attacking it.".into());}
    if let Some(result)=crate::royal_oak::attack(world,actor,npc){return result;}
    if world.get::<SoulData>(npc).unwrap().is_incapacitated() && (!lethal || world.get::<SoulData>(npc).unwrap().injury.is_dead()){return Err("That creature is already incapacitated.".into());}
    if world.resource::<Battles>().0.iter().any(|b|b.npc==npc){return Err("That creature is already in a duel.".into());}
    let trample=world.get::<NpcCombatBehavior>(npc).is_some_and(|b|b.can_be_trampled)
        && world.get::<SoulData>(actor).zip(world.get::<SoulData>(npc)).is_some_and(|(a,d)|!a.prone && a.size>d.size+2);
    if force_trample && !trample {return Err("Trampling requires a grounded target more than two sizes smaller than you.".into());}
    let now=world.resource::<Time>().elapsed_secs_f64();
    world.resource_mut::<Battles>().0.push(Battle{player:actor,npc,next_round:now,rounds:0,trample,lethal});
    let name=world.get::<GrimName>(npc).unwrap().0.clone();
    world.get_mut::<Character>(actor).unwrap().game_state.insert("combat_opponent".into(),serde_json::json!(name));
    reply(world,actor,format!("Combat begins: {name}{}.",if trample {" (trample)"}else{""}));Ok(())
}
#[derive(Component)]
struct RewardClaimed;
fn combat_end(world:&mut World,actor:Entity,text:&str,xp:u32) {
    if let Some(mut c)=world.get_mut::<Character>(actor){c.game_state.remove("combat_opponent");}
    reply(world,actor,format!("{text} XP +{xp}."));
}
/// Describe the resolved defense without adding rolls or changing activations.
/// Active defense is Dodge; passive armed guard may be narrated as a parry/block.
fn avoided_strike(outcome:&rules::AttackOutcome,defender:&SoulData,pc_attacks:bool,name:&str,variant:usize)->String {
    let mut chars=name.chars();
    let capital=chars.next().map(|c|c.to_uppercase().collect::<String>()+chars.as_str()).unwrap_or_else(||"Opponent".into());
    let (subject,possessive)=if pc_attacks{(capital, "your".to_string())}else{("You".into(),format!("{name}'s"))};
    let verb=|you:&'static str,other:&'static str|if pc_attacks{other}else{you};
    if outcome.attack_total==0 {
        let attack=if pc_attacks{"Your strike".into()}else{format!("{}'s strike",subject_for_opponent(name))};
        return match variant%3 {0=>format!("{attack} goes wide."),1=>format!("{attack} connects with thin air."),_=>format!("{attack} misses the mark.")};
    }
    if defender.is_incapacitated() {
        return if pc_attacks{format!("Your strike falls short of {name}.")}else{format!("{}'s strike falls short of you.",subject_for_opponent(name))};
    }
    if outcome.defense_used {
        if defender.prone{return format!("{subject} {} clear of {possessive} strike.",verb("roll","rolls"));}
        return match variant%3 {
            0=>format!("{subject} {} {possessive} strike.",verb("sidestep","sidesteps")),
            1=>format!("{subject} {} {possessive} strike.",verb("dodge","dodges")),
            _=>format!("{subject} {} back from {possessive} strike.",verb("spring","springs")),
        };
    }
    if !defender.weapon.is_empty() && !defender.prone && outcome.defense_total-outcome.attack_total<=3 {
        let technique=match variant%3 {0=>verb("parry","parries"),1=>verb("block","blocks"),_=>verb("deflect","deflects")};
        return format!("{subject} {technique} {possessive} strike with {}.",defender.weapon);
    }
    if defender.prone{return format!("{possessive_cap} strike passes over {object}.",possessive_cap=if pc_attacks{"Your".into()}else{format!("{}'s",subject_for_opponent(name))},object=if pc_attacks{name}else{"you"});}
    match variant%3 {
        0=>format!("{subject} {} out of reach.",verb("step","steps")),
        1=>format!("{subject} {} aside from {possessive} strike.",verb("slip","slips")),
        _=>format!("{subject} {} under {possessive} strike.",verb("duck","ducks")),
    }
}
fn subject_for_opponent(name:&str)->String {let mut chars=name.chars();chars.next().map(|c|c.to_uppercase().collect::<String>()+chars.as_str()).unwrap_or_else(||"Opponent".into())}
fn tick_battles(world:&mut World) {
    let now=world.resource::<Time>().elapsed_secs_f64();
    let mut battles=world.remove_resource::<Battles>().unwrap_or_default();
    battles.0.retain_mut(|battle|{
        if world.get::<Player>(battle.player).is_none(){return false;}
        let same=world.get::<InRoom>(battle.player).zip(world.get::<InRoom>(battle.npc)).is_some_and(|(a,b)|a.room==b.room);
        if !same {combat_end(world,battle.player,"Combat ends: out of reach.",0);return false;}
        if now<battle.next_round{return true;}
        let Some(mut pc)=world.get::<SoulData>(battle.player).cloned()else{return false;};
        let Some(mut npc)=world.get::<SoulData>(battle.npc).cloned()else{return false;};
        let name=world.get::<GrimName>(battle.npc).map(|n|n.0.clone()).unwrap_or_else(||"opponent".into());
        let mut chars=name.chars();
        let capital=chars.next().map(|c|c.to_uppercase().collect::<String>()+chars.as_str()).unwrap_or_default();
        if battle_finished(&pc,&npc,battle.lethal){combat_end(world,battle.player,"Combat ends: incapacitated.",0);return false;}
        let detail=preferences::detail(world,battle.player);
        let mut lines=Vec::new();
        let leader_a=rules::resolve_task(pc.attribute("leadership"),0,false,||world.resource_mut::<Dice>().roll());
        let leader_b=rules::resolve_task(npc.attribute("leadership"),0,false,||world.resource_mut::<Dice>().roll());
        let (a,b)=(leader_a.total,leader_b.total);
        let first_pc=if hard_incapacitated(&npc){true}else if leader_a.critical_failure!=leader_b.critical_failure{!leader_a.critical_failure}else if a==b{world.resource_mut::<Dice>().roll()>=6}else{a>b};
        if detail==Detail::Detailed {lines.push(format!("Initiative: you {a}, {name} {b}; {} first.",if first_pc{"you"}else{&name}));}
        let behavior=world.get::<NpcCombatBehavior>(battle.npc).cloned().unwrap_or_default();
        let (mut pc_ready,mut npc_ready)=(true,true);
        // Trample is movement: a ready target may spend its activation on AMC
        // before damage/knockdown (rulebook 5.5.8.1 and 5.8.4). Otherwise a
        // slower creature spends every activation diving and never strikes.
        let counter=first_pc&&battle.trample&&!pc.is_incapacitated()&&!pc.prone&&pc.size>npc.size+2&&!npc.is_incapacitated();
        let turns=if counter{vec![(false,true),(true,false),(false,false)]}else{vec![(first_pc,false),(!first_pc,false)]};
        for (pc_attacks,moving_reaction) in turns {
            if battle_finished(&pc,&npc,battle.lethal){break;}
            if !pc_attacks&&hard_incapacitated(&npc){continue;}
            if (pc_attacks&&!pc_ready)||(!pc_attacks&&!npc_ready){continue;}
            if pc_attacks&&pc.stunned {pc.stunned=false;pc_ready=false;lines.push("You recover from stun.".into());continue;}
            if !pc_attacks&&npc.stunned {npc.stunned=false;npc_ready=false;lines.push(format!("{capital} recovers from stun."));continue;}
            let (attacker,defender)=if pc_attacks{(&pc,&mut npc)}else{(&npc,&mut pc)};
            let entity=if pc_attacks{battle.player}else{battle.npc};
            let profile=profile(world,entity,attacker);
            let active=defender.defense_mode!="none";
            let critical_defender=defender.injury==rules::InjuryCondition::CriticallyInjured&&!defender.stabilized;
            let trample=pc_attacks&&battle.trample&&!attacker.prone&&attacker.size>defender.size+2;
            let outcome=if trample{rules::resolve_trample(attacker,defender,||world.resource_mut::<Dice>().roll())}else{rules::resolve_attack(attacker,defender,profile,active,||world.resource_mut::<Dice>().roll())};
            if outcome.defense_used&&critical_defender {critical_action(world,defender);}
            // Active defense against AMC does not cancel the declared moving
            // action; stun/incapacitation can still interrupt it normally.
            if pc_attacks{pc_ready=false;if outcome.defense_used{npc_ready=false;}}else{npc_ready=false;if outcome.defense_used&&!moving_reaction{pc_ready=false;}}
            let actor=if pc_attacks{"You".into()}else{capital.clone()};
            let target=if pc_attacks{name.clone()}else{"you".into()};
            let avoidance=if !outcome.hit {
                let text=if trample{format!("{capital} dodges your trample.")}else{avoided_strike(&outcome,defender,pc_attacks,&name,battle.rounds as usize+usize::from(pc_attacks))};
                if moving_reaction {format!("{capital} counterattacks. {text}")}else{text}
            }else{String::new()};
            let impact=if outcome.hit {crate::combat_prose::landed(&outcome,defender,crate::combat_prose::Strike {
                player:pc_attacks,name:&name,weapon:&attacker.weapon,profile,trample,counter:moving_reaction,variant:battle.rounds as usize+usize::from(pc_attacks),
            })}else{String::new()};
            if detail==Detail::Detailed {
                let result=if outcome.hit{format!("SM {}, damage {}, wound {}",outcome.success_margin,outcome.total_damage,outcome.wound_level)}else{avoidance.clone()};
                lines.push(if !outcome.hit {format!("{avoidance} (attack {} vs defense {}).",outcome.attack_total,outcome.defense_total)}else{format!("{impact} (attack {} vs defense {}; {result}).",outcome.attack_total,outcome.defense_total)});
            } else if !defender.injury.is_dead() {
                if outcome.hit && detail==Detail::Normal {lines.push(impact);}
                else if outcome.hit {
                    if outcome.wound_level>0 {lines.push(format!("{actor} {} {target}: {}{}.",if trample{"trample"}else if moving_reaction{"counterattacks"}else if pc_attacks{"hit"}else{"hits"},defender.injury.label().to_ascii_lowercase(),if defender.stunned{", stunned"}else{""}));}
                    else {lines.push(format!("{actor} {} {target}: no wound.",if trample{"trample"}else if moving_reaction{"counterattacks"}else if pc_attacks{"hit"}else{"hits"}));}
                } else {lines.push(avoidance);}
            }
            if pc_attacks{critical_action(world,&mut pc);}else{critical_action(world,&mut npc);}
            if behavior.flee_when_injured&&!npc.stunned&&npc.injury!=rules::InjuryCondition::Uninjured{break;}
        }
        battle.rounds+=1;
        if battle.rounds%12==0 {
            for (soul,label) in [(&mut pc,"You"),(&mut npc,capital.as_str())] {
                let roll=rules::resolve_task(soul.attribute("will_power"),10,false,||world.resource_mut::<Dice>().roll());
                if !roll.success&&!soul.winded {soul.winded=true;lines.push(format!("{label} {} winded.",if label=="You"{"are"}else{"is"}));}
            }
        }
        let opponent_dead=npc.injury.is_dead();
        let finished=battle_finished(&pc,&npc,battle.lethal);
        let victory=finished&&hard_incapacitated(&npc);
        let mut earned=if victory&&world.get::<RewardClaimed>(battle.npc).is_none(){behavior.xp_reward.min(1_000_000)}else{0};
        if let Some(total)=pc.total_xp.checked_add(i64::from(earned)){pc.total_xp=total;}else{warn!("Combat XP overflow");earned=0;}
        if pc.injury.is_dead(){pc.stunned=false;}
        if npc.injury.is_dead(){npc.stunned=false;}
        let flee=!finished&&!npc.stunned&&((behavior.flee_when_injured&&npc.injury!=rules::InjuryCondition::Uninjured)||behavior.flee_after_rounds.is_some_and(|limit|battle.rounds>=limit.max(1)));
        if let Err(error)=commit(world,battle.player,pc,None){error!("Combat save failed: {error}");combat_end(world,battle.player,"Combat stopped: save failed.",0);return false;}
        if victory{world.entity_mut(battle.npc).insert(RewardClaimed);}
        if let Err(error)=commit(world,battle.npc,npc,None){error!("NPC state update: {error}");}
        if finished {
            let end=if opponent_dead{format!("{capital} killed.")}else if victory{format!("{capital} incapacitated.")}else{"Combat ends: you are incapacitated.".into()};
            lines.push(format!("{end} XP +{earned}."));
            if let Some(mut c)=world.get_mut::<Character>(battle.player){c.game_state.remove("combat_opponent");}
        }
        if !lines.is_empty(){reply(world,battle.player,lines.join(if detail==Detail::Brief{" "}else{"\n"}));}
        if finished{return false;}
        if flee{retreat_npc(world,battle.player,battle.npc);return false;}
        battle.next_round=now+5.0;true
    });
    world.insert_resource(battles);
}
fn retreat_npc(world:&mut World,player:Entity,npc:Entity) {
    if let Some(mut c)=world.get_mut::<Character>(player){c.game_state.remove("combat_opponent");}
    let name=world.get::<GrimName>(npc).map(|n|n.0.clone()).unwrap_or_else(||"creature".into());
    let Some(room)=world.get::<InRoom>(npc).map(|r|r.room) else {return;};
    let mut exits:Vec<_>=world.get::<grim::Exits>(room).map(|e|e.exits.iter().map(|(d,r)|(*d,*r)).collect()).unwrap_or_default();
    exits.retain(|(d,to)|world.get::<grim::Room>(*to).is_some() && !world.get::<grim::Doors>(room).and_then(|doors|doors.doors.get(d)).is_some_and(|door|!door.open));
    exits.sort_by_key(|(d,_)|d.to_string());
    if let Some((direction,to))=exits.first() {
        world.entity_mut(npc).insert(InRoom{room:*to});
        reply(world,player,format!("The {name} flees {direction}. Combat ends. XP +0."));
    } else {reply(world,player,format!("The {name} backs away. Combat ends. XP +0."));}
}
fn battle_finished(pc:&SoulData,npc:&SoulData,lethal:bool)->bool {hard_incapacitated(pc)||if lethal {npc.injury.is_dead()}else{hard_incapacitated(npc)}}
fn hard_incapacitated(soul:&SoulData)->bool {soul.injury.is_dead()||soul.unconscious||soul.comatose||soul.is_helpless()}
fn critical_action(world:&mut World,soul:&mut SoulData) {
    if soul.injury==rules::InjuryCondition::CriticallyInjured&&!soul.stabilized {
        let roll=rules::resolve_task(soul.attribute("will_power"),10,false,||world.resource_mut::<Dice>().roll());
        if !roll.success {soul.unconscious=true;}
    }
}
fn unix_now()->u64 {std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()}
fn tick_conditions(world:&mut World) {
    let time=world.resource::<Time>().elapsed_secs_f64();
    let mut resting=world.remove_resource::<Resting>().unwrap_or_default();
    resting.0.retain(|(e,room,end)|{
        if world.get::<InRoom>(*e).is_none_or(|r|r.room!=*room){return false;}
        if time<*end{return true;}
        if let Some(mut soul)=world.get::<SoulData>(*e).cloned(){let result=rules::resolve_task(soul.attribute("will_power"),10,false,||world.resource_mut::<Dice>().roll());if result.success{soul.winded=false;}if let Err(error)=commit(world,*e,soul,None){error!("Rest save: {error}");}reply(world,*e,if result.success{"You recover from being winded."}else{"Your Will Power attempt fails; you remain winded."});}
        false
    });world.insert_resource(resting);
    let now=unix_now();
    let critical:Vec<_>=world.query::<(Entity,&SoulData)>().iter(world).filter(|(_,s)|s.injury==rules::InjuryCondition::CriticallyInjured&&!s.stabilized).map(|(e,s)|(e,s.clone())).collect();
    for (e,mut soul) in critical {
        let mut changed=soul.critical_since.is_none();
        let start=*soul.critical_since.get_or_insert(now);
        if soul.hard_to_kill() {
            if now.saturating_sub(start)>=57600 {soul.stabilized=true;changed=true;reply(world,e,"Your critical injuries stabilize.");}
            if changed {if let Err(error)=commit(world,e,soul,None){error!("Critical injury save: {error}");}}
            continue;
        }
        let intervals=[120,600,3600,14400,57600];
        while soul.critical_checks<5 && now.saturating_sub(start)>=intervals[soul.critical_checks as usize] {
            let result=rules::resolve_task(soul.attribute("will_power"),10,false,||world.resource_mut::<Dice>().roll());
            soul.critical_checks+=1;changed=true;
            if !result.success {soul.injury=rules::InjuryCondition::Dead;reply(world,e,"Your critical injuries overwhelm you. You die.");break;}
            if soul.critical_checks==5{soul.stabilized=true;reply(world,e,"You have stabilized after surviving your critical injuries.");}
        }
        if changed {if let Err(error)=commit(world,e,soul,None){error!("Critical injury save: {error}");}}
    }
}
fn profile(world:&mut World,entity:Entity,soul:&SoulData)->AttackProfile {
    if !soul.weapon.is_empty() {
        let owned=world.query_filtered::<(&GrimName,&CarriedBy),With<Object>>().iter(world).any(|(n,h)|h.carrier==entity&&n.0==soul.weapon);
        if owned {if let Some(g)=world.resource::<EquipmentCatalog>().0.iter().find(|g|g.name.eq_ignore_ascii_case(&soul.weapon)&&g.kind=="weapon") {
            return AttackProfile{ranged:false,damage_class:soul.derived().strength+g.damage_modifier-soul.injury.penalty()-i32::from(soul.winded)-i32::from(soul.fatigued),attack_bonus:g.attack_bonus,damage_type:if g.sharp{DamageType::Sharp}else{DamageType::Blunt},armor_piercing:0};
        }}
    }
    AttackProfile::unarmed(soul)
}

#[cfg(test)]
mod retaliation_tests {
    use super::*;
    #[test]
    fn death_leaves_object_and_drops_held_items_once() {
        let mut world=World::new();
        let room=world.spawn_empty().id();
        let npc=world.spawn((Creature,Actor{race:"rat".into(),level:1,gender:grim::Gender::default()},GrimName("rat".into()),InRoom{room},SoulData::default(),NpcCombatBehavior::default())).id();
        let item=world.spawn((Object,GrimName("coin".into()),CarriedBy{carrier:npc})).id();
        let dead=SoulData{injury:rules::InjuryCondition::Dead,..Default::default()};
        commit(&mut world,npc,dead,None).unwrap();
        assert!(world.get::<Creature>(npc).is_none());
        assert!(world.get::<Actor>(npc).is_none());
        assert!(world.get::<SoulData>(npc).is_none());
        assert!(world.get::<NpcCombatBehavior>(npc).is_none());
        assert!(world.get::<Object>(npc).is_some());
        assert_eq!(world.get::<GrimName>(npc).unwrap().0,"corpse of the rat");
        assert_eq!(world.get::<InRoom>(item).unwrap().room,room);
        assert!(world.get::<CarriedBy>(item).is_none());
        assert_eq!(world.query_filtered::<Entity,With<Object>>().iter(&world).count(),2);
    }
    #[test]
    fn slower_trample_target_can_strike_without_gaining_a_second_attack() {
        let mut app=App::new();
        app.init_resource::<Time>().insert_resource(Dice).init_resource::<Battles>().add_message::<InfoMessage>();
        let world=app.world_mut();
        let room=world.spawn_empty().id();
        let connection=world.spawn_empty().id();
        // High resistance keeps both combatants able to act for every round.
        let mut pc=SoulData::default();
        pc.attribute_modifiers.insert("leadership".into(),100);
        pc.attribute_modifiers.insert("body".into(),100);
        let player=world.spawn((Player{connection},GrimName("Hunter".into()),InRoom{room},pc)).id();
        let mut rat=SoulData{size:0,mass:0,total_xp:0,defense_mode:"none".into(),..Default::default()};
        rat.attribute_modifiers.insert("leadership".into(),-100);
        rat.attribute_modifiers.insert("melee_attack".into(),100);
        rat.attribute_modifiers.insert("body".into(),100);
        let npc=world.spawn((Creature,GrimName("rat".into()),InRoom{room},rat)).id();
        world.resource_mut::<Battles>().0.push(Battle{player,npc,next_round:0.,rounds:0,trample:true,lethal:true});
        let mut cursor=MessageCursor::<InfoMessage>::default();
        let mut counterattacks=0;
        for _ in 0..16 {
            tick_battles(world);
            let text=cursor.read(world.resource::<Messages<InfoMessage>>()).map(|m|m.text.as_str()).collect::<Vec<_>>().join("\n");
            let counters=text.matches("Rat counterattacks").count();
            let ordinary=text.matches("Rat hits").count()+text.matches("Rat misses").count();
            assert_eq!(counters+ordinary,1,"Exactly one rat attack each round: {text}");
            if counters>0 {assert!(text.contains("You trample rat")||text.contains("dodges your trample"),"Active defense must not cancel the declared movement: {text}");}
            counterattacks+=counters;
            world.resource_mut::<Time>().advance_by(std::time::Duration::from_secs(5));
        }
        assert!(counterattacks>0,"A ready target must be able to interrupt trample movement");
    }
}

#[cfg(test)]
mod defense_narration_tests {
    use super::*;
    fn avoided(active:bool,total:i32)->rules::AttackOutcome {
        rules::AttackOutcome{attack_total:total,defense_total:12,success_margin:0,total_damage:0,wound_level:0,hit:false,defense_used:active,injury_before:rules::InjuryCondition::Uninjured,injury_after:rules::InjuryCondition::Uninjured,stunned:false,unconscious:false}
    }
    #[test]
    fn narration_matches_actual_defense_and_equipment() {
        let mut defender=SoulData::default();defender.weapon="Light Sword".into();
        assert_eq!(avoided_strike(&avoided(true,11),&defender,true,"artrell",0),"Artrell sidesteps your strike.");
        assert_eq!(avoided_strike(&avoided(false,11),&defender,false,"artrell",0),"You parry artrell's strike with Light Sword.");
        assert!(avoided_strike(&avoided(false,11),&defender,true,"artrell",1).contains("blocks"));
        defender.weapon.clear();
        for variant in 0..3 {let text=avoided_strike(&avoided(false,11),&defender,false,"artrell",variant);assert!(!text.contains("parry")&&!text.contains("block")&&!text.contains("deflect"));}
        assert_eq!(avoided_strike(&avoided(false,0),&defender,true,"artrell",0),"Your strike goes wide.");
        defender.stunned=true;
        assert!(avoided_strike(&avoided(false,11),&defender,false,"artrell",0).contains("falls short"));
        defender.stunned=false;defender.prone=true;
        assert!(avoided_strike(&avoided(true,11),&defender,false,"artrell",1).contains("roll clear"));
    }
}

/// Build changes invalidate pending recovery and read-only trainer requests.
pub(crate) fn after_rebuild(world:&mut World,actor:Entity) {
    if let Some(mut resting)=world.get_resource_mut::<Resting>() {resting.0.retain(|(e,_,_)|*e!=actor);}
    world.entity_mut(actor).remove::<crate::trainer::Conversation>();
    crate::trainer_ai::cancel(world,actor);
    crate::species::sync_visibility(world);
}

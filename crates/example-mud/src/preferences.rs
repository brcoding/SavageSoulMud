//! Live player preferences, persisted with the full character pack.
use bevy::ecs::message::MessageCursor;
use bevy::prelude::*;
use grim::{Actor, Character, ConfigDef, ConfigRegistry, ConfigScope, Player, StoredCharacter};
use grim::components::Name as GrimName;
use grim::events::{Command,EngineCommand,InfoMessage};
use std::collections::HashMap;

#[derive(Resource,Default)]
struct Seen(HashMap<Entity,HashMap<String,String>>);
#[derive(Clone,Copy,PartialEq,Eq)]
pub(crate) enum Detail {Brief,Normal,Detailed}
pub(crate) fn detail(world:&World,actor:Entity)->Detail {
    let Some(c)=world.get::<Character>(actor) else{return Detail::Normal;};
    if c.config.get("screenreader").is_some_and(|v|v=="on") {return Detail::Brief;}
    match c.config.get("combatlog").map(String::as_str) {Some("brief")=>Detail::Brief,Some("detailed")=>Detail::Detailed,_=>Detail::Normal}
}
pub(crate) fn register(app:&mut App) {
    app.init_resource::<Seen>();
    for (key,valid,default) in [
        ("screenreader",vec!["off","on"],"off"),
        ("combatlog",vec!["brief","normal","detailed"],"normal"),
        ("prompt",vec!["classic","compact","status","combat","accessible","off","custom"],"compact"),
        ("colors",vec!["on","off"],"on"),
    ] {app.world_mut().resource_mut::<ConfigRegistry>().register(ConfigDef{key:key.into(),valid:valid.into_iter().map(String::from).collect(),default:default.into(),scope:ConfigScope::Character});}
    app.add_systems(Update,(handle,persist_config).chain());
}
fn reply(world:&mut World,actor:Entity,text:impl Into<String>) {world.write_message(InfoMessage{target:actor,text:text.into()});}
fn transition(c:&mut Character,old:&HashMap<String,String>) {
    let before=old.get("screenreader").map(String::as_str);
    let after=c.config.get("screenreader").map(String::as_str);
    if before==after {return;}
    let keys=["minimap","colors","combatlog","prompt"];
    if after==Some("on") {
        let previous:serde_json::Map<String,serde_json::Value>=keys.iter().map(|k|((*k).into(),old.get(*k).map(|v|serde_json::json!(v)).unwrap_or(serde_json::Value::Null))).collect();
        c.game_state.insert("screenreader_previous".into(),previous.into());
        for (k,v) in [("minimap","off"),("colors","off"),("combatlog","brief"),("prompt","accessible")] {c.config.insert(k.into(),v.into());}
    } else if before==Some("on") {
        let previous=c.game_state.remove("screenreader_previous");
        for key in keys {
            match previous.as_ref().and_then(|v|v.get(key)).and_then(|v|v.as_str()) {
                Some(value)=>{c.config.insert(key.into(),value.into());},
                None=>{c.config.remove(key);},
            }
        }
    }
}
fn save(world:&mut World,actor:Entity,mut c:Character)->Result<(),String> {
    if let Some(soul)=world.get::<crate::savage_rules::SoulData>(actor){c.game_state.insert("savage_soul".into(),serde_json::to_value(soul).map_err(|e|e.to_string())?);}
    let mut stored=StoredCharacter::from_components(world.get::<GrimName>(actor).ok_or("Missing name.")?,world.get::<Actor>(actor).ok_or("Missing character.")?,&c);
    stored.inventory=crate::shop::held(world,actor).into_iter().map(|(_,o)|o).collect();
    stored.inventory.sort_by(|a,b|a.name.cmp(&b.name));
    crate::shop::persist(world,&stored).map_err(|e|format!("Could not save preferences: {e}."))?;
    world.resource_mut::<Seen>().0.insert(actor,c.config.clone());
    world.entity_mut(actor).insert(c);Ok(())
}
fn validate_template(template:&str)->Result<(),String> {
    if template.is_empty()||template.len()>160||!template.chars().all(|c|c.is_ascii()&&!c.is_control()) {return Err("Use 1-160 printable characters for a custom prompt.".into());}
    let mut rest=template;
    while let Some(start)=rest.find('{') {
        if rest[..start].contains('}') {return Err("Unmatched prompt brace.".into());}
        let end=rest[start..].find('}').ok_or("Unmatched prompt brace.")?+start;
        let token=&rest[start+1..end];
        if !["name","condition","xp","credits","mana","maxmana","opponent"].contains(&token) {return Err(format!("Unknown prompt token: {token}. Use name, condition, xp, credits, mana, maxmana or opponent."));}
        rest=&rest[end+1..];
    }
    if rest.contains('}') {return Err("Unmatched prompt brace.".into());}
    // Raw colour escapes are not a prompt language; templates stay readable.
    if template.contains('@') {return Err("Custom prompts use plain text and named tokens; @ colour codes are not supported.".into());}
    Ok(())
}
fn handle(world:&mut World,mut cursor:Local<MessageCursor<EngineCommand>>) {
    let input:Vec<_>=cursor.read(world.resource::<Messages<EngineCommand>>()).filter_map(|m|match &m.command {Command::SavageSoul{verb,arg} if ["screenreader","combatlog","prompt"].contains(&verb.as_str())=>Some((m.client,verb.clone(),arg.clone())),_=>None}).collect();
    for (actor,verb,arg) in input {
        if world.get::<Player>(actor).is_none(){continue;}
        if let Err(error)=command(world,actor,&verb,&arg){reply(world,actor,error);}
    }
}
fn command(world:&mut World,actor:Entity,verb:&str,arg:&str)->Result<(),String> {
    let mut c=world.get::<Character>(actor).ok_or("Missing character.")?.clone();
    let old=c.config.clone();
    let arg=arg.trim();
    if arg.is_empty()||arg=="show" {
        let current=c.config.get(verb).cloned().unwrap_or_else(||match verb{"screenreader"=>"off","combatlog"=>"normal",_=>"compact"}.into());
        let text=match verb {
            "screenreader"=>format!("Screen reader: {current}. Use screenreader on or off. Enabling uses plain text, no map, brief combat and a spoken prompt."),
            "combatlog"=>format!("Combat detail: {current}. Use combatlog brief, normal or detailed. Screen reader mode always uses brief."),
            _=>format!("Prompt: {current}. Presets: classic, compact, status, combat, accessible, off.\nUse prompt custom <text>. Tokens: {{name}} {{condition}} {{xp}} {{credits}} {{mana}} {{maxmana}} {{opponent}}."),
        };reply(world,actor,text);return Ok(());
    }
    if verb=="prompt" && arg.starts_with("custom ") {
        let template=arg.strip_prefix("custom ").unwrap();validate_template(template)?;
        c.game_state.insert("prompt_template".into(),serde_json::Value::String(template.into()));c.config.insert("prompt".into(),"custom".into());
    } else {
        let value=if verb=="prompt" {arg.strip_prefix("preset ").unwrap_or(arg)}else{arg};
        let registry=world.resource::<ConfigRegistry>();
        let def=registry.get(verb).ok_or("Unknown setting.")?;
        let canonical=def.canonical(value).ok_or_else(||format!("Use {verb} {}.",def.valid.join("|")))?.to_string();
        c.config.insert(verb.into(),canonical);
    }
    transition(&mut c,&old);
    let value=c.config.get(verb).cloned().unwrap_or_default();save(world,actor,c)?;
    reply(world,actor,format!("{verb}: {value}. Saved."));Ok(())
}
/// Also make the engine's existing `config` command save immediately.
fn persist_config(world:&mut World) {
    let people:Vec<_>=world.query_filtered::<(Entity,&Character),With<Player>>().iter(world).map(|(e,c)|(e,c.clone())).collect();
    let mut seen=world.remove_resource::<Seen>().unwrap_or_default();
    seen.0.retain(|e,_|people.iter().any(|(actor,_)|actor==e));
    let mut changes=Vec::new();
    for (actor,mut c) in people {
        if let Some(old)=seen.0.get(&actor) {if old!=&c.config {transition(&mut c,old);changes.push((actor,c,old.clone()));}}
        else {seen.0.insert(actor,c.config.clone());}
    }
    world.insert_resource(seen);
    for (actor,c,old) in changes {if let Err(error)=save(world,actor,c) {if let Some(mut c)=world.get_mut::<Character>(actor){c.config=old;}reply(world,actor,format!("{error} Previous settings restored."));}}
}

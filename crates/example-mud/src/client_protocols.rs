//! Game-owned Mudlet payloads. Transport negotiation stays in the telnet crate.
use bevy::{ecs::message::MessageCursor, prelude::*};
use grim::{Actor, Area, CarriedBy, Character, Connection, Doors, Exits, InRoom, Object, Player, Room, StoredCharacter, TelnetCapabilities, TelnetInput, TelnetOutput};
use grim::components::Name as GrimName;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
const REPORTABLE: &[&str] = &["INJURY", "MANA", "MANA_MAX", "EXPERIENCE", "EXPERIENCE_UNSPENT", "CREDITS", "WINDED", "FATIGUED", "STUNNED", "ROOM_VNUM", "ROOM_NAME", "AREA_NAME", "ROOM_EXITS", "CHARACTER_NAME"];
const INPUTS_PER_TICK: usize = 16;
const MSDP_VALUES_PER_INPUT: usize = 32;
const GMCP:u8=201; const MSDP:u8=69; const MSSP:u8=70; const ATCP:u8=200;

#[derive(Resource, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings { name:String, website:String, media_base_url:String, map_url:String, gui_url:String, gui_version:String, tls_port:u16, #[serde(skip)] boot_at:u64 }
impl Default for Settings { fn default()->Self { Self { name:"The Savage Soul".into(), website:String::new(),media_base_url:String::new(),map_url:String::new(),gui_url:String::new(),gui_version:"1".into(),tls_port:0, boot_at:std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() } } }
#[derive(Default)]
struct ClientState { owner:Option<Entity>, last:HashMap<String,Value>, reported:HashSet<String>, mssp_sent:bool, last_inventory:Value, links_shown:bool, map_offered:bool }
#[derive(Resource,Default,Deserialize)]
struct MapperIds {rooms:BTreeMap<String,u32>}
fn mapper_id(world:&World,id:&str)->Option<u32> {world.get_resource::<MapperIds>().and_then(|m|m.rooms.get(id).copied())}
#[derive(Resource,Default)]
struct State { clients:HashMap<Entity,ClientState>, next:f64 }

pub(crate) fn register(app:&mut App) {
    crate::client_auth::register(app);
    let path=app.world().get_resource::<crate::seed::AreaBlueprintDir>().map(|d|d.0.parent().unwrap_or(&d.0).join("mudlet.json")).unwrap_or_else(||"data/mudlet.json".into());
    let ids_path=path.with_file_name("mudlet-map-ids.json");
    let ids=std::fs::read(ids_path).ok().and_then(|v|serde_json::from_slice::<MapperIds>(&v).ok()).unwrap_or_default();
    app.insert_resource(ids);
    let settings=std::fs::read(path).ok().and_then(|data|serde_json::from_slice::<Settings>(&data).ok()).unwrap_or_default();
    app.insert_resource(settings).init_resource::<State>().add_message::<TelnetInput>().add_message::<TelnetOutput>().add_systems(Update,tick);
}
fn protocol(world:&mut World,connection:Entity,option:u8,payload:Vec<u8>) { world.write_message(TelnetOutput {connection,option,payload}); }
fn gmcp(world:&mut World,connection:Entity,name:&str,value:&Value) { protocol(world,connection,GMCP,format!("{name} {value}").into_bytes()); }
fn visible_exits(world:&World,location:Entity)->BTreeMap<String,Value> {
    world.get::<Exits>(location).into_iter().flat_map(|e|e.exits.iter()).filter(|(dir,_)|!world.get::<Doors>(location).and_then(|doors|doors.doors.get(dir)).is_some_and(|door|door.hidden))
        .filter_map(|(dir,target)|world.get::<Room>(*target).map(|room|(short_direction(&dir.to_string()).into(),json!(mapper_id(world,room.id.as_str()).unwrap_or(0))))).collect()
}
fn short_direction(dir:&str)->&str {match dir {"north"=>"n","east"=>"e","south"=>"s","west"=>"w","northeast"=>"ne","northwest"=>"nw","southeast"=>"se","southwest"=>"sw","up"=>"u","down"=>"d",_=>dir}}
fn snapshot(world:&World,actor:Entity)->BTreeMap<String,Value> {
    let Some(character)=world.get::<Character>(actor) else{return BTreeMap::new();};
    let Some(soul)=world.get::<crate::savage_rules::SoulData>(actor) else{return BTreeMap::new();};
    let derived=soul.derived();
    let mut data=BTreeMap::new();
    data.insert("Char.Vitals".into(),json!({"injury":soul.injury.label(),"mana":soul.mana,"maxmana":derived.mana_max,"xp":soul.total_xp,"unspentxp":soul.available_xp(),"credits":character.credits,"winded":soul.winded,"fatigued":soul.fatigued,"stunned":soul.stunned}));
    data.insert("Char.Status".into(),json!({"name":world.get::<GrimName>(actor).map(|n|n.0.as_str()).unwrap_or(""),"race":world.get::<Actor>(actor).map(|a|a.race.as_str()).unwrap_or(""),"system":"Savage Soul","build":"XP-based"}));
    let attributes:BTreeMap<_,_>=crate::savage_rules::ATTRIBUTES.into_iter().chain(["faith"]).map(|name|(name,json!({"base":soul.base_attribute(name),"effective":soul.attribute(name)}))).collect();
    data.insert("Char.Stats".into(),json!({"attributes":attributes,"strength":derived.strength,"passive_defense":derived.passive_defense,"dodge":derived.dodge,"quickness":derived.quickness}));
    if let Some(room_entity)=world.get::<InRoom>(actor).map(|ir|ir.room) {if let Some(room)=world.get::<Room>(room_entity) {
        if soul.vision_penalty>0 {data.insert("Room.Info".into(),json!({"num":-1,"name":"Darkness","area":"","exits":{}}));}
        else {
        data.insert("Room.Info".into(),json!({"num":mapper_id(world,room.id.as_str()).map(i64::from).unwrap_or(-1),"grim_id":room.id.as_str(),"name":room.name,"area":world.get::<Area>(room.area).map(|a|a.name.as_str()).unwrap_or(""),"exits":visible_exits(world,room_entity)}));
        }
    }}
    data
}
fn variables(data:&BTreeMap<String,Value>)->BTreeMap<String,Value> {
    let mut vars=BTreeMap::new();
    for (name,key) in [("INJURY","injury"),("MANA","mana"),("MANA_MAX","maxmana"),("EXPERIENCE","xp"),("EXPERIENCE_UNSPENT","unspentxp"),("CREDITS","credits"),("WINDED","winded"),("FATIGUED","fatigued"),("STUNNED","stunned")] {if let Some(value)=data.get("Char.Vitals").and_then(|v|v.get(key)) {vars.insert(name.into(),value.clone());}}
    for (name,key) in [("ROOM_VNUM","num"),("ROOM_NAME","name"),("AREA_NAME","area"),("ROOM_EXITS","exits")] {if let Some(value)=data.get("Room.Info").and_then(|v|v.get(key)) {vars.insert(name.into(),value.clone());}}
    if let Some(name)=data.get("Char.Status").and_then(|v|v.get("name")){vars.insert("CHARACTER_NAME".into(),name.clone());}
    vars
}
fn msdp_value(out:&mut Vec<u8>,value:&Value) {
    match value {
        Value::Object(fields)=>{out.push(3);for(key,val)in fields {out.push(1);out.extend(key.bytes().filter(|b|*b>=32));out.push(2);msdp_value(out,val);}out.push(4);},
        Value::Array(values)=>{out.push(5);for value in values {out.push(2);msdp_value(out,value);}out.push(6);},
        Value::String(text)=>out.extend(text.bytes().filter(|b|*b>=32&&*b!=127)),
        _=>out.extend(value.to_string().bytes()),
    }
}
fn msdp_pair(name:&str,value:&Value)->Vec<u8> {let mut out=vec![1];out.extend(name.bytes());out.push(2);msdp_value(&mut out,value);out}
/// Commands contain only printable names, flat values or an array of names.
fn msdp_commands(payload:&[u8])->Vec<(String,Vec<String>)> {
    if payload.len()>8192{return vec![];}
    let mut commands=Vec::new();
    let mut remaining=MSDP_VALUES_PER_INPUT;
    for part in payload.split(|byte|*byte==1).skip(1).take(MSDP_VALUES_PER_INPUT) {
        if remaining==0 {break;}
        let Some(split)=part.iter().position(|b|*b==2) else{continue;};
        let name=String::from_utf8_lossy(&part[..split]).to_uppercase();
        let values:Vec<_>=part[split+1..].split(|byte|matches!(*byte,2|5|6)).map(|value|String::from_utf8_lossy(value).trim().to_uppercase()).filter(|value|!value.is_empty()&&value.len()<=64&&value.bytes().all(|byte|byte.is_ascii_alphanumeric()||byte==b'_')).take(remaining).collect();
        remaining-=values.len();
        commands.push((name,values));
    }
    commands
}
fn mssp(world:&mut World,connection:Entity) {
    let settings=world.resource::<Settings>().clone();let players=world.query::<&Player>().iter(world).count();
    let mut fields=vec![("NAME",settings.name.clone()),("PLAYERS",players.to_string()),("UPTIME",settings.boot_at.to_string()),("PORT","4000".into()),("CODEBASE","Grim".into()),("FAMILY","Custom".into()),("GAMESYSTEM","Savage Soul".into()),("STATUS","Development".into()),("LANGUAGE","English".into()),("CHARSET","UTF-8".into()),("GMCP","1".into()),("MSDP","1".into()),("MSSP","1".into()),("MXP","1".into()),("MSP","1".into()),("MTTS","1".into()),("ANSI","1".into()),("INTERMUD","I3,Zebedee".into())];
    if !settings.website.is_empty(){fields.push(("WEBSITE",settings.website.clone()));}
    if settings.tls_port!=0 {fields.push(("TLS",settings.tls_port.to_string()));}
    let mut bytes=Vec::new();for(name,value)in fields {bytes.push(1);bytes.extend(name.bytes());bytes.push(2);bytes.extend(value.bytes().filter(|b|*b>=32&&*b!=127));}protocol(world,connection,MSSP,bytes);
}
fn package_supported(caps:&TelnetCapabilities,package:&str)->bool {
    caps.gmcp_supports.is_empty()||caps.gmcp_supports.iter().any(|supported| {let name=supported.split_whitespace().next().unwrap_or("");name==package||package.strip_prefix(name).is_some_and(|tail|tail.starts_with('.'))})
}
fn tick(world:&mut World,mut cursor:Local<MessageCursor<TelnetInput>>) {
    let inputs:Vec<_>=cursor.read(world.resource::<Messages<TelnetInput>>()).cloned().collect();
    let mut state=world.remove_resource::<State>().unwrap_or_default();
    let online:HashMap<Entity,Entity>=world.query_filtered::<(Entity,&Player),With<Character>>().iter(world).map(|(actor,player)|(player.connection,actor)).collect();
    let mut budgets:HashMap<Entity,usize>=HashMap::new();
    let mut snapshots=HashMap::new();
    let mut inventories=HashMap::new();
    let mut inventory_replied=HashSet::new();
    for input in inputs {
        let used=budgets.entry(input.connection).or_default();
        if *used>=INPUTS_PER_TICK {continue;}
        *used+=1;
        if world.get::<Connection>(input.connection).is_none(){continue;}
        let Some(caps)=world.get::<TelnetCapabilities>(input.connection).cloned() else{continue;};
        let client=state.clients.entry(input.connection).or_default();
        update_owner(world,input.connection,&caps,client,online.get(&input.connection).copied());
        if input.option==MSSP&&caps.mssp {mssp(world,input.connection);client.mssp_sent=true;}
        if input.option==GMCP&&caps.gmcp {
            let text=String::from_utf8_lossy(&input.payload);
            if text.trim()=="Core.Ping" {protocol(world,input.connection,GMCP,b"Core.Ping".to_vec());}
            if text.starts_with("Core.Supports.")||input.payload.is_empty(){client.last.clear();}
            if let Some(actor)=online.get(&input.connection) {
                if text.trim()=="Char.Items.Inv" && inventory_replied.insert(input.connection) {
                    let items=inventories.entry(*actor).or_insert_with(||json!(inventory(world,*actor)));
                    gmcp(world,input.connection,"Char.Items.List",&json!({"location":"inv","items":items}));
                }
            }
        }
        if input.option==MSDP&&caps.msdp {
            for(command,values)in msdp_commands(&input.payload) {
                for value in values {
                    match command.as_str() {
                        "LIST"=>{let list=match value.as_str(){"COMMANDS"=>Some(json!(["LIST","SEND","REPORT","UNREPORT","RESET"])),"LISTS"=>Some(json!(["COMMANDS","LISTS","REPORTABLE_VARIABLES","REPORTED_VARIABLES","SENDABLE_VARIABLES"])),"REPORTABLE_VARIABLES"|"SENDABLE_VARIABLES"=>Some(json!(REPORTABLE)),"REPORTED_VARIABLES"=>Some(json!(client.reported.iter().collect::<Vec<_>>())),_=>None};if let Some(list)=list{protocol(world,input.connection,MSDP,msdp_pair(&value,&list));}},
                        "REPORT" if REPORTABLE.contains(&value.as_str())=>{client.reported.insert(value);client.last.clear();},
                        "UNREPORT"=>{client.reported.remove(&value);},
                        "RESET"=>client.reported.clear(),
                        "SEND" if REPORTABLE.contains(&value.as_str())=>{
                            if let Some(actor)=online.get(&input.connection) {
                                let vars=snapshots.entry(*actor).or_insert_with(||variables(&snapshot(world,*actor)));
                                if let Some(data)=vars.get(&value){protocol(world,input.connection,MSDP,msdp_pair(&value,data));}
                            }
                        },_=>{}
                    }
                }
            }
        }
    }
    let now=world.resource::<Time>().elapsed_secs_f64();
    if now>=state.next {state.next=now+0.5;
        let connections:Vec<_>=world.query::<(Entity,&Connection,&TelnetCapabilities)>().iter(world).map(|(entity,_,caps)|(entity,caps.clone())).collect();
        let alive:HashSet<_>=connections.iter().map(|(entity,_)|*entity).collect();state.clients.retain(|connection,_|alive.contains(connection));
        for(connection,caps)in connections {
            let client=state.clients.entry(connection).or_default();
            if caps.gmcp && !client.map_offered {
                let settings=world.resource::<Settings>().clone();
                if settings.map_url.starts_with("https://") && !world.resource::<MapperIds>().rooms.is_empty() {
                    gmcp(world,connection,"Client.Map",&json!({"url":settings.map_url}));
                    if settings.gui_url.starts_with("https://") {gmcp(world,connection,"Client.GUI",&json!({"version":settings.gui_version,"url":settings.gui_url}));}
                    client.map_offered=true;
                }
            }

            if caps.mssp&&!client.mssp_sent {mssp(world,connection);client.mssp_sent=true;}
            let actor=online.get(&connection).copied();
            update_owner(world,connection,&caps,client,actor);
            let Some(actor)=actor else{continue;};
            if !client.links_shown&&world.get::<Character>(actor).is_some_and(|c|c.config.get("client_links").is_some_and(|v|v=="on")){client.links_shown=client_links(world,connection,&caps);}
            let data=snapshot(world,actor);
            if caps.gmcp {for(package,value)in &data {if package_supported(&caps,package)&&client.last.get(package)!=Some(value){gmcp(world,connection,package,value);client.last.insert(package.clone(),value.clone());}}
                if package_supported(&caps,"Char.Items.List") {let items=inventories.entry(actor).or_insert_with(||json!(inventory(world,actor)));if *items!=client.last_inventory{gmcp(world,connection,"Char.Items.List",&json!({"location":"inv","items":items}));client.last_inventory=items.clone();}}
            }
            if caps.msdp {for(name,value)in variables(&data){let key=format!("MSDP:{name}");if client.reported.contains(&name)&&client.last.get(&key)!=Some(&value){protocol(world,connection,MSDP,msdp_pair(&name,&value));client.last.insert(key,value);}}}
            if caps.atcp&&!caps.gmcp {if let Some(room)=data.get("Room.Info"){if client.last.get("ATCP:Room")!=Some(room){protocol(world,connection,ATCP,format!("Room.Brief {}",room["name"].as_str().unwrap_or("")).into_bytes());client.last.insert("ATCP:Room".into(),room.clone());}}}
        }
    }
    world.insert_resource(state);
}
fn update_owner(world:&mut World,connection:Entity,caps:&TelnetCapabilities,client:&mut ClientState,actor:Option<Entity>) {
    if actor!=client.owner {
        if caps.gmcp&&client.owner.is_some() {
            for name in ["Char.Vitals","Char.Status","Char.Stats","Room.Info"] {gmcp(world,connection,name,&json!({}));}
            gmcp(world,connection,"Char.Items.List",&json!({"location":"inv","items":[]}));
        }
        client.last.clear();client.owner=actor;client.last_inventory=Value::Null;client.links_shown=false;
    }
}
fn inventory(world:&mut World,actor:Entity)->Vec<Value> {
    world.query_filtered::<(Entity,&CarriedBy,&GrimName),With<Object>>().iter(world).filter(|(_,held,_)|held.carrier==actor).map(|(entity,_,name)|json!({"id":entity.to_bits().to_string(),"name":name.0})).collect()
}
fn preference(world:&mut World,actor:Entity,key:&str,value:&str)->Result<(),String> {
    let mut character=world.get::<Character>(actor).ok_or("No character.")?.clone();character.config.insert(key.into(),value.into());
    if let Some(soul)=world.get::<crate::savage_rules::SoulData>(actor) {character.game_state.insert("savage_soul".into(),serde_json::to_value(soul).map_err(|e|e.to_string())?);}
    let mut stored=StoredCharacter::from_components(world.get::<GrimName>(actor).ok_or("No name.")?,world.get::<Actor>(actor).ok_or("No actor.")?,&character);
    stored.inventory=crate::shop::held(world,actor).into_iter().map(|(_,item)|item).collect();crate::shop::persist(world,&stored).map_err(|e|format!("Could not save preference: {e}"))?;world.entity_mut(actor).insert(character);Ok(())
}
fn client_links(world:&mut World,connection:Entity,caps:&TelnetCapabilities)->bool {let mut bytes=Vec::new();if caps.mxp {bytes.extend(b"\x1b[1z<SEND HREF=\"help\">Help</SEND> | <SEND HREF=\"score\">Score</SEND> | <SEND HREF=\"inventory\">Inventory</SEND> | <SEND HREF=\"look\">Look</SEND>\x1b[7z\n");}if caps.terminal_types.iter().any(|term|term.to_ascii_uppercase().contains("MUDLET")){bytes.extend(b"\x1b]8;;https://wiki.mudlet.org/w/Manual:Supported_Protocols\x1b\\Mudlet protocol reference\x1b]8;;\x1b\\\n");}let sent=!bytes.is_empty();if sent{protocol(world,connection,0,bytes);}sent}
pub(crate) fn command(world:&mut World,actor:Entity,arg:&str)->Result<(),String> {
    let connection=world.get::<Player>(actor).ok_or("You are not connected.")?.connection;
    let caps=world.get::<TelnetCapabilities>(connection).cloned().unwrap_or_default();
    let mut reply=String::new();
    match arg.trim() {
        "links on"=>{preference(world,actor,"client_links","on")?;world.resource_mut::<State>().clients.entry(connection).or_default().links_shown=false;reply.push_str("Client links enabled and saved. Use protocols links off to disable.");},
        "links off"=>{preference(world,actor,"client_links","off")?;reply.push_str("Client links disabled and saved.");},
        "sound on"=>{preference(world,actor,"client_sound","on")?;reply.push_str("Client sound enabled and saved. Use protocols sound test to play the sample.");},
        "sound off"=>{preference(world,actor,"client_sound","off")?;reply.push_str("Client sound disabled and saved.");},
        "sound test"=>{if !world.get::<Character>(actor).is_some_and(|c|c.config.get("client_sound").is_some_and(|value|value=="on")){return Err("Enable optional audio first with protocols sound on.".into());}let url=world.resource::<Settings>().media_base_url.clone();if !(url.starts_with("http://")||url.starts_with("https://"))||url.chars().any(|c|c.is_control()||c.is_whitespace()||matches!(c,'(' | ')' | '\"' | '\'')){return Err("No valid media URL is configured.".into());}if caps.gmcp&&package_supported(&caps,"Client.Media"){gmcp(world,connection,"Client.Media.Play",&json!({"name":"protocol-ready.wav","type":"sound","url":url,"volume":20}));}else if caps.msp{protocol(world,connection,0,format!("!!SOUND(protocol-ready.wav V=20 U={url})\n").into_bytes());}else{return Err("Your client has not enabled MSP or GMCP Client.Media.".into());}reply.push_str("Sent the optional sound sample to your client.");},
        ""|"status"=>{reply=format!("Client protocols: GMCP {}, MSDP {}, MSSP {}, MXP {}, MSP {}, ATCP {}, UTF-8 {}. Terminal: {}x{}; {}.\nUse protocols links on|off, protocols sound on|off, or protocols sound test. TLS connects on the separately advertised port; capability flags alone do not prove encryption.",caps.gmcp,caps.msdp,caps.mssp,caps.mxp,caps.msp,caps.atcp,caps.utf8,caps.width,caps.height,caps.terminal_types.join(", "));},
        _=>return Err("Use protocols [status|links on|links off|sound on|sound off|sound test].".into())
    }
    world.write_message(grim::events::InfoMessage{target:actor,text:reply});Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn msdp_caps_total_values_and_commands_across_the_payload() {
        let payload=b"\x01SEND\x02MANA".repeat(100);
        let commands=msdp_commands(&payload);
        assert_eq!(commands.iter().map(|(_,values)|values.len()).sum::<usize>(),MSDP_VALUES_PER_INPUT);
        assert!(commands.len()<=MSDP_VALUES_PER_INPUT);
        let payload=b"\x01\x02".repeat(2000);
        assert!(msdp_commands(&payload).len()<=MSDP_VALUES_PER_INPUT);
    }

    #[test]
    fn protocol_bursts_obey_each_connections_tick_budget() {
        let mut app=App::new();
        app.init_resource::<Time>().init_resource::<State>().init_resource::<Settings>().init_resource::<MapperIds>();
        app.add_message::<TelnetInput>().add_message::<TelnetOutput>().add_systems(Update,tick);
        let mut connections=Vec::new();
        for id in 1..=2 {
            let connection=app.world_mut().spawn((Connection{id,addr:"127.0.0.1:1234".parse().unwrap(),echo_hidden:false},TelnetCapabilities{gmcp:true,..Default::default()})).id();
            connections.push(connection);
            for _ in 0..100 {app.world_mut().write_message(TelnetInput{connection,option:GMCP,payload:b"Core.Ping".to_vec()});}
        }
        app.update();
        let outputs=app.world().resource::<Messages<TelnetOutput>>();
        let mut cursor=outputs.get_cursor();
        let frames:Vec<_>=cursor.read(outputs).collect();
        for connection in connections {assert_eq!(frames.iter().filter(|f|f.connection==connection&&f.payload==b"Core.Ping").count(),INPUTS_PER_TICK);}
    }
    #[test]
    fn msdp_array_and_table_are_structured_not_json_text() {
        assert_eq!(msdp_pair("ROOM_EXITS",&json!({"n":"abc"})),b"\x01ROOM_EXITS\x02\x03\x01n\x02abc\x04");
        let commands=msdp_commands(b"\x01REPORT\x02\x05\x02MANA\x02INJURY\x06");assert_eq!(commands[0].1,vec!["MANA","INJURY"]);
    }
    #[test]
    fn module_supports_does_not_match_another_namespace() {
        let caps=TelnetCapabilities{gmcp_supports:vec!["Char 1".into()],..Default::default()};
        assert!(package_supported(&caps,"Char.Vitals"));assert!(!package_supported(&caps,"Room.Info"));
    }
}

//! Spawn ownership, population budgets and constrained NPC movement.
use std::collections::{HashMap,HashSet,VecDeque};
use bevy::prelude::*;
use grim::{Creature,InRoom,Room,Exits,Doors,Cardinal};
use grim::components::Name as GrimName;
use serde::Deserialize;

#[derive(Clone,Deserialize)]
#[serde(default)]
pub(crate) struct SpawnConfig {
    pub id:Option<String>,pub max_alive:u16,pub respawn_seconds:u64,
    pub pool:Option<String>,pub max_total:Option<u16>,pub movement:Movement,
}
impl Default for SpawnConfig {fn default()->Self{Self{id:None,max_alive:1,respawn_seconds:60,pool:None,max_total:None,movement:Movement::default()}}}
#[derive(Clone,Deserialize)]
#[serde(default)]
pub(crate) struct Movement {pub mode:Mode,pub interval_seconds:u64,pub max_distance:Option<u16>,pub same_area:bool,pub target:Option<String>}
impl Default for Movement {fn default()->Self{Self{mode:Mode::Stay,interval_seconds:15,max_distance:None,same_area:true,target:None}}}
#[derive(Clone,Copy,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
pub(crate) enum Mode {Stay,Wander,GuardRoom,GuardNpc}
#[derive(Resource,Deserialize,Default)]
#[serde(default)]
struct Settings {profiles:HashMap<String,SpawnConfig>}
/// Can be disabled by deterministic command tests; live games default to on.
#[derive(Resource)]
pub struct PopulationEnabled(pub bool);
impl Default for PopulationEnabled {fn default()->Self{Self(true)}}
#[derive(Component,Clone)]
pub(crate) struct Candidate {pub key:String,pub home:Entity,pub area:String,pub template:crate::seed::NpcBlueprint}
#[derive(Component,Clone)]
pub(crate) struct Member {pub point:String,pub home:Entity,pub next_move:f64}
struct Point {home:Entity,area:String,template:crate::seed::NpcBlueprint,config:SpawnConfig,members:HashSet<Entity>,pending:VecDeque<f64>}
#[derive(Resource,Default)]
struct Population {points:HashMap<String,Point>,cooldowns:HashMap<String,Vec<f64>>,next_tick:f64}

pub(crate) fn register(app:&mut App) {
    let path=app.world().get_resource::<crate::seed::AreaBlueprintDir>().map(|d|d.0.parent().unwrap_or(&d.0).join("savage-soul/population.json")).unwrap_or_else(||"data/savage-soul/population.json".into());
    let settings=match std::fs::read(&path) {Ok(b)=>serde_json::from_slice(&b).unwrap_or_else(|e|{warn!("Invalid NPC population settings: {e}");Settings::default()}),Err(_)=>Settings::default()};
    app.insert_resource(settings).init_resource::<Population>().init_resource::<PopulationEnabled>();
    app.add_observer(hold_combatants);
}
fn hold_combatants(mut walk:On<grim::AttemptWalk>,members:Query<&Member>,battles:Res<crate::savage_soul::Battles>,souls:Query<&crate::savage_rules::SoulData>) {
    if members.contains(walk.actor)&&(battles.contains(walk.actor)||souls.get(walk.actor).is_ok_and(|s|s.is_incapacitated())){walk.event_mut().denied=true;}
}
fn discover(world:&mut World,pop:&mut Population,now:f64) {
    let candidates:Vec<_>=world.query::<(Entity,&Candidate)>().iter(world).map(|(e,c)|(e,c.clone())).collect();
    for (entity,c) in candidates {
        crate::medical::remember(world,&c.template);
        world.entity_mut(entity).remove::<Candidate>();
        let config=c.template.spawn.clone().or_else(||world.resource::<Settings>().profiles.get(&c.template.name.to_ascii_lowercase()).cloned());
        let Some(mut config)=config else{continue;};
        config.max_alive=config.max_alive.min(100);config.respawn_seconds=config.respawn_seconds.clamp(1,86400);
        config.movement.interval_seconds=config.movement.interval_seconds.clamp(1,3600);
        if let Some(limit)=config.max_total {config.max_total=Some(limit.min(1000));}
        let key=config.id.clone().unwrap_or(c.key);
        if pop.points.contains_key(&key){warn!("Duplicate NPC spawn id {key}; skipping duplicate initial NPC");world.despawn(entity);continue;}
        let point=Point{home:c.home,area:c.area,template:c.template,config:config.clone(),members:HashSet::from([entity]),pending:VecDeque::new()};
        info!("NPC spawn registered: {} at {}, cap {}, delay {}s",point.template.name,world.get::<Room>(point.home).map(|r|r.friendly_id.as_str()).unwrap_or("missing"),config.max_alive,config.respawn_seconds);
        world.entity_mut(entity).insert(Member{point:key.clone(),home:c.home,next_move:now+config.movement.interval_seconds as f64});
        pop.points.insert(key,point);
    }
}
fn pool_limit(pop:&Population,pool:&str)->usize {
    pop.points.values().filter(|p|p.config.pool.as_deref()==Some(pool)).filter_map(|p|p.config.max_total).min().unwrap_or(1000)as usize
}
fn pool_count(pop:&Population,pool:&str)->usize {pop.points.values().filter(|p|p.config.pool.as_deref()==Some(pool)).map(|p|p.members.len()).sum()}

pub(crate) fn tick(world:&mut World) {
    if !world.resource::<PopulationEnabled>().0{return;}
    let now=world.resource::<Time>().elapsed_secs_f64();
    let mut pop=world.remove_resource::<Population>().unwrap_or_default();
    if now<pop.next_tick{world.insert_resource(pop);return;}
    pop.next_tick=now+1.;
    discover(world,&mut pop,now);
    for cooldown in pop.cooldowns.values_mut(){cooldown.retain(|at|*at>now);}
    let mut keys:Vec<_>=pop.points.keys().cloned().collect();keys.sort();
    for key in &keys {
        let point=pop.points.get_mut(key).unwrap();
        let gone:Vec<_>=point.members.iter().copied().filter(|e|world.get::<Creature>(*e).is_none()).collect();
        for entity in gone {
            point.members.remove(&entity);
            let due=now+point.config.respawn_seconds as f64;
            info!("NPC spawn vacancy: {} at {}, replacement in {}s",point.template.name,world.get::<Room>(point.home).map(|r|r.friendly_id.as_str()).unwrap_or("missing"),point.config.respawn_seconds);
            point.pending.push_back(due);
            if let Some(pool)=&point.config.pool{pop.cooldowns.entry(pool.clone()).or_default().push(due);}
        }
        while point.members.len()+point.pending.len()<(point.config.max_alive as usize){point.pending.push_back(now);}
        while point.members.len()>(point.config.max_alive as usize) {
            let Some(entity)=point.members.iter().copied().max()else{break;};point.members.remove(&entity);world.despawn(entity);
        }
        // Pending initial slots may be earlier than slots freed by later deaths.
        point.pending.make_contiguous().sort_by(f64::total_cmp);
    }
    // Enforce shared budgets on initial placements as well as replacements.
    let pools:HashSet<_>=pop.points.values().filter_map(|p|p.config.pool.clone()).collect();
    for pool in pools {
        while pool_count(&pop,&pool)>pool_limit(&pop,&pool) {
            let Some(key)=keys.iter().rev().find(|k|pop.points[*k].config.pool.as_deref()==Some(&pool)&&!pop.points[*k].members.is_empty()).cloned()else{break;};
            let point=pop.points.get_mut(&key).unwrap();let entity=*point.members.iter().max().unwrap();
            point.members.remove(&entity);point.pending.push_back(now);world.despawn(entity);
        }
    }
    for key in &keys {
        loop {
            let point=&pop.points[key];
            if point.members.len()>=point.config.max_alive as usize||point.pending.front().is_none_or(|at|*at>now)||world.get::<Room>(point.home).is_none(){break;}
            if let Some(pool)=&point.config.pool {
                let waiting=pop.cooldowns.get(pool).map(Vec::len).unwrap_or(0);
                if pool_count(&pop,pool)+waiting>=pool_limit(&pop,pool){break;}
            }
            let point=pop.points.get_mut(key).unwrap();
            let entity={let mut commands=world.commands();crate::seed::spawn_npc(&mut commands,&point.area,&point.template,point.home)};
            world.flush();
            world.entity_mut(entity).insert(Member{point:key.clone(),home:point.home,next_move:now+point.config.movement.interval_seconds as f64});
            point.pending.pop_front();point.members.insert(entity);
            info!("NPC respawned: {} at {}",point.template.name,world.get::<Room>(point.home).map(|r|r.friendly_id.as_str()).unwrap_or("missing"));
        }
    }
    let members:Vec<_>=world.query_filtered::<(Entity,&Member,&InRoom),With<Creature>>().iter(world).map(|(e,m,r)|(e,m.clone(),r.room)).collect();
    for (entity,member,room) in members {
        if now<member.next_move{continue;}
        let Some(point)=pop.points.get(&member.point)else{continue;};
        let movement=&point.config.movement;
        if let Some(mut live)=world.get_mut::<Member>(entity){live.next_move=now+movement.interval_seconds as f64;}
        if movement.mode==Mode::Stay||crate::savage_soul::in_combat(world,entity)||world.get::<crate::savage_rules::SoulData>(entity).is_some_and(|s|s.is_incapacitated()){continue;}
        let bounds=allowed_rooms(world,member.home,movement);
        let next=match movement.mode {
            Mode::Stay=>None,
            Mode::Wander=>{
                let choices=edges(world,room).into_iter().filter(|(_,to)|bounds.contains(to)).collect::<Vec<_>>();
                if choices.is_empty(){None}else{Some(choices[rand::random_range(0..choices.len())].0)}
            },
            Mode::GuardRoom=>{
                let target=movement.target.as_deref().unwrap_or("home");
                let goal=if target=="home"{Some(member.home)}else{
                    let found:Vec<_>=world.query::<(Entity,&Room)>().iter(world).filter(|(_,r)|r.friendly_id==target||r.id.to_string()==target).map(|(e,_)|e).collect();
                    if found.len()==1{found.first().copied()}else{None}
                };
                goal.and_then(|goal|route(world,room,goal,&bounds))
            },
            Mode::GuardNpc=>movement.target.as_deref().and_then(|target|{
                let found:Vec<_>=world.query_filtered::<(Entity,&GrimName,&InRoom,Option<&Member>),With<Creature>>().iter(world).filter(|(e,n,_,m)|*e!=entity&&(m.is_some_and(|m|m.point==target)||n.0.eq_ignore_ascii_case(target))).map(|(_,_,r,_)|r.room).collect();
                if found.len()==1{route(world,room,found[0],&bounds)}else{None}
            }),
        };
        if let Some(direction)=next {world.write_message(grim::events::EngineCommand{client:entity,command:grim::events::Command::Move{direction}});}
    }
    world.insert_resource(pop);
}
fn edges(world:&World,room:Entity)->Vec<(Cardinal,Entity)> {
    let mut edges:Vec<_>=world.get::<Exits>(room).map(|e|e.exits.iter().filter(|(d,to)|world.get::<Room>(**to).is_some()&&!world.get::<Doors>(room).and_then(|doors|doors.doors.get(d)).is_some_and(|door|!door.open||door.hidden)).map(|(d,to)|(*d,*to)).collect()).unwrap_or_default();
    edges.sort_by_key(|(d,_)|*d);edges
}
fn allowed_rooms(world:&World,home:Entity,movement:&Movement)->HashSet<Entity> {
    let area=world.get::<Room>(home).map(|r|r.area);
    let mut found=HashSet::from([home]);let mut queue=VecDeque::from([(home,0u16)]);
    while let Some((room,distance))=queue.pop_front() {
        if movement.max_distance.is_some_and(|max|distance>=max){continue;}
        for (_,to) in edges(world,room) {
            if world.get::<Room>(to).is_some_and(|r|!movement.same_area||Some(r.area)==area)&&found.insert(to){queue.push_back((to,distance.saturating_add(1)));}
        }
    }
    found
}
fn route(world:&World,from:Entity,to:Entity,bounds:&HashSet<Entity>)->Option<Cardinal> {
    if from==to||!bounds.contains(&to){return None;}
    let mut seen=HashSet::from([from]);let mut queue=VecDeque::from([(from,None)]);
    while let Some((room,first))=queue.pop_front(){for (direction,next) in edges(world,room){
        if !bounds.contains(&next)||!seen.insert(next){continue;}
        let step=first.unwrap_or(direction);if next==to{return Some(step);}
        queue.push_back((next,Some(step)));
    }}None
}
pub(crate) fn status(world:&World,actor:Entity,filter:&str)->Result<String,String> {
    if !world.get::<grim::Character>(actor).is_some_and(|c|c.is_admin()){return Err("That command requires wizard privileges.".into());}
    let now=world.resource::<Time>().elapsed_secs_f64();let pop=world.resource::<Population>();
    let mut keys:Vec<_>=pop.points.keys().collect();keys.sort();
    let room_name=|room:Entity|world.get::<Room>(room).map(|r|r.friendly_id.clone()).unwrap_or_else(||"missing room".into());
    let mut lines=vec!["NPC spawn points".to_string()];
    for key in keys {let point=&pop.points[key];
        if !filter.trim().is_empty()&&!format!("{} {} {}",point.template.name,key,point.config.pool.as_deref().unwrap_or("")).to_ascii_lowercase().contains(&filter.trim().to_ascii_lowercase()){continue;}
        let mode=match point.config.movement.mode {Mode::Stay=>"stay",Mode::Wander=>"wander",Mode::GuardRoom=>"guard room",Mode::GuardNpc=>"guard NPC"};
        let next=point.pending.iter().map(|due|(*due-now).max(0.).ceil()as u64).min().map(|secs|format!("{secs}s")).unwrap_or_else(||"none".into());
        lines.push(format!("{} | home {} | alive {}/{} | respawn {}s, next {next} | {mode}",point.template.name,room_name(point.home),point.members.len(),point.config.max_alive,point.config.respawn_seconds));
        lines.push(format!("  spawn id: {key}"));
        let mut members:Vec<_>=point.members.iter().collect();members.sort();
        for entity in members {if let Some(room)=world.get::<InRoom>(*entity){lines.push(format!("  current: {}",room_name(room.room)));}}
        if let Some(pool)=&point.config.pool{lines.push(format!("  pool {pool}: {}/{} alive",pool_count(pop,pool),pool_limit(pop,pool)));}
    }
    if lines.len()==1{lines.push("No matching managed spawns.".into());}Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world()->World {
        let mut w=World::new();w.init_resource::<Time>();w.init_resource::<Population>();w.init_resource::<Settings>();w.init_resource::<PopulationEnabled>();w.init_resource::<Messages<grim::events::EngineCommand>>();w
    }
    fn room(w:&mut World,area:Entity,name:&str)->Entity {
        w.spawn((Room{id:grim::GrimId::new(),friendly_id:name.into(),name:name.into(),description:String::new(),area},Exits::default())).id()
    }
    fn link(w:&mut World,a:Entity,b:Entity) {w.get_mut::<Exits>(a).unwrap().exits.insert(Cardinal::East,b);w.get_mut::<Exits>(b).unwrap().exits.insert(Cardinal::West,a);}
    fn add(w:&mut World,home:Entity,key:&str,movement:Movement)->Entity {
        let mut template:crate::seed::NpcBlueprint=serde_json::from_value(serde_json::json!({"name":"rat","description":["A rat."],"soul":{"size":0,"mass":0,"total_xp":0},"combat_behavior":{"xp_reward":5}})).unwrap();
        template.spawn=Some(SpawnConfig{max_alive:1,respawn_seconds:60,pool:Some("rats".into()),max_total:Some(2),movement,..Default::default()});
        let entity={let mut c=w.commands();crate::seed::spawn_npc(&mut c,"test",&template,home)};w.flush();
        w.entity_mut(entity).insert(Candidate{key:key.into(),home,area:"test".into(),template});entity
    }
    fn advance(w:&mut World,seconds:u64){w.resource_mut::<Time>().advance_by(std::time::Duration::from_secs(seconds));tick(w);}
    #[test]
    fn moved_rat_counts_toward_cap_and_death_replaces_once_at_home_after_sixty_seconds() {
        let mut w=world();let area=w.spawn_empty().id();let a=room(&mut w,area,"a");let b=room(&mut w,area,"b");link(&mut w,a,b);
        let rat=add(&mut w,a,"a-rat",Movement::default());let other=add(&mut w,b,"b-rat",Movement::default());tick(&mut w);
        w.get_mut::<InRoom>(rat).unwrap().room=b;advance(&mut w,90);
        assert_eq!(w.query_filtered::<Entity,With<Creature>>().iter(&w).count(),2);
        assert_eq!(w.get::<Member>(rat).unwrap().home,a);
        // A corpse still exists, but it no longer occupies a living spawn slot.
        w.entity_mut(rat).remove::<Creature>().insert(grim::Object);tick(&mut w);
        advance(&mut w,1); // the one-second sweep notices death here
        advance(&mut w,59);
        assert_eq!(w.query_filtered::<Entity,With<Creature>>().iter(&w).count(),1);
        advance(&mut w,1);
        assert_eq!(w.query_filtered::<Entity,With<Creature>>().iter(&w).count(),2);
        let replacement=*w.resource::<Population>().points["a-rat"].members.iter().next().unwrap();
        assert_ne!(replacement,rat);assert_eq!(w.get::<InRoom>(replacement).unwrap().room,a);
        assert_eq!(w.get::<crate::savage_rules::SoulData>(replacement).unwrap().size,0);
        assert_eq!(w.get::<crate::savage_soul::NpcCombatBehavior>(replacement).unwrap().xp_reward,5);
        assert!(w.get::<Creature>(other).is_some());advance(&mut w,600);
        assert_eq!(w.query_filtered::<Entity,With<Creature>>().iter(&w).count(),2);
    }
    #[test]
    fn shared_pool_never_exceeds_cap_and_vacancy_waits_for_cooldown() {
        let mut w=world();let area=w.spawn_empty().id();let home=room(&mut w,area,"home");
        let first=add(&mut w,home,"first",Movement::default());add(&mut w,home,"second",Movement::default());add(&mut w,home,"third",Movement::default());tick(&mut w);
        assert_eq!(w.query_filtered::<Entity,With<Creature>>().iter(&w).count(),2);
        w.despawn(first);advance(&mut w,1);advance(&mut w,59);
        assert_eq!(w.query_filtered::<Entity,With<Creature>>().iter(&w).count(),1);
        advance(&mut w,1);assert_eq!(w.query_filtered::<Entity,With<Creature>>().iter(&w).count(),2);
        advance(&mut w,600);assert_eq!(w.query_filtered::<Entity,With<Creature>>().iter(&w).count(),2);
    }
    #[test]
    fn wander_obeys_radius_area_and_closed_doors() {
        let mut w=world();let area=w.spawn_empty().id();let foreign=w.spawn_empty().id();
        let a=room(&mut w,area,"a");let b=room(&mut w,area,"b");let c=room(&mut w,area,"c");let d=room(&mut w,area,"d");let z=room(&mut w,foreign,"z");
        link(&mut w,a,b);link(&mut w,b,c);link(&mut w,c,d);w.get_mut::<Exits>(c).unwrap().exits.insert(Cardinal::North,z);
        let movement=Movement{mode:Mode::Wander,interval_seconds:1,max_distance:Some(2),..Default::default()};
        let rat=add(&mut w,a,"rat",movement.clone());tick(&mut w);w.get_mut::<InRoom>(rat).unwrap().room=c;
        let allowed=allowed_rooms(&w,a,&movement);assert!(allowed.contains(&c));assert!(!allowed.contains(&d));assert!(!allowed.contains(&z));
        advance(&mut w,1);
        let mut cursor=bevy::ecs::message::MessageCursor::<grim::events::EngineCommand>::default();
        assert!(cursor.read(w.resource::<Messages<grim::events::EngineCommand>>()).any(|m|m.client==rat&&m.command==grim::events::Command::Move{direction:Cardinal::West}));
        w.entity_mut(c).insert(Doors{doors:HashMap::from([(Cardinal::West,grim::Door{name:"gate".into(),keywords:vec![],open:false,hidden:false})])});
        assert!(edges(&w,c).iter().all(|(d,_)|*d!=Cardinal::West));
    }
    #[test]
    fn guard_objectives_route_to_room_or_unique_npc_without_teleporting() {
        let mut w=world();let area=w.spawn_empty().id();let a=room(&mut w,area,"a");let b=room(&mut w,area,"b");link(&mut w,a,b);
        let guard=add(&mut w,a,"guard",Movement{mode:Mode::GuardRoom,target:Some("b".into()),interval_seconds:1,..Default::default()});tick(&mut w);advance(&mut w,1);
        let mut cursor=bevy::ecs::message::MessageCursor::<grim::events::EngineCommand>::default();
        assert!(cursor.read(w.resource::<Messages<grim::events::EngineCommand>>()).any(|m|m.client==guard&&m.command==grim::events::Command::Move{direction:Cardinal::East}));
        assert_eq!(w.get::<InRoom>(guard).unwrap().room,a); // movement is queued through the normal engine
        w.spawn((Creature,GrimName("leader".into()),InRoom{room:b}));
        w.resource_mut::<Population>().points.get_mut("guard").unwrap().config.movement=Movement{mode:Mode::GuardNpc,target:Some("leader".into()),interval_seconds:1,..Default::default()};
        advance(&mut w,1);
        assert!(cursor.read(w.resource::<Messages<grim::events::EngineCommand>>()).any(|m|m.client==guard&&m.command==grim::events::Command::Move{direction:Cardinal::East}));
    }
}

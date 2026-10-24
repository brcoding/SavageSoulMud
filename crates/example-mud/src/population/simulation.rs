//! simulation implementation, split from the parent subsystem.

use super::*;

pub(crate) fn tick(world: &mut World) {
    if !world.resource::<PopulationEnabled>().0 {
        return;
    }
    let now = world.resource::<Time>().elapsed_secs_f64();
    let mut pop = world.remove_resource::<Population>().unwrap_or_default();
    if now < pop.next_tick {
        world.insert_resource(pop);
        return;
    }
    pop.next_tick = now + 1.;
    discover(world, &mut pop, now);
    for cooldown in pop.cooldowns.values_mut() {
        cooldown.retain(|at| *at > now);
    }
    let mut keys: Vec<_> = pop.points.keys().cloned().collect();
    keys.sort();
    reconcile_slots(world, &mut pop, &keys, now);
    // Enforce shared budgets on initial placements as well as replacements.
    let pools: HashSet<_> = pop
        .points
        .values()
        .filter_map(|p| p.config.pool.clone())
        .collect();
    for pool in pools {
        while pool_count(&pop, &pool) > pool_limit(&pop, &pool) {
            let Some(key) = keys
                .iter()
                .rev()
                .find(|k| {
                    pop.points[*k].config.pool.as_deref() == Some(&pool)
                        && !pop.points[*k].members.is_empty()
                })
                .cloned()
            else {
                break;
            };
            let point = pop.points.get_mut(&key).unwrap();
            let entity = *point.members.iter().max().unwrap();
            point.members.remove(&entity);
            point.pending.push_back(now);
            world.despawn(entity);
        }
    }
    for key in &keys {
        loop {
            let point = &pop.points[key];
            if point.members.len() >= point.config.max_alive as usize
                || point.pending.front().is_none_or(|at| *at > now)
                || world.get::<Room>(point.home).is_none()
            {
                break;
            }
            if let Some(pool) = &point.config.pool {
                let waiting = pop.cooldowns.get(pool).map(Vec::len).unwrap_or(0);
                if pool_count(&pop, pool) + waiting >= pool_limit(&pop, pool) {
                    break;
                }
            }
            let point = pop.points.get_mut(key).unwrap();
            let entity = {
                let mut commands = world.commands();
                crate::seed::spawn_npc(&mut commands, &point.area, &point.template, point.home)
            };
            world.flush();
            world.entity_mut(entity).insert(Member {
                point: key.clone(),
                home: point.home,
                next_move: now + point.config.movement.interval_seconds as f64,
            });
            point.pending.pop_front();
            point.members.insert(entity);
            info!(
                "NPC respawned: {} at {}",
                point.template.name,
                world
                    .get::<Room>(point.home)
                    .map(|r| r.friendly_id.as_str())
                    .unwrap_or("missing")
            );
        }
    }
    move_members(world, &pop, now);
    world.insert_resource(pop);
}

pub(crate) fn status(world: &World, actor: Entity, filter: &str) -> Result<String, String> {
    if !world
        .get::<grim::Character>(actor)
        .is_some_and(|c| c.is_admin())
    {
        return Err("That command requires wizard privileges.".into());
    }
    let now = world.resource::<Time>().elapsed_secs_f64();
    let pop = world.resource::<Population>();
    let mut keys: Vec<_> = pop.points.keys().collect();
    keys.sort();
    let room_name = |room: Entity| {
        world
            .get::<Room>(room)
            .map(|r| r.friendly_id.clone())
            .unwrap_or_else(|| "missing room".into())
    };
    let mut lines = vec!["NPC spawn points".to_string()];
    for key in keys {
        let point = &pop.points[key];
        if !filter.trim().is_empty()
            && !format!(
                "{} {} {}",
                point.template.name,
                key,
                point.config.pool.as_deref().unwrap_or("")
            )
            .to_ascii_lowercase()
            .contains(&filter.trim().to_ascii_lowercase())
        {
            continue;
        }
        let mode = match point.config.movement.mode {
            Mode::Stay => "stay",
            Mode::Wander => "wander",
            Mode::GuardRoom => "guard room",
            Mode::GuardNpc => "guard NPC",
        };
        let next = point
            .pending
            .iter()
            .map(|due| (*due - now).max(0.).ceil() as u64)
            .min()
            .map(|secs| format!("{secs}s"))
            .unwrap_or_else(|| "none".into());
        lines.push(format!(
            "{} | home {} | alive {}/{} | respawn {}s, next {next} | {mode}",
            point.template.name,
            room_name(point.home),
            point.members.len(),
            point.config.max_alive,
            point.config.respawn_seconds
        ));
        lines.push(format!("  spawn id: {key}"));
        let mut members: Vec<_> = point.members.iter().collect();
        members.sort();
        for entity in members {
            if let Some(room) = world.get::<InRoom>(*entity) {
                lines.push(format!("  current: {}", room_name(room.room)));
            }
        }
        if let Some(pool) = &point.config.pool {
            lines.push(format!(
                "  pool {pool}: {}/{} alive",
                pool_count(pop, pool),
                pool_limit(pop, pool)
            ));
        }
    }
    if lines.len() == 1 {
        lines.push("No matching managed spawns.".into());
    }
    Ok(lines.join("\n"))
}

fn move_members(world: &mut World, pop: &Population, now: f64) {
    let members: Vec<_> = world
        .query_filtered::<(Entity, &Member, &InRoom), With<Creature>>()
        .iter(world)
        .map(|(e, m, r)| (e, m.clone(), r.room))
        .collect();
    for (entity, member, room) in members {
        if now < member.next_move {
            continue;
        }
        let Some(point) = pop.points.get(&member.point) else {
            continue;
        };
        let movement = &point.config.movement;
        if let Some(mut live) = world.get_mut::<Member>(entity) {
            live.next_move = now + movement.interval_seconds as f64;
        }
        if movement.mode == Mode::Stay
            || crate::savage_soul::in_combat(world, entity)
            || world
                .get::<crate::savage_rules::SoulData>(entity)
                .is_some_and(|s| s.is_incapacitated())
        {
            continue;
        }
        let bounds = allowed_rooms(world, member.home, movement);
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
        if let Some(direction) = next {
            world.write_message(grim::events::EngineCommand {
                client: entity,
                command: grim::events::Command::Move { direction },
            });
        }
    }
}

fn reconcile_slots(world: &mut World, pop: &mut Population, keys: &[String], now: f64) {
    for key in keys {
        let point = pop.points.get_mut(key).unwrap();
        let gone: Vec<_> = point
            .members
            .iter()
            .copied()
            .filter(|e| world.get::<Creature>(*e).is_none())
            .collect();
        for entity in gone {
            point.members.remove(&entity);
            let due = now + point.config.respawn_seconds as f64;
            info!(
                "NPC spawn vacancy: {} at {}, replacement in {}s",
                point.template.name,
                world
                    .get::<Room>(point.home)
                    .map(|r| r.friendly_id.as_str())
                    .unwrap_or("missing"),
                point.config.respawn_seconds
            );
            point.pending.push_back(due);
            if let Some(pool) = &point.config.pool {
                pop.cooldowns.entry(pool.clone()).or_default().push(due);
            }
        }
        while point.members.len() + point.pending.len() < (point.config.max_alive as usize) {
            point.pending.push_back(now);
        }
        while point.members.len() > (point.config.max_alive as usize) {
            let Some(entity) = point.members.iter().copied().max() else {
                break;
            };
            point.members.remove(&entity);
            world.despawn(entity);
        }
        // Pending initial slots may be earlier than slots freed by later deaths.
        point.pending.make_contiguous().sort_by(f64::total_cmp);
    }
}

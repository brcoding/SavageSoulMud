//! Spawn ownership, population budgets and constrained NPC movement.
use bevy::prelude::*;
use grim::components::Name as GrimName;
use grim::{Cardinal, Creature, Doors, Exits, InRoom, Room};
use serde::Deserialize;
use std::collections::{HashMap, HashSet, VecDeque};
mod navigation;
use navigation::{allowed_rooms, edges, route};
mod simulation;
pub(crate) use simulation::{status, tick};

#[derive(Clone, Deserialize)]
#[serde(default)]
pub(crate) struct SpawnConfig {
    pub id: Option<String>,
    pub max_alive: u16,
    pub respawn_seconds: u64,
    pub pool: Option<String>,
    pub max_total: Option<u16>,
    pub movement: Movement,
}
impl Default for SpawnConfig {
    fn default() -> Self {
        Self {
            id: None,
            max_alive: 1,
            respawn_seconds: 60,
            pool: None,
            max_total: None,
            movement: Movement::default(),
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(default)]
pub(crate) struct Movement {
    pub mode: Mode,
    pub interval_seconds: u64,
    pub max_distance: Option<u16>,
    pub same_area: bool,
    pub target: Option<String>,
}
impl Default for Movement {
    fn default() -> Self {
        Self {
            mode: Mode::Stay,
            interval_seconds: 15,
            max_distance: None,
            same_area: true,
            target: None,
        }
    }
}
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    Stay,
    Wander,
    GuardRoom,
    GuardNpc,
}
#[derive(Resource, Deserialize, Default)]
#[serde(default)]
struct Settings {
    profiles: HashMap<String, SpawnConfig>,
}
/// Can be disabled by deterministic command tests; live games default to on.
#[derive(Resource)]
pub struct PopulationEnabled(pub bool);
impl Default for PopulationEnabled {
    fn default() -> Self {
        Self(true)
    }
}
#[derive(Component, Clone)]
pub(crate) struct Candidate {
    pub key: String,
    pub home: Entity,
    pub area: String,
    pub template: crate::seed::NpcBlueprint,
}
#[derive(Component, Clone)]
pub(crate) struct Member {
    pub point: String,
    pub home: Entity,
    pub next_move: f64,
}
struct Point {
    home: Entity,
    area: String,
    template: crate::seed::NpcBlueprint,
    config: SpawnConfig,
    members: HashSet<Entity>,
    pending: VecDeque<f64>,
}
#[derive(Resource, Default)]
struct Population {
    points: HashMap<String, Point>,
    cooldowns: HashMap<String, Vec<f64>>,
    next_tick: f64,
}

pub(crate) fn register(app: &mut App) {
    let path = app
        .world()
        .get_resource::<crate::seed::AreaBlueprintDir>()
        .map(|d| {
            d.0.parent()
                .unwrap_or(&d.0)
                .join("savage-soul/population.json")
        })
        .unwrap_or_else(|| "data/savage-soul/population.json".into());
    let settings = match std::fs::read(&path) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            warn!("Invalid NPC population settings: {e}");
            Settings::default()
        }),
        Err(_) => Settings::default(),
    };
    app.insert_resource(settings)
        .init_resource::<Population>()
        .init_resource::<PopulationEnabled>();
    app.add_observer(hold_combatants);
}
fn hold_combatants(
    mut walk: On<grim::AttemptWalk>,
    members: Query<&Member>,
    battles: Res<crate::savage_soul::Battles>,
    souls: Query<&crate::savage_rules::SoulData>,
) {
    if members.contains(walk.actor)
        && (battles.contains(walk.actor)
            || souls.get(walk.actor).is_ok_and(|s| s.is_incapacitated()))
    {
        walk.event_mut().denied = true;
    }
}
fn discover(world: &mut World, pop: &mut Population, now: f64) {
    let candidates: Vec<_> = world
        .query::<(Entity, &Candidate)>()
        .iter(world)
        .map(|(e, c)| (e, c.clone()))
        .collect();
    for (entity, c) in candidates {
        crate::medical::remember(world, &c.template);
        world.entity_mut(entity).remove::<Candidate>();
        let config = c.template.spawn.clone().or_else(|| {
            world
                .resource::<Settings>()
                .profiles
                .get(&c.template.name.to_ascii_lowercase())
                .cloned()
        });
        let Some(mut config) = config else {
            continue;
        };
        config.max_alive = config.max_alive.min(100);
        config.respawn_seconds = config.respawn_seconds.clamp(1, 86400);
        config.movement.interval_seconds = config.movement.interval_seconds.clamp(1, 3600);
        if let Some(limit) = config.max_total {
            config.max_total = Some(limit.min(1000));
        }
        let key = config.id.clone().unwrap_or(c.key);
        if pop.points.contains_key(&key) {
            warn!("Duplicate NPC spawn id {key}; skipping duplicate initial NPC");
            world.despawn(entity);
            continue;
        }
        let point = Point {
            home: c.home,
            area: c.area,
            template: c.template,
            config: config.clone(),
            members: HashSet::from([entity]),
            pending: VecDeque::new(),
        };
        info!(
            "NPC spawn registered: {} at {}, cap {}, delay {}s",
            point.template.name,
            world
                .get::<Room>(point.home)
                .map(|r| r.friendly_id.as_str())
                .unwrap_or("missing"),
            config.max_alive,
            config.respawn_seconds
        );
        world.entity_mut(entity).insert(Member {
            point: key.clone(),
            home: c.home,
            next_move: now + config.movement.interval_seconds as f64,
        });
        pop.points.insert(key, point);
    }
}
fn pool_limit(pop: &Population, pool: &str) -> usize {
    pop.points
        .values()
        .filter(|p| p.config.pool.as_deref() == Some(pool))
        .filter_map(|p| p.config.max_total)
        .min()
        .unwrap_or(1000) as usize
}
fn pool_count(pop: &Population, pool: &str) -> usize {
    pop.points
        .values()
        .filter(|p| p.config.pool.as_deref() == Some(pool))
        .map(|p| p.members.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world() -> World {
        let mut w = World::new();
        w.init_resource::<Time>();
        w.init_resource::<Population>();
        w.init_resource::<Settings>();
        w.init_resource::<PopulationEnabled>();
        w.init_resource::<Messages<grim::events::EngineCommand>>();
        w
    }
    fn room(w: &mut World, area: Entity, name: &str) -> Entity {
        w.spawn((
            Room {
                id: grim::GrimId::new(),
                friendly_id: name.into(),
                name: name.into(),
                description: String::new(),
                area,
            },
            Exits::default(),
        ))
        .id()
    }
    fn link(w: &mut World, a: Entity, b: Entity) {
        w.get_mut::<Exits>(a)
            .unwrap()
            .exits
            .insert(Cardinal::East, b);
        w.get_mut::<Exits>(b)
            .unwrap()
            .exits
            .insert(Cardinal::West, a);
    }
    fn add(w: &mut World, home: Entity, key: &str, movement: Movement) -> Entity {
        let mut template:crate::seed::NpcBlueprint=serde_json::from_value(serde_json::json!({"name":"rat","description":["A rat."],"soul":{"size":0,"mass":0,"total_xp":0},"combat_behavior":{"xp_reward":5}})).unwrap();
        template.spawn = Some(SpawnConfig {
            max_alive: 1,
            respawn_seconds: 60,
            pool: Some("rats".into()),
            max_total: Some(2),
            movement,
            ..Default::default()
        });
        let entity = {
            let mut c = w.commands();
            crate::seed::spawn_npc(&mut c, "test", &template, home)
        };
        w.flush();
        w.entity_mut(entity).insert(Candidate {
            key: key.into(),
            home,
            area: "test".into(),
            template,
        });
        entity
    }
    fn advance(w: &mut World, seconds: u64) {
        w.resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(seconds));
        tick(w);
    }
    #[test]
    fn moved_rat_counts_toward_cap_and_death_replaces_once_at_home_after_sixty_seconds() {
        let mut w = world();
        let area = w.spawn_empty().id();
        let a = room(&mut w, area, "a");
        let b = room(&mut w, area, "b");
        link(&mut w, a, b);
        let rat = add(&mut w, a, "a-rat", Movement::default());
        let other = add(&mut w, b, "b-rat", Movement::default());
        tick(&mut w);
        w.get_mut::<InRoom>(rat).unwrap().room = b;
        advance(&mut w, 90);
        assert_eq!(
            w.query_filtered::<Entity, With<Creature>>()
                .iter(&w)
                .count(),
            2
        );
        assert_eq!(w.get::<Member>(rat).unwrap().home, a);
        // A corpse still exists, but it no longer occupies a living spawn slot.
        w.entity_mut(rat).remove::<Creature>().insert(grim::Object);
        tick(&mut w);
        advance(&mut w, 1); // the one-second sweep notices death here
        advance(&mut w, 59);
        assert_eq!(
            w.query_filtered::<Entity, With<Creature>>()
                .iter(&w)
                .count(),
            1
        );
        advance(&mut w, 1);
        assert_eq!(
            w.query_filtered::<Entity, With<Creature>>()
                .iter(&w)
                .count(),
            2
        );
        let replacement = *w.resource::<Population>().points["a-rat"]
            .members
            .iter()
            .next()
            .unwrap();
        assert_ne!(replacement, rat);
        assert_eq!(w.get::<InRoom>(replacement).unwrap().room, a);
        assert_eq!(
            w.get::<crate::savage_rules::SoulData>(replacement)
                .unwrap()
                .size,
            0
        );
        assert_eq!(
            w.get::<crate::savage_soul::NpcCombatBehavior>(replacement)
                .unwrap()
                .xp_reward,
            5
        );
        assert!(w.get::<Creature>(other).is_some());
        advance(&mut w, 600);
        assert_eq!(
            w.query_filtered::<Entity, With<Creature>>()
                .iter(&w)
                .count(),
            2
        );
    }
    #[test]
    fn shared_pool_never_exceeds_cap_and_vacancy_waits_for_cooldown() {
        let mut w = world();
        let area = w.spawn_empty().id();
        let home = room(&mut w, area, "home");
        let first = add(&mut w, home, "first", Movement::default());
        add(&mut w, home, "second", Movement::default());
        add(&mut w, home, "third", Movement::default());
        tick(&mut w);
        assert_eq!(
            w.query_filtered::<Entity, With<Creature>>()
                .iter(&w)
                .count(),
            2
        );
        w.despawn(first);
        advance(&mut w, 1);
        advance(&mut w, 59);
        assert_eq!(
            w.query_filtered::<Entity, With<Creature>>()
                .iter(&w)
                .count(),
            1
        );
        advance(&mut w, 1);
        assert_eq!(
            w.query_filtered::<Entity, With<Creature>>()
                .iter(&w)
                .count(),
            2
        );
        advance(&mut w, 600);
        assert_eq!(
            w.query_filtered::<Entity, With<Creature>>()
                .iter(&w)
                .count(),
            2
        );
    }
    #[test]
    fn wander_obeys_radius_area_and_closed_doors() {
        let mut w = world();
        let area = w.spawn_empty().id();
        let foreign = w.spawn_empty().id();
        let a = room(&mut w, area, "a");
        let b = room(&mut w, area, "b");
        let c = room(&mut w, area, "c");
        let d = room(&mut w, area, "d");
        let z = room(&mut w, foreign, "z");
        link(&mut w, a, b);
        link(&mut w, b, c);
        link(&mut w, c, d);
        w.get_mut::<Exits>(c)
            .unwrap()
            .exits
            .insert(Cardinal::North, z);
        let movement = Movement {
            mode: Mode::Wander,
            interval_seconds: 1,
            max_distance: Some(2),
            ..Default::default()
        };
        let rat = add(&mut w, a, "rat", movement.clone());
        tick(&mut w);
        w.get_mut::<InRoom>(rat).unwrap().room = c;
        let allowed = allowed_rooms(&w, a, &movement);
        assert!(allowed.contains(&c));
        assert!(!allowed.contains(&d));
        assert!(!allowed.contains(&z));
        advance(&mut w, 1);
        let mut cursor =
            bevy::ecs::message::MessageCursor::<grim::events::EngineCommand>::default();
        assert!(cursor
            .read(w.resource::<Messages<grim::events::EngineCommand>>())
            .any(|m| m.client == rat
                && m.command
                    == grim::events::Command::Move {
                        direction: Cardinal::West
                    }));
        w.entity_mut(c).insert(Doors {
            doors: HashMap::from([(
                Cardinal::West,
                grim::Door {
                    name: "gate".into(),
                    keywords: vec![],
                    open: false,
                    hidden: false,
                },
            )]),
        });
        assert!(edges(&w, c).iter().all(|(d, _)| *d != Cardinal::West));
    }
    #[test]
    fn guard_objectives_route_to_room_or_unique_npc_without_teleporting() {
        let mut w = world();
        let area = w.spawn_empty().id();
        let a = room(&mut w, area, "a");
        let b = room(&mut w, area, "b");
        link(&mut w, a, b);
        let guard = add(
            &mut w,
            a,
            "guard",
            Movement {
                mode: Mode::GuardRoom,
                target: Some("b".into()),
                interval_seconds: 1,
                ..Default::default()
            },
        );
        tick(&mut w);
        advance(&mut w, 1);
        let mut cursor =
            bevy::ecs::message::MessageCursor::<grim::events::EngineCommand>::default();
        assert!(cursor
            .read(w.resource::<Messages<grim::events::EngineCommand>>())
            .any(|m| m.client == guard
                && m.command
                    == grim::events::Command::Move {
                        direction: Cardinal::East
                    }));
        assert_eq!(w.get::<InRoom>(guard).unwrap().room, a); // movement is queued through the normal engine
        w.spawn((Creature, GrimName("leader".into()), InRoom { room: b }));
        w.resource_mut::<Population>()
            .points
            .get_mut("guard")
            .unwrap()
            .config
            .movement = Movement {
            mode: Mode::GuardNpc,
            target: Some("leader".into()),
            interval_seconds: 1,
            ..Default::default()
        };
        advance(&mut w, 1);
        assert!(cursor
            .read(w.resource::<Messages<grim::events::EngineCommand>>())
            .any(|m| m.client == guard
                && m.command
                    == grim::events::Command::Move {
                        direction: Cardinal::East
                    }));
    }
}

//! World seeding from **area blueprints**.
//!
//! Area/room definitions live in `data/areas/*.json` (committed world content),
//! not in code. Each file is one area blueprint plus a `rooms` array of room
//! blueprints. They are read **from the filesystem at startup**, so a MUD author
//! can edit an area's JSON and restart without recompiling. The directory is
//! [`AreaBlueprintDir`] (default `data/areas`, resolved against the process's
//! working directory — `/opt/grim` in production, where the deploy ships the
//! `data/areas` folder alongside the binary).
//!
//! On startup every blueprint with `"canonical": true` is loaded — `canonical`
//! is, for now, simply the flag that gates startup loading (see
//! `docs/adr/0001-area-identity-and-instancing.md`).
//!
//! Every area and room carries a stable [`GrimId`] (`id`) in the file, and all
//! references — a room's `exits` and the area's `starting_room` — point at those
//! **ids**, never slugs. Slugs are only a human alias, so renaming a room's slug
//! never breaks a link.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use bevy::log::{error, info, warn};
use bevy::prelude::*;
use grim::prelude::{
    compile, Actor, Area, Cardinal, CompiledTrigger, Creature, Description, Doors, Exits, Gender,
    GrimId, InRoom, Keywords, Name as GrimName, Object, Room, RoomDescription, ScriptTriggers,
    StartingRoom, TriggerDef,
};
use serde::Deserialize;

/// Directory the area blueprints (`*.json`) are read from at startup. Resolved
/// against the process working directory when relative. Insert this resource
/// before [`seed_world`] runs to override the default (`data/areas`).
#[derive(Resource, Clone, Debug)]
pub struct AreaBlueprintDir(pub PathBuf);

impl Default for AreaBlueprintDir {
    fn default() -> Self {
        Self(PathBuf::from("data/areas"))
    }
}

/// An area definition on disk: the area itself plus its rooms.
#[derive(Deserialize)]
struct AreaBlueprint {
    /// Stable Grim ID — the identity that references point at.
    id: GrimId,
    slug: String,
    name: String,
    /// Only `canonical` areas are stamped into the world at startup.
    #[serde(default)]
    canonical: bool,
    /// Grim ID of the room new characters (and the ultimate fallback) start in.
    #[serde(default)]
    starting_room: Option<GrimId>,
    #[serde(default)]
    rooms: Vec<RoomBlueprint>,
}

/// A room definition within an [`AreaBlueprint`].
#[derive(Deserialize)]
struct RoomBlueprint {
    /// Stable Grim ID — what `exits`/`starting_room` reference.
    id: GrimId,
    slug: String,
    name: String,
    description: String,
    #[serde(default)]
    dark: bool,
    /// direction name (`"north"`) -> destination room **Grim ID** (not slug),
    /// so renaming a room's slug never breaks the link.
    #[serde(default)]
    exits: HashMap<String, GrimId>,
    /// direction name (`"east"`) -> the door guarding that exit. `open`
    /// defaults to closed; the privy omits it.
    #[serde(default)]
    pub(crate) doors: HashMap<String, crate::seed_doors::DoorBlueprint>,
    #[serde(default)]
    npcs: Vec<NpcBlueprint>,
    #[serde(default)]
    objects: Vec<ObjectBlueprint>,
    /// Inspectable room fixtures, deliberately not pickable objects.
    #[serde(default)]
    scenery: Vec<ObjectBlueprint>,
}

/// A non-player character placed in a room.
#[derive(Clone,Deserialize)]
pub(crate) struct NpcBlueprint {
    #[serde(default)]
    pub(crate) spawn: Option<crate::population::SpawnConfig>,
    #[serde(default)]
    combat_behavior: Option<crate::savage_soul::NpcCombatBehavior>,
    #[serde(default)]
    soul: Option<crate::savage_rules::SoulData>,
    #[serde(default)]
    shop: Option<crate::shop::Merchant>,
    #[serde(default)]
    race: String,
    #[serde(default = "default_npc_level")]
    level: u32,
    #[serde(default)]
    gender: String,
    pub(crate) name: String,
    /// Look paragraphs (`look <name>` shows them newline-joined).
    description: Vec<String>,
    /// Extra `look <keyword>` words (matched case-insensitively).
    #[serde(default)]
    keywords: Vec<String>,
    /// Room-listing line shown under the room description. Empty falls back
    /// to `"<name> is here."`.
    #[serde(default)]
    room_description: String,
    /// Scripted reactions (`{on, script}` with inline Lua). A script that
    /// fails to compile is logged and skipped — the mob still spawns.
    #[serde(default)]
    triggers: Vec<TriggerDef>,
}

fn default_npc_level() -> u32 { 1 }

/// A pickable object placed in a room.
#[derive(Deserialize)]
struct ObjectBlueprint {
    #[serde(default)]
    hackable: Option<crate::hack::Hackable>,
    #[serde(default)]
    condition: Option<grim::components::ItemCondition>,
    /// Short name: inventory rows, pickup lines, `get`/`drop` name matching.
    name: String,
    /// Look paragraphs (`look <name>` shows them newline-joined).
    description: Vec<String>,
    /// Extra `get <keyword>` words (matched case-insensitively).
    #[serde(default)]
    keywords: Vec<String>,
    /// Long room-listing line shown under the creatures. Empty falls back to
    /// the short name.
    #[serde(default)]
    room_description: String,
}

/// Seed the initial world by reading area blueprints from [`AreaBlueprintDir`]
/// at startup. Loads every `canonical` area (in sorted filename order for
/// determinism), then sets [`StartingRoom`] from the first area that names a
/// resolvable one. Called once at startup.
pub fn seed_world(mut commands: Commands, dir: Option<Res<AreaBlueprintDir>>) {
    let dir = dir
        .map(|d| d.0.clone())
        .unwrap_or_else(|| AreaBlueprintDir::default().0);

    // Load every canonical blueprint first so exits can resolve across areas
    // (ADR-0001: cross-area links bind Canonical → Canonical). Rooms are
    // stamped before any exit is wired, so an exit may target any loaded area.
    let blueprints = load_canonical_blueprints(&dir);

    // Pass 1: stamp every area and room, recording Grim ID -> entity globally.
    let mut room_ents: HashMap<GrimId, Entity> = HashMap::new();
    let mut pending: Vec<AreaBlueprint> = Vec::new();
    for bp in blueprints {
        if stamp_area_rooms(&mut commands, &bp, &mut room_ents).is_some() {
            pending.push(bp);
        }
    }

    // Pass 2: wire exits (by Grim ID, across all stamped areas) and place
    // NPCs/objects. A target that resolves to no stamped room is a dangling
    // exit: log and skip it, never fail startup over one bad link.
    for bp in &pending {
        wire_area_contents(&mut commands, bp, &room_ents);
    }
    let starting = pending
        .iter()
        .find_map(|bp| bp.starting_room.and_then(|id| room_ents.get(&id).copied()));

    match starting {
        Some(room) => commands.insert_resource(StartingRoom(room)),
        // Fail fast, and loudly. The scene systems take `Res<StartingRoom>`, so a
        // missing resource would otherwise panic cryptically on the first tick.
        // A MUD with no reachable world cannot serve logins — surface the real
        // cause at startup instead.
        None => panic!(
            "no starting room resolved from area blueprints in {dir:?}: need at least one \
             `canonical` area declaring a resolvable `starting_room`"
        ),
    }
}

/// Read every `*.json` blueprint in `dir` (sorted filenames for determinism),
/// returning the `canonical` ones. Unreadable or unparseable files log and skip.
fn load_canonical_blueprints(dir: &PathBuf) -> Vec<AreaBlueprint> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| match entry {
                Ok(e) => Some(e.path()),
                Err(e) => {
                    error!("skipping unreadable entry in area dir {dir:?}: {e}");
                    None
                }
            })
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
            .collect(),
        Err(e) => {
            error!("cannot read area blueprint dir {dir:?}: {e} — no areas loaded");
            Vec::new()
        }
    };
    files.sort(); // deterministic load order

    let mut blueprints = Vec::new();
    for path in &files {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                error!("cannot read area blueprint {path:?}: {e} — skipping");
                continue;
            }
        };
        let blueprint: AreaBlueprint = match serde_json::from_str(&raw) {
            Ok(bp) => bp,
            Err(e) => {
                error!("skipping unparseable area blueprint {path:?}: {e}");
                continue;
            }
        };
        // `canonical` gates startup loading (instancing is not built yet).
        if !blueprint.canonical {
            continue;
        }
        blueprints.push(blueprint);
    }
    blueprints
}

/// Stamp one area and its rooms, recording Grim ID -> entity in `room_ents`.
/// A duplicate room id — within the area or across already-stamped areas —
/// would leave one room spawned but unreferenced, so it rejects the whole
/// area instead of a half-built one. Returns the area entity.
fn stamp_area_rooms(
    commands: &mut Commands,
    bp: &AreaBlueprint,
    room_ents: &mut HashMap<GrimId, Entity>,
) -> Option<Entity> {
    let mut seen = HashSet::new();
    for r in &bp.rooms {
        if !seen.insert(r.id) {
            error!(
                "area '{}': duplicate room id {} — skipping area",
                bp.slug, r.id
            );
            return None;
        }
        if room_ents.contains_key(&r.id) {
            error!(
                "area '{}': room id {} already stamped by another area — skipping area",
                bp.slug, r.id
            );
            return None;
        }
    }

    let area = commands
        .spawn(Area {
            id: bp.id,
            friendly_id: bp.slug.clone(),
            name: bp.name.clone(),
        })
        .id();
    for r in &bp.rooms {
        let entity = commands
            .spawn((
                Room {
                    id: r.id,
                    friendly_id: r.slug.clone(),
                    name: r.name.clone(),
                    description: r.description.clone(),
                    area,
                },
                GrimName(r.name.clone()),
                Exits::default(),
            ))
            .id();
        if r.dark {commands.entity(entity).insert(grim::RoomDarkness);}
        room_ents.insert(r.id, entity);
    }
    Some(area)
}

/// Wire one stamped area's exits against the global room map and place its
/// NPCs and objects.
fn wire_area_contents(
    commands: &mut Commands,
    bp: &AreaBlueprint,
    room_ents: &HashMap<GrimId, Entity>,
) {
    for r in &bp.rooms {
        let Some(&from) = room_ents.get(&r.id) else {
            continue;
        };
        commands.entity(from).insert(Exits {
            exits: resolve_exits(&bp.slug, &r.slug, &r.exits, room_ents),
        });
        commands.entity(from).insert(Doors {
            doors: crate::seed_doors::resolve_doors(&bp.slug, &r.slug, &r.doors),
        });

        for (index,npc) in r.npcs.iter().enumerate() {
            let entity=spawn_npc(commands, &bp.slug, npc, from);
            commands.entity(entity).insert(crate::population::Candidate{
                key:format!("{}:{}:{index}",bp.id,r.id),home:from,area:bp.slug.clone(),template:npc.clone(),
            });
        }
        for obj in &r.objects {
            commands.spawn((
                Object,
                GrimName(obj.name.clone()),
                Description(obj.description.clone()),
                Keywords(obj.keywords.clone()),
                RoomDescription(obj.room_description.clone()),
                obj.condition.clone().unwrap_or_default(),
                InRoom { room: from },
            ));
        }
        for fixture in &r.scenery {
            let mut entity = commands.spawn((
                GrimName(fixture.name.clone()),
                Description(fixture.description.clone()),
                Keywords(fixture.keywords.clone()),
                InRoom { room: from },
            ));
            if let Some(hackable) = &fixture.hackable { entity.insert(hackable.clone()); }
        }
    }
}

/// Resolve one room's exits (direction name -> destination Grim ID) to room
/// entities. Bad directions and unknown targets log and skip — a dangling
/// exit never fails startup.
fn resolve_exits(
    area_slug: &str,
    room_slug: &str,
    exits: &HashMap<String, GrimId>,
    room_ents: &HashMap<GrimId, Entity>,
) -> HashMap<Cardinal, Entity> {
    let mut resolved = HashMap::new();
    for (dir, target) in exits {
        let Some(cardinal) = Cardinal::parse(dir) else {
            warn!("area '{area_slug}' room '{room_slug}': bad exit direction '{dir}'");
            continue;
        };
        match room_ents.get(target) {
            Some(&to) => {
                resolved.insert(cardinal, to);
            }
            None => warn!(
                "area '{area_slug}' room '{room_slug}': exit '{dir}' -> unknown room id {target}, skipped"
            ),
        }
    }
    resolved
}

/// Stamp one NPC blueprint into `room`: the being bundle plus its compiled
/// script triggers. A trigger that fails to compile is logged and skipped —
/// the mob still spawns, triggerless for that moment.
pub(crate) fn spawn_npc(commands: &mut Commands, area_slug: &str, npc: &NpcBlueprint, room: Entity)->Entity {
    // Compile each trigger once now: a typo fails loudly at startup (logged,
    // trigger skipped) instead of on a player's move.
    let mut compiled = Vec::with_capacity(npc.triggers.len());
    for def in &npc.triggers {
        match compile(&def.script) {
            Ok(bytecode) => compiled.push(CompiledTrigger {
                on: def.on,
                bytecode,
            }),
            Err(error) => error!(
                "area '{area_slug}' npc '{}': skipping trigger that does not compile: {error}",
                npc.name
            ),
        }
    }
    let mut mob = commands.spawn((
        Creature,
        // Seeded mobs carry the shared `Actor` base with sensible
        // defaults (no race/build data in blueprints yet): empty race,
        // level 1, neutral gender.
        Actor {
            race: npc.race.clone(),
            level: npc.level,
            gender: match npc.gender.as_str() {
                "male" => Gender::Male,
                "female" => Gender::Female,
                _ => Gender::Neutral,
            },
        },
        GrimName(npc.name.clone()),
        Description(npc.description.clone()),
        Keywords(npc.keywords.clone()),
        RoomDescription(npc.room_description.clone()),
        InRoom { room },
    ));
    if let Some(shop) = &npc.shop { mob.insert(shop.clone()); }
    if let Some(soul) = &npc.soul { mob.insert(soul.clone()); }
    if let Some(behavior) = &npc.combat_behavior { mob.insert(behavior.clone()); }
    if !compiled.is_empty() {
        info!(
            "area '{area_slug}' npc '{}': {} script trigger(s) loaded",
            npc.name,
            compiled.len()
        );
        mob.insert(ScriptTriggers(compiled));
    }
    mob.id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim::prelude::{CarriedBy, TriggerKind};

    /// The repo's committed area blueprints, resolved from this crate's manifest
    /// dir so the test works regardless of the process working directory.
    fn areas_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/areas")
    }

    #[test]
    fn haven_blueprint_parses_and_references_by_grim_id() {
        let raw = std::fs::read_to_string(areas_dir().join("haven.json")).unwrap();
        let bp: AreaBlueprint = serde_json::from_str(&raw).unwrap();
        assert_eq!(bp.slug, "haven");
        assert!(bp.canonical);
        assert_eq!(bp.rooms.len(), 14);

        let tavern = &bp.rooms[0];
        let square = &bp.rooms[1];
        assert_eq!(tavern.slug, "tavern");
        // starting_room references the tavern by Grim ID, not slug.
        assert_eq!(bp.starting_room, None);
        // The tavern's north exit references the square by Grim ID.
        assert_eq!(tavern.exits.get("north"), Some(&square.id));
        // The privy hangs east of the tavern behind a closed door; the
        // tavern's north door starts open (listed under Exits, not Doors).
        let privy = bp
            .rooms
            .iter()
            .find(|r| r.slug == "privy")
            .expect("privy room");
        assert_eq!(tavern.exits.get("east"), Some(&privy.id));
        let door = tavern.doors.get("east").expect("tavern east door");
        assert_eq!(door.name, "the privy door");
        assert!(!door.open);
        let back = privy.doors.get("west").expect("privy west door");
        assert_eq!(back.name, "the privy door");
        assert!(!back.open);
        let tavern_door = tavern.doors.get("north").expect("tavern north door");
        assert_eq!(tavern_door.name, "the tavern door");
        assert!(tavern_door.open);
        let square_back = square.doors.get("south").expect("square south door");
        assert_eq!(square_back.name, "the tavern door");
        assert!(square_back.open);
        // The loft hangs above the tavern on a plain exit; the cellar hangs
        // below behind a one-way secret (hidden from the tavern side only,
        // closed on both sides).
        let loft = bp
            .rooms
            .iter()
            .find(|r| r.slug == "tavern-loft")
            .expect("loft room");
        assert_eq!(tavern.exits.get("up"), Some(&loft.id));
        assert_eq!(loft.exits.get("down"), Some(&tavern.id));
        assert!(!tavern.doors.contains_key("up"));
        let cellar = bp
            .rooms
            .iter()
            .find(|r| r.slug == "cellar")
            .expect("cellar room");
        assert_eq!(tavern.exits.get("down"), Some(&cellar.id));
        assert_eq!(cellar.exits.get("up"), Some(&tavern.id));
        let secret = tavern.doors.get("down").expect("tavern cellar door");
        assert_eq!(secret.name, "the cellar door");
        assert!(!secret.open);
        assert!(secret.hidden);
        let back = cellar.doors.get("up").expect("cellar stair door");
        assert_eq!(back.name, "the cellar door");
        assert!(!back.open);
        assert!(!back.hidden);
    }

    #[test]
    fn seed_spawns_areas_rooms_exits_and_starting_room() {
        let mut app = App::new();
        app.insert_resource(AreaBlueprintDir(areas_dir()));
        app.add_systems(Startup, seed_world);
        app.update();

        // Two areas: Haven (14 rooms) plus Whisperwood (4 rooms).
        let areas = app.world_mut().query::<&Area>().iter(app.world()).count();
        assert_eq!(areas, 10);
        let rooms: Vec<String> = app
            .world_mut()
            .query::<&Room>()
            .iter(app.world())
            .map(|r| r.friendly_id.clone())
            .collect();
        assert_eq!(rooms.len(), 801);
        assert!(rooms.contains(&"tavern".to_string()));
        assert!(rooms.contains(&"bear-cavern".to_string()));

        // Starting room resolved and points at the tavern.
        let starting = app.world().resource::<StartingRoom>().0;
        let name = app
            .world()
            .get::<Room>(starting)
            .unwrap()
            .friendly_id
            .clone();
        assert_eq!(name, "start");

        // Tavern's north exit is wired to the square.
        let (tavern_entity, _) = app
            .world_mut()
            .query::<(Entity, &Room)>()
            .iter(app.world())
            .find(|(_, r)| r.friendly_id == "tavern")
            .unwrap();
        let exits = app.world().get::<Exits>(tavern_entity).unwrap();
        let north = exits.exits.get(&Cardinal::North).copied().unwrap();
        assert_eq!(
            app.world().get::<Room>(north).unwrap().friendly_id,
            "square"
        );
        // Cross-area exits are covered in `seed_wires_exits_across_areas`.

        // Both mobs are present: Grimmok in Haven, the bear in Whisperwood.
        let creatures = app
            .world_mut()
            .query::<&Creature>()
            .iter(app.world())
            .count();
        assert_eq!(creatures, 143);

        // Grimmok spawned with both greeting triggers compiled.
        let mut scripted = app.world_mut().query::<&ScriptTriggers>();
        let triggers = scripted.iter(app.world()).next().expect("Grimmok scripted");
        assert_eq!(triggers.0.len(), 2);
        assert!(triggers.0.iter().any(|t| t.on == TriggerKind::Enter));
        assert!(triggers.0.iter().any(|t| t.on == TriggerKind::AttemptLeave));

        // Grimmok carries its look paragraphs, keywords, and room line.
        let mut descs = app.world_mut().query::<(&GrimName, &Description)>();
        let (_, desc) = descs
            .iter(app.world())
            .find(|(name, _)| name.0 == "Grimmok Ironhand")
            .unwrap();
        assert_eq!(desc.0.len(), 2);
        let mut bear_rooms = app.world_mut().query::<(&GrimName, &InRoom)>();
        let (_, bear_room) = bear_rooms
            .iter(app.world())
            .find(|(name, _)| name.0 == "Old Cave Bear")
            .expect("bear spawned");
        assert_eq!(
            app.world().get::<Room>(bear_room.room).unwrap().friendly_id,
            "bear-cavern"
        );
        let mut keys = app.world_mut().query::<(&GrimName, &Keywords)>();
        assert!(keys
            .iter(app.world())
            .find(|(name, _)| name.0 == "Grimmok Ironhand")
            .unwrap()
            .1
             .0
            .contains(&"smith".to_string()));
        let mut lines = app.world_mut().query::<(&GrimName, &RoomDescription)>();
        assert_eq!(
            lines
                .iter(app.world())
                .find(|(name, _)| name.0 == "Grimmok Ironhand")
                .unwrap()
                .1
                 .0,
            "Grimmok Ironhand stands here, hammering metal."
        );
    }

    #[test]
    fn seed_wires_exits_across_areas() {
        let mut app = App::new();
        app.insert_resource(AreaBlueprintDir(areas_dir()));
        app.add_systems(Startup, seed_world);
        app.update();

        // Haven's east road reaches Whisperwood's forest edge, and back.
        let (road_entity, _) = app
            .world_mut()
            .query::<(Entity, &Room)>()
            .iter(app.world())
            .find(|(_, r)| r.friendly_id == "east-road")
            .unwrap();
        let road_exits = app.world().get::<Exits>(road_entity).unwrap();
        let east = road_exits.exits.get(&Cardinal::East).copied().unwrap();
        assert_eq!(
            app.world().get::<Room>(east).unwrap().friendly_id,
            "forest-edge"
        );
        let edge_exits = app.world().get::<Exits>(east).unwrap();
        let west = edge_exits.exits.get(&Cardinal::West).copied().unwrap();
        assert_eq!(
            app.world().get::<Room>(west).unwrap().friendly_id,
            "east-road"
        );
    }

    #[test]
    fn seed_spawns_blueprint_objects_in_their_room() {
        let mut app = App::new();
        app.insert_resource(AreaBlueprintDir(areas_dir()));
        app.add_systems(Startup, seed_world);
        app.update();

        // The tavern's brass lantern: short name, keywords, long room line,
        // placed in the tavern with no carrier.
        let mut objects = app.world_mut().query::<(
            Entity,
            &Object,
            &GrimName,
            &Keywords,
            &RoomDescription,
            &InRoom,
        )>();
        let found: Vec<(String, Vec<String>, String, Entity)> = objects
            .iter(app.world())
            .map(|(_, _, nm, kw, line, ir)| (nm.0.clone(), kw.0.clone(), line.0.clone(), ir.room))
            .collect();
        assert_eq!(found.len(), 1);
        let (name, keywords, line, room) = &found[0];
        assert_eq!(name, "brass lantern");
        assert!(keywords.contains(&"lantern".to_string()));
        assert_eq!(
            line,
            "A brass lantern rests here, its glass dusty but intact."
        );
        assert_eq!(
            app.world().get::<Room>(*room).unwrap().friendly_id,
            "tavern"
        );
        let mut carried = app.world_mut().query::<&CarriedBy>();
        assert!(carried.iter(app.world()).next().is_none());
    }

    #[test]
    #[should_panic(expected = "no starting room resolved")]
    fn seed_panics_when_no_starting_room() {
        // An empty area dir yields no canonical area, so no StartingRoom can be
        // resolved. Seeding must fail fast rather than leave the resource unset
        // (which would panic cryptically in the scene systems).
        let empty = std::env::temp_dir().join(format!("grim_seed_empty_{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        let mut app = App::new();
        app.insert_resource(AreaBlueprintDir(empty));
        app.add_systems(Startup, seed_world);
        app.update();
    }
}

/// Validate editor input with the same Rust DTOs the live seed loader uses.
/// This parses only; it never spawns entities or executes scripted triggers.
pub fn validate_area_json(text: &str) -> Result<(), String> {
    let area: AreaBlueprint = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut ids = std::collections::HashSet::new();
    let mut slugs = std::collections::HashSet::new();
    for room in &area.rooms {
        if !ids.insert(room.id) { return Err(format!("Duplicate room ID {}", room.id)); }
        if !slugs.insert(&room.slug) { return Err(format!("Duplicate room slug {}", room.slug)); }
    }
    Ok(())
}

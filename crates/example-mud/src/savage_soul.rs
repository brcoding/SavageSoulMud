//! Savage Soul game integration. See migration/SAVAGE-SOUL-IMPLEMENTATION.md.
use crate::preferences::{self, Detail};
use crate::savage_rules::{self as rules, AttackProfile, DamageType, SoulData};
use bevy::ecs::message::MessageCursor;
use bevy::prelude::*;
use grim::components::Name as GrimName;
use grim::events::{Command, EngineCommand, InfoMessage};
use grim::{Actor, CarriedBy, Character, Creature, InRoom, Object, Player, StoredCharacter};
use serde::Deserialize;
mod state;
pub(crate) use state::after_rebuild;
use state::{commit, hydrate};
mod commands;
use commands::{equipment_display, handle_commands};
mod narration;
use narration::avoided_strike;
mod combat;
use combat::{begin_battle, combat_end, tick_battles};
mod conditions;
use conditions::{critical_action, profile, tick_conditions, unix_now};

const STATE: &str = "savage_soul";
#[derive(Resource, Default)]
pub(crate) struct Battles(Vec<Battle>);
impl Battles {
    pub(crate) fn contains(&self, entity: Entity) -> bool {
        self.0.iter().any(|b| b.player == entity || b.npc == entity)
    }
}
struct Battle {
    player: Entity,
    npc: Entity,
    next_round: f64,
    rounds: u32,
    trample: bool,
    lethal: bool,
}
#[derive(Resource, Default)]
struct Resting(Vec<(Entity, Entity, f64)>);
#[derive(Resource)]
struct Dice;
impl Dice {
    fn roll(&mut self) -> i32 {
        rand::random_range(1..=10)
    }
}
#[derive(Clone, Deserialize)]
pub struct Equipment {
    pub name: String,
    pub keywords: Vec<String>,
    pub price: u64,
    pub sell_price: u64,
    pub weight_kg: f64,
    pub kind: String,
    #[serde(default)]
    pub damage_modifier: i32,
    #[serde(default)]
    pub attack_bonus: i32,
    #[serde(default)]
    pub sharp: bool,
    #[serde(default)]
    pub armor_bonus: i32,
    #[serde(default)]
    pub sharp_armor_bonus: i32,
    #[serde(default)]
    pub armor_coverage: i32,
    #[serde(default)]
    pub min_size: i32,
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
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<grim::StartingCredits>() {
            app.insert_resource(grim::StartingCredits(1000));
        }
        let dir = app
            .world()
            .get_resource::<crate::seed::AreaBlueprintDir>()
            .map(|d| d.0.parent().unwrap_or(&d.0).join("savage-soul/items.json"))
            .unwrap_or_else(|| "data/savage-soul/items.json".into());
        let catalog = std::fs::read(&dir)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        crate::game_commands::register(app);
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
        app.insert_resource(Dice)
            .insert_resource(EquipmentCatalog(catalog))
            .init_resource::<Battles>()
            .init_resource::<Resting>()
            .init_resource::<crate::royal_oak::Progress>()
            .add_systems(
                Update,
                (
                    hydrate,
                    handle_commands,
                    tick_battles,
                    tick_conditions,
                    crate::desk::tick_venom,
                    crate::royal_oak::tick,
                    crate::conditions::tick,
                    crate::population::tick,
                    crate::medical::tick,
                    crate::morlock::tick,
                )
                    .chain(),
            );
    }
}
fn reply(world: &mut World, actor: Entity, text: impl Into<String>) {
    world.write_message(InfoMessage {
        target: actor,
        text: text.into(),
    });
}
pub(crate) fn in_combat(world: &World, entity: Entity) -> bool {
    world
        .get_resource::<Battles>()
        .is_some_and(|b| b.contains(entity))
}

#[derive(Component)]
struct RewardClaimed;

#[cfg(test)]
mod retaliation_tests {
    use super::*;
    #[test]
    fn death_leaves_object_and_drops_held_items_once() {
        let mut world = World::new();
        let room = world.spawn_empty().id();
        let npc = world
            .spawn((
                Creature,
                Actor {
                    race: "rat".into(),
                    level: 1,
                    gender: grim::Gender::default(),
                },
                GrimName("rat".into()),
                InRoom { room },
                SoulData::default(),
                NpcCombatBehavior::default(),
            ))
            .id();
        let item = world
            .spawn((Object, GrimName("coin".into()), CarriedBy { carrier: npc }))
            .id();
        let dead = SoulData {
            injury: rules::InjuryCondition::Dead,
            ..Default::default()
        };
        commit(&mut world, npc, dead, None).unwrap();
        assert!(world.get::<Creature>(npc).is_none());
        assert!(world.get::<Actor>(npc).is_none());
        assert!(world.get::<SoulData>(npc).is_none());
        assert!(world.get::<NpcCombatBehavior>(npc).is_none());
        assert!(world.get::<Object>(npc).is_some());
        assert_eq!(world.get::<GrimName>(npc).unwrap().0, "corpse of the rat");
        assert_eq!(world.get::<InRoom>(item).unwrap().room, room);
        assert!(world.get::<CarriedBy>(item).is_none());
        assert_eq!(
            world
                .query_filtered::<Entity, With<Object>>()
                .iter(&world)
                .count(),
            2
        );
    }
    #[test]
    fn slower_trample_target_can_strike_without_gaining_a_second_attack() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .insert_resource(Dice)
            .init_resource::<Battles>()
            .add_message::<InfoMessage>();
        let world = app.world_mut();
        let room = world.spawn_empty().id();
        let connection = world.spawn_empty().id();
        // High resistance keeps both combatants able to act for every round.
        let mut pc = SoulData::default();
        pc.attribute_modifiers.insert("leadership".into(), 100);
        pc.attribute_modifiers.insert("body".into(), 100);
        let player = world
            .spawn((
                Player { connection },
                GrimName("Hunter".into()),
                InRoom { room },
                pc,
            ))
            .id();
        let mut rat = SoulData {
            size: 0,
            mass: 0,
            total_xp: 0,
            defense_mode: "none".into(),
            ..Default::default()
        };
        rat.attribute_modifiers.insert("leadership".into(), -100);
        rat.attribute_modifiers.insert("melee_attack".into(), 100);
        rat.attribute_modifiers.insert("body".into(), 100);
        let npc = world
            .spawn((Creature, GrimName("rat".into()), InRoom { room }, rat))
            .id();
        world.resource_mut::<Battles>().0.push(Battle {
            player,
            npc,
            next_round: 0.,
            rounds: 0,
            trample: true,
            lethal: true,
        });
        let mut cursor = MessageCursor::<InfoMessage>::default();
        let mut counterattacks = 0;
        for _ in 0..16 {
            tick_battles(world);
            let text = cursor
                .read(world.resource::<Messages<InfoMessage>>())
                .map(|m| m.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let counters = text.matches("Rat counterattacks").count();
            let ordinary = text.matches("Rat hits").count() + text.matches("Rat misses").count();
            assert_eq!(
                counters + ordinary,
                1,
                "Exactly one rat attack each round: {text}"
            );
            if counters > 0 {
                assert!(
                    text.contains("You trample rat") || text.contains("dodges your trample"),
                    "Active defense must not cancel the declared movement: {text}"
                );
            }
            counterattacks += counters;
            world
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs(5));
        }
        assert!(
            counterattacks > 0,
            "A ready target must be able to interrupt trample movement"
        );
    }
}

#[cfg(test)]
mod defense_narration_tests {
    use super::*;
    fn avoided(active: bool, total: i32) -> rules::AttackOutcome {
        rules::AttackOutcome {
            attack_total: total,
            defense_total: 12,
            success_margin: 0,
            total_damage: 0,
            wound_level: 0,
            hit: false,
            defense_used: active,
            injury_before: rules::InjuryCondition::Uninjured,
            injury_after: rules::InjuryCondition::Uninjured,
            stunned: false,
            unconscious: false,
        }
    }
    #[test]
    fn narration_matches_actual_defense_and_equipment() {
        let mut defender = SoulData {
            weapon: "Light Sword".into(),
            ..Default::default()
        };
        assert_eq!(
            avoided_strike(&avoided(true, 11), &defender, true, "artrell", 0),
            "Artrell sidesteps your strike."
        );
        assert_eq!(
            avoided_strike(&avoided(false, 11), &defender, false, "artrell", 0),
            "You parry artrell's strike with Light Sword."
        );
        assert!(
            avoided_strike(&avoided(false, 11), &defender, true, "artrell", 1).contains("blocks")
        );
        defender.weapon.clear();
        for variant in 0..3 {
            let text = avoided_strike(&avoided(false, 11), &defender, false, "artrell", variant);
            assert!(
                !text.contains("parry") && !text.contains("block") && !text.contains("deflect")
            );
        }
        assert_eq!(
            avoided_strike(&avoided(false, 0), &defender, true, "artrell", 0),
            "Your strike goes wide."
        );
        defender.stunned = true;
        assert!(
            avoided_strike(&avoided(false, 11), &defender, false, "artrell", 0)
                .contains("falls short")
        );
        defender.stunned = false;
        defender.prone = true;
        assert!(
            avoided_strike(&avoided(true, 11), &defender, false, "artrell", 1)
                .contains("roll clear")
        );
    }
}

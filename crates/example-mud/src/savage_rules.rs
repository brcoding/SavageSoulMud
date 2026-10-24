//! Savage Soul core. References: rulebook §§2.2.2–2.2.3, 3.2.4–5,
//! 7.2–7.5, 7B.8.1, 9.12, 9.16, 9.30; Soulgen Char./Training.
//! Faith follows the rulebook: it cannot be independently trained, despite
//! the older Soulgen training formula permitting it.
//! Attribute cost uses the normative triangular formula, not the PDF's
//! erroneous level-7+ cumulative table. Peculiarity effects are supplied by
//! the integration through attribute_modifiers; merely owning a name grants
//! no invented effect. Dice injection leaves optional natural-10 rerolls
//! explicit; automatic combat chooses to keep a 10.
use bevy::prelude::Component;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
mod training;
pub use training::{attribute_cost, normalize_attribute, supported_peculiarities};
mod resolution;
pub use resolution::{apply_wound, resolve_attack, resolve_task, resolve_trample};

pub const ATTRIBUTES: [&str; 12] = [
    "body",
    "athletics",
    "craft",
    "defense",
    "intellect",
    "leadership",
    "melee_attack",
    "perception",
    "performance",
    "ranged_attack",
    "stealth",
    "will_power",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjuryCondition {
    #[default]
    Uninjured,
    BarelyInjured,
    Injured,
    SeriouslyInjured,
    CriticallyInjured,
    Dead,
    Destroyed,
    Obliterated,
}
impl InjuryCondition {
    pub fn penalty(self) -> i32 {
        match self {
            Self::Injured => 1,
            Self::SeriouslyInjured => 2,
            Self::CriticallyInjured => 3,
            _ => 0,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Uninjured => "Uninjured",
            Self::BarelyInjured => "Barely injured",
            Self::Injured => "Injured",
            Self::SeriouslyInjured => "Seriously injured",
            Self::CriticallyInjured => "Critically injured",
            Self::Dead => "Dead",
            Self::Destroyed => "Destroyed",
            Self::Obliterated => "Obliterated",
        }
    }
    pub fn is_dead(self) -> bool {
        matches!(self, Self::Dead | Self::Destroyed | Self::Obliterated)
    }
}
#[derive(Component, Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SoulData {
    pub species: Option<crate::species::SpeciesBuild>,
    #[serde(skip)]
    pub vision_penalty: i32,
    /// Nonrefundable XP spent on committed race changes; never buys capacity.
    pub rebuild_xp: i64,
    pub total_xp: i64,
    pub attribute_levels: BTreeMap<String, u32>,
    pub peculiarities: BTreeMap<String, u32>,
    pub peculiarity_xp: i64,
    pub attribute_modifiers: BTreeMap<String, i32>,
    pub size: i32,
    pub mass: i32,
    pub injury: InjuryCondition,
    pub stunned: bool,
    pub unconscious: bool,
    pub comatose: bool,
    pub prone: bool,
    pub winded: bool,
    pub fatigued: bool,
    pub mana: i32,
    pub armor_bonus: i32,
    pub sharp_armor_bonus: i32,
    pub armor_coverage: i32,
    pub weapon: String,
    pub armor: String,
    pub defense_mode: String,
    pub critical_since: Option<u64>,
    pub critical_checks: u8,
    pub stabilized: bool,
    pub load_kg: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct DerivedStats {
    pub strength: i32,
    pub passive_defense: i32,
    pub dodge: i32,
    pub quickness: i32,
    pub mana_max: i32,
    pub base_dr: i32,
    pub base_sharp_dr: i32,
    pub armor_dr: i32,
    pub armor_sharp_dr: i32,
    pub melee_dc: i32,
    pub injury_penalty: i32,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageType {
    #[default]
    Blunt,
    Sharp,
    Energy,
}
#[derive(Debug, Clone, Copy)]
pub struct AttackProfile {
    pub ranged: bool,
    pub damage_class: i32,
    pub attack_bonus: i32,
    pub damage_type: DamageType,
    pub armor_piercing: i32,
}
impl AttackProfile {
    pub fn unarmed(soul: &SoulData) -> Self {
        let trained = soul.peculiarity_level("unarmed fighter");
        let bonus = match trained {
            0 => 0,
            1 | 2 => 1,
            _ => 2,
        };
        let penalty = soul.injury.penalty() + i32::from(soul.winded) + i32::from(soul.fatigued);
        let blunt = soul.derived().strength - 1 - penalty
            + bonus
            + i32::from(soul.trait_level("special strike") > 0);
        let sharp =
            soul.derived().strength - 2 - penalty + bonus + soul.trait_level("sharp strike") as i32;
        let claws = soul.trait_level("sharp strike") > 0 && sharp >= blunt;
        Self {
            ranged: false,
            damage_class: if claws { sharp } else { blunt },
            attack_bonus: i32::from(trained >= 1),
            damage_type: if claws {
                DamageType::Sharp
            } else {
                DamageType::Blunt
            },
            armor_piercing: 0,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct TaskOutcome {
    pub total: i32,
    pub success: bool,
    pub margin: i32,
    pub critical_failure: bool,
    pub natural: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct AttackOutcome {
    pub attack_total: i32,
    pub defense_total: i32,
    pub success_margin: i32,
    pub total_damage: i32,
    pub wound_level: i32,
    pub hit: bool,
    pub defense_used: bool,
    pub injury_before: InjuryCondition,
    pub injury_after: InjuryCondition,
    pub stunned: bool,
    pub unconscious: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn load_and_prone_apply_specific_penalties() {
        let mut soul = SoulData {
            load_kg: 25.,
            ..SoulData::default()
        };
        assert_eq!(soul.load_penalty(), 2);
        assert_eq!(soul.attribute("defense"), 3);
        assert_eq!(soul.attribute("melee_attack"), 5);
        soul.prone = true;
        assert_eq!(soul.attribute("melee_attack"), 4);
        assert_eq!(soul.derived().dodge, 2);
        soul.load_kg = 60.;
        assert!(soul.is_helpless());
        assert!(!soul.can_move());
        assert_eq!(soul.derived().passive_defense, 7);
    }
    #[test]
    fn peculiarity_levels_spend_cs_and_apply_exact_bonuses() {
        let mut s = SoulData::default();
        for _ in 0..3 {
            assert!(s.learn("CS: Unarmed Fighter").is_ok());
        }
        assert_eq!(s.available_xp(), 600);
        assert_eq!(AttackProfile::unarmed(&s).damage_class, 6);
        assert_eq!(s.derived().dodge, 6);
        assert!(s.learn("quickness").is_ok());
        assert_eq!(s.available_xp(), 500);
        assert_eq!(s.derived().quickness, 6);
        assert!(s.train("faith").is_err());
    }
    #[test]
    fn xp_cost_and_cap() {
        let mut s = SoulData::default();
        assert!(s.train("body").is_ok());
        assert!(s.train("body").is_ok());
        assert_eq!(s.available_xp(), 300);
        assert!(s.train("body").is_err());
        assert_eq!(attribute_cost("athletics", 7), 1400);
    }
    #[test]
    fn double_one_and_defense_ten() {
        let mut rolls = [1, 2].into_iter();
        assert!(resolve_task(50, 10, false, || rolls.next().unwrap()).critical_failure);
        let a = SoulData::default();
        let mut d = a.clone();
        let mut rolls = [9, 10].into_iter();
        assert!(
            !resolve_attack(&a, &mut d, AttackProfile::unarmed(&a), true, || rolls
                .next()
                .unwrap())
            .hit
        );
    }
    #[test]
    fn cumulative_wounds_and_body_unpenalized() {
        let a = SoulData::default();
        let mut d = a.clone();
        d.injury = InjuryCondition::Injured;
        let profile = AttackProfile {
            damage_class: 8,
            ..AttackProfile::unarmed(&a)
        };
        let mut rolls = [7, 9].into_iter();
        let r = resolve_attack(&a, &mut d, profile, false, || rolls.next().unwrap());
        assert_eq!(r.wound_level, 5);
        assert_eq!(d.injury, InjuryCondition::SeriouslyInjured);
        assert_eq!(d.attribute("body"), 5);
        assert_eq!(d.attribute("melee_attack"), 3);
    }
}

#[cfg(test)]
mod trample_tests {
    use super::*;
    fn rat() -> SoulData {
        let mut r = SoulData {
            size: 0,
            mass: 0,
            total_xp: 0,
            ..Default::default()
        };
        r.attribute_modifiers.insert("body".into(), 1);
        r
    }
    #[test]
    fn trample_uses_mass_damage_and_knockdown_not_passive_defense() {
        let attacker = SoulData::default();
        let mut target = rat();
        let hit = resolve_trample(&attacker, &mut target, || 5);
        assert!(hit.hit);
        assert_eq!(hit.success_margin, 0);
        assert_eq!(hit.total_damage, 5);
        assert_eq!(hit.wound_level, 4);
        assert!(target.prone);
        assert!(hit.defense_used);
    }
    #[test]
    fn target_can_avoid_trample_with_desperation_dive() {
        let mut target = rat();
        let hit = resolve_trample(&SoulData::default(), &mut target, || 10);
        assert!(!hit.hit);
        assert_eq!(target.injury, InjuryCondition::Uninjured);
    }
    #[test]
    fn natural_ten_can_continue_when_the_first_total_cannot_hit() {
        let mut attacker = SoulData::default();
        attacker
            .attribute_modifiers
            .insert("melee_attack".into(), -3);
        let mut target = rat();
        let profile = AttackProfile::unarmed(&attacker);
        let hit = resolve_attack(&attacker, &mut target, profile, false, || 10);
        assert_eq!(hit.attack_total, 17);
        assert!(hit.hit);
    }
}

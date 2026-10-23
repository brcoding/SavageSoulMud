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
impl Default for SoulData {
    fn default() -> Self {
        Self {
            species: None,
            vision_penalty: 0,
            rebuild_xp: 0,
            total_xp: 600,
            attribute_levels: BTreeMap::new(),
            peculiarities: BTreeMap::new(),
            peculiarity_xp: 0,
            attribute_modifiers: BTreeMap::new(),
            size: 5,
            mass: 5,
            injury: InjuryCondition::Uninjured,
            stunned: false,
            unconscious: false,
            comatose: false,
            prone: false,
            winded: false,
            fatigued: false,
            mana: 10,
            armor_bonus: 0,
            sharp_armor_bonus: 0,
            armor_coverage: 0,
            weapon: String::new(),
            armor: String::new(),
            defense_mode: "auto".into(),
            critical_since: None,
            critical_checks: 0,
            stabilized: false,
            load_kg: 0.0,
        }
    }
}
pub fn normalize_attribute(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}
pub fn attribute_cost(name: &str, level: u32) -> i64 {
    let n = i64::from(level);
    let factor = if name == "body" { 100 } else { 50 };
    n.saturating_mul(n.saturating_add(1))
        .saturating_div(2)
        .saturating_mul(factor)
}
impl SoulData {
    pub fn free_combat_levels(&self) -> i64 {
        let xp = self.total_xp.max(0);
        if xp <= 1000 {
            xp / 200
        } else if xp <= 2500 {
            5 + (xp - 1000) / 300
        } else if xp <= 4500 {
            10 + (xp - 2500) / 400
        } else {
            15 + (xp - 4500) / 500
        }
    }
    pub fn learn(&mut self, name: &str) -> Result<String, String> {
        let name = name.trim().to_ascii_lowercase();
        let name = name
            .split_once(':')
            .map_or(name.as_str(), |(_, n)| n.trim());
        if !supported_peculiarities().contains(&name) {
            return Err("That Peculiarity is not mechanically implemented yet.".into());
        }
        if name=="quickness" && self.trait_level("no mutations")>0 {
            return Err("Your No Mutations disadvantage prevents learning this Mutation Peculiarity.".into());
        }
        let am = ["brainy", "brutish", "burly", "genteel"];
        if am.contains(&name)
            && am.iter().any(|key|self.peculiarity_level(key)>0)
        {
            return Err("Only one Attribute Modifier Peculiarity is allowed.".into());
        }
        let level = self.peculiarity_level(name) + 1;
        let max = if am.contains(&name) { 1 } else { 3 };
        if level > max {
            return Err("That Peculiarity is already at its maximum implemented level.".into());
        }
        let cost = if self.available_combat_levels()>0 {
            0
        } else {
            100
        };
        if cost > self.available_xp() {
            return Err("You need a free C or 100 unspent XP.".into());
        }
        self.peculiarity_xp += cost;
        self.peculiarities.insert(name.into(), level);
        let modifiers: &[(&str, i32)] = match name {
            "brainy" => &[("intellect", 1), ("strength", -1)],
            "brutish" => &[("strength", 1), ("intellect", -1)],
            "burly" => &[("body", 1), ("athletics", -1), ("defense", -1)],
            "genteel" => &[
                ("faith", 1),
                ("leadership", 1),
                ("performance", 1),
                ("body", -1),
            ],
            _ => &[],
        };
        for (key, value) in modifiers {
            *self.attribute_modifiers.entry((*key).into()).or_default() += value;
        }
        self.mana = self.mana.min(self.derived().mana_max);
        Ok(format!(
            "Learned {name} level {level} for 1 additional C{}.",
            if cost == 100 {
                " (purchased for 100 XP)"
            } else {
                ""
            }
        ))
    }
    pub fn peculiarity_level(&self, name: &str) -> u32 {
        self.peculiarities.get(name).copied().unwrap_or(0)+self.trait_level(name)
    }
    pub fn trait_level(&self,name:&str)->u32 {self.species.as_ref().map(|s|s.level(name)).unwrap_or(0)}
    pub fn used_combat_levels(&self)->i64 {self.peculiarities.values().map(|v|i64::from(*v)).sum::<i64>()+self.species.as_ref().map(|s|s.used_c()).unwrap_or(0)}
    pub fn available_combat_levels(&self)->i64 {self.free_combat_levels()+self.peculiarity_xp/100+self.species.as_ref().map(|s|s.bonus_c).unwrap_or(0)-self.used_combat_levels()}
    pub fn hard_to_kill(&self)->bool {self.trait_level("hard to kill")>0}
    pub fn needs_repair(&self)->bool {self.trait_level("heal harm as object")>0}
    pub fn species_modifier(&self,name:&str)->i32 {self.species.as_ref().map(|s|s.modifier(name)).unwrap_or(0)}
    pub fn available_xp(&self) -> i64 {
        self.attribute_levels.iter().fold(
            self.total_xp.saturating_sub(self.rebuild_xp).saturating_sub(self.peculiarity_xp),
            |xp, (name, n)| xp.saturating_sub(attribute_cost(name, *n)),
        )
    }
    pub fn base_attribute(&self, name: &str) -> i32 {
        let name = normalize_attribute(name);
        let mass = if name == "body" { self.mass - 5 } else { 0 };
        5 + mass
            + self
                .attribute_levels
                .get(&name)
                .copied()
                .unwrap_or(0)
                .min(i32::MAX as u32) as i32
            + self.attribute_modifiers.get(&name).copied().unwrap_or(0)
            + self.species_modifier(&name)
    }
    pub fn attribute(&self, name: &str) -> i32 {
        let name = normalize_attribute(name);
        let injury = if matches!(
            name.as_str(),
            "athletics"
                | "craft"
                | "defense"
                | "melee_attack"
                | "ranged_attack"
                | "stealth"
                | "will_power"
        ) {
            self.injury.penalty()
        } else {
            0
        };
        let fatigue = if matches!(name.as_str(), "body" | "faith" | "intellect") {
            0
        } else {
            i32::from(self.winded) + i32::from(self.fatigued)
        };
        let load = if matches!(name.as_str(), "athletics" | "craft" | "defense" | "stealth") {
            self.load_penalty()
        } else {
            0
        };
        let prone = i32::from(name == "melee_attack" && self.prone);
        let vision=if matches!(name.as_str(),"melee_attack"|"ranged_attack"|"defense"|"perception"){self.vision_penalty}else{0};
        self.base_attribute(&name) - injury - fatigue - load - prone - vision
    }
    pub fn train(&mut self, name: &str) -> Result<String, String> {
        let name = normalize_attribute(name);
        if name == "faith" {
            return Err("Faith cannot be independently trained; it changes through applicable Peculiarities and effects.".into());
        }
        if !ATTRIBUTES.contains(&name.as_str()) {
            return Err("Choose one of the listed attributes.".into());
        }
        let old = self.attribute_levels.get(&name).copied().unwrap_or(0);
        let next = old.checked_add(1).ok_or("Attribute level overflow.")?;
        let total = attribute_cost(&name, next);
        let cost = total - attribute_cost(&name, old);
        if total > self.total_xp / 2 {
            return Err("A single attribute cannot cost more than half your total XP.".into());
        }
        if cost > self.available_xp() {
            return Err(format!("You need {cost} unspent XP."));
        }
        self.attribute_levels.insert(name.clone(), next);
        self.mana = self.mana.min(self.derived().mana_max);
        Ok(format!(
            "{} is now {} (spent {cost} XP).",
            name.replace('_', " "),
            self.base_attribute(&name)
        ))
    }
    pub fn derived(&self) -> DerivedStats {
        let body = self.base_attribute("body");
        let penalty = self.injury.penalty() + i32::from(self.winded) + i32::from(self.fatigued);
        let strength = body + self.species_modifier("strength")
            + self
                .attribute_modifiers
                .get("strength")
                .copied()
                .unwrap_or(0);
        DerivedStats {
            strength,
            passive_defense: 15
                - self.size
                - if self.is_helpless() || self.unconscious || self.comatose {
                    3
                } else {
                    0
                },
            dodge: self.attribute("defense") + 5 - self.mass
                + i32::from(self.peculiarity_level("unarmed fighter") >= 2)
                - i32::from(self.prone),
            quickness: (self.attribute("athletics") + self.attribute("perception") + 1)
                .div_euclid(2)
                + match self.peculiarity_level("quickness") {
                    0 => 0,
                    1 => 1,
                    2 => 3,
                    _ => 5,
                },
            mana_max: if self.trait_level("no mana")>0 {0} else {self.base_attribute("intellect") + self.base_attribute("will_power")},
            base_dr: body + i32::from(self.trait_level("natural armor")==1),
            base_sharp_dr: body - if self.hard_to_kill(){0}else{2},
            armor_dr: body + self.effective_armor(false).0.max(i32::from(self.trait_level("natural armor")==1)),
            armor_sharp_dr: body - if self.hard_to_kill(){0}else{2} + self.effective_armor(true).0,
            melee_dc: strength - 1 - penalty,
            injury_penalty: self.injury.penalty(),
        }
    }
    /// Natural and worn armor never stack; use the stronger layer, then coverage.
    pub fn effective_armor(&self,sharp:bool)->(i32,i32) {
        let hard=if self.hard_to_kill()&&sharp {2}else{0};
        let worn=(if sharp {self.sharp_armor_bonus-hard}else{self.armor_bonus}, self.armor_coverage);
        let level=self.trait_level("natural armor");
        let natural=match level {
            2..=4=>((level as i32-1)*if sharp{2}else{1}-hard,6+(self.size-6).max(0)),
            _=>(0,0),
        };
        worn.max(natural).max((0,0))
    }
    pub fn is_incapacitated(&self) -> bool {
        self.injury.is_dead()
            || self.stunned
            || self.unconscious
            || self.comatose
            || self.is_helpless()
    }
    fn hindered_threshold(&self) -> f64 {
        const KG: [f64; 25] = [
            1., 2., 4., 6., 10., 15., 20., 30., 50., 75., 100., 150., 250., 350., 550., 800.,
            1200., 1800., 2500., 4000., 6000., 10000., 15000., 20000., 30000.,
        ];
        let strength = self.base_attribute("body") + self.species_modifier("strength")
            + self
                .attribute_modifiers
                .get("strength")
                .copied()
                .unwrap_or(0);
        if strength <= 0 {
            0.
        } else {
            KG[(strength as usize - 1).min(24)]
        }
    }
    pub fn load_penalty(&self) -> i32 {
        let h = self.hindered_threshold();
        if self.load_kg <= 0. {
            0
        } else if self.load_kg >= h * 4. {
            3
        } else if self.load_kg >= h * 2. {
            2
        } else if self.load_kg >= h {
            1
        } else {
            0
        }
    }
    pub fn is_helpless(&self) -> bool {
        self.load_kg > 0. && self.load_kg >= self.hindered_threshold() * 6.
    }
    pub fn load_label(&self) -> &'static str {
        if self.is_helpless() {
            "Helpless"
        } else {
            match self.load_penalty() {
                0 => "Unencumbered",
                1 => "Hindered",
                2 => "Encumbered",
                _ => "Heavily encumbered",
            }
        }
    }
    pub fn can_move(&self) -> bool {
        !self.is_incapacitated()
    }
    /// Explicit administrative/healing operation, never automatic between attacks.
    pub fn reset_conditions(&mut self) {
        self.injury = InjuryCondition::Uninjured;
        self.stunned = false;
        self.unconscious = false;
        self.comatose = false;
        self.prone = false;
        self.critical_since = None;
        self.critical_checks = 0;
        self.stabilized = false;
        self.winded = false;
        self.fatigued = false;
        self.mana = self.derived().mana_max;
    }
}
/// Appendix A2, all four one-level, 1C mutually exclusive modifiers.
pub fn supported_peculiarities() -> Vec<&'static str> {
    vec![
        "brainy",
        "brutish",
        "burly",
        "genteel",
        "unarmed fighter",
        "quickness",
    ]
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
        let trained=soul.peculiarity_level("unarmed fighter");
        let bonus=match trained {0=>0,1|2=>1,_=>2};
        let penalty=soul.injury.penalty()+i32::from(soul.winded)+i32::from(soul.fatigued);
        let blunt=soul.derived().strength-1-penalty+bonus+i32::from(soul.trait_level("special strike")>0);
        let sharp=soul.derived().strength-2-penalty+bonus+soul.trait_level("sharp strike") as i32;
        let claws=soul.trait_level("sharp strike")>0&&sharp>=blunt;
        Self {ranged:false,damage_class:if claws {sharp}else{blunt},attack_bonus:i32::from(trained>=1),damage_type:if claws {DamageType::Sharp}else{DamageType::Blunt},armor_piercing:0}
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
/// Ordinary task; `reroll_tens` is the caller's explicit optional choice.
pub fn resolve_task(
    bonus: i32,
    df: i32,
    reroll_tens: bool,
    mut dice: impl FnMut() -> i32,
) -> TaskOutcome {
    let natural = dice().clamp(1, 10);
    let mut total = bonus + natural;
    let mut critical = false;
    if natural == 1 {
        let d5 = (dice().clamp(1, 10) + 1) / 2;
        total = total - 5 + d5;
        critical = d5 == 1;
    } else if natural == 10 && reroll_tens {
        let mut roll = 10;
        while roll == 10 {
            roll = dice().clamp(1, 10);
            total = total - 5 + roll;
        }
    }
    let success = !critical && total >= df;
    TaskOutcome {
        total,
        success,
        margin: if success {
            total - df
        } else {
            (df - total).max(0)
        },
        critical_failure: critical || (!success && df - total >= 5),
        natural,
    }
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

/// Resolve an attack with optional Dodge and its immediate wound/resistance.
/// Active-defense natural 10 succeeds automatically; natural 1 contributes 0.
pub fn resolve_attack(
    attacker: &SoulData,
    defender: &mut SoulData,
    attack: AttackProfile,
    active_defense: bool,
    mut dice: impl FnMut() -> i32,
) -> AttackOutcome {
    let before = defender.injury;
    let mut d = defender.derived();
    if attack.ranged && defender.prone {
        d.passive_defense += 1;
    }
    let roll = dice().clamp(1, 10);
    let mut attack_total = if roll == 1 {
        0
    } else {
        attacker.attribute(if attack.ranged {
            "ranged_attack"
        } else {
            "melee_attack"
        }) + attack.attack_bonus
            + roll
    };
    if roll==10 && attack_total<d.passive_defense {
        let mut next=10;
        while next==10 && attack_total<d.passive_defense {
            next=dice().clamp(1,10);
            attack_total=attack_total-5+next;
        }
    }
    let mut outcome = AttackOutcome {
        attack_total,
        defense_total: d.passive_defense,
        success_margin: 0,
        total_damage: 0,
        wound_level: 0,
        hit: false,
        defense_used: false,
        injury_before: before,
        injury_after: before,
        stunned: defender.stunned,
        unconscious: defender.unconscious,
    };
    if roll == 1 || attack_total < d.passive_defense {
        return outcome;
    }
    if active_defense && !defender.is_incapacitated() {
        outcome.defense_used = true;
        let r = dice().clamp(1, 10);
        let total = if r == 1 { 0 } else { d.dodge + r };
        outcome.defense_total = total.max(d.passive_defense);
        if r == 10 || total >= attack_total {
            return outcome;
        }
    }
    let sm = attack_total - outcome.defense_total;
    finish_hit(defender, attack, outcome, sm, dice)
}

/// Damage from a completed hit; shared by strikes and automatic-SM tramples.
fn finish_hit(defender: &mut SoulData, attack: AttackProfile, mut outcome: AttackOutcome, sm:i32, mut dice:impl FnMut()->i32)->AttackOutcome {
    let d=defender.derived();
    outcome.hit = true;
    outcome.success_margin = sm;
    let sharp = matches!(attack.damage_type, DamageType::Sharp);
    let (armor,coverage)=defender.effective_armor(sharp);
    let ignored=armor<=0 || sm>coverage;
    let dr = if sharp {
        if ignored {
            d.base_sharp_dr
        } else {
            d.armor_sharp_dr
        }
    } else if ignored {
        d.base_dr
    } else {
        d.armor_dr
    };
    outcome.total_damage = attack.damage_class
        + sm.min(5)
        + if ignored {
            0
        } else {
            attack.armor_piercing.max(0).min(armor.max(0))
        };
    let mut wl = outcome.total_damage - dr;
    if sharp && ignored && wl > -5 {
        wl = wl.max(1);
    }
    outcome.wound_level = wl;
    if wl <= 0 {
        return outcome;
    }
    let column = match wl {
        1..=2 => 0,
        3..=4 => 1,
        5..=6 => 2,
        7..=8 => 3,
        9 => 4,
        10..=11 => 5,
        _ => 6,
    };
    apply_wound(defender, wl);
    if matches!(attack.damage_type, DamageType::Energy)
        && defender.injury == InjuryCondition::Destroyed
    {
        defender.injury = InjuryCondition::Obliterated;
    }
    let df = match column {
        0 => 6,
        1 => 7,
        2 => 8,
        3 => 9,
        _ => 10,
    };
    let resistance = if wl >= 10 {
        None
    } else {
        Some(resolve_task(
            defender.attribute("will_power"),
            df,
            false,
            &mut dice,
        ))
    };
    let failed = resistance.is_none_or(|r| !r.success);
    if resistance.is_some_and(|r| r.critical_failure) {
        defender.prone = true;
    }
    if failed {
        if wl >= 9 || defender.stunned {
            defender.unconscious = true;
            defender.prone = true;
            defender.stunned = false;
        } else {
            defender.stunned = true;
        }
    }
    if defender.unconscious && defender.injury == InjuryCondition::CriticallyInjured {
        defender.comatose = true;
    }
    outcome.injury_after = defender.injury;
    outcome.stunned = defender.stunned;
    outcome.unconscious = defender.unconscious;
    outcome
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

/// Apply a rulebook wound level without armor or a separate attack roll.
pub fn apply_wound(soul: &mut SoulData, wl: i32) {
    if wl <= 0 { return; }
    let column = match wl {
        1..=2 => 0,
        3..=4 => 1,
        5..=6 => 2,
        7..=8 => 3,
        9 => 4,
        10..=11 => 5,
        _ => 6,
    };
    // §7.4.5 cumulative injuries; discriminants correspond to these rows.
    const TABLE: [[usize; 7]; 8] = [
        [1, 1, 2, 3, 4, 5, 6],
        [1, 2, 2, 3, 4, 5, 6],
        [2, 3, 3, 4, 5, 5, 6],
        [3, 4, 4, 4, 5, 5, 6],
        [4, 5, 5, 5, 5, 5, 6],
        [5, 5, 5, 5, 5, 6, 6],
        [6, 6, 6, 6, 6, 6, 7],
        [7, 7, 7, 7, 7, 7, 7],
    ];
    const STATES: [InjuryCondition; 8] = [
        InjuryCondition::Uninjured,
        InjuryCondition::BarelyInjured,
        InjuryCondition::Injured,
        InjuryCondition::SeriouslyInjured,
        InjuryCondition::CriticallyInjured,
        InjuryCondition::Dead,
        InjuryCondition::Destroyed,
        InjuryCondition::Obliterated,
    ];
    soul.injury = if wl >= 16 {
        InjuryCondition::Obliterated
    } else {
        STATES[TABLE[soul.injury as usize][column]]
    };
}

/// Room-scale Trample: the caller checks grounded target and size eligibility.
/// The defender chooses Desperation Dive (Perception DF12), not normal Dodge.
pub fn resolve_trample(attacker:&SoulData, defender:&mut SoulData, mut dice:impl FnMut()->i32)->AttackOutcome {
    let before=defender.injury;
    let can_react=!defender.is_incapacitated();
    let avoided=can_react && resolve_task(defender.attribute("perception"),12,false,&mut dice).success;
    let outcome=AttackOutcome {attack_total:0,defense_total:0,success_margin:0,total_damage:0,wound_level:0,hit:false,defense_used:can_react,injury_before:before,injury_after:before,stunned:defender.stunned,unconscious:defender.unconscious};
    if avoided {return outcome;}
    let knocked_down=!defender.prone;
    let attack=AttackProfile {ranged:false,damage_class:attacker.mass,attack_bonus:0,damage_type:DamageType::Blunt,armor_piercing:0};
    let mut outcome=finish_hit(defender,attack,outcome,0,dice);
    defender.prone=true;
    // A knockdown consumes all remaining Ready tokens even without stun.
    outcome.defense_used|=knocked_down;
    outcome
}

#[cfg(test)]
mod trample_tests {
    use super::*;
    fn rat()->SoulData {let mut r=SoulData{size:0,mass:0,total_xp:0,..Default::default()};r.attribute_modifiers.insert("body".into(),1);r}
    #[test]
    fn trample_uses_mass_damage_and_knockdown_not_passive_defense() {
        let attacker=SoulData::default();let mut target=rat();
        let hit=resolve_trample(&attacker,&mut target,||5);
        assert!(hit.hit);assert_eq!(hit.success_margin,0);
        assert_eq!(hit.total_damage,5);assert_eq!(hit.wound_level,4);
        assert!(target.prone);assert!(hit.defense_used);
    }
    #[test]
    fn target_can_avoid_trample_with_desperation_dive() {
        let mut target=rat();let hit=resolve_trample(&SoulData::default(),&mut target,||10);
        assert!(!hit.hit);assert_eq!(target.injury,InjuryCondition::Uninjured);
    }
    #[test]
    fn natural_ten_can_continue_when_the_first_total_cannot_hit() {
        let mut attacker=SoulData::default();attacker.attribute_modifiers.insert("melee_attack".into(),-3);
        let mut target=rat();let profile=AttackProfile::unarmed(&attacker);
        let hit=resolve_attack(&attacker,&mut target,profile,false,||10);
        assert_eq!(hit.attack_total,17);assert!(hit.hit);
    }
}

//! training implementation, split from the parent subsystem.

use super::*;

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
        if name == "quickness" && self.trait_level("no mutations") > 0 {
            return Err(
                "Your No Mutations disadvantage prevents learning this Mutation Peculiarity."
                    .into(),
            );
        }
        let am = ["brainy", "brutish", "burly", "genteel"];
        if am.contains(&name) && am.iter().any(|key| self.peculiarity_level(key) > 0) {
            return Err("Only one Attribute Modifier Peculiarity is allowed.".into());
        }
        let level = self.peculiarity_level(name) + 1;
        let max = if am.contains(&name) { 1 } else { 3 };
        if level > max {
            return Err("That Peculiarity is already at its maximum implemented level.".into());
        }
        let cost = if self.available_combat_levels() > 0 {
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
        self.peculiarities.get(name).copied().unwrap_or(0) + self.trait_level(name)
    }
    pub fn trait_level(&self, name: &str) -> u32 {
        self.species.as_ref().map(|s| s.level(name)).unwrap_or(0)
    }
    pub fn used_combat_levels(&self) -> i64 {
        self.peculiarities
            .values()
            .map(|v| i64::from(*v))
            .sum::<i64>()
            + self.species.as_ref().map(|s| s.used_c()).unwrap_or(0)
    }
    pub fn available_combat_levels(&self) -> i64 {
        self.free_combat_levels()
            + self.peculiarity_xp / 100
            + self.species.as_ref().map(|s| s.bonus_c).unwrap_or(0)
            - self.used_combat_levels()
    }
    pub fn hard_to_kill(&self) -> bool {
        self.trait_level("hard to kill") > 0
    }
    pub fn needs_repair(&self) -> bool {
        self.trait_level("heal harm as object") > 0
    }
    pub fn species_modifier(&self, name: &str) -> i32 {
        self.species.as_ref().map(|s| s.modifier(name)).unwrap_or(0)
    }
    pub fn available_xp(&self) -> i64 {
        self.attribute_levels.iter().fold(
            self.total_xp
                .saturating_sub(self.rebuild_xp)
                .saturating_sub(self.peculiarity_xp),
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
        let vision = if matches!(
            name.as_str(),
            "melee_attack" | "ranged_attack" | "defense" | "perception"
        ) {
            self.vision_penalty
        } else {
            0
        };
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
        let strength = body
            + self.species_modifier("strength")
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
            mana_max: if self.trait_level("no mana") > 0 {
                0
            } else {
                self.base_attribute("intellect") + self.base_attribute("will_power")
            },
            base_dr: body + i32::from(self.trait_level("natural armor") == 1),
            base_sharp_dr: body - if self.hard_to_kill() { 0 } else { 2 },
            armor_dr: body
                + self
                    .effective_armor(false)
                    .0
                    .max(i32::from(self.trait_level("natural armor") == 1)),
            armor_sharp_dr: body - if self.hard_to_kill() { 0 } else { 2 }
                + self.effective_armor(true).0,
            melee_dc: strength - 1 - penalty,
            injury_penalty: self.injury.penalty(),
        }
    }
    /// Natural and worn armor never stack; use the stronger layer, then coverage.
    pub fn effective_armor(&self, sharp: bool) -> (i32, i32) {
        let hard = if self.hard_to_kill() && sharp { 2 } else { 0 };
        let worn = (
            if sharp {
                self.sharp_armor_bonus - hard
            } else {
                self.armor_bonus
            },
            self.armor_coverage,
        );
        let level = self.trait_level("natural armor");
        let natural = match level {
            2..=4 => (
                (level as i32 - 1) * if sharp { 2 } else { 1 } - hard,
                6 + (self.size - 6).max(0),
            ),
            _ => (0, 0),
        };
        worn.max(natural).max((0, 0))
    }
    pub fn is_incapacitated(&self) -> bool {
        self.injury.is_dead()
            || self.stunned
            || self.unconscious
            || self.comatose
            || self.is_helpless()
    }
    pub(super) fn hindered_threshold(&self) -> f64 {
        const KG: [f64; 25] = [
            1., 2., 4., 6., 10., 15., 20., 30., 50., 75., 100., 150., 250., 350., 550., 800.,
            1200., 1800., 2500., 4000., 6000., 10000., 15000., 20000., 30000.,
        ];
        let strength = self.base_attribute("body")
            + self.species_modifier("strength")
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

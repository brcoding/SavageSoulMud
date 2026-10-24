//! Versioned MUD-authored species templates using book Peculiarities.
//! Templates are snapshotted at creation; edits affect future characters only.
use crate::savage_rules::SoulData;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
mod runtime;
pub(crate) use runtime::{allows_food, buy_off, command, sync_visibility};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub slug: String,
    pub name: String,
    pub abbrev: String,
    pub version: u32,
    pub description: String,
    pub source: String,
    #[serde(default)]
    pub appearance: Option<String>,
    #[serde(default)]
    pub advantages: BTreeMap<String, u32>,
    #[serde(default)]
    pub disadvantages: BTreeMap<String, u32>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeciesBuild {
    pub template: Option<Template>,
    pub traits: BTreeMap<String, u32>,
    pub cost_c: i64,
    pub cost_nc: i64,
    pub bonus_c: i64,
    pub bonus_nc: i64,
    pub converted_c: i64,
    pub paid_xp: i64,
    pub bought_off: BTreeMap<String, u32>,
}
impl SpeciesBuild {
    pub fn level(&self, name: &str) -> u32 {
        self.traits.get(name).copied().unwrap_or(0)
    }
    pub fn used_c(&self) -> i64 {
        self.cost_c + self.converted_c
    }
    pub fn spare_nc(&self) -> i64 {
        self.bonus_nc + 2 * self.converted_c - self.cost_nc
    }
    pub fn modifier(&self, name: &str) -> i32 {
        let has = |n: &str| i32::from(self.level(n) > 0);
        match name {
            "body" => has("burly") - has("genteel"),
            "athletics" | "defense" => -has("burly") - self.level("clumsy") as i32,
            "craft" => -(self.level("clumsy") as i32),
            "intellect" => {
                has("brainy") - has("brutish") - self.level("intellectually challenged") as i32
            }
            "strength" => has("brutish") - has("brainy"),
            "faith" | "leadership" | "performance" => has("genteel"),
            _ => 0,
        }
    }
    pub fn summary(&self) -> String {
        let Some(t) = &self.template else {
            return "Legacy build; no species template applied.".into();
        };
        let mut details=format!("{} (template v{})\n{}\nPackage: {}C + {}NC; disadvantages grant {}C + {}NC. {}C converted to NC; {} XP paid at creation. {} NC unused.\nInnate traits: {}.\n{}",t.name,t.version,t.description,self.cost_c,self.cost_nc,self.bonus_c,self.bonus_nc,self.converted_c,self.paid_xp,self.spare_nc(),self.traits.iter().map(|(k,v)|format!("{k} {v}")).collect::<Vec<_>>().join(", "),t.source);
        if !self.bought_off.is_empty() {
            details.push_str(&format!(
                "\nBought off since creation: {}.",
                self.bought_off
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        details
    }
}

#[derive(Resource, Clone)]
pub struct Catalog(pub Vec<Template>);
const EMBEDDED: &str = include_str!("../../../data/savage-soul/races.json");
const AMS: [&str; 4] = ["brainy", "brutish", "burly", "genteel"];

// Cost tables: book A1, A2, A3, A6, A8. Only supported levels are accepted.
fn cost(name: &str, level: u32, disadvantage: bool) -> Result<(i64, i64), String> {
    let value = match (name, level, disadvantage) {
        ("no mana", 1, true) => (1, 0),
        ("no mutations", 1, true) => (2, 0),
        ("carnivore", 1, true) => (0, 1),
        ("clumsy", 1, true) => (2, 1),
        ("clumsy", 2, true) => (4, 2),
        ("intellectually challenged", 1, true) => (1, 0),
        ("intellectually challenged", 2, true) => (1, 1),
        ("brainy" | "brutish" | "burly" | "genteel", 1, false) => (1, 0),
        ("electronic", 1, false) => (5, 4),
        ("deceptive appearance", 1, false) => (1, 1),
        ("natural armor", 1, false) => (1, 0),
        ("natural armor", 2, false) => (2, 0),
        ("natural armor", 3, false) => (4, 0),
        ("natural armor", 4, false) => (6, 0),
        ("sharp strike", 1, false) => (1, 0),
        ("sharp strike", 2, false) => (2, 0),
        ("special strike", 1, false) => (1, 0),
        ("stature large", 1, false) => (1, 0),
        ("stature large", 2, false) => (3, 0),
        ("enhanced vision", 1, false) => (1, 1),
        _ => return Err(format!("Unsupported species trait/level: {name} {level}")),
    };
    Ok(value)
}
impl Template {
    pub fn build(&self) -> Result<SoulData, String> {
        self.build_at_xp(600)
    }
    pub fn build_at_xp(&self, total_xp: i64) -> Result<SoulData, String> {
        if total_xp < 0 {
            return Err("Total XP cannot be negative".into());
        }
        if self.version == 0
            || self.slug.is_empty()
            || !self
                .slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'-')
        {
            return Err("Invalid species identity/version".into());
        }
        if self.disadvantages.len() > 5 {
            return Err("At most five species disadvantage slots".into());
        }
        let mut b = SpeciesBuild {
            template: Some(self.clone()),
            ..Default::default()
        };
        for (disadvantage, entries) in [(false, &self.advantages), (true, &self.disadvantages)] {
            for (name, level) in entries {
                let (c, nc) = cost(name, *level, disadvantage)?;
                if disadvantage {
                    b.bonus_c += c;
                    b.bonus_nc += nc;
                } else {
                    b.cost_c += c;
                    b.cost_nc += nc;
                }
                if b.traits.insert(name.clone(), *level).is_some() {
                    return Err("Duplicate advantage/disadvantage".into());
                }
            }
        }
        if AMS.iter().filter(|n| b.level(n) > 0).count() > 1 {
            return Err("Only one Attribute Modifier".into());
        }
        if b.level("electronic") > 0 {
            for (name, level) in [
                ("chemically disadvantaged", 2),
                ("hard to kill", 1),
                ("heal harm as object", 1),
                ("no strings attached", 1),
                ("resistant to aging", 1),
                ("resistant to cold", 2),
                ("resistant to disease", 2),
                ("resistant to heat", 2),
                ("resistant to poison", 2),
                ("resistant to pressure", 1),
                ("resistant to radiation", 2),
                ("resistant to starvation and dehydration", 2),
                ("resistant to vacuum", 1),
                ("respiration", 3),
                ("unusual substance inorganic", 1),
                ("non mutant augmentation", 1),
            ] {
                b.traits.insert(name.into(), level);
            }
        }
        if b.level("no mutations") > 0
            && (!["human", "human-sheltie"].contains(&self.slug.as_str())
                || b.level("electronic") > 0
                || b.level("sharp strike") > 0
                || b.level("enhanced vision") > 0
                || b.level("natural armor") >= 2
                || b.level("deceptive appearance") > 0)
        {
            return Err("No Mutations conflicts with a mandatory M/S trait".into());
        }
        let mut soul = SoulData {
            total_xp,
            ..Default::default()
        };
        b.converted_c = ((b.cost_nc - b.bonus_nc).max(0) + 1) / 2;
        b.paid_xp = (b.used_c() - b.bonus_c - soul.free_combat_levels()).max(0) * 100;
        if b.paid_xp > soul.total_xp {
            return Err("Species package exceeds starting XP budget".into());
        }
        soul.peculiarity_xp = b.paid_xp;
        let large = b.level("stature large");
        if large > 0 {
            soul.size = 5 + large as i32;
            soul.mass = soul.size;
        }
        if !["human", "human-sheltie"].contains(&self.slug.as_str()) {
            b.traits.insert("species".into(), 1);
        }
        soul.species = Some(b);
        soul.mana = soul.derived().mana_max;
        Ok(soul)
    }
}
impl Catalog {
    pub fn parse(text: &str) -> Result<Self, String> {
        let entries: Vec<Template> = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if entries.is_empty() {
            return Err("Species catalogue cannot be empty".into());
        }
        let mut seen = std::collections::HashSet::new();
        for t in &entries {
            if !seen.insert(&t.slug) {
                return Err("Duplicate species slug".into());
            }
            t.build().map_err(|e| format!("{}: {e}", t.slug))?;
        }
        Ok(Self(entries))
    }
    pub fn load() -> Self {
        let text = match std::fs::read_to_string("data/savage-soul/races.json") {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => EMBEDDED.into(),
            Err(e) => panic!("Cannot read races.json: {e}"),
        };
        Self::parse(&text).unwrap_or_else(|e| panic!("Invalid races.json: {e}"))
    }
    fn registry(&self) -> grim::RaceRegistry {
        grim::RaceRegistry(
            self.0
                .iter()
                .map(|t| {
                    let soul = t.build().expect("validated template");
                    grim::RaceDef {
                        slug: t.slug.clone(),
                        name: t.name.clone(),
                        abbrev: t.abbrev.clone(),
                        description: format!(
                            "{} [Starting XP: {} unspent; package costs {} XP]",
                            t.description,
                            soul.available_xp(),
                            soul.peculiarity_xp
                        ),
                    }
                })
                .collect(),
        )
    }
}
pub fn install(app: &mut App) {
    let catalog = Catalog::load();
    let mut defaults = HashMap::new();
    for template in &catalog.0 {
        let state = serde_json::to_value(template.build().expect("validated template"))
            .expect("species serialization");
        defaults.insert(
            template.slug.clone(),
            HashMap::from([("savage_soul".into(), state)]),
        );
    }
    app.insert_resource(catalog.registry())
        .insert_resource(grim::CharacterCreationDefaults(defaults))
        .insert_resource(catalog);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn soul(slug: &str) -> SoulData {
        Catalog::parse(EMBEDDED)
            .unwrap()
            .0
            .iter()
            .find(|t| t.slug == slug)
            .unwrap()
            .build()
            .unwrap()
    }
    #[test]
    fn species_budgets_effects_and_save_roundtrip() {
        for (slug, xp, c) in [
            ("human", 600, 3),
            ("human-sheltie", 600, 6),
            ("android", 200, 0),
            ("gurnin", 600, 0),
            ("tater", 500, 0),
            ("morlock", 500, 0),
            ("replicant", 0, 0),
            ("poleepkwa", 500, 0),
        ] {
            let s = soul(slug);
            assert_eq!(s.available_xp(), xp, "{slug}");
            assert_eq!(s.available_combat_levels(), c, "{slug}");
            let restored: SoulData =
                serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
            assert_eq!(restored.available_xp(), xp);
            assert_eq!(
                restored.species.unwrap().summary(),
                s.species.unwrap().summary()
            );
        }
        let t = soul("tater");
        assert_eq!(
            (
                t.size,
                t.mass,
                t.base_attribute("body"),
                t.derived().dodge,
                t.derived().passive_defense
            ),
            (7, 7, 8, 2, 8)
        );
        let g = soul("gurnin");
        assert_eq!(
            (g.base_attribute("intellect"), g.derived().strength),
            (4, 6)
        );
        assert!(matches!(
            crate::savage_rules::AttackProfile::unarmed(&soul("poleepkwa")).damage_type,
            crate::savage_rules::DamageType::Sharp
        ));
    }
    #[test]
    fn restrictions_buyoff_and_innate_modifier_cannot_be_bypassed() {
        let mut s = soul("human-sheltie");
        assert_eq!(s.derived().mana_max, 0);
        assert!(s.learn("quickness").is_err());
        buy_off(&mut s, "no mutations").unwrap();
        assert_eq!(s.available_combat_levels(), 4);
        s.learn("quickness").unwrap();
        buy_off(&mut s, "no mana").unwrap();
        assert!(s.derived().mana_max > 0);
        assert!(buy_off(&mut s, "no mana").is_err());
        let mut g = soul("gurnin");
        assert!(g.learn("brainy").is_err());
        assert!(g.learn("brutish").is_err());
        let mut m = soul("morlock");
        assert!(allows_food(&m, "meat"));
        assert!(!allows_food(&m, "mixed"));
        let xp = m.available_xp();
        buy_off(&mut m, "carnivore").unwrap();
        assert_eq!(m.available_xp(), xp - 100);
        assert_eq!(m.trait_level("carnivore"), 0);
        assert_eq!(m.available_combat_levels(), 0);
    }
    #[test]
    fn electronic_and_natural_armor_tradeoffs_are_real() {
        let mut a = soul("android");
        assert!(a.hard_to_kill() && a.needs_repair());
        assert_eq!(a.trait_level("resistant to poison"), 2);
        assert_eq!(a.derived().base_sharp_dr, 5);
        assert_eq!(a.trait_level("resistant to stun"), 0);
        assert_eq!(a.trait_level("resistant to fatigue"), 0);
        a.armor_bonus = 2;
        a.sharp_armor_bonus = 4;
        a.armor_coverage = 6;
        assert_eq!(a.derived().armor_sharp_dr, 7);
        let mut g = soul("gurnin");
        assert_eq!(g.derived().armor_dr, 6);
        assert_eq!(g.derived().armor_sharp_dr, 5);
        g.armor_bonus = 2;
        g.sharp_armor_bonus = 4;
        g.armor_coverage = 6;
        assert_eq!(g.derived().armor_dr, 7);
        assert_eq!(g.derived().armor_sharp_dr, 7);
        let mut t = Catalog::parse(EMBEDDED).unwrap().0[2].clone();
        t.disadvantages.insert("no mutations".into(), 1);
        assert!(t.build().is_err());
    }
    #[test]
    fn dark_room_visibility_and_combat_context_follow_traits_and_lights() {
        let mut world = World::new();
        let room = world.spawn(grim::RoomDarkness).id();
        let human = world.spawn((soul("human"), grim::InRoom { room })).id();
        let morlock = world.spawn((soul("morlock"), grim::InRoom { room })).id();
        sync_visibility(&mut world);
        assert_eq!(
            world
                .get::<SoulData>(human)
                .unwrap()
                .attribute("melee_attack"),
            2
        );
        assert_eq!(world.get::<SoulData>(morlock).unwrap().vision_penalty, 0);
        let lamp = world
            .spawn((
                grim::Object,
                grim::components::Keywords(vec!["light:active".into()]),
                grim::CarriedBy { carrier: human },
            ))
            .id();
        sync_visibility(&mut world);
        assert_eq!(world.get::<SoulData>(human).unwrap().vision_penalty, 0);
        world.despawn(lamp);
        sync_visibility(&mut world);
        assert_eq!(world.get::<SoulData>(human).unwrap().vision_penalty, 3);
    }
}

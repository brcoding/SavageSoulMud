//! Read-only trainer advice drawn from the same rules used by train/learn.
use crate::savage_rules::{self as rules, SoulData};

#[derive(bevy::prelude::Component)]
pub(crate) struct Conversation {
    pub npc: bevy::prelude::Entity,
    pub attribute: String,
}

/// Explicit game vocabulary wins over a model's guess. Unknown phrasing still
/// goes through the optional translator; players need no special syntax.
pub(crate) fn direct_topic(question: &str) -> Option<String> {
    let q = question.to_lowercase().replace(['_', '-'], " ");
    let words = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if words.split_whitespace().any(|w| {
        [
            "race",
            "races",
            "species",
            "android",
            "replicant",
            "sheltie",
            "morlock",
            "gurnin",
            "tater",
            "poleepkwa",
        ]
        .contains(&w)
    }) {
        return Some("species".into());
    }
    let padded = format!(" {words} ");
    let has = |phrase: &str| padded.contains(&format!(" {phrase} "));
    if has("fatigue") || has("fatigued") || has("winded") || has("tired") {
        return Some("fatigue".into());
    }
    for name in rules::supported_peculiarities() {
        if has(name) {
            return Some(name.into());
        }
    }
    for name in rules::ATTRIBUTES {
        if has(&name.replace('_', " ")) {
            return Some(name.replace('_', " "));
        }
    }
    if has("faith") {
        return Some("faith".into());
    }
    if has("capacity") || has("combat levels") || has("combat capacity") {
        return Some("capacity".into());
    }
    if has("peculiarities") || has("peculiarity") || has("pecs") {
        return Some(
            if has("how many") {
                "capacity"
            } else {
                "peculiarities"
            }
            .into(),
        );
    }
    if (has("attributes") || has("attribute"))
        && ![
            "spend", "spending", "cost", "costs", "limit", "limits", "cap", "afford",
        ]
        .iter()
        .any(|word| has(word))
    {
        return Some("attributes".into());
    }
    None
}

fn attribute_advice(soul: &SoulData, name: &str) -> String {
    let old = soul.attribute_levels.get(name).copied().unwrap_or(0);
    let Some(next) = old.checked_add(1) else {
        return "That attribute is already at its maximum supported training level.".into();
    };
    let spent = rules::attribute_cost(name, old);
    let total = rules::attribute_cost(name, next);
    let cost = total - spent;
    let budget_gap = (cost - soul.available_xp()).max(0);
    let cap_gap = total.saturating_mul(2).saturating_sub(soul.total_xp).max(0);
    let needed = budget_gap.max(cap_gap);
    let readable = name.replace('_', " ");
    let effect = if ["intellect", "will_power"].contains(&name) && soul.trait_level("no mana") > 0 {
        "No Mana keeps your mana reserve at zero even if you train this attribute. "
    } else {
        match name {
            "body" => "Body improves Strength and damage resistance. ",
            "defense" => "Defense improves your dodge. ",
            "melee_attack" => "Melee Attack helps you land close-combat attacks. ",
            "ranged_attack" => "Ranged Attack helps you land ranged attacks. ",
            "leadership" => "Leadership helps you act first in combat. ",
            "athletics" | "perception" => "This attribute contributes to Quickness. ",
            "intellect" | "will_power" => "This attribute contributes to your mana reserve. ",
            _ => "",
        }
    };
    let mut text = format!(
        "{effect}Your base {readable} is {}. The next level costs {cost} XP and raises it to {}. ",
        soul.base_attribute(name),
        soul.base_attribute(name).saturating_add(1)
    );
    if needed == 0 {
        text.push_str(&format!(
            "You can afford it ({} XP unspent). Use train {readable}.",
            soul.available_xp()
        ));
    } else {
        text.push_str(&format!(
            "Earn at least {needed} more XP first, without other spending.{}",
            if cap_gap > 0 {
                " That purchase exceeds your half-total-XP limit."
            } else {
                " You do not have enough unspent XP."
            }
        ));
    }
    text
}

pub(crate) fn advice(soul: &SoulData, question: &str) -> String {
    let question = direct_topic(question).unwrap_or_else(|| question.into());
    let q = question.to_lowercase().replace(['_', '-'], " ");
    let words = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let padded = format!(" {words} ");
    let has = |phrase: &str| padded.contains(&format!(" {phrase} "));
    let text = if has("species") || has("race") || has("races") {
        let detail=soul.species.as_ref().map(|s|s.summary()).unwrap_or_else(||"Your existing character has its original build; new templates are applied only during creation.".into());
        format!("{detail}\nUse races for available templates, race <name> for another template, or buyoff <disadvantage> to repay capacity and remove one of your template restrictions.")
    } else if has("attributes") {
        "Attributes are your character's abilities. Body helps you withstand damage; Melee Attack helps you hit; Defense helps you dodge. Spend XP with train <attribute> to improve one. Use train to list all twelve, or ask about a name for your next cost. Faith cannot be trained directly.".into()
    } else if has("fatigue") {
        format!("Being winded or fatigued gives -1 to most task attributes and weapon damage per condition; Body, Faith and Intellect scores are exempt. You are {}winded and {}fatigued. Outside combat, rest makes a Will Power recovery attempt for winded; it does not clear the separate fatigued condition. The medical table clears both.", if soul.winded { "" } else { "not " }, if soul.fatigued { "" } else { "not " })
    } else if has("faith") {
        "Faith cannot be bought directly with XP. Applicable Peculiarities and effects can change it. Genteel, for example, raises Faith, Leadership and Performance but lowers Body; only one Attribute Modifier Peculiarity is allowed.".into()
    } else if let Some(name) = rules::ATTRIBUTES
        .iter()
        .find(|name| has(&name.replace('_', " ")))
    {
        attribute_advice(soul, name)
    } else if has("exchange") || has("credits") || has("money") {
        format!("Exchange converts 1 unspent XP into 100 credits. It is available only before any attribute or Peculiarity purchases; it reduces total XP, so it also reduces training limits and can reduce earned Combat capacity. You currently have {} unspent XP. Use help exchange before choosing exchange <amount>.", soul.available_xp())
    } else if has("respec") || has("refund") || has("undo") || has("reset") {
        "Visit Royal Oak Med Room and use creator to rebuild your character. It keeps earned XP, refunds training into a draft, and shows the credit charge, race-change XP fee and any body parts required. Nothing is spent until creator confirm. Ask about an attribute before buying it; my advice never spends XP.".into()
    } else if has("capacity") || has("combat levels") {
        let earned = soul.free_combat_levels();
        let used = soul.used_combat_levels();
        let remaining = soul.available_combat_levels().max(0);
        let next_xp = if earned < 5 {
            (earned + 1) * 200
        } else if earned < 10 {
            1000 + (earned - 4) * 300
        } else if earned < 15 {
            2500 + (earned - 9) * 400
        } else {
            4500 + (earned - 14) * 500
        };
        format!("Combat capacity (C) pays for Peculiarity levels. You have {remaining} C available ({earned} earned, {used} used; species refunds and purchased capacity included). Your next earned C arrives at {next_xp} total XP ({} more XP). If no capacity is left, learn buys another C for 100 XP.", (next_xp - soul.total_xp).max(0))
    } else if has("peculiarities") || has("peculiarity") || has("pecs") {
        format!("Peculiarities are special traits. Available: {}. Ask about one to see its effects and your cost; use learn <name> to buy a level.", rules::supported_peculiarities().join(", "))
    } else if let Some(name) = rules::supported_peculiarities()
        .iter()
        .find(|name| has(name))
    {
        peculiarity_advice(soul, name)
    } else if has("cap") || has("limit") || has("limits") || has("maximum") || has("half") {
        format!("Each attribute's cumulative training cost must stay at or below half your total XP: {} XP for your {} total XP. This is separate from the unspent-XP check. Spending on another attribute reduces your available budget, but does not lower this limit. Ask about a named attribute to see both checks and exactly how much more XP you need.", soul.total_xp / 2, soul.total_xp)
    } else if has("afford")
        || has("spend")
        || has("spending")
        || has("build")
        || has("recommend")
        || has("should")
        || has("stronger")
        || has("better")
    {
        let mut text = format!(
            "You have {} unspent XP. Next training purchases:\n",
            soul.available_xp()
        );
        for name in rules::ATTRIBUTES {
            let mut preview = soul.clone();
            let result = preview.train(name);
            text.push_str(&format!(
                "{}: {}\n",
                name.replace('_', " "),
                match result {
                    Ok(_) => format!(
                        "{} XP; affordable",
                        soul.available_xp() - preview.available_xp()
                    ),
                    Err(reason) => reason,
                }
            ));
        }
        text.push_str("For close combat, consider Melee Attack to land attacks, Defense for active defense and Body for durability and strength. Spread spending rather than ignoring your defenses. Ask about an attribute for its exact cost and limit. These are individual options, not a combined shopping list; nothing has been purchased.");
        text
    } else if has("cost") || has("costs") || has("formula") || has("expensive") {
        "Buying purchased level n costs 50 times n XP, or 100 times n for Body. Cumulative cost is 50*n*(n+1)/2, doubled for Body. Ordinary levels 1, 2 and 3 therefore cost 50, 100 and 150 XP (300 total); Body costs 100, 200 and 300 (600 total). Your starting score, Mass and Peculiarity bonuses do not count as purchased levels. Ask about a specific attribute for your personal figures.".into()
    } else if has("level")
        || has("levels")
        || has("leveling")
        || has("levelling")
        || has("advance")
        || has("advancement")
    {
        "You advance by earning and spending XP on attributes and Peculiarities; there is no automatic class-level increase that raises all your stats. New characters start with 600 XP. Combat rewards depend on the creature, and some quests grant XP too. Total XP also raises per-attribute spending limits and earned Combat capacity. Score shows your record. Ask about spending XP, limits, or Peculiarities for details.".into()
    } else if (has("earn") || has("get") || has("gain")) && (has("xp") || has("experience")) {
        "Defeating creatures with an XP reward and completing rewarded quests earns XP. The amount depends on the encounter; combat reports it at the end. Earning XP raises your total XP and unspent budget, and can unlock a higher attribute spending limit or another Combat capacity level. Training spends your budget; it does not earn XP.".into()
    } else if has("xp") || has("experience") || has("balance") || has("earned") {
        format!("You have {} total XP, {} spent XP and {} unspent XP. Earned XP increases both your total and available XP; train and learn spend available XP without reducing total XP. Total XP sets each attribute's half-XP limit and earned Combat capacity. Use score for your record, or ask me what you can afford.", soul.total_xp, soul.total_xp - soul.available_xp(), soul.available_xp())
    } else {
        format!("You have {} unspent XP. I can explain leveling, spending XP, costs, limits, Peculiarities, Faith, exchange and respecs. Ask about an attribute or a supported Peculiarity for a personal preview. Try: ask tully about what can I afford, or ask tully about melee attack. Use train <attribute> or learn <name> when ready; my advice never makes a purchase.", soul.available_xp())
    };
    format!("Tully says: {text}")
}

fn peculiarity_advice(soul: &SoulData, name: &&str) -> String {
    // Simulate the real purchase on a clone: no player state changes.
    let mut preview = soul.clone();
    let effect = match *name {
            "brainy" => "Brainy gives +1 Intellect and -1 Strength.",
            "brutish" => "Brutish gives +1 Strength and -1 Intellect.",
            "burly" => "Burly gives +1 Body, -1 Athletics and -1 Defense.",
            "genteel" => "Genteel gives +1 Faith, Leadership and Performance, but -1 Body.",
            "quickness" => "Quickness adds +1, +3 or +5 to derived Quickness at levels 1, 2 or 3.",
            _ => "Unarmed Fighter improves unarmed attacks and damage at level 1, adds +1 dodge at level 2, and increases unarmed damage again at level 3.",
        };
    let restriction = if ["brainy", "brutish", "burly", "genteel"].contains(name) {
        " You may have only one Attribute Modifier Peculiarity."
    } else {
        ""
    };
    match preview.learn(name) {
        Ok(_) => format!(
            "{effect}{restriction} Your next level costs {}. Use learn {name}.",
            if preview.available_xp() == soul.available_xp() {
                "1 C from your available capacity"
            } else {
                "100 XP for 1 C"
            }
        ),
        Err(reason) => {
            format!("{effect}{restriction} You cannot learn another level now: {reason}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advice_tracks_actual_training_cost_and_cap_without_spending() {
        let mut soul = SoulData::default();
        soul.train("body").unwrap();
        let text = advice(&soul, "how much will body cost?");
        assert!(text.contains("next level costs 200 XP"));
        soul.train("body").unwrap();
        assert!(advice(&soul, "body").contains("Earn at least 600 more XP"));
        assert_eq!(soul.available_xp(), 300);
    }
    #[test]
    fn peculiarity_preview_and_capacity_use_live_rules() {
        let soul = SoulData::default();
        assert!(advice(&soul, "can I learn quickness?").contains("Quickness adds +1, +3 or +5"));
        assert!(advice(&soul, "capacity").contains("800 total XP"));
        assert!(soul.peculiarities.is_empty());
        assert!(advice(&soul, "can I train faith?").contains("cannot be bought"));
    }
    #[test]
    fn reported_questions_keep_their_topic_and_answers_brief() {
        let soul = SoulData {
            total_xp: 4660,
            ..Default::default()
        };
        for question in ["about attributes", "what is an attribute?"] {
            let text = advice(&soul, question);
            assert!(text.contains("Attributes are your character's abilities"));
            assert!(!text.contains("Your athletics"));
            assert!(!text.contains("Combat capacity"));
        }
        assert!(advice(&soul, "about fatigue").contains("Being winded or fatigued"));
        let list = advice(&soul, "for a list of peculiarities");
        assert!(list.contains("Available: brainy, brutish"));
        assert!(!list.contains("next earned C"));
        for (question, expected) in [
            ("body", "next level costs 100 XP"),
            ("brutish", "+1 Strength and -1 Intellect"),
            ("quickness", "+1, +3 or +5"),
        ] {
            let text = advice(&soul, question);
            assert!(text.contains(expected), "{text}");
            assert!(text.split_whitespace().count() < 65, "{text}");
            assert!(direct_topic(question).is_some());
        }
    }
}

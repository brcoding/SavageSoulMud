//! narration implementation, split from the parent subsystem.

use super::*;

/// Describe the resolved defense without adding rolls or changing activations.
/// Active defense is Dodge; passive armed guard may be narrated as a parry/block.
pub(super) fn avoided_strike(
    outcome: &rules::AttackOutcome,
    defender: &SoulData,
    pc_attacks: bool,
    name: &str,
    variant: usize,
) -> String {
    let mut chars = name.chars();
    let capital = chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_else(|| "Opponent".into());
    let (subject, possessive) = if pc_attacks {
        (capital, "your".to_string())
    } else {
        ("You".into(), format!("{name}'s"))
    };
    let verb = |you: &'static str, other: &'static str| if pc_attacks { other } else { you };
    if outcome.attack_total == 0 {
        let attack = if pc_attacks {
            "Your strike".into()
        } else {
            format!("{}'s strike", subject_for_opponent(name))
        };
        return match variant % 3 {
            0 => format!("{attack} goes wide."),
            1 => format!("{attack} connects with thin air."),
            _ => format!("{attack} misses the mark."),
        };
    }
    if defender.is_incapacitated() {
        return if pc_attacks {
            format!("Your strike falls short of {name}.")
        } else {
            format!(
                "{}'s strike falls short of you.",
                subject_for_opponent(name)
            )
        };
    }
    if outcome.defense_used {
        if defender.prone {
            return format!(
                "{subject} {} clear of {possessive} strike.",
                verb("roll", "rolls")
            );
        }
        return match variant % 3 {
            0 => format!(
                "{subject} {} {possessive} strike.",
                verb("sidestep", "sidesteps")
            ),
            1 => format!("{subject} {} {possessive} strike.", verb("dodge", "dodges")),
            _ => format!(
                "{subject} {} back from {possessive} strike.",
                verb("spring", "springs")
            ),
        };
    }
    if !defender.weapon.is_empty()
        && !defender.prone
        && outcome.defense_total - outcome.attack_total <= 3
    {
        let technique = match variant % 3 {
            0 => verb("parry", "parries"),
            1 => verb("block", "blocks"),
            _ => verb("deflect", "deflects"),
        };
        return format!(
            "{subject} {technique} {possessive} strike with {}.",
            defender.weapon
        );
    }
    if defender.prone {
        return format!(
            "{possessive_cap} strike passes over {object}.",
            possessive_cap = if pc_attacks {
                "Your".into()
            } else {
                format!("{}'s", subject_for_opponent(name))
            },
            object = if pc_attacks { name } else { "you" }
        );
    }
    match variant % 3 {
        0 => format!("{subject} {} out of reach.", verb("step", "steps")),
        1 => format!(
            "{subject} {} aside from {possessive} strike.",
            verb("slip", "slips")
        ),
        _ => format!(
            "{subject} {} under {possessive} strike.",
            verb("duck", "ducks")
        ),
    }
}

pub(super) fn subject_for_opponent(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_else(|| "Opponent".into())
}

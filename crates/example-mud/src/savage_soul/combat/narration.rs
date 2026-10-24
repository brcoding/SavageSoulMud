//! narration implementation, split from the parent subsystem.

use super::*;

pub(super) fn describe_strike(
    battle: &Battle,
    detail: Detail,
    lines: &mut Vec<String>,
    defender: &SoulData,
    outcome: &rules::AttackOutcome,
    context: crate::combat_prose::Strike,
) {
    let pc_attacks = context.player;
    let moving_reaction = context.counter;
    let trample = context.trample;
    let name = context.name.to_string();
    let capital = capitalized(&name);
    let actor = if pc_attacks {
        "You".into()
    } else {
        capital.clone()
    };
    let target = if pc_attacks {
        name.clone()
    } else {
        "you".into()
    };
    let avoidance = if !outcome.hit {
        let text = if trample {
            format!("{capital} dodges your trample.")
        } else {
            avoided_strike(
                outcome,
                defender,
                pc_attacks,
                &name,
                battle.rounds as usize + usize::from(pc_attacks),
            )
        };
        if moving_reaction {
            format!("{capital} counterattacks. {text}")
        } else {
            text
        }
    } else {
        String::new()
    };
    let impact = if outcome.hit {
        crate::combat_prose::landed(outcome, defender, context)
    } else {
        String::new()
    };
    if detail == Detail::Detailed {
        let result = if outcome.hit {
            format!(
                "SM {}, damage {}, wound {}",
                outcome.success_margin, outcome.total_damage, outcome.wound_level
            )
        } else {
            avoidance.clone()
        };
        lines.push(if !outcome.hit {
            format!(
                "{avoidance} (attack {} vs defense {}).",
                outcome.attack_total, outcome.defense_total
            )
        } else {
            format!(
                "{impact} (attack {} vs defense {}; {result}).",
                outcome.attack_total, outcome.defense_total
            )
        });
    } else if !defender.injury.is_dead() {
        if outcome.hit && detail == Detail::Normal {
            lines.push(impact);
        } else if outcome.hit {
            if outcome.wound_level > 0 {
                lines.push(format!(
                    "{actor} {} {target}: {}{}.",
                    if trample {
                        "trample"
                    } else if moving_reaction {
                        "counterattacks"
                    } else if pc_attacks {
                        "hit"
                    } else {
                        "hits"
                    },
                    defender.injury.label().to_ascii_lowercase(),
                    if defender.stunned { ", stunned" } else { "" }
                ));
            } else {
                lines.push(format!(
                    "{actor} {} {target}: no wound.",
                    if trample {
                        "trample"
                    } else if moving_reaction {
                        "counterattacks"
                    } else if pc_attacks {
                        "hit"
                    } else {
                        "hits"
                    }
                ));
            }
        } else {
            lines.push(avoidance);
        }
    }
}

fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    let capital = chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default();
    capital
}

//! resolution implementation, split from the parent subsystem.

use super::*;

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
    if roll == 10 && attack_total < d.passive_defense {
        let mut next = 10;
        while next == 10 && attack_total < d.passive_defense {
            next = dice().clamp(1, 10);
            attack_total = attack_total - 5 + next;
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
pub(super) fn finish_hit(
    defender: &mut SoulData,
    attack: AttackProfile,
    mut outcome: AttackOutcome,
    sm: i32,
    mut dice: impl FnMut() -> i32,
) -> AttackOutcome {
    let d = defender.derived();
    outcome.hit = true;
    outcome.success_margin = sm;
    let sharp = matches!(attack.damage_type, DamageType::Sharp);
    let (armor, coverage) = defender.effective_armor(sharp);
    let ignored = armor <= 0 || sm > coverage;
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

/// Apply a rulebook wound level without armor or a separate attack roll.
pub fn apply_wound(soul: &mut SoulData, wl: i32) {
    if wl <= 0 {
        return;
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
pub fn resolve_trample(
    attacker: &SoulData,
    defender: &mut SoulData,
    mut dice: impl FnMut() -> i32,
) -> AttackOutcome {
    let before = defender.injury;
    let can_react = !defender.is_incapacitated();
    let avoided =
        can_react && resolve_task(defender.attribute("perception"), 12, false, &mut dice).success;
    let outcome = AttackOutcome {
        attack_total: 0,
        defense_total: 0,
        success_margin: 0,
        total_damage: 0,
        wound_level: 0,
        hit: false,
        defense_used: can_react,
        injury_before: before,
        injury_after: before,
        stunned: defender.stunned,
        unconscious: defender.unconscious,
    };
    if avoided {
        return outcome;
    }
    let knocked_down = !defender.prone;
    let attack = AttackProfile {
        ranged: false,
        damage_class: attacker.mass,
        attack_bonus: 0,
        damage_type: DamageType::Blunt,
        armor_piercing: 0,
    };
    let mut outcome = finish_hit(defender, attack, outcome, 0, dice);
    defender.prone = true;
    // A knockdown consumes all remaining Ready tokens even without stun.
    outcome.defense_used |= knocked_down;
    outcome
}

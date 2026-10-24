//! combat implementation, split from the parent subsystem.

use super::*;
mod lifecycle;
pub(super) use lifecycle::{
    battle_finished, begin_battle, combat_end, hard_incapacitated, retreat_npc,
};
mod narration;
use narration::describe_strike;

pub(super) fn tick_battles(world: &mut World) {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let mut battles = world.remove_resource::<Battles>().unwrap_or_default();
    battles
        .0
        .retain_mut(|battle| advance_battle(world, battle, now));
    world.insert_resource(battles);
}

struct CombatRound {
    pc: SoulData,
    npc: SoulData,
    name: String,
    capital: String,
    detail: Detail,
    lines: Vec<String>,
    pc_ready: bool,
    npc_ready: bool,
}

fn advance_battle(world: &mut World, battle: &mut Battle, now: f64) -> bool {
    if world.get::<Player>(battle.player).is_none() {
        return false;
    }
    let same = world
        .get::<InRoom>(battle.player)
        .zip(world.get::<InRoom>(battle.npc))
        .is_some_and(|(a, b)| a.room == b.room);
    if !same {
        combat_end(world, battle.player, "Combat ends: out of reach.", 0);
        return false;
    }
    if now < battle.next_round {
        return true;
    }
    let Some(pc) = world.get::<SoulData>(battle.player).cloned() else {
        return false;
    };
    let Some(npc) = world.get::<SoulData>(battle.npc).cloned() else {
        return false;
    };
    let name = world
        .get::<GrimName>(battle.npc)
        .map(|n| n.0.clone())
        .unwrap_or_else(|| "opponent".into());
    let mut chars = name.chars();
    let capital = chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default();
    if battle_finished(&pc, &npc, battle.lethal) {
        combat_end(world, battle.player, "Combat ends: incapacitated.", 0);
        return false;
    }
    let mut round = CombatRound {
        pc,
        npc,
        name,
        capital,
        detail: preferences::detail(world, battle.player),
        lines: Vec::new(),
        pc_ready: true,
        npc_ready: true,
    };
    let first_pc = initiative(world, &mut round);
    let behavior = world
        .get::<NpcCombatBehavior>(battle.npc)
        .cloned()
        .unwrap_or_default();
    activate_round(world, battle, &mut round, &behavior, first_pc);
    finish_round(world, battle, round, &behavior, now)
}

fn initiative(world: &mut World, round: &mut CombatRound) -> bool {
    let name = &round.name;
    let leader_a = rules::resolve_task(round.pc.attribute("leadership"), 0, false, || {
        world.resource_mut::<Dice>().roll()
    });
    let leader_b = rules::resolve_task(round.npc.attribute("leadership"), 0, false, || {
        world.resource_mut::<Dice>().roll()
    });
    let (a, b) = (leader_a.total, leader_b.total);
    let first_pc = if hard_incapacitated(&round.npc) {
        true
    } else if leader_a.critical_failure != leader_b.critical_failure {
        !leader_a.critical_failure
    } else if a == b {
        world.resource_mut::<Dice>().roll() >= 6
    } else {
        a > b
    };
    if round.detail == Detail::Detailed {
        round.lines.push(format!(
            "Initiative: you {a}, {name} {b}; {} first.",
            if first_pc { "you" } else { &round.name }
        ));
    }
    first_pc
}

fn activate_round(
    world: &mut World,
    battle: &Battle,
    round: &mut CombatRound,
    behavior: &NpcCombatBehavior,
    first_pc: bool,
) {
    let capital = round.capital.clone();
    // Trample is movement: a ready target may spend its activation on AMC
    // before damage/knockdown (rulebook 5.5.8.1 and 5.8.4). Otherwise a
    // slower creature spends every activation diving and never strikes.
    let counter = first_pc
        && battle.trample
        && !round.pc.is_incapacitated()
        && !round.pc.prone
        && round.pc.size > round.npc.size + 2
        && !round.npc.is_incapacitated();
    let turns = if counter {
        vec![(false, true), (true, false), (false, false)]
    } else {
        vec![(first_pc, false), (!first_pc, false)]
    };
    for (pc_attacks, moving_reaction) in turns {
        if battle_finished(&round.pc, &round.npc, battle.lethal) {
            break;
        }
        if !pc_attacks && hard_incapacitated(&round.npc) {
            continue;
        }
        if (pc_attacks && !round.pc_ready) || (!pc_attacks && !round.npc_ready) {
            continue;
        }
        if pc_attacks && round.pc.stunned {
            round.pc.stunned = false;
            round.pc_ready = false;
            round.lines.push("You recover from stun.".into());
            continue;
        }
        if !pc_attacks && round.npc.stunned {
            round.npc.stunned = false;
            round.npc_ready = false;
            round.lines.push(format!("{capital} recovers from stun."));
            continue;
        }
        strike(world, battle, round, pc_attacks, moving_reaction);
        if behavior.flee_when_injured
            && !round.npc.stunned
            && round.npc.injury != rules::InjuryCondition::Uninjured
        {
            break;
        }
    }
}

fn strike(
    world: &mut World,
    battle: &Battle,
    round: &mut CombatRound,
    pc_attacks: bool,
    moving_reaction: bool,
) {
    let (attacker, defender) = if pc_attacks {
        (&round.pc, &mut round.npc)
    } else {
        (&round.npc, &mut round.pc)
    };
    let entity = if pc_attacks {
        battle.player
    } else {
        battle.npc
    };
    let profile = profile(world, entity, attacker);
    let active = defender.defense_mode != "none";
    let critical_defender =
        defender.injury == rules::InjuryCondition::CriticallyInjured && !defender.stabilized;
    let trample =
        pc_attacks && battle.trample && !attacker.prone && attacker.size > defender.size + 2;
    let outcome = if trample {
        rules::resolve_trample(attacker, defender, || world.resource_mut::<Dice>().roll())
    } else {
        rules::resolve_attack(attacker, defender, profile, active, || {
            world.resource_mut::<Dice>().roll()
        })
    };
    if outcome.defense_used && critical_defender {
        critical_action(world, defender);
    }
    // Active defense against AMC does not cancel the declared moving
    // action; stun/incapacitation can still interrupt it normally.
    if pc_attacks {
        round.pc_ready = false;
        if outcome.defense_used {
            round.npc_ready = false;
        }
    } else {
        round.npc_ready = false;
        if outcome.defense_used && !moving_reaction {
            round.pc_ready = false;
        }
    }
    describe_strike(
        battle,
        round.detail,
        &mut round.lines,
        defender,
        &outcome,
        crate::combat_prose::Strike {
            player: pc_attacks,
            name: &round.name,
            weapon: &attacker.weapon,
            profile,
            trample,
            counter: moving_reaction,
            variant: battle.rounds as usize + usize::from(pc_attacks),
        },
    );
    if pc_attacks {
        critical_action(world, &mut round.pc);
    } else {
        critical_action(world, &mut round.npc);
    }
}

fn finish_round(
    world: &mut World,
    battle: &mut Battle,
    mut round: CombatRound,
    behavior: &NpcCombatBehavior,
    now: f64,
) -> bool {
    let capital = &round.capital;
    battle.rounds += 1;
    if battle.rounds.is_multiple_of(12) {
        for (soul, label) in [
            (&mut round.pc, "You"),
            (&mut round.npc, round.capital.as_str()),
        ] {
            let roll = rules::resolve_task(soul.attribute("will_power"), 10, false, || {
                world.resource_mut::<Dice>().roll()
            });
            if !roll.success && !soul.winded {
                soul.winded = true;
                round.lines.push(format!(
                    "{label} {} winded.",
                    if label == "You" { "are" } else { "is" }
                ));
            }
        }
    }
    let opponent_dead = round.npc.injury.is_dead();
    let finished = battle_finished(&round.pc, &round.npc, battle.lethal);
    let victory = finished && hard_incapacitated(&round.npc);
    let mut earned = if victory && world.get::<RewardClaimed>(battle.npc).is_none() {
        behavior.xp_reward.min(1_000_000)
    } else {
        0
    };
    if let Some(total) = round.pc.total_xp.checked_add(i64::from(earned)) {
        round.pc.total_xp = total;
    } else {
        warn!("Combat XP overflow");
        earned = 0;
    }
    if round.pc.injury.is_dead() {
        round.pc.stunned = false;
    }
    if round.npc.injury.is_dead() {
        round.npc.stunned = false;
    }
    let flee = !finished
        && !round.npc.stunned
        && ((behavior.flee_when_injured && round.npc.injury != rules::InjuryCondition::Uninjured)
            || behavior
                .flee_after_rounds
                .is_some_and(|limit| battle.rounds >= limit.max(1)));
    if let Err(error) = commit(world, battle.player, round.pc, None) {
        error!("Combat save failed: {error}");
        combat_end(world, battle.player, "Combat stopped: save failed.", 0);
        return false;
    }
    if victory {
        world.entity_mut(battle.npc).insert(RewardClaimed);
    }
    if let Err(error) = commit(world, battle.npc, round.npc, None) {
        error!("NPC state update: {error}");
    }
    if finished {
        let end = if opponent_dead {
            format!("{capital} killed.")
        } else if victory {
            format!("{capital} incapacitated.")
        } else {
            "Combat ends: you are incapacitated.".into()
        };
        round.lines.push(format!("{end} XP +{earned}."));
        if let Some(mut c) = world.get_mut::<Character>(battle.player) {
            c.game_state.remove("combat_opponent");
        }
    }
    if !round.lines.is_empty() {
        reply(
            world,
            battle.player,
            round.lines.join(if round.detail == Detail::Brief {
                " "
            } else {
                "\n"
            }),
        );
    }
    if finished {
        return false;
    }
    if flee {
        retreat_npc(world, battle.player, battle.npc);
        return false;
    }
    battle.next_round = now + 5.0;
    true
}

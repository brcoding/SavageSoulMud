//! conditions implementation, split from the parent subsystem.

use super::*;

pub(super) fn critical_action(world: &mut World, soul: &mut SoulData) {
    if soul.injury == rules::InjuryCondition::CriticallyInjured && !soul.stabilized {
        let roll = rules::resolve_task(soul.attribute("will_power"), 10, false, || {
            world.resource_mut::<Dice>().roll()
        });
        if !roll.success {
            soul.unconscious = true;
        }
    }
}

pub(super) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn tick_conditions(world: &mut World) {
    let time = world.resource::<Time>().elapsed_secs_f64();
    let mut resting = world.remove_resource::<Resting>().unwrap_or_default();
    resting.0.retain(|(e, room, end)| {
        if world.get::<InRoom>(*e).is_none_or(|r| r.room != *room) {
            return false;
        }
        if time < *end {
            return true;
        }
        if let Some(mut soul) = world.get::<SoulData>(*e).cloned() {
            let result = rules::resolve_task(soul.attribute("will_power"), 10, false, || {
                world.resource_mut::<Dice>().roll()
            });
            if result.success {
                soul.winded = false;
            }
            if let Err(error) = commit(world, *e, soul, None) {
                error!("Rest save: {error}");
            }
            reply(
                world,
                *e,
                if result.success {
                    "You recover from being winded."
                } else {
                    "Your Will Power attempt fails; you remain winded."
                },
            );
        }
        false
    });
    world.insert_resource(resting);
    let now = unix_now();
    let critical: Vec<_> = world
        .query::<(Entity, &SoulData)>()
        .iter(world)
        .filter(|(_, s)| s.injury == rules::InjuryCondition::CriticallyInjured && !s.stabilized)
        .map(|(e, s)| (e, s.clone()))
        .collect();
    for (e, mut soul) in critical {
        let mut changed = soul.critical_since.is_none();
        let start = *soul.critical_since.get_or_insert(now);
        if soul.hard_to_kill() {
            if now.saturating_sub(start) >= 57600 {
                soul.stabilized = true;
                changed = true;
                reply(world, e, "Your critical injuries stabilize.");
            }
            if changed {
                if let Err(error) = commit(world, e, soul, None) {
                    error!("Critical injury save: {error}");
                }
            }
            continue;
        }
        let intervals = [120, 600, 3600, 14400, 57600];
        while soul.critical_checks < 5
            && now.saturating_sub(start) >= intervals[soul.critical_checks as usize]
        {
            let result = rules::resolve_task(soul.attribute("will_power"), 10, false, || {
                world.resource_mut::<Dice>().roll()
            });
            soul.critical_checks += 1;
            changed = true;
            if !result.success {
                soul.injury = rules::InjuryCondition::Dead;
                reply(world, e, "Your critical injuries overwhelm you. You die.");
                break;
            }
            if soul.critical_checks == 5 {
                soul.stabilized = true;
                reply(
                    world,
                    e,
                    "You have stabilized after surviving your critical injuries.",
                );
            }
        }
        if changed {
            if let Err(error) = commit(world, e, soul, None) {
                error!("Critical injury save: {error}");
            }
        }
    }
}

pub(super) fn profile(world: &mut World, entity: Entity, soul: &SoulData) -> AttackProfile {
    if !soul.weapon.is_empty() {
        let owned = world
            .query_filtered::<(&GrimName, &CarriedBy), With<Object>>()
            .iter(world)
            .any(|(n, h)| h.carrier == entity && n.0 == soul.weapon);
        if owned {
            if let Some(g) = world
                .resource::<EquipmentCatalog>()
                .0
                .iter()
                .find(|g| g.name.eq_ignore_ascii_case(&soul.weapon) && g.kind == "weapon")
            {
                return AttackProfile {
                    ranged: false,
                    damage_class: soul.derived().strength + g.damage_modifier
                        - soul.injury.penalty()
                        - i32::from(soul.winded)
                        - i32::from(soul.fatigued),
                    attack_bonus: g.attack_bonus,
                    damage_type: if g.sharp {
                        DamageType::Sharp
                    } else {
                        DamageType::Blunt
                    },
                    armor_piercing: 0,
                };
            }
        }
    }
    AttackProfile::unarmed(soul)
}

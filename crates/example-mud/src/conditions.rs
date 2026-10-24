//! Game-owned corpse lifecycle; generic instance condition lives in grim-core.
use bevy::prelude::*;
use grim::components::{ItemCondition, ItemDecay, Keywords, Name as GrimName, RoomDescription};
use grim::{Actor, CarriedBy, Character, InRoom, Object, Player, StoredCharacter};
use serde::Deserialize;

#[derive(Resource, Deserialize)]
#[serde(default)]
struct Settings {
    corpse_decay_seconds: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            corpse_decay_seconds: 60,
        }
    }
}
#[derive(Resource, Default)]
struct Sweep(f64);
pub(crate) fn register(app: &mut App) {
    let path = app
        .world()
        .get_resource::<crate::seed::AreaBlueprintDir>()
        .map(|d| d.0.parent().unwrap_or(&d.0).join("savage-soul/world.json"))
        .unwrap_or_else(|| "data/savage-soul/world.json".into());
    let mut settings = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes).unwrap_or_else(|e| {
            warn!("Invalid corpse settings: {e}; using defaults");
            Settings::default()
        }),
        Err(_) => Settings::default(),
    };
    settings.corpse_decay_seconds = settings.corpse_decay_seconds.clamp(1, 86400);
    app.insert_resource(settings).init_resource::<Sweep>();
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(crate) fn fresh(world: &World) -> ItemCondition {
    let started_at = now();
    let delay = world
        .get_resource::<Settings>()
        .map(|s| s.corpse_decay_seconds)
        .unwrap_or(60);
    ItemCondition {
        quality: 100,
        decay: Some(ItemDecay {
            started_at,
            expires_at: started_at.saturating_add(delay),
        }),
    }
}
fn corpse(name: &str, keywords: Option<&Keywords>) -> bool {
    (name.starts_with("corpse of ") || name.starts_with("remains of "))
        && keywords.is_some_and(|k| k.0.iter().any(|k| k == "corpse"))
}
pub(crate) fn tick(world: &mut World) {
    let elapsed = world.resource::<Time>().elapsed_secs_f64();
    if elapsed < world.resource::<Sweep>().0 {
        return;
    }
    world.resource_mut::<Sweep>().0 = elapsed + 1.;
    tick_at(world, now());
}
fn tick_at(world: &mut World, at: u64) {
    let objects:Vec<_>=world.query_filtered::<(Entity,&GrimName,Option<&Keywords>,Option<&ItemCondition>),With<Object>>().iter(world)
        .map(|(e,n,k,c)|(e,n.0.clone(),corpse(&n.0,k),c.cloned())).collect();
    let mut expired = Vec::new();
    let mut initialized = Vec::new();
    for (e, name, is_corpse, condition) in objects {
        let mut c = condition.clone().unwrap_or_default();
        if is_corpse && c.decay.is_none() {
            c = fresh(world);
            initialized.push((e, condition.clone()));
        }
        if let Some(clock) = &c.decay {
            if at >= clock.expires_at {
                expired.push((e, name));
                continue;
            }
            let duration = clock.expires_at.saturating_sub(clock.started_at).max(1);
            c.quality = ((u128::from(clock.expires_at.saturating_sub(at)) * 100
                / u128::from(duration))
            .min(100)) as u8;
        }
        if condition.as_ref() != Some(&c) {
            if is_corpse {
                let adjective = match c.quality {
                    0..=29 => "badly decayed ",
                    30..=59 => "decaying ",
                    60..=79 => "slightly decayed ",
                    _ => "",
                };
                let verb = if name.starts_with("remains ") {
                    "lie"
                } else {
                    "lies"
                };
                world.entity_mut(e).insert((
                    RoomDescription(format!("The {adjective}{name} {verb} here.")),
                    grim::components::Description(vec![format!("The {name}.")]),
                ));
            }
            world.entity_mut(e).insert(c);
        }
    }
    // Persist complete packs before removing carried remains. Group by carrier
    // so simultaneous expirations cannot overwrite each other's inventory save.
    let mut carriers: Vec<_> = expired
        .iter()
        .filter_map(|(e, _)| world.get::<CarriedBy>(*e).map(|h| h.carrier))
        .collect();
    carriers.extend(
        initialized
            .iter()
            .filter_map(|(e, _)| world.get::<CarriedBy>(*e).map(|h| h.carrier)),
    );
    carriers.sort();
    carriers.dedup();
    let mut failed = Vec::new();
    for actor in carriers {
        let Some(mut character) = world.get::<Character>(actor).cloned() else {
            continue;
        };
        if let Some(soul) = world.get::<crate::savage_rules::SoulData>(actor) {
            character
                .game_state
                .insert("savage_soul".into(), serde_json::to_value(soul).unwrap());
        }
        let Some(name) = world.get::<GrimName>(actor) else {
            failed.push(actor);
            continue;
        };
        let Some(base) = world.get::<Actor>(actor) else {
            failed.push(actor);
            continue;
        };
        let mut saved = StoredCharacter::from_components(name, base, &character);
        saved.inventory = crate::shop::held(world, actor)
            .into_iter()
            .filter(|(e, _)| !expired.iter().any(|(dead, _)| dead == e))
            .map(|(_, o)| o)
            .collect();
        saved.inventory.sort_by(|a, b| a.name.cmp(&b.name));
        match crate::shop::persist(world, &saved) {
            Ok(()) => {
                world.entity_mut(actor).insert(character);
            }
            Err(error) => {
                warn!("Could not save corpse decay: {error}");
                failed.push(actor);
            }
        }
    }
    for (e, old) in initialized {
        if world
            .get::<CarriedBy>(e)
            .is_some_and(|h| failed.contains(&h.carrier))
        {
            if let Some(c) = old {
                world.entity_mut(e).insert(c);
            } else {
                world.entity_mut(e).remove::<ItemCondition>();
            }
        }
    }
    remove_expired(world, expired, &failed);
}

fn remove_expired(world: &mut World, expired: Vec<(Entity, String)>, failed: &[Entity]) {
    for (e, name) in expired {
        let holder = world.get::<CarriedBy>(e).map(|h| h.carrier);
        if holder.is_some_and(|h| failed.contains(&h)) {
            continue;
        }
        let room = world.get::<InRoom>(e).map(|r| r.room);
        let viewers: Vec<_> = if let Some(holder) = holder {
            vec![holder]
        } else {
            world
                .query_filtered::<(Entity, &InRoom), With<Player>>()
                .iter(world)
                .filter(|(_, r)| Some(r.room) == room)
                .map(|(e, _)| e)
                .collect()
        };
        for target in viewers {
            world.write_message(grim::events::InfoMessage {
                target,
                text: format!("The {name} rots away."),
            });
        }
        world.despawn(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_carried_corpse_gets_saved_deadline_and_expiry_removes_only_it() {
        let mut world = World::new();
        world.init_resource::<Messages<grim::events::InfoMessage>>();
        let dir = std::env::temp_dir().join(format!("grim-decay-{}", rand::random::<u64>()));
        world.insert_resource(grim::plugins::PersistenceConfig { dir: dir.clone() });
        let stored:StoredCharacter=serde_json::from_value(serde_json::json!({
            "id":grim::GrimId::new(),"account_id":grim::GrimId::new(),"name":"Reader","created_at":"2026-10-17T00:00:00Z"
        })).unwrap();
        let actor = world.spawn(stored.into_components()).id();
        let item = world
            .spawn((
                Object,
                GrimName("corpse of the rat".into()),
                Keywords(vec!["corpse".into()]),
                CarriedBy { carrier: actor },
            ))
            .id();
        let coin = world
            .spawn((
                Object,
                GrimName("coin".into()),
                CarriedBy { carrier: actor },
            ))
            .id();
        let other = world
            .spawn((
                Object,
                GrimName("corpse of the rat".into()),
                Keywords(vec!["corpse".into()]),
                CarriedBy { carrier: actor },
                ItemCondition {
                    quality: 100,
                    decay: Some(ItemDecay {
                        started_at: now(),
                        expires_at: now() + 3600,
                    }),
                },
            ))
            .id();
        tick_at(&mut world, now());
        let read = || {
            serde_json::from_slice::<StoredCharacter>(
                &std::fs::read(dir.join("characters/Reader.json")).unwrap(),
            )
            .unwrap()
        };
        let saved = read();
        let deadline = world
            .get::<ItemCondition>(item)
            .unwrap()
            .decay
            .as_ref()
            .unwrap()
            .expires_at;
        assert!(saved.inventory.iter().any(|o| o
            .condition
            .as_ref()
            .and_then(|c| c.decay.as_ref())
            .is_some_and(|d| d.expires_at == deadline)));
        tick_at(&mut world, deadline - 1);
        assert_eq!(
            world
                .get::<ItemCondition>(item)
                .unwrap()
                .decay
                .as_ref()
                .unwrap()
                .expires_at,
            deadline
        );
        tick_at(&mut world, deadline);
        assert!(world.get_entity(item).is_err());
        assert!(world.get_entity(coin).is_ok());
        assert!(world.get_entity(other).is_ok());
        let remaining = read();
        assert_eq!(
            remaining
                .inventory
                .iter()
                .map(|o| o.name.as_str())
                .collect::<Vec<_>>(),
            vec!["coin", "corpse of the rat"]
        );
        assert_eq!(
            remaining.inventory[1]
                .condition
                .as_ref()
                .unwrap()
                .decay
                .as_ref()
                .unwrap()
                .expires_at,
            world
                .get::<ItemCondition>(other)
                .unwrap()
                .decay
                .as_ref()
                .unwrap()
                .expires_at
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn corpse_stages_keep_the_same_deadline_and_expire() {
        let mut world = World::new();
        world.init_resource::<Messages<grim::events::InfoMessage>>();
        let room = world.spawn_empty().id();
        let item = world
            .spawn((
                Object,
                GrimName("corpse of the rat".into()),
                Keywords(vec!["corpse".into()]),
                InRoom { room },
                ItemCondition {
                    quality: 100,
                    decay: Some(ItemDecay {
                        started_at: 100,
                        expires_at: 400,
                    }),
                },
            ))
            .id();
        tick_at(&mut world, 200);
        assert_eq!(
            world.get::<ItemCondition>(item).unwrap().label(),
            "starting to decay"
        );
        tick_at(&mut world, 260);
        assert_eq!(
            world.get::<ItemCondition>(item).unwrap().label(),
            "decaying"
        );
        tick_at(&mut world, 350);
        assert_eq!(
            world.get::<ItemCondition>(item).unwrap().label(),
            "badly decayed"
        );
        assert_eq!(
            world
                .get::<ItemCondition>(item)
                .unwrap()
                .decay
                .as_ref()
                .unwrap()
                .expires_at,
            400
        );
        tick_at(&mut world, 400);
        assert!(world.get_entity(item).is_err());
    }
}

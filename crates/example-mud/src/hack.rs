//! Persistent, text-only terminal puzzles. Effects belong to this game, not the engine.
use crate::savage_rules::SoulData;
use bevy::prelude::*;
use grim::components::{Keywords, Name as GrimName};
use grim::{Actor, Area, Cardinal, Character, Doors, Exits, InRoom, Room, StoredCharacter};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
mod challenge;
use challenge::{display, fresh, likeness};

const WORDS: [&str; 8] = [
    "CRANE", "CRATE", "TRACE", "GRACE", "BRACE", "BRAKE", "FLAME", "FRAME",
];
fn attempts() -> u8 {
    4
}
fn cooldown() -> u64 {
    60
}
#[derive(Component, Clone, Deserialize)]
pub(crate) struct Hackable {
    id: String,
    #[serde(default = "attempts")]
    attempts: u8,
    #[serde(default = "cooldown")]
    lockout_seconds: u64,
    effect: Effect,
}
#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Effect {
    Access,
    OpenExit {
        area: String,
        room: String,
        direction: String,
    },
    RoyalOakGate,
}
#[derive(Default, Serialize, Deserialize)]
struct State {
    active: Option<String>,
    devices: BTreeMap<String, Challenge>,
}
#[derive(Serialize, Deserialize)]
struct Challenge {
    room: String,
    device: String,
    words: Vec<String>,
    answer: String,
    remaining: u8,
    guesses: Vec<(String, usize)>,
    locked_until: u64,
    solved: bool,
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn reply(world: &mut World, actor: Entity, text: String) {
    world.write_message(grim::events::InfoMessage {
        target: actor,
        text,
    });
}
fn save(
    world: &mut World,
    actor: Entity,
    mut character: Character,
    soul: SoulData,
    state: &State,
) -> Result<(), String> {
    character.game_state.insert(
        "hacking".into(),
        serde_json::to_value(state).map_err(|e| e.to_string())?,
    );
    character.game_state.insert(
        "savage_soul".into(),
        serde_json::to_value(&soul).map_err(|e| e.to_string())?,
    );
    let mut stored = StoredCharacter::from_components(
        world.get::<GrimName>(actor).ok_or("Missing name.")?,
        world.get::<Actor>(actor).ok_or("Missing actor.")?,
        &character,
    );
    stored.inventory = crate::shop::held(world, actor)
        .into_iter()
        .map(|(_, o)| o)
        .collect();
    stored.inventory.sort_by(|a, b| a.name.cmp(&b.name));
    crate::shop::persist(world, &stored)
        .map_err(|e| format!("Could not save hacking progress: {e}. Nothing changed."))?;
    world.entity_mut(actor).insert((character, soul));
    Ok(())
}
/// Resolve and validate both sides before charging attempts or granting rewards.
fn effect_doors(world: &mut World, effect: &Effect) -> Result<Vec<(Entity, Cardinal)>, String> {
    let (area, slug, dir) = match effect {
        Effect::Access => return Ok(vec![]),
        Effect::OpenExit {
            area,
            room,
            direction,
        } => (
            area.as_str(),
            room.as_str(),
            Cardinal::parse(direction).ok_or("Terminal has an invalid exit direction.")?,
        ),
        Effect::RoyalOakGate => ("savsoul", "romainstwallgate", Cardinal::South),
    };
    let room = world
        .query::<(Entity, &Room)>()
        .iter(world)
        .find(|(_, r)| {
            r.friendly_id == slug
                && world
                    .get::<Area>(r.area)
                    .is_some_and(|a| a.friendly_id == area)
        })
        .map(|(e, _)| e)
        .ok_or("Terminal's controlled room is unavailable.")?;
    let other = world
        .get::<Exits>(room)
        .and_then(|e| e.exits.get(&dir))
        .copied()
        .ok_or("Terminal's controlled exit is unavailable.")?;
    if !world
        .get::<Doors>(room)
        .is_some_and(|d| d.doors.contains_key(&dir))
    {
        return Err("Terminal's controlled door is unavailable.".into());
    }
    let back = dir.opposite();
    let mut out = vec![(room, dir)];
    if world.get::<Exits>(other).and_then(|e| e.exits.get(&back)) == Some(&room)
        && world
            .get::<Doors>(other)
            .is_some_and(|d| d.doors.contains_key(&back))
    {
        out.push((other, back));
    }
    Ok(out)
}
fn reward(
    character: &mut Character,
    soul: &mut SoulData,
    effect: &Effect,
) -> Result<String, String> {
    if !matches!(effect, Effect::RoyalOakGate) {
        return Ok(String::new());
    }
    let mut quests: Vec<String> = character
        .game_state
        .get("quests")
        .map(|v| serde_json::from_value(v.clone()).map_err(|_| "Your quest record needs repair."))
        .transpose()?
        .unwrap_or_default();
    if quests.iter().any(|q| q == "Escape from Royal Oak") {
        return Ok(String::new());
    }
    let points = character
        .game_state
        .get("quest_points")
        .map(|v| v.as_u64().ok_or("Your quest-point record needs repair."))
        .transpose()?
        .unwrap_or(0);
    soul.total_xp = soul.total_xp.checked_add(2000).ok_or("XP overflow.")?;
    quests.push("Escape from Royal Oak".into());
    character
        .game_state
        .insert("quests".into(), serde_json::json!(quests));
    character.game_state.insert(
        "quest_points".into(),
        serde_json::json!(points.checked_add(10).ok_or("Quest-point overflow.")?),
    );
    Ok("\nQuest complete: Escape from Royal Oak. XP +2000. Quest points +10.".into())
}
pub(crate) fn command(world: &mut World, actor: Entity, arg: &str) -> Result<(), String> {
    let mut character = world
        .get::<Character>(actor)
        .cloned()
        .ok_or("Missing character.")?;
    let mut soul = world
        .get::<SoulData>(actor)
        .cloned()
        .ok_or("Missing character rules.")?;
    let mut state: State = character
        .game_state
        .get("hacking")
        .map(|v| serde_json::from_value(v.clone()).map_err(|_| "Your hacking record needs repair."))
        .transpose()?
        .unwrap_or_default();
    let input = arg.trim();
    let lower = input.to_ascii_lowercase();
    if lower == "help" {
        reply(world,actor,"Use hack terminal, hack try WORD, hack status, or hack abort. Match the secret five-letter code in four guesses. Each guess reports letters in the correct positions. Failed attempts lock that device for 60 seconds; reconnecting does not reset attempts.".into());
        return Ok(());
    }
    if lower.is_empty() || lower == "status" {
        let c = state
            .active
            .as_ref()
            .and_then(|id| state.devices.get(id))
            .ok_or("No active terminal. Use hack terminal or help hack.")?;
        reply(world, actor, display(c));
        return Ok(());
    }
    if lower == "abort" {
        state.active.take().ok_or("No active terminal.")?;
        save(world, actor, character, soul, &state)?;
        reply(
            world,
            actor,
            "Disconnected. Your challenge and remaining attempts are saved.".into(),
        );
        return Ok(());
    }
    if crate::savage_soul::in_combat(world, actor) {
        return Err("Finish combat before hacking.".into());
    }
    if soul.is_incapacitated() {
        return Err("You cannot hack in your current condition.".into());
    }
    let guess = lower
        .strip_prefix("try ")
        .or_else(|| lower.strip_prefix("guess "))
        .map(|w| w.trim().to_ascii_uppercase());
    let (room_id, key, device, fixtures) =
        select_device(world, actor, input, guess.is_some(), &state)?;
    if device.id.is_empty() || device.attempts < 4 || device.attempts > 20 {
        return Err("This terminal needs its hacking configuration repaired.".into());
    }
    if fixtures
        .iter()
        .filter(|(_, h, _, _)| h.id == device.id)
        .count()
        != 1
    {
        return Err("This terminal has an ambiguous device identity.".into());
    }
    let doors = effect_doors(world, &device.effect)?;
    state.active = Some(key.clone());
    let c = state
        .devices
        .entry(key)
        .or_insert_with(|| fresh(room_id.clone(), device.id.clone(), device.attempts));
    if c.locked_until > now() {
        return Err(format!(
            "Terminal locked. Try again in {} seconds.",
            c.locked_until.saturating_sub(now())
        ));
    }
    if !c.solved && c.remaining == 0 {
        *c = fresh(room_id, device.id.clone(), device.attempts);
    }
    let mut message = attempt(c, guess, &device, &mut character, &mut soul)?;
    let solved = c.solved;
    save(world, actor, character, soul, &state)?;
    if solved {
        for (room, dir) in doors {
            if let Some(mut d) = world.get_mut::<Doors>(room) {
                if let Some(d) = d.doors.get_mut(&dir) {
                    d.open = true;
                }
            }
        }
        if matches!(device.effect, Effect::RoyalOakGate) {
            message.push_str(
                "\nThe city gate slides open. Its wrist scanner still requires a carried Soulboy.",
            );
        }
    }
    reply(world, actor, message);
    Ok(())
}

type Fixture = (Entity, Hackable, String, Vec<String>);
fn select_device(
    world: &mut World,
    actor: Entity,
    input: &str,
    guessing: bool,
    state: &State,
) -> Result<(String, String, Hackable, Vec<Fixture>), String> {
    let room = world
        .get::<InRoom>(actor)
        .ok_or("You are not in a room.")?
        .room;
    let room_id = world
        .get::<Room>(room)
        .ok_or("Missing room.")?
        .id
        .to_string();
    let fixtures: Vec<_> = world
        .query::<(Entity, &Hackable, &InRoom, &GrimName, Option<&Keywords>)>()
        .iter(world)
        .filter(|(_, _, r, _, _)| r.room == room)
        .map(|(e, h, _, n, k)| {
            (
                e,
                h.clone(),
                n.0.clone(),
                k.map(|k| k.0.clone()).unwrap_or_default(),
            )
        })
        .collect();
    let (key, device) = if guessing {
        let key = state
            .active
            .clone()
            .ok_or("Connect first with hack terminal.")?;
        let c = state
            .devices
            .get(&key)
            .ok_or("Missing terminal progress.")?;
        if c.room != room_id {
            return Err("Return to the terminal before guessing.".into());
        }
        let device = fixtures
            .iter()
            .find(|(_, h, _, _)| h.id == c.device)
            .map(|(_, h, _, _)| h.clone())
            .ok_or("That terminal is no longer here.")?;
        (key, device)
    } else {
        let target = grim::parse_target(input, grim::ParseOptions::BEING)
            .ok_or("Name one terminal to hack.")?;
        let matches = grim::query(
            &target,
            fixtures
                .iter()
                .map(|(e, _, n, k)| (*e, n.as_str(), k.as_slice())),
        );
        let device = fixtures
            .iter()
            .find(|(e, _, _, _)| Some(e) == matches.first())
            .map(|(_, h, _, _)| h.clone())
            .ok_or("There is no hackable device matching that here.")?;
        (format!("{room_id}:{}", device.id), device)
    };
    Ok((room_id, key, device, fixtures))
}

fn attempt(
    c: &mut Challenge,
    guess: Option<String>,
    device: &Hackable,
    character: &mut Character,
    soul: &mut SoulData,
) -> Result<String, String> {
    let mut message: String;
    if c.solved {
        message = "Access granted. Override reapplied; no additional reward.".into();
    } else if let Some(guess) = guess {
        if !c.words.contains(&guess) {
            return Err(
                "Choose one of the listed codes. Use hack status to review them; no attempt used."
                    .into(),
            );
        }
        if c.guesses.iter().any(|(g, _)| g == &guess) {
            return Err("You already tried that code. No attempt used.".into());
        }
        let score = likeness(&guess, &c.answer);
        c.remaining -= 1;
        c.guesses.push((guess.clone(), score));
        if score == 5 {
            c.solved = true;
            message = "Access granted.".into();
            message.push_str(&reward(character, soul, &device.effect)?);
        } else if c.remaining == 0 {
            c.locked_until = now().saturating_add(device.lockout_seconds);
            message = format!(
                "{guess}: {score}/5. No attempts left. Terminal locked for {} seconds.",
                device.lockout_seconds
            );
        } else {
            message = format!("{guess}: {score}/5. {} attempts left.", c.remaining);
        }
    } else {
        message = display(c);
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    // A player can always deduce this bank within the advertised attempt budget.
    fn solvable(candidates: &[&str], depth: usize) -> bool {
        if candidates.len() <= depth {
            return true;
        }
        if depth == 0 {
            return false;
        }
        WORDS.iter().any(|guess| {
            (0..5).all(|score| {
                let branch: Vec<_> = candidates
                    .iter()
                    .copied()
                    .filter(|w| likeness(w, guess) == score)
                    .collect();
                branch.is_empty() || solvable(&branch, depth - 1)
            })
        })
    }
    #[test]
    fn bank_is_solvable_and_feedback_is_positional() {
        assert!(solvable(&WORDS, 4));
        assert_eq!(likeness("CRANE", "CRATE"), 4);
        assert_eq!(likeness("CRANE", "TRACE"), 3);
    }
}

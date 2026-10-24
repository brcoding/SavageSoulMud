//! `finger <name>`: a character sheet, online or off (the `sockets.rs`
//! precedent — one concern module holding its own lookup + render).
//!
//! Online characters answer from live components (case-insensitive name);
//! offline ones load the stored record from disk (exact filename match) with
//! the default description — custom descriptions do not persist yet.

use bevy::prelude::*;
use grim_color::escape_codes;
use grim_core::components::{Description, Gender};
use grim_persistence::{load_character_by_name, PersistenceConfig};
use grim_text::tr;

use crate::params::PlayerChars;

/// The `finger` sheet for one character name.
pub(crate) fn format(
    target: &str,
    player_chars: &PlayerChars,
    descriptions: &Query<&Description>,
    persistence: &PersistenceConfig,
    viewer: Entity,
) -> String {
    let observer = player_chars
        .get(viewer)
        .ok()
        .and_then(|(_, _, _, _, c, _, _)| c);
    if let Some((e, n, _, actor, character, _, _)) =
        player_chars.iter().find(|(_, n, _, _, character, _, _)| {
            character.is_some() && n.0.eq_ignore_ascii_case(target)
        })
    {
        if character.is_some_and(|c| !c.visible_to(observer, e == viewer)) {
            return tr!("finger.not_found");
        }
        let desc = descriptions.get(e).map(|d| d.0.clone()).unwrap_or_default();
        let actor = actor.as_ref();
        return render_sheet(
            &n.0,
            actor.map(|a| a.level).unwrap_or(1),
            actor.map(|a| &a.gender).unwrap_or(&Gender::Neutral),
            actor.map(|a| a.race.as_str()).unwrap_or(""),
            character.map(|c| c.class.as_str()).unwrap_or(""),
            &desc,
        );
    }
    if let Some(stored) = load_character_by_name(persistence, target) {
        if !crate::staff::stored_character(persistence, &stored).visible_to(observer, false) {
            return tr!("finger.not_found");
        }
        return render_sheet(
            &stored.name,
            stored.level,
            &stored.gender,
            &stored.race,
            &stored.class,
            &[tr!("character.default_description")],
        );
    }
    tr!("finger.not_found")
}

/// Render the sheet: identity fields plus the description paragraphs joined
/// by single newlines (no extra blank line — the author owns spacing).
/// Name/race/class are escaped: they are data and must never read as colour
/// markup.
fn render_sheet(
    name: &str,
    level: u32,
    gender: &Gender,
    race: &str,
    class: &str,
    desc: &[String],
) -> String {
    let gender = match gender {
        Gender::Male => "Male",
        Gender::Female => "Female",
        Gender::Neutral => "Neutral",
    };
    format!(
        "Name: {}\nLevel: {}\nGender: {}\nRace: {}\nClass: {}\nDescription:\n{}\n",
        escape_codes(name),
        level,
        gender,
        escape_codes(race),
        escape_codes(class),
        desc.join("\n")
    )
}

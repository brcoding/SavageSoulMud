//! dialogue implementation, split from the parent subsystem.

use super::*;

/// Recovered trainer interaction, adapted to Savage Soul XP rather than LP skills.
pub(crate) fn ask(world: &mut World, actor: Entity, arg: &str) -> Result<(), String> {
    let input = arg.trim().to_lowercase();
    let room = world.get::<InRoom>(actor).ok_or("You are nowhere.")?.room;
    let candidates: Vec<_> = world
        .query::<(Entity, &GrimName, &InRoom, Option<&Keywords>, &Creature)>()
        .iter(world)
        .filter(|(_, _, location, _, _)| location.room == room)
        .map(|(entity, name, _, keywords, _)| {
            (
                entity,
                name.0.clone(),
                keywords.map(|k| k.0.clone()).unwrap_or_default(),
            )
        })
        .collect();
    let input = input.strip_prefix("to ").unwrap_or(&input);
    let mut names: Vec<_> = candidates
        .iter()
        .flat_map(|(_, name, keywords)| {
            std::iter::once(name.to_lowercase()).chain(keywords.iter().map(|k| k.to_lowercase()))
        })
        .collect();
    names.sort_by_key(|name| std::cmp::Reverse(name.len()));
    let target = names.iter().find(|name| input == name.as_str() || input.strip_prefix(name.as_str()).is_some_and(|tail| tail.starts_with(' '))).ok_or("Name someone here, followed by your question. For example: ask tully how do I level up?")?;
    let topic = input[target.len()..].trim();
    let topic = topic
        .strip_prefix("for ")
        .or_else(|| topic.strip_prefix("about "))
        .unwrap_or(topic);
    let spec = grim::parse_target(target, grim::ParseOptions::BEING)
        .ok_or("Who would you like to ask?")?;
    let matches = grim::query(
        &spec,
        candidates
            .iter()
            .map(|(entity, name, keywords)| (*entity, name.as_str(), keywords.as_slice())),
    );
    let npc = *matches.first().ok_or("That person is not here.")?;
    let name = &world
        .get::<GrimName>(npc)
        .ok_or("That person is not here.")?
        .0;
    if !name.eq_ignore_ascii_case("tully") {
        reply(world, actor, format!("{name} has no advice about that."));
        return Ok(());
    }
    let mut question = topic.to_string();
    if let Some(previous) = world
        .get::<crate::trainer::Conversation>(actor)
        .filter(|previous| previous.npc == npc)
    {
        let words: Vec<_> = topic.split(|c: char| !c.is_alphanumeric()).collect();
        if words
            .iter()
            .any(|word| ["it", "that", "this"].contains(word))
            && !crate::savage_rules::ATTRIBUTES
                .iter()
                .any(|a| topic.contains(&a.replace('_', " ")))
        {
            question.push(' ');
            question.push_str(&previous.attribute.replace('_', " "));
        }
    }
    if crate::trainer_ai::submit(world, actor, npc, &question) {
        return Ok(());
    }
    let soul = world
        .get::<SoulData>(actor)
        .ok_or("Your Savage Soul record could not be loaded.")?;
    let text = crate::trainer::advice(soul, &question);
    if let Some(attribute) = crate::savage_rules::ATTRIBUTES
        .iter()
        .find(|a| question.contains(&a.replace('_', " ")))
    {
        world
            .entity_mut(actor)
            .insert(crate::trainer::Conversation {
                npc,
                attribute: attribute.to_string(),
            });
    }
    reply(world, actor, text);
    Ok(())
}

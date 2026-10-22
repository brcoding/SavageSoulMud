//! Game-owned command vocabulary and extensions to ordinary engine verbs.
use bevy::prelude::*;
use grim::{events::Command, Cardinal, CommandRegistry};

pub(crate) fn register(app: &mut App) {
    let Some(mut registry) = app.world_mut().get_resource_mut::<CommandRegistry<Command>>() else {
        return;
    };
    for word in ["creator", "rebuild", "respec", "race", "races", "buyoff", "ask", "talk", "score", "stats", "train", "pecs", "learn", "wield", "wear", "remove", "attack", "kill", "stop", "defend", "rest", "recover", "task", "soulheal", "awardxp", "exchange", "drink", "eat", "disarm", "activate", "trample", "stomp", "screenreader", "combatlog", "prompt", "spawns", "put", "place", "sit", "lie", "lay", "stand", "hack", "craft", "reduce", "recipes", "reductions", "resources", "press", "intermud", "i3", "imud", "zebedee", "i2", "protocols", "grapevine", "gv"] {
        let verb = word.to_string();
        registry.register(word, move |rest| Some(Command::SavageSoul { verb: verb.clone(), arg: rest.into() }));
        registry.deprioritize(word);
    }
    registry.replace("look", look);
    registry.replace("read", |rest| (!rest.is_empty()).then(|| look(rest)).flatten());
    registry.replace("get", |rest| {
        if rest.is_empty() { return None; }
        Some(if rest.to_ascii_lowercase().contains(" from ") {
            Command::SavageSoul { verb: "get_from".into(), arg: rest.into() }
        } else { Command::Get { target: rest.into() } })
    });
    registry.replace("open", |rest| door(rest, true));
    registry.replace("close", |rest| door(rest, false));
}

fn look(rest: &str) -> Option<Command> {
    let lower = rest.to_ascii_lowercase();
    Some(if ["desk", "drawer", "wooden desk", "in desk", "in drawer", "in wooden desk"].contains(&lower.as_str()) {
        Command::SavageSoul { verb: "inspect".into(), arg: lower }
    } else if ["note", "dagger note", "paper", "slip"].contains(&lower.as_str()) {
        Command::SavageSoul { verb: "read_note".into(), arg: rest.into() }
    } else { Command::Look { target: (!rest.is_empty()).then(|| rest.into()) } })
}

fn door(rest: &str, open: bool) -> Option<Command> {
    if rest.is_empty() { return None; }
    Some(match (Cardinal::parse(rest), open) {
        (Some(direction), true) => Command::Open { direction },
        (Some(direction), false) => Command::Close { direction },
        (None, _) => Command::SavageSoul { verb: if open { "open_object" } else { "close_object" }.into(), arg: rest.into() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plugin_commands_preserve_engine_prefixes_and_directional_doors() {
        let mut app = App::new();
        app.add_plugins(grim::GrimHeadlessPlugins);
        register(&mut app);
        let registry = app.world().resource::<CommandRegistry<Command>>();
        assert_eq!(registry.resolve("n", ""), Some(Command::Move { direction: Cardinal::North }));
        assert!(matches!(registry.resolve("l", "desk"), Some(Command::SavageSoul { verb, .. }) if verb == "inspect"));
        assert!(matches!(registry.resolve("open", "east"), Some(Command::Open { .. })));
        assert!(matches!(registry.resolve("get", "all from corpse"), Some(Command::SavageSoul { verb, .. }) if verb == "get_from"));
        assert!(matches!(registry.resolve("read", "sign"), Some(Command::Look { .. })));
        assert!(matches!(registry.resolve("kill", "rat"), Some(Command::SavageSoul { .. })));
    }
}

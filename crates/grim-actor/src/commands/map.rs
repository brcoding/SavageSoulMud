//! The `map` command: render the area around the actor's room as ASCII.

use std::collections::HashMap;

use bevy::prelude::*;
use grim_core::events::{Command, EngineCommand, InfoMessage};
use grim_world::{render_map, Doors, Exits, MapConfig};

use crate::placement::InRoom;

/// `map`: show the area around the actor's room (`@` self, `o` rooms,
/// `--`/`|` exits, `,`/`'` up/down markers, coloured per
/// [`grim_world::render_map`]). Reads [`InRoom`] plus the world
/// topology and answers only the actor via [`InfoMessage`]. An actor with no
/// room is silently ignored (fail closed, like `look`). Hidden exits never
/// draw (secret doors stay off the canvas for everyone; admins see them in
/// the `look` text instead).
pub(crate) fn handle_map(
    mut engine: MessageReader<EngineCommand>,
    inroom: Query<&InRoom>,
    characters: Query<&crate::Character>,
    exits: Query<(Entity, &Exits)>,
    doors: Query<&Doors>,
    visibility: crate::visibility::Visibility,
    mut info: MessageWriter<InfoMessage>,
) {
    for cmd in engine.read() {
        let Command::Map = &cmd.command else {
            continue;
        };
        let actor = cmd.client;
        let Ok(actor_room) = inroom.get(actor) else {
            continue;
        };
        if !visibility.can_see(actor, actor_room.room) {
            info.write(InfoMessage {
                target: actor,
                text: "It is too dark to read the surroundings. Bring a light or use night vision."
                    .into(),
            });
            continue;
        }
        let mut snapshot = HashMap::new();
        for (room, links) in exits.iter() {
            let mut visible = links.exits.clone();
            if let Ok(d) = doors.get(room) {
                visible.retain(|dir, _| !d.doors.get(dir).is_some_and(|door| door.hidden));
            }
            snapshot.insert(room, visible);
        }
        if characters
            .get(actor)
            .is_ok_and(|c| c.config.get("screenreader").is_some_and(|v| v == "on"))
        {
            let mut directions: Vec<_> = snapshot
                .get(&actor_room.room)
                .into_iter()
                .flat_map(|links| links.keys())
                .copied()
                .collect();
            directions.sort();
            let names = directions
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            // tr-bypass: accessible structural listing of dynamic exit names.
            info.write(InfoMessage {
                target: actor,
                text: format!(
                    "Exits: {}. Use look for room descriptions and doors.\n",
                    if names.is_empty() {
                        "none".to_string()
                    } else {
                        names.join(", ")
                    }
                ),
            });
            continue;
        }
        let rows = render_map(actor_room.room, &snapshot, &MapConfig::MAP);
        info.write(InfoMessage {
            target: actor,
            text: rows.join("\n") + "\n",
        });
    }
}

/// Wire the `map` handler and the messages it reads/emits.
pub(crate) fn register(app: &mut App) {
    app.add_message::<EngineCommand>()
        .add_message::<InfoMessage>()
        .add_systems(Update, handle_map);
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_core::cardinal::Cardinal;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        register(&mut app);
        app
    }

    fn send_map(app: &mut App, actor: Entity) {
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Map,
        });
        app.update();
    }

    fn info_texts(app: &App) -> Vec<String> {
        let m = app.world().resource::<Messages<InfoMessage>>();
        let mut c = m.get_cursor();
        c.read(m).map(|i| i.text.clone()).collect()
    }

    #[test]
    fn map_renders_actor_room_and_neighbor() {
        let mut app = test_app();
        let north = app.world_mut().spawn(Exits::default()).id();
        let mut links = HashMap::new();
        links.insert(Cardinal::North, north);
        let room = app.world_mut().spawn(Exits { exits: links }).id();
        let actor = app.world_mut().spawn(InRoom { room }).id();

        send_map(&mut app, actor);

        let texts = info_texts(&app);
        assert_eq!(texts.len(), 1);
        // Self centered on row 10, the northern room two rows above.
        let rows: Vec<&str> = texts[0].lines().collect();
        assert_eq!(rows.len(), 20);
        assert_eq!(rows[10], format!("{:40}{{R@@{{x", ""));
        assert_eq!(rows[9], format!("{:40}{{w|{{x", ""));
        assert_eq!(rows[8], format!("{:40}{{go{{x", ""));
    }

    #[test]
    fn map_for_roomless_actor_is_silent() {
        let mut app = test_app();
        let drifter = app.world_mut().spawn(()).id();

        send_map(&mut app, drifter);

        assert!(info_texts(&app).is_empty());
    }
}

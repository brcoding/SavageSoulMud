//! commands implementation, split from the parent subsystem.

use super::*;

pub(super) fn preference(
    world: &mut World,
    actor: Entity,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let mut character = world
        .get::<Character>(actor)
        .ok_or("No character.")?
        .clone();
    character.config.insert(key.into(), value.into());
    if let Some(soul) = world.get::<crate::savage_rules::SoulData>(actor) {
        character.game_state.insert(
            "savage_soul".into(),
            serde_json::to_value(soul).map_err(|e| e.to_string())?,
        );
    }
    let mut stored = StoredCharacter::from_components(
        world.get::<GrimName>(actor).ok_or("No name.")?,
        world.get::<Actor>(actor).ok_or("No actor.")?,
        &character,
    );
    stored.inventory = crate::shop::held(world, actor)
        .into_iter()
        .map(|(_, item)| item)
        .collect();
    crate::shop::persist(world, &stored).map_err(|e| format!("Could not save preference: {e}"))?;
    world.entity_mut(actor).insert(character);
    Ok(())
}

pub(super) fn client_links(
    world: &mut World,
    connection: Entity,
    caps: &TelnetCapabilities,
) -> bool {
    let mut bytes = Vec::new();
    if caps.mxp {
        bytes.extend(b"\x1b[1z<SEND HREF=\"help\">Help</SEND> | <SEND HREF=\"score\">Score</SEND> | <SEND HREF=\"inventory\">Inventory</SEND> | <SEND HREF=\"look\">Look</SEND>\x1b[7z\n");
    }
    if caps
        .terminal_types
        .iter()
        .any(|term| term.to_ascii_uppercase().contains("MUDLET"))
    {
        bytes.extend(b"\x1b]8;;https://wiki.mudlet.org/w/Manual:Supported_Protocols\x1b\\Mudlet protocol reference\x1b]8;;\x1b\\\n");
    }
    let sent = !bytes.is_empty();
    if sent {
        protocol(world, connection, 0, bytes);
    }
    sent
}

pub(crate) fn command(world: &mut World, actor: Entity, arg: &str) -> Result<(), String> {
    let connection = world
        .get::<Player>(actor)
        .ok_or("You are not connected.")?
        .connection;
    let caps = world
        .get::<TelnetCapabilities>(connection)
        .cloned()
        .unwrap_or_default();
    let mut reply = String::new();
    match arg.trim() {
        "links on" => {
            preference(world, actor, "client_links", "on")?;
            world
                .resource_mut::<State>()
                .clients
                .entry(connection)
                .or_default()
                .links_shown = false;
            reply.push_str("Client links enabled and saved. Use protocols links off to disable.");
        }
        "links off" => {
            preference(world, actor, "client_links", "off")?;
            reply.push_str("Client links disabled and saved.");
        }
        "sound on" => {
            preference(world, actor, "client_sound", "on")?;
            reply.push_str(
                "Client sound enabled and saved. Use protocols sound test to play the sample.",
            );
        }
        "sound off" => {
            preference(world, actor, "client_sound", "off")?;
            reply.push_str("Client sound disabled and saved.");
        }
        "sound test" => {
            if !world.get::<Character>(actor).is_some_and(|c| {
                c.config
                    .get("client_sound")
                    .is_some_and(|value| value == "on")
            }) {
                return Err("Enable optional audio first with protocols sound on.".into());
            }
            let url = world.resource::<Settings>().media_base_url.clone();
            if !(url.starts_with("http://") || url.starts_with("https://"))
                || url.chars().any(|c| {
                    c.is_control() || c.is_whitespace() || matches!(c, '(' | ')' | '\"' | '\'')
                })
            {
                return Err("No valid media URL is configured.".into());
            }
            if caps.gmcp && package_supported(&caps, "Client.Media") {
                gmcp(
                    world,
                    connection,
                    "Client.Media.Play",
                    &json!({"name":"protocol-ready.wav","type":"sound","url":url,"volume":20}),
                );
            } else if caps.msp {
                protocol(
                    world,
                    connection,
                    0,
                    format!("!!SOUND(protocol-ready.wav V=20 U={url})\n").into_bytes(),
                );
            } else {
                return Err("Your client has not enabled MSP or GMCP Client.Media.".into());
            }
            reply.push_str("Sent the optional sound sample to your client.");
        }
        "" | "status" => {
            reply=format!("Client protocols: GMCP {}, MSDP {}, MSSP {}, MXP {}, MSP {}, ATCP {}, UTF-8 {}. Terminal: {}x{}; {}.\nUse protocols links on|off, protocols sound on|off, or protocols sound test. TLS connects on the separately advertised port; capability flags alone do not prove encryption.",caps.gmcp,caps.msdp,caps.mssp,caps.mxp,caps.msp,caps.atcp,caps.utf8,caps.width,caps.height,caps.terminal_types.join(", "));
        }
        _ => {
            return Err(
                "Use protocols [status|links on|links off|sound on|sound off|sound test].".into(),
            )
        }
    }
    world.write_message(grim::events::InfoMessage {
        target: actor,
        text: reply,
    });
    Ok(())
}

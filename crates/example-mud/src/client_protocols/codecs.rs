//! codecs implementation, split from the parent subsystem.

use super::*;

pub(super) fn msdp_value(out: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Object(fields) => {
            out.push(3);
            for (key, val) in fields {
                out.push(1);
                out.extend(key.bytes().filter(|b| *b >= 32));
                out.push(2);
                msdp_value(out, val);
            }
            out.push(4);
        }
        Value::Array(values) => {
            out.push(5);
            for value in values {
                out.push(2);
                msdp_value(out, value);
            }
            out.push(6);
        }
        Value::String(text) => out.extend(text.bytes().filter(|b| *b >= 32 && *b != 127)),
        _ => out.extend(value.to_string().bytes()),
    }
}

pub(super) fn msdp_pair(name: &str, value: &Value) -> Vec<u8> {
    let mut out = vec![1];
    out.extend(name.bytes());
    out.push(2);
    msdp_value(&mut out, value);
    out
}

/// Commands contain only printable names, flat values or an array of names.
pub(super) fn msdp_commands(payload: &[u8]) -> Vec<(String, Vec<String>)> {
    if payload.len() > 8192 {
        return vec![];
    }
    let mut commands = Vec::new();
    let mut remaining = MSDP_VALUES_PER_INPUT;
    for part in payload
        .split(|byte| *byte == 1)
        .skip(1)
        .take(MSDP_VALUES_PER_INPUT)
    {
        if remaining == 0 {
            break;
        }
        let Some(split) = part.iter().position(|b| *b == 2) else {
            continue;
        };
        let name = String::from_utf8_lossy(&part[..split]).to_uppercase();
        let values: Vec<_> = part[split + 1..]
            .split(|byte| matches!(*byte, 2 | 5 | 6))
            .map(|value| String::from_utf8_lossy(value).trim().to_uppercase())
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 64
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            })
            .take(remaining)
            .collect();
        remaining -= values.len();
        commands.push((name, values));
    }
    commands
}

pub(super) fn mssp(world: &mut World, connection: Entity) {
    let settings = world.resource::<Settings>().clone();
    let players = world
        .query::<(&Player, Option<&Character>)>()
        .iter(world)
        .filter(|(_, c)| c.is_none_or(|c| !c.is_hidden()))
        .count();
    let mut fields = vec![
        ("NAME", settings.name.clone()),
        ("PLAYERS", players.to_string()),
        ("UPTIME", settings.boot_at.to_string()),
        ("PORT", "4000".into()),
        ("CODEBASE", "Grim".into()),
        ("FAMILY", "Custom".into()),
        ("GAMESYSTEM", "Savage Soul".into()),
        ("STATUS", "Development".into()),
        ("LANGUAGE", "English".into()),
        ("CHARSET", "UTF-8".into()),
        ("GMCP", "1".into()),
        ("MSDP", "1".into()),
        ("MSSP", "1".into()),
        ("MXP", "1".into()),
        ("MSP", "1".into()),
        ("MTTS", "1".into()),
        ("ANSI", "1".into()),
        ("INTERMUD", "I3,Zebedee".into()),
    ];
    if !settings.website.is_empty() {
        fields.push(("WEBSITE", settings.website.clone()));
    }
    if settings.tls_port != 0 {
        fields.push(("TLS", settings.tls_port.to_string()));
    }
    let mut bytes = Vec::new();
    for (name, value) in fields {
        bytes.push(1);
        bytes.extend(name.bytes());
        bytes.push(2);
        bytes.extend(value.bytes().filter(|b| *b >= 32 && *b != 127));
    }
    protocol(world, connection, MSSP, bytes);
}

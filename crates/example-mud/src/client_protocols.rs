//! Game-owned Mudlet payloads. Transport negotiation stays in the telnet crate.
use bevy::{ecs::message::MessageCursor, prelude::*};
use grim::components::Name as GrimName;
use grim::{
    Actor, Area, CarriedBy, Character, Connection, Doors, Exits, InRoom, Object, Player, Room,
    StoredCharacter, TelnetCapabilities, TelnetInput, TelnetOutput,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
mod snapshot;
use snapshot::{inventory, snapshot, variables};
mod codecs;
use codecs::{msdp_commands, msdp_pair, mssp};
mod commands;
use commands::client_links;
pub(crate) use commands::command;
mod session;
use session::tick;

const REPORTABLE: &[&str] = &[
    "INJURY",
    "MANA",
    "MANA_MAX",
    "EXPERIENCE",
    "EXPERIENCE_UNSPENT",
    "CREDITS",
    "WINDED",
    "FATIGUED",
    "STUNNED",
    "ROOM_VNUM",
    "ROOM_NAME",
    "AREA_NAME",
    "ROOM_EXITS",
    "CHARACTER_NAME",
];
const INPUTS_PER_TICK: usize = 16;
const INPUT_BACKLOG: usize = 128;
const MSDP_VALUES_PER_INPUT: usize = 32;
const GMCP: u8 = 201;
const MSDP: u8 = 69;
const MSSP: u8 = 70;
const ATCP: u8 = 200;

#[derive(Resource, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    name: String,
    website: String,
    media_base_url: String,
    map_url: String,
    gui_url: String,
    gui_version: String,
    tls_port: u16,
    #[serde(skip)]
    boot_at: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            name: "The Savage Soul".into(),
            website: String::new(),
            media_base_url: String::new(),
            map_url: String::new(),
            gui_url: String::new(),
            gui_version: "1".into(),
            tls_port: 0,
            boot_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}
#[derive(Default)]
struct ClientState {
    owner: Option<Entity>,
    last: HashMap<String, Value>,
    reported: HashSet<String>,
    mssp_sent: bool,
    last_inventory: Value,
    links_shown: bool,
    map_offered: bool,
    pending: VecDeque<TelnetInput>,
    overloaded: bool,
}
#[derive(Resource, Default, Deserialize)]
struct MapperIds {
    rooms: BTreeMap<String, u32>,
}
fn mapper_id(world: &World, id: &str) -> Option<u32> {
    world
        .get_resource::<MapperIds>()
        .and_then(|m| m.rooms.get(id).copied())
}
#[derive(Resource, Default)]
struct State {
    clients: HashMap<Entity, ClientState>,
    next: f64,
}

pub(crate) fn register(app: &mut App) {
    crate::client_auth::register(app);
    let path = app
        .world()
        .get_resource::<crate::seed::AreaBlueprintDir>()
        .map(|d| d.0.parent().unwrap_or(&d.0).join("mudlet.json"))
        .unwrap_or_else(|| "data/mudlet.json".into());
    let ids_path = path.with_file_name("mudlet-map-ids.json");
    let ids = std::fs::read(ids_path)
        .ok()
        .and_then(|v| serde_json::from_slice::<MapperIds>(&v).ok())
        .unwrap_or_default();
    app.insert_resource(ids);
    let settings = std::fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice::<Settings>(&data).ok())
        .unwrap_or_default();
    app.insert_resource(settings)
        .init_resource::<State>()
        .add_message::<TelnetInput>()
        .add_message::<TelnetOutput>()
        .add_systems(Update, tick.after(grim::SceneSystems::Staff));
}
fn protocol(world: &mut World, connection: Entity, option: u8, payload: Vec<u8>) {
    world.write_message(TelnetOutput {
        connection,
        option,
        payload,
    });
}
fn gmcp(world: &mut World, connection: Entity, name: &str, value: &Value) {
    protocol(
        world,
        connection,
        GMCP,
        format!("{name} {value}").into_bytes(),
    );
}

fn package_supported(caps: &TelnetCapabilities, package: &str) -> bool {
    caps.gmcp_supports.is_empty()
        || caps.gmcp_supports.iter().any(|supported| {
            let name = supported.split_whitespace().next().unwrap_or("");
            name == package
                || package
                    .strip_prefix(name)
                    .is_some_and(|tail| tail.starts_with('.'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_player_count_excludes_hidden_staff() {
        let mut world = World::new();
        world.init_resource::<Settings>();
        world.init_resource::<Messages<TelnetOutput>>();
        for (name, roles) in [("Visible", vec![]), ("Hidden", vec![grim::Role::Tester])] {
            let mut stored:StoredCharacter=serde_json::from_value(json!({"id":grim::GrimId::new(),"account_id":grim::GrimId::new(),"name":name,"created_at":"2026-10-23T00:00:00Z"})).unwrap();
            stored.roles = roles;
            world.spawn(stored.into_components()).insert(Player {
                connection: Entity::PLACEHOLDER,
            });
        }
        mssp(&mut world, Entity::PLACEHOLDER);
        let messages = world.resource::<Messages<TelnetOutput>>();
        let mut cursor = messages.get_cursor();
        let payload = &cursor.read(messages).next().unwrap().payload;
        assert!(payload
            .windows(b"\x01PLAYERS\x021\x01".len())
            .any(|part| part == b"\x01PLAYERS\x021\x01"));
    }
    #[test]
    fn deferred_reports_are_processed_in_order_on_the_next_tick() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<State>()
            .init_resource::<Settings>()
            .init_resource::<MapperIds>();
        app.add_message::<TelnetInput>()
            .add_message::<TelnetOutput>()
            .add_systems(Update, tick.after(grim::SceneSystems::Staff));
        let connection = app
            .world_mut()
            .spawn((
                Connection {
                    id: 1,
                    addr: "127.0.0.1:1234".parse().unwrap(),
                    echo_hidden: false,
                },
                TelnetCapabilities {
                    gmcp: true,
                    msdp: true,
                    ..Default::default()
                },
            ))
            .id();
        for _ in 0..3 {
            app.world_mut().write_message(TelnetInput {
                connection,
                option: GMCP,
                payload: b"Core.Ping".to_vec(),
            });
        }
        for variable in REPORTABLE {
            app.world_mut().write_message(TelnetInput {
                connection,
                option: MSDP,
                payload: msdp_pair("REPORT", &json!(variable)),
            });
        }
        app.update();
        let client = &app.world().resource::<State>().clients[&connection];
        assert_eq!(client.reported.len(), 13);
        assert_eq!(client.pending.len(), 1);
        assert!(!client.reported.contains("CHARACTER_NAME"));
        app.update();
        let client = &app.world().resource::<State>().clients[&connection];
        assert_eq!(client.reported.len(), REPORTABLE.len());
        assert!(client.pending.is_empty());
        assert!(client.reported.contains("CHARACTER_NAME"));
    }

    #[test]
    fn backlog_overflow_requests_an_explicit_disconnect() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<State>()
            .init_resource::<Settings>()
            .init_resource::<MapperIds>();
        app.add_message::<TelnetInput>()
            .add_message::<TelnetOutput>()
            .add_message::<grim::ConnectionOutput>()
            .add_message::<grim::DisconnectRequest>()
            .add_systems(Update, tick.after(grim::SceneSystems::Staff));
        let connection = app
            .world_mut()
            .spawn((
                Connection {
                    id: 1,
                    addr: "127.0.0.1:1234".parse().unwrap(),
                    echo_hidden: false,
                },
                TelnetCapabilities {
                    gmcp: true,
                    ..Default::default()
                },
            ))
            .id();
        for _ in 0..=INPUT_BACKLOG {
            app.world_mut().write_message(TelnetInput {
                connection,
                option: GMCP,
                payload: b"Core.Ping".to_vec(),
            });
        }
        app.update();
        let client = &app.world().resource::<State>().clients[&connection];
        assert!(client.overloaded);
        assert!(client.pending.is_empty());
        let requests = app.world().resource::<Messages<grim::DisconnectRequest>>();
        assert_eq!(
            requests
                .get_cursor()
                .read(requests)
                .filter(|r| r.connection == connection)
                .count(),
            1
        );
        let messages = app.world().resource::<Messages<grim::ConnectionOutput>>();
        assert!(messages
            .get_cursor()
            .read(messages)
            .any(|m| m.connection == connection && m.text.contains("reconnect to resynchronize")));
    }
    #[test]
    fn msdp_caps_total_values_and_commands_across_the_payload() {
        let payload = b"\x01SEND\x02MANA".repeat(100);
        let commands = msdp_commands(&payload);
        assert_eq!(
            commands
                .iter()
                .map(|(_, values)| values.len())
                .sum::<usize>(),
            MSDP_VALUES_PER_INPUT
        );
        assert!(commands.len() <= MSDP_VALUES_PER_INPUT);
        let payload = b"\x01\x02".repeat(2000);
        assert!(msdp_commands(&payload).len() <= MSDP_VALUES_PER_INPUT);
    }

    #[test]
    fn protocol_bursts_obey_each_connections_tick_budget() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<State>()
            .init_resource::<Settings>()
            .init_resource::<MapperIds>();
        app.add_message::<TelnetInput>()
            .add_message::<TelnetOutput>()
            .add_systems(Update, tick.after(grim::SceneSystems::Staff));
        let mut connections = Vec::new();
        for id in 1..=2 {
            let connection = app
                .world_mut()
                .spawn((
                    Connection {
                        id,
                        addr: "127.0.0.1:1234".parse().unwrap(),
                        echo_hidden: false,
                    },
                    TelnetCapabilities {
                        gmcp: true,
                        ..Default::default()
                    },
                ))
                .id();
            connections.push(connection);
            for _ in 0..100 {
                app.world_mut().write_message(TelnetInput {
                    connection,
                    option: GMCP,
                    payload: b"Core.Ping".to_vec(),
                });
            }
        }
        app.update();
        let outputs = app.world().resource::<Messages<TelnetOutput>>();
        let mut cursor = outputs.get_cursor();
        let frames: Vec<_> = cursor.read(outputs).collect();
        for connection in connections {
            assert_eq!(
                frames
                    .iter()
                    .filter(|f| f.connection == connection && f.payload == b"Core.Ping")
                    .count(),
                INPUTS_PER_TICK
            );
        }
    }
    #[test]
    fn msdp_array_and_table_are_structured_not_json_text() {
        assert_eq!(
            msdp_pair("ROOM_EXITS", &json!({"n":"abc"})),
            b"\x01ROOM_EXITS\x02\x03\x01n\x02abc\x04"
        );
        let commands = msdp_commands(b"\x01REPORT\x02\x05\x02MANA\x02INJURY\x06");
        assert_eq!(commands[0].1, vec!["MANA", "INJURY"]);
    }
    #[test]
    fn module_supports_does_not_match_another_namespace() {
        let caps = TelnetCapabilities {
            gmcp_supports: vec!["Char 1".into()],
            ..Default::default()
        };
        assert!(package_supported(&caps, "Char.Vitals"));
        assert!(!package_supported(&caps, "Room.Info"));
    }
}

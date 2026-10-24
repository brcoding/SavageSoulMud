//! Optional game-local Grapevine 2.3 adapter. Credentials stay in private local state.
//! Protocol reference: https://grapevine.haus/docs
use bevy::prelude::*;
use grim::{components::Name as GrimName, events::InfoMessage, Character, Player};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tungstenite::{protocol::WebSocketConfig, stream::MaybeTlsStream, Message, WebSocket};
mod transport;
#[cfg(test)]
use transport::connect;
use transport::worker;
#[cfg(test)]
use transport::{auth, run_connection, ws_config};
mod commands;
pub(crate) use commands::command;

mod delivery;
#[cfg(test)]
use delivery::deliver;
use delivery::{tick, update_presence};
mod history;
use history::{history_worker, mark_history, private_path};
#[cfg(test)]
use history::{load_history, save_history};

const ENDPOINT: &str = "wss://grapevine.haus/socket";
const MAX_PACKET: usize = 64 * 1024;
/// Headless fixtures must never connect to a configured live network.
#[derive(Resource)]
pub struct DisableGrapevine;
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    enabled: bool,
    endpoint: String,
    credentials_file: String,
    channels: Vec<String>,
    activity_file: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: ENDPOINT.into(),
            credentials_file: std::path::PathBuf::from(
                std::env::var_os("XDG_STATE_HOME").unwrap_or_else(|| {
                    std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                        .join(".local/state")
                        .into_os_string()
                }),
            )
            .join("savsoul-grapevine/credentials.json")
            .to_string_lossy()
            .into_owned(),
            channels: vec!["gossip".into(), "testing".into()],
            activity_file: String::new(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    client_id: String,
    client_secret: String,
}
#[derive(Default)]
struct Snapshot {
    actors: HashMap<Entity, String>,
    channels: HashSet<String>,
}
struct Job {
    actor: Entity,
    generation: u64,
    queued: Instant,
    packet: Value,
}
enum Incoming {
    State {
        ready: bool,
        generation: u64,
        text: String,
    },
    Packet(Value),
    Dropped(String),
    History(VecDeque<Activity>),
    Subscribed {
        channel: String,
        success: bool,
    },
}
struct Pending {
    actor: Entity,
    event: String,
    filter: String,
    at: Instant,
    replies: usize,
    echo: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Activity {
    at: u64,
    channel: String,
    speaker: String,
    game: String,
    message: String,
}
#[derive(Resource)]
struct Bridge {
    tx: SyncSender<Job>,
    rx: Mutex<Receiver<Incoming>>,
    snapshot: Arc<Mutex<Snapshot>>,
    reload: Arc<AtomicBool>,
    ready: bool,
    generation: u64,
    status: String,
    pending: HashMap<String, Pending>,
    listeners: HashMap<Entity, HashSet<String>>,
    history: VecDeque<Activity>,
    history_store: Arc<Mutex<HistoryStore>>,
    confirmed: HashSet<String>,
    cooldown: HashMap<Entity, Instant>,
}
pub(crate) fn register(app: &mut App) {
    app.add_systems(Startup, start)
        .add_systems(Update, tick.after(grim::SceneSystems::Staff));
}
fn start(world: &mut World) {
    if world.contains_resource::<DisableGrapevine>() {
        return;
    }
    let path = world
        .get_resource::<crate::seed::AreaBlueprintDir>()
        .and_then(|dir| dir.0.parent())
        .map(|dir| dir.join("grapevine.json"))
        .unwrap_or_else(|| std::path::PathBuf::from("data/grapevine.json"));
    let config = match std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Config>(&b).ok())
    {
        Some(c) if c.enabled => c,
        _ => return,
    };
    if config.endpoint != ENDPOINT
        || config.channels.len() > 32
        || config.channels.iter().any(|c| !valid_channel(c))
    {
        warn!("Grapevine disabled: invalid endpoint or channel configuration");
        return;
    }
    let (tx, requests) = mpsc::sync_channel(64);
    let (events, rx) = mpsc::sync_channel(128);
    let snapshot = Arc::new(Mutex::new(Snapshot {
        actors: HashMap::new(),
        channels: config.channels.iter().cloned().collect(),
    }));
    let reload = Arc::new(AtomicBool::new(false));
    let history_store = Arc::new(Mutex::new(HistoryStore::default()));
    if !config.activity_file.is_empty() {
        let path = config.activity_file.clone();
        let store = history_store.clone();
        let history_events = events.clone();
        let _ = std::thread::Builder::new()
            .name("grapevine-history".into())
            .spawn(move || history_worker(path, store, history_events));
    }
    let worker_config = config.clone();
    let worker_snapshot = snapshot.clone();
    let worker_reload = reload.clone();
    if std::thread::Builder::new()
        .name("grapevine-wss".into())
        .spawn(move || {
            worker(
                worker_config,
                requests,
                events,
                worker_snapshot,
                worker_reload,
            )
        })
        .is_err()
    {
        warn!("Grapevine worker could not start");
        return;
    }
    world.insert_resource(Bridge {
        tx,
        rx: Mutex::new(rx),
        snapshot,
        reload,
        ready: false,
        generation: 0,
        status: "Starting Grapevine worker.".into(),
        pending: HashMap::new(),
        listeners: HashMap::new(),
        history: VecDeque::new(),
        history_store,
        confirmed: HashSet::new(),
        cooldown: HashMap::new(),
    });
}
fn allowed(world: &World, e: Entity) -> bool {
    world.get::<Player>(e).is_some() && world.get::<Character>(e).is_some_and(Character::is_admin)
}
fn tell(world: &mut World, e: Entity, text: impl Into<String>) {
    world.write_message(InfoMessage {
        target: e,
        text: text.into(),
    });
}
fn valid_channel(s: &str) -> bool {
    (3..=15).contains(&s.len())
        && s.bytes()
            .all(|c| c.is_ascii_alphabetic() || matches!(c, b'_' | b'-'))
}
fn bounded(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
fn remote(s: &str) -> String {
    // Flatten all controls; suppress ANSI CSI/OSC and MXP delimiters before colour escaping.
    let mut out = String::new();
    let mut escape = false;
    let mut csi = false;
    let mut osc = false;
    let mut osc_escape = false;
    for c in s.chars().take(4096) {
        if osc {
            if c == '\x07' || osc_escape && c == '\\' {
                osc = false;
            }
            osc_escape = c == '\x1b';
            continue;
        }
        if escape {
            if !csi && c == '[' {
                csi = true;
                continue;
            }
            if !csi && c == ']' {
                osc = true;
                escape = false;
                continue;
            }
            if !csi || ('@'..='~').contains(&c) {
                escape = false;
                csi = false;
            }
            continue;
        }
        if c == '\x1b' {
            escape = true;
        } else if c.is_control() {
            out.push(' ');
        } else if c == '<' {
            out.push('‹');
        } else if c == '>' {
            out.push('›');
        } else {
            out.push(c);
        }
    }
    grim::color::escape_codes(&out)
}
fn field(v: &Value, key: &str) -> String {
    remote(v[key].as_str().unwrap_or("?"))
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

struct Outcome {
    authenticated: bool,
    retry_secs: Option<u64>,
    status: &'static str,
}

#[derive(Default)]
struct HistoryStore {
    records: VecDeque<Activity>,
    dirty: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channel_grammar_and_remote_markup() {
        assert!(valid_channel("d-chat"));
        for bad in ["aa", "a1b", "a b", "abcdefghijklmnop"] {
            assert!(!valid_channel(bad));
        }
        let text = remote("\x1b[31mHi\x1b]0;bad\x07\n{x<send>🙂");
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('\n'));
        assert!(!text.contains("<send>"));
        assert!(text.contains('🙂'));
        assert_ne!(text, "Hi {x‹send›🙂");
    }
    #[test]
    fn auth_declares_only_supported_features() {
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let c = Credentials {
            client_id: "fixture".into(),
            client_secret: "private".into(),
        };
        let packet = auth(&c, &snapshot);
        assert_eq!(packet["payload"]["version"], "2.3.0");
        assert_eq!(
            packet["payload"]["supports"],
            json!(["channels", "players", "tells", "games"])
        );
    }
    fn fixture() -> (World, Bridge, Entity) {
        let mut world = World::new();
        world.init_resource::<Messages<InfoMessage>>();
        let stored:grim::StoredCharacter=serde_json::from_value(json!({"id":"abcdefghijkl","account_id":"mnopqrstuvwx","name":"Rawr","created_at":"2026-10-18T00:00:00Z","roles":["admin"]})).unwrap();
        let (n, a, c) = stored.into_components();
        let conn = world.spawn_empty().id();
        let actor = world.spawn((n, a, c, Player { connection: conn })).id();
        let (tx, _) = mpsc::sync_channel(8);
        let (_, rx) = mpsc::sync_channel(8);
        let b = Bridge {
            tx,
            rx: Mutex::new(rx),
            snapshot: Arc::new(Mutex::new(Snapshot::default())),
            reload: Arc::new(AtomicBool::new(false)),
            ready: true,
            generation: 1,
            status: String::new(),
            pending: HashMap::new(),
            listeners: HashMap::new(),
            history: VecDeque::new(),
            history_store: Arc::new(Mutex::new(HistoryStore::default())),
            confirmed: HashSet::from(["gossip".into()]),
            cooldown: HashMap::new(),
        };
        (world, b, actor)
    }
    #[test]
    fn private_delivery_and_query_replies_recheck_roles() {
        let (mut world, mut b, actor) = fixture();
        let mut cursor = bevy::ecs::message::MessageCursor::<InfoMessage>::default();
        let packet = json!({"event":"tells/receive","payload":{"to_name":"rawr","from_name":"Other","from_game":"Remote","message":"hello"}});
        deliver(&mut world, &mut b, packet.clone());
        assert_eq!(
            cursor
                .read(world.resource::<Messages<InfoMessage>>())
                .count(),
            1
        );
        b.pending.insert(
            "query".into(),
            Pending {
                actor,
                event: "games/status".into(),
                filter: String::new(),
                at: Instant::now(),
                replies: 0,
                echo: None,
            },
        );
        for game in ["One", "Two"] {
            deliver(
                &mut world,
                &mut b,
                json!({"event":"games/status","ref":"query","payload":{"game":game,"display_name":game,"players_online_count":1}}),
            );
        }
        assert_eq!(b.pending["query"].replies, 2);
        assert_eq!(
            cursor
                .read(world.resource::<Messages<InfoMessage>>())
                .count(),
            2
        );
        world.get_mut::<Character>(actor).unwrap().roles.clear();
        deliver(&mut world, &mut b, packet);
        deliver(
            &mut world,
            &mut b,
            json!({"event":"games/status","ref":"query","payload":{"game":"Three"}}),
        );
        assert_eq!(
            cursor
                .read(world.resource::<Messages<InfoMessage>>())
                .count(),
            0
        );
        world.insert_resource(b);
        assert!(command(&mut world, actor, "recent").is_err());
    }
    #[test]
    fn outgoing_echo_waits_for_correlated_ack() {
        let (mut world, mut b, actor) = fixture();
        let mut cursor = bevy::ecs::message::MessageCursor::<InfoMessage>::default();
        b.pending.insert(
            "send".into(),
            Pending {
                actor,
                event: "channels/send".into(),
                filter: "gossip".into(),
                at: Instant::now(),
                replies: 0,
                echo: Some("local echo".into()),
            },
        );
        deliver(
            &mut world,
            &mut b,
            json!({"event":"channels/send","ref":"unknown"}),
        );
        assert_eq!(
            cursor
                .read(world.resource::<Messages<InfoMessage>>())
                .count(),
            0
        );
        deliver(
            &mut world,
            &mut b,
            json!({"event":"channels/send","ref":"send"}),
        );
        assert_eq!(
            cursor
                .read(world.resource::<Messages<InfoMessage>>())
                .map(|m| m.text.clone())
                .collect::<Vec<_>>(),
            vec!["local echo"]
        );
    }
    #[test]
    fn websocket_fixture_auth_heartbeat_and_stale_queue() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (mut world, mut b, actor) = fixture();
        update_presence(&mut world, &mut b);
        b.snapshot.lock().unwrap().channels.insert("gossip".into());
        let snapshot = b.snapshot.clone();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut socket = tungstenite::accept_with_config(tcp, Some(ws_config())).unwrap();
            let packet: Value =
                serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(packet["event"], "authenticate");
            assert_eq!(packet["payload"]["client_secret"], "fixture-secret");
            assert_eq!(packet["payload"]["channels"], json!([]));
            socket
                .send(Message::Text(
                    json!({"event":"authenticate","status":"success"})
                        .to_string()
                        .into(),
                ))
                .unwrap();
            socket
                .send(Message::Text(
                    json!({"event":"heartbeat"}).to_string().into(),
                ))
                .unwrap();
            let mut signed_in = false;
            loop {
                let msg: Value =
                    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                if msg["event"] == "channels/subscribe" {
                    assert_eq!(msg["payload"]["channel"], "gossip");
                    socket.send(Message::Text(json!({"event":"channels/subscribe","ref":msg["ref"],"status":"failure"}).to_string().into())).unwrap();
                }
                if msg["event"] == "players/sign-in" {
                    signed_in = true;
                    assert_eq!(msg["payload"]["name"], "Rawr");
                }
                if msg["event"] == "heartbeat" {
                    assert_eq!(msg["payload"]["players"], json!(["Rawr"]));
                    break;
                }
            }
            assert!(signed_in);
            socket.close(None).unwrap();
        });
        let tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let (mut socket, _) = tungstenite::client::client_with_config(
            format!("ws://127.0.0.1:{port}"),
            tcp,
            Some(ws_config()),
        )
        .unwrap();
        let c = Credentials {
            client_id: "fixture-id".into(),
            client_secret: "fixture-secret".into(),
        };
        socket
            .send(Message::Text(auth(&c, &snapshot).to_string().into()))
            .unwrap();
        let (tx, rx) = mpsc::sync_channel(8);
        let (events, incoming) = mpsc::sync_channel(16);
        tx.send(Job{actor,generation:999,queued:Instant::now(),packet:json!({"event":"channels/send","ref":"stale","payload":{"name":"Rawr","channel":"gossip","message":"never send"}})}).unwrap();
        let outcome = run_connection(
            &mut socket,
            &rx,
            &events,
            &snapshot,
            &Arc::new(AtomicBool::new(false)),
            1,
        );
        assert!(outcome.authenticated);
        let all: Vec<_> = incoming.try_iter().collect();
        assert!(all
            .iter()
            .any(|e| matches!(e,Incoming::Subscribed{channel,success:false}if channel=="gossip")));
        assert!(all
            .iter()
            .any(|e| matches!(e,Incoming::Dropped(r)if r=="stale")));
        server.join().unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn bounded_public_history_persists_and_rejects_invalid_records() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("grapevine-history-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.join("activity.json");
        let records = VecDeque::from([Activity {
            at: now(),
            channel: "gossip".into(),
            speaker: "Name".into(),
            game: "Game".into(),
            message: "hello".into(),
        }]);
        save_history(path.to_str().unwrap(), &records).unwrap();
        assert_eq!(load_history(path.to_str().unwrap()).len(), 1);
        assert_eq!(path.metadata().unwrap().permissions().mode() & 0o077, 0);
        std::fs::write(&path, b"[{\"at\":0}]").unwrap();
        assert!(load_history(path.to_str().unwrap()).is_empty());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    #[ignore = "Manual read-only official WSS upgrade; sends no authentication or chat"]
    fn official_tls_upgrade_only() {
        let mut socket = connect().expect("verified official WSS upgrade");
        let _ = socket.close(None);
    }
}

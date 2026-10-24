//! Zebedee/inetd Intermud 2. Game-local, wizard-only, bounded off-loop UDP.
use bevy::prelude::*;
use grim::{components::Name as GrimName, events::InfoMessage, Character, Player};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs, UdpSocket},
    sync::{
        mpsc::{self, Receiver, SyncSender},
        Mutex,
    },
    time::{Duration, Instant},
};
mod codec;
#[cfg(test)]
use codec::decode;
use codec::{encode, header};
mod transport;
#[cfg(test)]
use transport::relay_worker;
use transport::worker;
mod commands;
pub(crate) use commands::command;
mod delivery;
#[cfg(test)]
use delivery::deliver;
use delivery::tick;

const MAX_PACKET: usize = 24 * 1024;
const MAX_PENDING: usize = 64;
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    enabled: bool,
    bind: String,
    mud_name: String,
    advertised_port: u16,
    peers: Vec<Peer>,
    channels: Vec<String>,
    relay_host: String,
    relay_port: u16,
    relay_key_file: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "0.0.0.0:4002".into(),
            mud_name: "The Savage Soul Dev".into(),
            advertised_port: 4002,
            peers: vec![],
            channels: vec!["intermud".into(), "intercode".into(), "d-chat".into()],
            relay_host: String::new(),
            relay_port: 4084,
            relay_key_file: String::new(),
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Peer {
    name: String,
    host: String,
    port: u16,
}
#[derive(Clone, Debug, Default)]
struct Packet {
    headers: BTreeMap<String, String>,
    data: String,
}
impl Packet {
    fn get(&self, key: &str) -> &str {
        self.headers.get(key).map(String::as_str).unwrap_or("")
    }
    fn set(&mut self, key: &str, value: impl ToString) {
        self.headers.insert(key.into(), value.to_string());
    }
}

struct Fragments {
    at: Instant,
    pieces: Vec<Option<Vec<u8>>>,
}
#[derive(Default)]
struct Assembly {
    groups: HashMap<(usize, String), Fragments>,
}

struct Outgoing {
    peer: usize,
    packet: Packet,
}
enum Incoming {
    Ready(String),
    Failed(String),
    Packet(usize, Packet),
}
struct Pending {
    actor: Entity,
    peer: usize,
    packet: Packet,
    at: Instant,
    retries: u8,
}
#[derive(Resource)]
struct Bridge {
    config: Config,
    tx: SyncSender<Outgoing>,
    rx: Mutex<Receiver<Incoming>>,
    ready: bool,
    status: String,
    next: u64,
    pending: HashMap<u64, Pending>,
    listeners: HashMap<Entity, HashSet<String>>,
    seen: HashMap<(usize, String, String), Instant>,
    cooldown: HashMap<Entity, Instant>,
    last_seen: HashMap<usize, Instant>,
}
pub(crate) fn register(app: &mut App) {
    app.add_systems(Startup, start)
        .add_systems(Update, tick.after(grim::SceneSystems::Staff));
}
fn start(world: &mut World) {
    let Ok(bytes) = std::fs::read("data/zebedee.json") else {
        return;
    };
    let config: Config = match serde_json::from_slice(&bytes) {
        Ok(c) => c,
        Err(e) => {
            warn!("Zebedee disabled: invalid config: {e}");
            return;
        }
    };
    if !config.enabled {
        return;
    }
    if config.peers.len() > 64
        || !header(&config.mud_name)
        || config.advertised_port == 0
        || config.peers.iter().any(|p| !header(&p.name) || p.port == 0)
        || config.channels.len() > 32
        || config.channels.iter().any(|s| !valid_channel(s))
    {
        warn!("Zebedee disabled: invalid configuration bounds");
        return;
    }
    let (tx, requests) = mpsc::sync_channel(64);
    let (events, rx) = mpsc::sync_channel(128);
    let worker_config = config.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("zebedee-udp".into())
        .spawn(move || worker(worker_config, requests, events))
    {
        warn!("Zebedee disabled: {e}");
        return;
    }
    world.insert_resource(Bridge {
        config,
        tx,
        rx: Mutex::new(rx),
        ready: false,
        status: "starting UDP worker".into(),
        next: 1,
        pending: HashMap::new(),
        listeners: HashMap::new(),
        seen: HashMap::new(),
        cooldown: HashMap::new(),
        last_seen: HashMap::new(),
    });
}

fn allowed(world: &World, actor: Entity) -> bool {
    world.get::<Player>(actor).is_some()
        && world
            .get::<Character>(actor)
            .is_some_and(Character::is_admin)
}
fn say(world: &mut World, actor: Entity, text: impl Into<String>) {
    world.write_message(InfoMessage {
        target: actor,
        text: text.into(),
    });
}
fn plain(input: &str) -> String {
    let mut out = String::new();
    let mut ansi = false;
    for c in input.chars().take(4096) {
        if ansi {
            if ('@'..='~').contains(&c) && c != '[' {
                ansi = false;
            }
            continue;
        }
        if c == '\x1b' {
            ansi = true;
        } else if c.is_ascii_graphic() || c == ' ' {
            out.push(c);
        } else {
            out.push(' ');
        }
    }
    grim::color::escape_codes(&out)
}
fn valid_channel(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}
fn base(config: &Config, req: &str) -> Packet {
    let mut p = Packet::default();
    p.set("NAME", &config.mud_name);
    p.set("UDP", config.advertised_port);
    p.set("REQ", req);
    p
}
fn send(bridge: &mut Bridge, peer: usize, packet: Packet) -> Result<(), String> {
    bridge
        .tx
        .try_send(Outgoing { peer, packet })
        .map_err(|_| "Zebedee queue is busy or worker unavailable.".into())
}

/// Fragment groups need transport identity even for chat, which expects no reply.
fn fragment_id(packet: &Packet) -> String {
    if !packet.get("ID").is_empty() {
        return packet.get("ID").to_string();
    }
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    };
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    NEXT.get_or_init(|| AtomicU64::new(rand::random_range(1_000_000..u64::MAX / 2)))
        .fetch_add(1, Ordering::Relaxed)
        .to_string()
}
fn request(
    bridge: &mut Bridge,
    actor: Entity,
    peer: usize,
    mut packet: Packet,
) -> Result<(), String> {
    if !bridge.ready {
        return Err(bridge.status.clone());
    }
    if bridge.pending.len() >= MAX_PENDING {
        return Err("Zebedee has too many unanswered requests.".into());
    }
    bridge.next = bridge.next.wrapping_add(1).max(1);
    let id = bridge.next;
    packet.set("ID", id);
    send(bridge, peer, packet.clone())?;
    bridge.pending.insert(
        id,
        Pending {
            actor,
            peer,
            packet,
            at: Instant::now(),
            retries: 0,
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untracked_chat_has_unique_transport_fragment_ids() {
        let mut packet = Packet::default();
        packet.set("REQ", "channel");
        packet.data = "x".repeat(1024);
        assert!(encode(&packet).unwrap().len() > 900);
        let first = fragment_id(&packet);
        let second = fragment_id(&packet);
        assert_ne!(first, second);
        assert!(first.parse::<u64>().is_ok());
        assert_eq!(packet.get("ID"), "");
        packet.set("ID", 42);
        assert_eq!(fragment_id(&packet), "42");
    }
    #[test]
    fn framing_preserves_data_delimiters_and_quoted_strings() {
        let mut p = Packet::default();
        p.set("REQ", "tell");
        p.set("SND", "$name");
        p.set("ID", 9);
        p.data = "a|DATA: value".into();
        let q = decode(&encode(&p).unwrap()).unwrap();
        assert_eq!(q.get("SND"), "$name");
        assert_eq!(q.get("ID"), "9");
        assert_eq!(q.data, p.data);
        assert!(decode(b"REQ:who|REQ:tell").is_none());
    }
    #[test]
    fn fragments_are_bounded_and_out_of_order() {
        let mut a = Assembly::default();
        assert!(a.receive(0, b"PKT:M:1:2/2|llo").is_none());
        let p = a.receive(0, b"PKT:M:1:1/2|REQ:reply|DATA:he").unwrap();
        assert_eq!(p.data, "hello");
        assert!(a.receive(0, b"PKT:M:2:1/999|x").is_none());
    }
    #[test]
    fn remote_markup_and_controls_cannot_inject() {
        let s = plain("\x1b[31mred\n{x@xRGB");
        assert!(!s.contains('\x1b'));
        assert!(!s.contains('\n'));
        assert_ne!(s, "red {x@xRGB");
    }
    #[test]
    fn authorization_fails_closed() {
        let mut world = World::new();
        let actor = world.spawn_empty().id();
        assert!(!allowed(&world, actor));
    }
    #[test]
    fn tells_and_pending_replies_recheck_online_wizard_roles() {
        let mut world = World::new();
        world.init_resource::<Messages<InfoMessage>>();
        let mut actors = Vec::new();
        for (name, admin) in [("Rawr", true), ("Mortal", false)] {
            let stored:grim::StoredCharacter=serde_json::from_value(serde_json::json!({"id":"abcdefghijkl","account_id":"mnopqrstuvwx","name":name,"created_at":"2026-10-18T00:00:00Z","roles":if admin{serde_json::json!(["admin"])}else{serde_json::json!([])}})).unwrap();
            let (n, a, c) = stored.into_components();
            let conn = world.spawn_empty().id();
            actors.push(world.spawn((n, a, c, Player { connection: conn })).id());
        }
        let (tx, _) = mpsc::sync_channel(8);
        let (_, rx) = mpsc::sync_channel(8);
        let mut config = Config::default();
        config.peers.push(Peer {
            name: "Peer".into(),
            host: "127.0.0.1".into(),
            port: 4246,
        });
        let mut b = Bridge {
            config,
            tx,
            rx: Mutex::new(rx),
            ready: true,
            status: String::new(),
            next: 1,
            pending: HashMap::new(),
            listeners: HashMap::new(),
            seen: HashMap::new(),
            cooldown: HashMap::new(),
            last_seen: HashMap::new(),
        };
        let mut p = Packet::default();
        p.set("REQ", "tell");
        p.set("SND", "Remote");
        p.set("RCPNT", "rawr");
        p.data = "hello".into();
        let mut cursor = bevy::ecs::message::MessageCursor::<InfoMessage>::default();
        deliver(&mut world, &mut b, 0, p.clone());
        let delivered: Vec<_> = cursor
            .read(world.resource::<Messages<InfoMessage>>())
            .map(|m| m.target)
            .collect();
        assert_eq!(delivered, vec![actors[0]]);
        world.get_mut::<Character>(actors[0]).unwrap().roles.clear();
        deliver(&mut world, &mut b, 0, p);
        b.pending.insert(
            10,
            Pending {
                actor: actors[0],
                peer: 0,
                packet: Packet::default(),
                at: Instant::now(),
                retries: 0,
            },
        );
        let mut response = Packet::default();
        response.set("REQ", "reply");
        response.set("ID", 10);
        response.data = "private reply".into();
        deliver(&mut world, &mut b, 0, response);
        assert_eq!(
            cursor
                .read(world.resource::<Messages<InfoMessage>>())
                .count(),
            0
        );
        assert!(!allowed(&world, actors[1]));
    }
    #[test]
    fn relay_transport_authenticates_and_validates_source_before_delivery() {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let keyfile = std::env::temp_dir().join(format!(
            "zebedee-relay-test-{}-{port}.key",
            std::process::id()
        ));
        std::fs::write(&keyfile, "ab".repeat(32)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut input = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            input.read_line(&mut line).unwrap();
            let auth: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(auth["key"], "ab".repeat(32));
            stream.write_all(b"{\"ready\":true}\n").unwrap();
            line.clear();
            input.read_line(&mut line).unwrap();
            let sent: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(sent["peer"], "Fixture");
            let packet = b"NAME:Fixture|UDP:4246|REQ:reply|ID:7|DATA:alive";
            let frame=serde_json::json!({"source_ip":"127.0.0.1","source_port":4246,"data":packet.to_vec()}).to_string()+"\n";
            stream.write_all(frame.as_bytes()).unwrap();
        });
        let mut config = Config {
            relay_host: "127.0.0.1".into(),
            relay_port: port,
            relay_key_file: keyfile.to_string_lossy().into(),
            ..Default::default()
        };
        config.peers.push(Peer {
            name: "Fixture".into(),
            host: "127.0.0.1".into(),
            port: 4246,
        });
        let (tx, rx) = mpsc::sync_channel(8);
        let (events, incoming) = mpsc::sync_channel(8);
        let task = std::thread::spawn(move || relay_worker(config, rx, events));
        assert!(matches!(
            incoming.recv_timeout(Duration::from_secs(3)).unwrap(),
            Incoming::Ready(_)
        ));
        let mut packet = Packet::default();
        packet.set("REQ", "ping");
        packet.set("ID", 7);
        tx.send(Outgoing { peer: 0, packet }).unwrap();
        match incoming.recv_timeout(Duration::from_secs(3)).unwrap() {
            Incoming::Packet(peer, p) => {
                assert_eq!(peer, 0);
                assert_eq!(p.data, "alive");
            }
            _ => panic!("expected validated packet"),
        }
        drop(tx);
        task.join().unwrap();
        server.join().unwrap();
        std::fs::remove_file(keyfile).unwrap();
    }
}

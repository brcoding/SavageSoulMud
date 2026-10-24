//! Game-local authenticated bridge to the independently running Intermud3 gateway.
use bevy::{ecs::message::MessageCursor, prelude::*};
use grim::{
    components::Name as GrimName,
    events::{Command, EngineCommand, InfoMessage},
    Character, Player,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpStream},
    sync::{
        mpsc::{self, Receiver, SyncSender},
        Mutex,
    },
    time::{Duration, Instant},
};
mod history;
use history::{
    activity_command, activity_path, load_activity, record_activity, save_activity, unix_time,
};
#[cfg(test)]
use history::{bounded, valid_record};
mod transport;
#[cfg(test)]
use transport::send;
use transport::worker;
mod subscriptions;
#[cfg(test)]
use subscriptions::save_subscriptions;
use subscriptions::{
    forward_packet, load_subscriptions, public_channels, pump_subscriptions, queue_subscriptions,
    rejoin, subscription_command,
};
mod commands;
use commands::{parse, request};
mod delivery;
use delivery::tick;
#[cfg(test)]
use delivery::{deliver, recipients, request_notice};

#[derive(Clone, Deserialize)]
#[serde(default)]
struct Config {
    enabled: bool,
    host: String,
    port: u16,
    api_key_file: String,
    channels: Vec<String>,
    wizard_only: bool,
    activity_file: String,
    subscriptions_file: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".into(),
            port: 4082,
            api_key_file: String::new(),
            channels: vec![],
            wizard_only: true,
            activity_file: String::new(),
            subscriptions_file: String::new(),
        }
    }
}
struct Request {
    id: u64,
    method: String,
    params: Value,
}
enum Incoming {
    Connected,
    Disconnected,
    Packet(Value),
}
#[derive(Resource)]
struct Bridge {
    config: Config,
    tx: SyncSender<Request>,
    rx: Mutex<Receiver<Incoming>>,
    connected: bool,
    next: u64,
    pending: HashMap<u64, (String, String, Instant)>,
    cooldown: HashMap<String, Instant>,
    presence: Value,
    last_presence: Instant,
    activity: VecDeque<Activity>,
    activity_dirty: bool,
    last_activity_save: Instant,
    subscriptions: VecDeque<(String, bool)>,
    confirmed: Vec<String>,
    last_subscription: Instant,
    subscription_failures: usize,
}
const ACTIVITY_LIMIT: usize = 200;
const ACTIVITY_BYTES: u64 = 256 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Activity {
    at: u64,
    channel: String,
    speaker: String,
    mud: String,
    message: String,
    emote: bool,
}

/// Optional plugin: no network thread or credentials are opened unless configured.
pub struct IntermudPlugin;
impl Plugin for IntermudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start)
            .add_systems(Update, tick.after(grim::SceneSystems::Staff));
    }
}
fn start(world: &mut World) {
    let mut config: Config = match std::fs::read("data/intermud.json") {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(c) => c,
            Err(_) => {
                warn!("Intermud configuration is invalid; disabled");
                return;
            }
        },
        Err(_) => return,
    };
    config.wizard_only = true; // Compatibility field cannot weaken the wizard-only boundary.
    if let Some(channels) = load_subscriptions(&config.subscriptions_file) {
        config.channels = channels;
    }
    if !config.enabled {
        return;
    }
    if config.channels.len() > 512 || config.channels.iter().any(|c| !valid_channel(c)) {
        warn!("Intermud channel configuration is invalid; disabled");
        return;
    }
    let Ok(ip) = config.host.parse::<IpAddr>() else {
        warn!("Intermud gateway must use a numeric loopback address");
        return;
    };
    if !ip.is_loopback() {
        warn!("Intermud gateway must be loopback");
        return;
    }
    let (tx, requests) = mpsc::sync_channel(64);
    let (events, rx) = mpsc::sync_channel(128);
    let cfg = config.clone();
    if std::thread::Builder::new()
        .name("intermud".into())
        .spawn(move || worker(cfg, requests, events))
        .is_err()
    {
        warn!("Could not start optional Intermud worker; disabled");
        return;
    }
    let activity = load_activity(&config.activity_file);
    world.insert_resource(Bridge {
        config,
        tx,
        rx: Mutex::new(rx),
        connected: false,
        next: 10,
        pending: HashMap::new(),
        cooldown: HashMap::new(),
        presence: Value::Null,
        last_presence: Instant::now() - Duration::from_secs(60),
        activity,
        activity_dirty: false,
        last_activity_save: Instant::now() - Duration::from_secs(5),
        subscriptions: VecDeque::new(),
        confirmed: Vec::new(),
        last_subscription: Instant::now() - Duration::from_secs(2),
        subscription_failures: 0,
    });
}

fn allowed(world: &World, e: Entity, _wizard: bool) -> bool {
    world.get::<Player>(e).is_some() && world.get::<Character>(e).is_some_and(Character::is_admin)
}
fn reply(world: &mut World, e: Entity, text: impl Into<String>) {
    world.write_message(InfoMessage {
        target: e,
        text: text.into(),
    });
}
fn plain(value: &str) -> String {
    // Remove ANSI sequences entirely; flatten all other controls so remote text cannot impersonate prompts.
    let mut output = String::new();
    let mut escape = false;
    let mut csi = false;
    for c in value.chars().take(4096) {
        if escape {
            if !csi && c == '[' {
                csi = true;
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
            continue;
        }
        if c.is_control() {
            output.push(' ');
        } else {
            output.push(c);
        }
    }
    grim::color::escape_codes(&output)
}
fn text(v: &Value, key: &str) -> String {
    plain(v[key].as_str().unwrap_or("?"))
}

fn valid_channel(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn router_recovery_survives_a_saturated_event_queue() {
        let (tx, rx) = mpsc::sync_channel(1);
        forward_packet(&tx, json!({"method":"channel_message"})).unwrap();
        forward_packet(&tx, json!({"method":"channel_message"})).unwrap();
        let worker = std::thread::spawn(move || {
            forward_packet(&tx, json!({"method":"gateway_reconnected"}))
        });
        let Incoming::Packet(first) = rx.recv_timeout(Duration::from_secs(2)).unwrap() else {
            panic!("packet expected")
        };
        assert_eq!(first["method"], "channel_message");
        let Incoming::Packet(control) = rx.recv_timeout(Duration::from_secs(2)).unwrap() else {
            panic!("control expected")
        };
        assert_eq!(control["method"], "gateway_reconnected");
        worker.join().unwrap().unwrap();
    }
    #[test]
    fn discovery_notice_is_local_sanitized_and_has_no_reply_data() {
        let p = json!({"request":"who-req","from_user":"Crawler","from_mud":"MUIndex","users":["private"]});
        assert_eq!(
            request_notice(&p).unwrap(),
            "SYSTEM <intermud> [Crawler@MUIndex requests the who list]"
        );
        assert!(request_notice(&json!({"request":"tell"})).is_none());
        let p = json!({"request":"finger-req","from_user":"bad\n{R","from_mud":"remote\u{001b}"});
        let notice = request_notice(&p).unwrap();
        assert!(!notice.contains('\n'));
        assert!(!notice.contains('\u{001b}'));
    }
    #[test]
    fn commands_preserve_identity_and_multiword_muds() {
        let (m, p) = parse("Rawr", "tell ally@Some Mud: hello there").unwrap();
        assert_eq!(m, "tell");
        assert_eq!(p["from_user"], "Rawr");
        assert_eq!(p["target_mud"], "Some Mud");
        assert_eq!(p["message"], "hello there");
        assert!(parse("Rawr", "chat gossip").is_err());
        assert!(parse("Rawr", "listen bad\nchannel").is_err());
    }
    #[test]
    fn remote_text_cannot_inject_color_or_lines() {
        let s = plain("hi\x1b[31m!\n{RRED");
        assert!(!s.contains('\x1b'));
        assert!(!s.contains('\n'));
        assert!(s.contains("{{R"));
    }
    #[test]
    fn authenticated_gateway_roundtrip_and_shutdown() {
        use std::{
            io::{BufRead, BufReader},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let keyfile =
            std::env::temp_dir().join(format!("grim-i3-test-{}-{port}", std::process::id()));
        std::fs::write(&keyfile, "test-private-key").unwrap();
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(4)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let auth: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(auth["method"], "authenticate");
            assert_eq!(auth["params"]["api_key"], "test-private-key");
            writeln!(
                stream,
                "{}",
                json!({"jsonrpc":"2.0","id":1,"result":{"status":"authenticated"}})
            )
            .unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            let req: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(req["method"], "status");
            writeln!(
                stream,
                "{}",
                json!({"jsonrpc":"2.0","id":req["id"],"result":{"connected":true}})
            )
            .unwrap();
            let mut b = [0];
            assert_eq!(reader.read(&mut b).unwrap(), 0);
        });
        let (tx, requests) = mpsc::sync_channel(4);
        let (events, rx) = mpsc::sync_channel(8);
        let config = Config {
            enabled: true,
            port,
            api_key_file: keyfile.to_string_lossy().into(),
            ..Config::default()
        };
        let thread = std::thread::spawn(move || worker(config, requests, events));
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(4)).unwrap(),
            Incoming::Connected
        ));
        tx.send(Request {
            id: 101,
            method: "status".into(),
            params: json!({}),
        })
        .unwrap();
        let Incoming::Packet(packet) = rx.recv_timeout(Duration::from_secs(4)).unwrap() else {
            panic!("expected response")
        };
        assert_eq!(packet["id"], 101);
        drop(tx);
        thread.join().unwrap();
        peer.join().unwrap();
        std::fs::remove_file(keyfile).unwrap();
    }
    #[test]
    fn private_replies_require_named_online_wizard_and_fresh_role() {
        let mut world = World::new();
        world.init_resource::<Messages<InfoMessage>>();
        let mut actors = Vec::new();
        for (name, admin) in [("Rawr", true), ("Other", true), ("Mortal", false)] {
            let stored:grim::StoredCharacter=serde_json::from_value(json!({"id":"abcdefghijkl","account_id":"mnopqrstuvwx","name":name,"created_at":"2026-10-18T00:00:00Z","roles":if admin{json!(["admin"])}else{json!([])}})).unwrap();
            let (n, a, c) = stored.into_components();
            let conn = world.spawn_empty().id();
            actors.push(world.spawn((n, a, c, Player { connection: conn })).id());
        }
        let (tx, _requests) = mpsc::sync_channel(4);
        let (_events, rx) = mpsc::sync_channel(4);
        let mut bridge = Bridge {
            config: Config::default(),
            tx,
            rx: Mutex::new(rx),
            connected: true,
            next: 100,
            pending: HashMap::new(),
            cooldown: HashMap::new(),
            presence: Value::Null,
            last_presence: Instant::now(),
            activity: VecDeque::new(),
            activity_dirty: false,
            last_activity_save: Instant::now() - Duration::from_secs(5),
            subscriptions: VecDeque::new(),
            confirmed: Vec::new(),
            last_subscription: Instant::now() - Duration::from_secs(2),
            subscription_failures: 0,
        };
        let tell = json!({"method":"tell_received","params":{"from_mud":"SomeMud","from_user":"Ally","to_user":"rawr","message":"private"}});
        let mut cursor = MessageCursor::<InfoMessage>::default();
        deliver(&mut world, &mut bridge, tell.clone());
        let delivered: Vec<_> = cursor
            .read(world.resource::<Messages<InfoMessage>>())
            .map(|m| m.target)
            .collect();
        assert_eq!(delivered, vec![actors[0]]);
        world.get_mut::<Character>(actors[0]).unwrap().roles.clear();
        deliver(&mut world, &mut bridge, tell);
        assert_eq!(
            cursor
                .read(world.resource::<Messages<InfoMessage>>())
                .count(),
            0
        );
        assert!(recipients(&mut world, false, Some("Mortal")).is_empty());
        assert!(!allowed(&world, actors[2], false));
        bridge.pending.insert(
            202,
            (
                "Rawr".into(),
                "channel_list#listen_all".into(),
                Instant::now(),
            ),
        );
        deliver(
            &mut world,
            &mut bridge,
            json!({"id":202,"result":{"channels":[{"name":"public","type":0}]}}),
        );
        assert!(bridge.config.channels.is_empty());
        assert!(bridge.subscriptions.is_empty());
        world.entity_mut(actors[1]).remove::<Player>();
        assert!(recipients(&mut world, true, Some("Other")).is_empty());
    }
    #[test]
    fn public_history_is_bounded_filtered_and_private_free() {
        let mut history = VecDeque::new();
        let now = unix_time();
        let p =
            json!({"channel":"intergossip","from_user":"Rawr","from_mud":"Dev","message":"public"});
        assert!(!record_activity(&mut history, "tell_received", &p, now));
        assert!(!record_activity(&mut history, "emoteto_received", &p, now));
        assert!(history.is_empty());
        for i in 0..205 {
            let mut item = p.clone();
            item["message"] = json!(format!("message {i}"));
            record_activity(&mut history, "channel_message", &item, now);
        }
        assert_eq!(history.len(), 200);
        assert_eq!(history.front().unwrap().message, "message 5");
        assert_eq!(history.back().unwrap().message, "message 204");
        let (tx, _requests) = mpsc::sync_channel(4);
        let (_events, rx) = mpsc::sync_channel(4);
        let bridge = Bridge {
            config: Config::default(),
            tx,
            rx: Mutex::new(rx),
            connected: false,
            next: 100,
            pending: HashMap::new(),
            cooldown: HashMap::new(),
            presence: Value::Null,
            last_presence: Instant::now(),
            activity: history,
            activity_dirty: false,
            last_activity_save: Instant::now() - Duration::from_secs(5),
            subscriptions: VecDeque::new(),
            confirmed: Vec::new(),
            last_subscription: Instant::now() - Duration::from_secs(2),
            subscription_failures: 0,
        };
        let output = activity_command(&bridge, "recent INTERGOSSIP").unwrap();
        assert!(output.contains("message 185"));
        assert!(!output.contains("message 184"));
        assert!(output.find("message 185").unwrap() < output.find("message 204").unwrap());
        assert!(output.contains("intergossip"));
        assert!(output.contains("Rawr@Dev"));
        assert!(activity_command(&bridge, "recent other")
            .unwrap()
            .contains("No observed"));
        assert!(activity_command(&bridge, "activity")
            .unwrap()
            .contains("200 retained"));
        assert!(!valid_record(
            &Activity {
                at: now + 5,
                channel: "chat".into(),
                speaker: "x".into(),
                mud: "x".into(),
                message: "x".into(),
                emote: false
            },
            now
        ));
        assert!(bounded(&"é".repeat(900), 1024).len() <= 1024);
    }
    #[cfg(unix)]
    #[test]
    fn public_history_survives_atomic_save_and_reload() {
        let home = std::env::var("HOME").unwrap();
        let dir = std::path::PathBuf::from(home).join(format!(
            ".cache/grim-i3-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("activity.json");
        let path = path.to_str().unwrap();
        let mut history = VecDeque::new();
        record_activity(
            &mut history,
            "channel_emote",
            &json!({"channel":"chat","from_user":"Somebody","from_mud":"Elsewhere","message":"{Rhello"}),
            unix_time(),
        );
        save_activity(path, &history).unwrap();
        let loaded = load_activity(path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].message, "{Rhello");
        assert_eq!(plain(&loaded[0].message), "{{Rhello");
        assert!(loaded[0].emote);
        std::fs::write(path, b"not valid history").unwrap();
        assert!(load_activity(path).is_empty());
        std::fs::remove_file(path).unwrap();
        let subscriptions = dir.join("subscriptions.json");
        let subscriptions = subscriptions.to_str().unwrap();
        save_subscriptions(subscriptions, &["intergossip".into(), "chat".into()]).unwrap();
        assert_eq!(
            load_subscriptions(subscriptions).unwrap(),
            vec!["intergossip", "chat"]
        );
        save_subscriptions(subscriptions, &[]).unwrap();
        assert!(load_subscriptions(subscriptions).unwrap().is_empty());
        std::fs::remove_file(subscriptions).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn bulk_subscriptions_filter_restricted_and_are_paced_beyond_wire_capacity() {
        let entries: Vec<_> = (0..200)
            .map(|i| json!({"name":format!("channel-{i}"),"type":0}))
            .chain([
                json!({"name":"restricted","type":1}),
                json!({"name":"private","type":2}),
                json!({"name":"invalid name","type":0}),
            ])
            .collect();
        let (channels, skipped) = public_channels(&json!({"channels":entries}));
        assert_eq!(channels.len(), 200);
        assert_eq!(skipped, 3);
        let (tx, requests) = mpsc::sync_channel(64);
        let (_events, rx) = mpsc::sync_channel(128);
        let mut bridge = Bridge {
            config: Config::default(),
            tx,
            rx: Mutex::new(rx),
            connected: true,
            next: 100,
            pending: HashMap::new(),
            cooldown: HashMap::new(),
            presence: Value::Null,
            last_presence: Instant::now(),
            activity: VecDeque::new(),
            activity_dirty: false,
            last_activity_save: Instant::now() - Duration::from_secs(5),
            subscriptions: VecDeque::new(),
            confirmed: Vec::new(),
            last_subscription: Instant::now() - Duration::from_secs(2),
            subscription_failures: 0,
        };
        assert_eq!(
            queue_subscriptions(&mut bridge, channels, true).unwrap(),
            200
        );
        assert_eq!(bridge.config.channels.len(), 200);
        pump_subscriptions(&mut bridge);
        assert_eq!(requests.try_iter().count(), 1);
        assert_eq!(bridge.subscriptions.len(), 199);
        assert_eq!(bridge.pending.len(), 1);
        pump_subscriptions(&mut bridge);
        assert_eq!(requests.try_iter().count(), 0);
        assert_eq!(bridge.subscriptions.len(), 199);
        bridge.last_subscription = Instant::now() - Duration::from_secs(2);
        pump_subscriptions(&mut bridge);
        assert_eq!(requests.try_iter().count(), 1);
        assert_eq!(bridge.pending.len(), 2);
        rejoin(&mut bridge);
        assert_eq!(bridge.subscriptions.len(), 200);
        assert!(bridge.confirmed.is_empty());
        let output = subscription_command(&mut bridge, "unlisten all").unwrap();
        assert!(output.contains("200"));
        assert!(bridge.config.channels.is_empty());
        assert!(bridge.subscriptions.iter().all(|(_, join)| !*join));
        let mut world = World::new();
        world.init_resource::<Messages<InfoMessage>>();
        deliver(
            &mut world,
            &mut bridge,
            json!({"method":"channel_message","params":{"channel":"channel-1","from_user":"Remote","from_mud":"Other","message":"queued before leave"}}),
        );
        assert!(bridge.activity.is_empty());
    }
    #[test]
    fn jsonrpc_wire_is_one_line() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let peer = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut result = Vec::new();
            let mut b = [0; 1];
            while s.read_exact(&mut b).is_ok() {
                result.push(b[0]);
                if b[0] == b'\n' {
                    break;
                }
            }
            serde_json::from_slice::<Value>(&result).unwrap()
        });
        let mut stream = TcpStream::connect(address).unwrap();
        send(&mut stream, 42, "tell", json!({"message":"hello\nworld"})).unwrap();
        let v = peer.join().unwrap();
        assert_eq!(v["id"], 42);
        assert_eq!(v["params"]["message"], "hello\nworld");
    }
}

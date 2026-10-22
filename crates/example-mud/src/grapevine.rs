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
            credentials_file: std::path::PathBuf::from(std::env::var_os("XDG_STATE_HOME").unwrap_or_else(|| std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state").into_os_string())).join("savsoul-grapevine/credentials.json").to_string_lossy().into_owned(),
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
    app.add_systems(Startup, start).add_systems(Update, tick);
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
fn credentials(path: &str) -> Result<Credentials, &'static str> {
    if !std::path::Path::new(path).exists() {
        return Err("Grapevine credentials are missing. Run configure-grapevine.ps1, then grapevine reload.");
    }
    if private_path(path).is_none() {
        return Err("Credentials must be in private local Linux state; see help grapevine.");
    }
    let meta = std::fs::symlink_metadata(path).map_err(|_| {
        "Grapevine credentials are missing. Run configure-grapevine.ps1, then grapevine reload."
    })?;
    if !meta.is_file() || meta.len() > 8192 {
        return Err("Grapevine credentials file is invalid; rerun the private setup helper.");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("Grapevine credentials need chmod 600; refusing to expose them.");
        }
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(8193).read_to_end(&mut bytes))
        .map_err(|_| "Grapevine credentials cannot be read.")?;
    let c: Credentials = serde_json::from_slice(&bytes)
        .map_err(|_| "Grapevine credentials JSON is invalid; rerun private setup.")?;
    if c.client_id.trim().is_empty()
        || c.client_secret.trim().is_empty()
        || c.client_id.len() > 512
        || c.client_secret.len() > 4096
    {
        return Err("Grapevine credentials are incomplete; rerun private setup.");
    }
    Ok(c)
}
fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .read_buffer_size(8192)
        .write_buffer_size(0)
        .max_write_buffer_size(128 * 1024)
        .max_message_size(Some(MAX_PACKET))
        .max_frame_size(Some(MAX_PACKET))
}
fn auth(c: &Credentials, _snapshot: &Arc<Mutex<Snapshot>>) -> Value {
    // Subscribe with correlated refs after authentication; auth success alone does not confirm channels.
    let channels: Vec<String> = Vec::new();
    json!({"event":"authenticate","payload":{"client_id":c.client_id,"client_secret":c.client_secret,"supports":["channels","players","tells","games"],"channels":channels,"version":"2.3.0","user_agent":"The Savage Soul / Grim"}})
}
fn set_timeout(stream: &mut MaybeTlsStream<TcpStream>, read: Duration) {
    match stream {
        MaybeTlsStream::Plain(s) => {
            let _ = s.set_read_timeout(Some(read));
            let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
        }
        MaybeTlsStream::Rustls(s) => {
            let _ = s.sock.set_read_timeout(Some(read));
            let _ = s.sock.set_write_timeout(Some(Duration::from_secs(2)));
        }
        _ => {}
    }
}
fn connect() -> Result<WebSocket<MaybeTlsStream<TcpStream>>, &'static str> {
    // Tungstenite deliberately does not select a crypto provider. Install the audited
    // ring provider explicitly; an already-installed process provider stays intact.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let addresses = ("grapevine.haus", 443)
        .to_socket_addrs()
        .map_err(|_| "Grapevine DNS lookup failed; retrying.")?;
    for addr in addresses.take(4) {
        if let Ok(tcp) = TcpStream::connect_timeout(&addr, Duration::from_secs(4)) {
            let _ = tcp.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = tcp.set_write_timeout(Some(Duration::from_secs(5)));
            if let Ok((mut socket, _)) =
                tungstenite::client_tls_with_config(ENDPOINT, tcp, Some(ws_config()), None)
            {
                set_timeout(socket.get_mut(), Duration::from_millis(50));
                return Ok(socket);
            }
        }
    }
    Err("Grapevine TLS/WebSocket connection failed; retrying. Certificate verification stays enabled.")
}
fn emit(events: &SyncSender<Incoming>, ready: bool, generation: u64, text: &str) {
    let _ = events.try_send(Incoming::State {
        ready,
        generation,
        text: text.into(),
    });
}
fn worker(
    config: Config,
    requests: Receiver<Job>,
    events: SyncSender<Incoming>,
    snapshot: Arc<Mutex<Snapshot>>,
    reload: Arc<AtomicBool>,
) {
    let mut generation = 0;
    let mut backoff = 2u64;
    loop {
        // A connection generation never inherits a prior generation's chat/tell queue.
        loop {
            match requests.try_recv() {
                Ok(job) => {
                    let reference = job.packet["ref"].as_str().unwrap_or("").to_owned();
                    let _ = events.try_send(Incoming::Dropped(reference));
                }
                Err(mpsc::TryRecvError::Disconnected) => return,
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        reload.store(false, Ordering::Relaxed);
        let mut wait = backoff;
        match credentials(&config.credentials_file) {
            Err(text) => {
                emit(&events, false, generation, text);
                wait = 10;
            }
            Ok(c) => match connect() {
                Err(text) => emit(&events, false, generation, text),
                Ok(mut socket) => {
                    generation = generation.wrapping_add(1);
                    emit(
                        &events,
                        false,
                        generation,
                        "Grapevine socket connected; authenticating.",
                    );
                    let packet = auth(&c, &snapshot);
                    let _ = socket.send(Message::Text(packet.to_string().into()));
                    let outcome = run_connection(
                        &mut socket,
                        &requests,
                        &events,
                        &snapshot,
                        &reload,
                        generation,
                    );
                    if outcome.authenticated {
                        backoff = 2;
                    }
                    wait = outcome.retry_secs.unwrap_or(wait).clamp(1, 300);
                    emit(&events, false, generation, outcome.status);
                    let _ = socket.close(None);
                }
            },
        }
        backoff = (backoff * 2).min(60);
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(wait) {
            if reload.load(Ordering::Relaxed) {
                break;
            }
            match requests.try_recv() {
                Ok(job) => {
                    let _ = events.try_send(Incoming::Dropped(
                        job.packet["ref"].as_str().unwrap_or("").into(),
                    ));
                }
                Err(mpsc::TryRecvError::Disconnected) => return,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
struct Outcome {
    authenticated: bool,
    retry_secs: Option<u64>,
    status: &'static str,
}
fn run_connection<S: Read + Write>(
    socket: &mut WebSocket<S>,
    requests: &Receiver<Job>,
    events: &SyncSender<Incoming>,
    snapshot: &Arc<Mutex<Snapshot>>,
    reload: &Arc<AtomicBool>,
    generation: u64,
) -> Outcome {
    let mut authenticated = false;
    let started = Instant::now();
    let mut last_heartbeat = Instant::now();
    let mut restart = None;
    let mut published = HashSet::<String>::new();
    let mut bootstrap = HashMap::<String, String>::new();
    let result = loop {
        if reload.load(Ordering::Relaxed) {
            break "Grapevine reloading private credentials.";
        }
        if !authenticated && started.elapsed() > Duration::from_secs(10) {
            break "Grapevine authentication timed out; check private credentials.";
        }
        if authenticated && last_heartbeat.elapsed() > Duration::from_secs(180) {
            break "Grapevine heartbeat timed out; reconnecting.";
        }
        if authenticated {
            let current: HashSet<_> = snapshot
                .lock()
                .ok()
                .map(|s| s.actors.values().cloned().collect())
                .unwrap_or_default();
            for (event, names) in [
                (
                    "players/sign-out",
                    published.difference(&current).cloned().collect::<Vec<_>>(),
                ),
                (
                    "players/sign-in",
                    current.difference(&published).cloned().collect::<Vec<_>>(),
                ),
            ] {
                for name in names {
                    if socket
                        .send(Message::Text(
                            json!({"event":event,"payload":{"name":name}})
                                .to_string()
                                .into(),
                        ))
                        .is_err()
                    {
                        return Outcome {
                            authenticated,
                            retry_secs: restart,
                            status: "Grapevine presence update failed; reconnecting.",
                        };
                    }
                }
            }
            published = current;
            for _ in 0..16 {
                let job = match requests.try_recv() {
                    Ok(j) => j,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        return Outcome {
                            authenticated,
                            retry_secs: Some(1),
                            status: "Grapevine stopped.",
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                };
                let authorized = snapshot.lock().ok().is_some_and(|s| {
                    s.actors.get(&job.actor).is_some_and(|name| {
                        match job.packet["event"].as_str() {
                            Some("channels/send") => {
                                job.packet["payload"]["name"].as_str() == Some(name.as_str())
                            }
                            Some("tells/send") => {
                                job.packet["payload"]["from_name"].as_str() == Some(name.as_str())
                            }
                            _ => true,
                        }
                    })
                });
                if !authorized
                    || job.generation != generation
                    || job.queued.elapsed() > Duration::from_secs(10)
                {
                    let _ = events.try_send(Incoming::Dropped(
                        job.packet["ref"].as_str().unwrap_or("").into(),
                    ));
                    continue;
                }
                if socket
                    .send(Message::Text(job.packet.to_string().into()))
                    .is_err()
                {
                    return Outcome {
                        authenticated,
                        retry_secs: restart,
                        status: "Grapevine disconnected; queued messages will not be replayed.",
                    };
                }
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let packet: Value = match serde_json::from_str(text.as_str()) {
                    Ok(p) => p,
                    Err(_) => break "Grapevine sent invalid JSON; reconnecting.",
                };
                let event = packet["event"].as_str().unwrap_or("");
                if event == "authenticate" {
                    if packet["status"] != "success" {
                        break "Grapevine authentication rejected; check private credentials.";
                    }
                    authenticated = true;
                    last_heartbeat = Instant::now();
                    emit(events, true, generation, "Grapevine authenticated.");
                    let channels: Vec<_> = snapshot
                        .lock()
                        .ok()
                        .map(|s| s.channels.iter().take(32).cloned().collect())
                        .unwrap_or_default();
                    for channel in channels {
                        let reference = uuid::Uuid::new_v4().to_string();
                        let packet = json!({"event":"channels/subscribe","ref":reference,"payload":{"channel":channel}});
                        if socket
                            .send(Message::Text(packet.to_string().into()))
                            .is_err()
                        {
                            return Outcome {
                                authenticated,
                                retry_secs: restart,
                                status: "Grapevine channel subscription failed; reconnecting.",
                            };
                        }
                        bootstrap.insert(reference, channel);
                    }
                    continue;
                }
                if !authenticated {
                    continue;
                }
                if event == "channels/subscribe" {
                    if let Some(channel) = packet["ref"].as_str().and_then(|r| bootstrap.remove(r))
                    {
                        let _ = events.try_send(Incoming::Subscribed {
                            channel,
                            success: packet["status"] != "failure",
                        });
                        continue;
                    }
                }
                if event == "heartbeat" {
                    last_heartbeat = Instant::now();
                    let players: Vec<_> = snapshot
                        .lock()
                        .ok()
                        .map(|s| s.actors.values().cloned().collect())
                        .unwrap_or_default();
                    if socket
                        .send(Message::Text(
                            json!({"event":"heartbeat","payload":{"players":players}})
                                .to_string()
                                .into(),
                        ))
                        .is_err()
                    {
                        break "Grapevine heartbeat could not be sent; reconnecting.";
                    }
                    continue;
                }
                if event == "restart" {
                    restart = Some(
                        packet["payload"]["downtime"]
                            .as_u64()
                            .unwrap_or(15)
                            .clamp(1, 300),
                    );
                    let _ = events.try_send(Incoming::Packet(packet));
                    continue;
                }
                if events.try_send(Incoming::Packet(packet)).is_err() {
                    break "Grapevine receive queue full; reconnecting without replay.";
                }
            }
            Ok(Message::Ping(data)) => {
                if socket.send(Message::Pong(data)).is_err() {
                    break "Grapevine ping failed; reconnecting.";
                }
            }
            Ok(Message::Close(_)) => break "Grapevine closed the connection; reconnecting.",
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break "Grapevine connection ended; reconnecting.",
        }
    };
    Outcome {
        authenticated,
        retry_secs: restart,
        status: result,
    }
}
fn queue(
    b: &mut Bridge,
    actor: Entity,
    event: &str,
    payload: Value,
    filter: String,
    echo: Option<String>,
) -> Result<(), String> {
    if !b.ready {
        return Err(b.status.clone());
    }
    if b.pending.len() >= 64 {
        return Err("Grapevine is busy; wait for pending requests.".into());
    }
    let reference = uuid::Uuid::new_v4().to_string();
    let packet = json!({"event":event,"ref":reference,"payload":payload});
    b.tx.try_send(Job {
        actor,
        generation: b.generation,
        queued: Instant::now(),
        packet,
    })
    .map_err(|_| "Grapevine queue unavailable; try again.".to_owned())?;
    b.pending.insert(
        reference,
        Pending {
            actor,
            event: event.into(),
            filter,
            at: Instant::now(),
            replies: 0,
            echo,
        },
    );
    Ok(())
}
pub(crate) fn command(world: &mut World, actor: Entity, arg: &str) -> Result<(), String> {
    if !allowed(world, actor) {
        return Err("Grapevine commands are wizard-only.".into());
    }
    let Some(mut b) = world.remove_resource::<Bridge>() else {
        return Err("Grapevine is disabled; see help grapevine.".into());
    };
    // Update the worker's send authorization before enqueuing commands in this frame.
    update_presence(world, &mut b);
    let result = command_inner(world, &mut b, actor, arg);
    world.insert_resource(b);
    result
}
fn command_inner(
    world: &mut World,
    b: &mut Bridge,
    actor: Entity,
    arg: &str,
) -> Result<(), String> {
    let (verb, rest) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
    let rest = rest.trim();
    match verb.to_ascii_lowercase().as_str() {
        "" | "status" => {
            tell(world,actor,format!("{} {} pending requests; {} channel messages retained locally. No credentials are displayed.",b.status,b.pending.len(),b.history.len()));
            return Ok(());
        }
        "reload" => {
            b.ready = false;
            b.reload.store(true, Ordering::Relaxed);
            tell(world,actor,"Grapevine will reconnect and reload private credentials. Unsent messages are discarded.");
            return Ok(());
        }
        "channels" => {
            let desired = b
                .snapshot
                .lock()
                .ok()
                .map(|s| {
                    let mut v: Vec<_> = s.channels.iter().cloned().collect();
                    v.sort();
                    v.join(", ")
                })
                .unwrap_or_default();
            let mut own: Vec<_> = b
                .listeners
                .get(&actor)
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            own.sort();
            tell(world,actor,format!("Grapevine channels: {desired}. You listen to: {}. Channel names contain 3–15 letters, _ or -.",if own.is_empty(){"none".into()}else{own.join(", ")}));
            return Ok(());
        }
        "recent" => {
            if !rest.is_empty() && !valid_channel(rest) {
                return Err("Use grapevine recent [channel].".into());
            }
            let rows: Vec<_> = b
                .history
                .iter()
                .rev()
                .filter(|h| rest.is_empty() || h.channel.eq_ignore_ascii_case(rest))
                .take(20)
                .map(|h| {
                    format!(
                        "[{}; {}s ago] {}@{}: {}",
                        remote(&h.channel),
                        now().saturating_sub(h.at),
                        remote(&h.speaker),
                        remote(&h.game),
                        remote(&h.message)
                    )
                })
                .collect();
            tell(
                world,
                actor,
                if rows.is_empty() {
                    "No recent Grapevine channel activity.".into()
                } else {
                    rows.into_iter().rev().collect::<Vec<_>>().join("\n")
                },
            );
            return Ok(());
        }
        "unlisten" => {
            if !valid_channel(rest) {
                return Err("Use grapevine unlisten <channel>.".into());
            }
            b.listeners.entry(actor).or_default().remove(rest);
            tell(
                world,
                actor,
                format!(
                    "You no longer listen to {}. Other wizards and recent history are unaffected.",
                    remote(rest)
                ),
            );
            return Ok(());
        }
        _ => {}
    }
    if b.cooldown
        .get(&actor)
        .is_some_and(|t| t.elapsed() < Duration::from_millis(750))
    {
        return Err("Wait a moment between Grapevine network requests.".into());
    }
    let name = world
        .get::<GrimName>(actor)
        .map(|n| n.0.clone())
        .ok_or("Character name unavailable.")?;
    match verb.to_ascii_lowercase().as_str(){
        "listen"=>{if !valid_channel(rest){return Err("Use grapevine listen <channel> (3–15 letters, _ or -).".into());}if b.snapshot.lock().ok().is_some_and(|s|s.channels.len()>=32&&!s.channels.contains(rest)){return Err("At most 32 channels may be tracked.".into());}
            if b.confirmed.contains(rest){b.listeners.entry(actor).or_default().insert(rest.into());tell(world,actor,format!("You now listen to {}.",remote(rest)));return Ok(());}
            queue(b,actor,"channels/subscribe",json!({"channel":rest}),rest.into(),None)?;
        },
        "games"=>{queue(b,actor,"games/status",json!({}),String::new(),None)?;},
        "who"=>{if rest.len()>100{return Err("Game name is too long.".into());}queue(b,actor,"players/status",json!({}),rest.into(),None)?;},
        "chat"=>{let(channel,message)=rest.split_once(' ').ok_or("Use grapevine chat <channel> <message>.")?;if !valid_channel(channel)||!b.confirmed.contains(channel){return Err("Listen to that channel successfully before sending.".into());}if message.trim().is_empty()||message.len()>1024{return Err("Use a message of 1–1024 bytes.".into());}queue(b,actor,"channels/send",json!({"channel":channel,"name":name,"message":message.trim()}),channel.into(),Some(format!("[Grapevine:{}] {}: {}",remote(channel),remote(&name),remote(message.trim()))))?;},
        "tell"=>{let(target,message)=rest.split_once(':').ok_or("Use grapevine tell <user>@<game>: <message>.")?;let(user,game)=target.trim().split_once('@').ok_or("Use grapevine tell <user>@<game>: <message>.")?;if user.is_empty()||game.is_empty()||user.len()>100||game.len()>100||message.trim().is_empty()||message.len()>1024{return Err("Use a recipient and a message of 1–1024 bytes.".into());}let sent_at=chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs,true);queue(b,actor,"tells/send",json!({"from_name":name,"to_game":game,"to_name":user,"sent_at":sent_at,"message":message.trim()}),String::new(),Some(format!("[Grapevine] You tell {}@{}: {}",remote(user),remote(game),remote(message.trim()))))?;},
        _=>return Err("Grapevine: status, reload, games, who [game], channels, listen/unlisten <channel>, chat <channel> <message>, tell user@game: message, recent [channel].".into())
    }
    b.cooldown.insert(actor, Instant::now());
    tell(world, actor, "Grapevine request queued.");
    Ok(())
}
fn update_presence(world: &mut World, b: &mut Bridge) {
    let actors: HashMap<_, _> = world
        .query::<(Entity, &GrimName, &Character, &Player)>()
        .iter(world)
        .filter(|(_, _, c, _)| c.is_admin())
        .take(128)
        .map(|(e, n, _, _)| (e, bounded(&n.0, 100)))
        .collect();
    if let Ok(mut s) = b.snapshot.lock() {
        s.actors = actors;
    }
}
fn tick(world: &mut World) {
    let Some(mut b) = world.remove_resource::<Bridge>() else {
        return;
    };
    update_presence(world, &mut b);
    let events: Vec<_> =
        b.rx.lock()
            .ok()
            .map(|rx| rx.try_iter().take(32).collect())
            .unwrap_or_default();
    for event in events {
        match event {
            Incoming::State {
                ready,
                generation,
                text,
            } => {
                b.ready = ready;
                b.generation = generation;
                b.status = text;
                if ready {
                    b.confirmed.clear();
                } else {
                    b.confirmed.clear();
                    let pending = std::mem::take(&mut b.pending);
                    for p in pending.into_values() {
                        if allowed(world, p.actor) {
                            tell(world,p.actor,"Grapevine disconnected. Pending requests were discarded; messages will not be replayed.");
                        }
                    }
                }
            }
            Incoming::Packet(packet) => deliver(world, &mut b, packet),
            Incoming::History(mut records) => {
                records.append(&mut b.history);
                while records.len() > 200 {
                    records.pop_front();
                }
                b.history = records;
                mark_history(&mut b);
            }
            Incoming::Subscribed { channel, success } => {
                if success {
                    b.confirmed.insert(channel);
                } else {
                    b.confirmed.remove(&channel);
                }
            }
            Incoming::Dropped(reference) => {
                if let Some(p) = b.pending.remove(&reference) {
                    if allowed(world, p.actor) {
                        tell(world,p.actor,"Grapevine request was discarded because the session changed or the connection ended.");
                    }
                }
            }
        }
    }
    let expired: Vec<_> = b
        .pending
        .iter()
        .filter(|(_, p)| p.at.elapsed() > Duration::from_secs(10))
        .map(|(r, _)| r.clone())
        .collect();
    for reference in expired {
        if let Some(p) = b.pending.remove(&reference) {
            if allowed(world, p.actor) {
                if p.replies == 0 {
                    tell(
                        world,
                        p.actor,
                        format!(
                            "Grapevine {}: no matching response within 10 seconds.",
                            remote(&p.event)
                        ),
                    );
                } else if matches!(p.event.as_str(), "games/status" | "players/status") {
                    tell(
                        world,
                        p.actor,
                        format!("Grapevine query complete: {} response(s).", p.replies),
                    );
                }
            }
        }
    }
    b.listeners.retain(|e, _| allowed(world, *e));
    b.cooldown
        .retain(|e, t| allowed(world, *e) && t.elapsed() < Duration::from_secs(60));
    world.insert_resource(b);
}
fn deliver(world: &mut World, b: &mut Bridge, packet: Value) {
    let event = packet["event"].as_str().unwrap_or("");
    let payload = &packet["payload"];
    match event {
        "channels/broadcast" => {
            let Some(channel) = payload["channel"].as_str().filter(|s| valid_channel(s)) else {
                return;
            };
            if !b.confirmed.contains(channel) {
                return;
            }
            let (Some(message), Some(name), Some(game)) = (
                payload["message"].as_str(),
                payload["name"].as_str(),
                payload["game"].as_str(),
            ) else {
                return;
            };
            let record = Activity {
                at: now(),
                channel: channel.into(),
                speaker: bounded(name, 100),
                game: bounded(game, 100),
                message: bounded(message, 1024),
            };
            let text = format!(
                "[Grapevine:{}] {}@{}: {}",
                remote(channel),
                remote(name),
                remote(game),
                remote(message)
            );
            b.history.push_back(record);
            while b.history.len() > 200 {
                b.history.pop_front();
            }
            mark_history(b);
            let targets: Vec<_> = b
                .listeners
                .iter()
                .filter(|(e, c)| allowed(world, **e) && c.contains(channel))
                .map(|(e, _)| *e)
                .collect();
            for actor in targets {
                tell(world, actor, &text);
            }
            return;
        }
        "tells/receive" => {
            let Some(target) = payload["to_name"].as_str() else {
                return;
            };
            let targets: Vec<_> = world
                .query::<(Entity, &GrimName, &Character, &Player)>()
                .iter(world)
                .filter(|(_, n, c, _)| c.is_admin() && n.0.eq_ignore_ascii_case(target))
                .map(|(e, _, _, _)| e)
                .collect();
            for actor in targets {
                tell(
                    world,
                    actor,
                    format!(
                        "[Grapevine] {}@{} tells you: {}",
                        field(payload, "from_name"),
                        field(payload, "from_game"),
                        field(payload, "message")
                    ),
                );
            }
            return;
        }
        "restart" => {
            b.status = "Grapevine announced a restart; automatic reconnect is ready.".into();
            return;
        }
        _ => {}
    }
    let Some(reference) = packet["ref"].as_str() else {
        return;
    };
    let Some(mut pending) = b.pending.remove(reference) else {
        return;
    };
    if pending.event != event || !allowed(world, pending.actor) {
        return;
    }
    if packet["status"] == "failure" {
        tell(world,pending.actor,format!("Grapevine {} failed. Check the channel, game, player, or private credentials; remote error details are suppressed.",remote(event)));
        return;
    }
    match event {
        "channels/subscribe" => {
            let channel = pending.filter.clone();
            b.confirmed.insert(channel.clone());
            if let Ok(mut s) = b.snapshot.lock() {
                s.channels.insert(channel.clone());
            }
            b.listeners
                .entry(pending.actor)
                .or_default()
                .insert(channel.clone());
            tell(
                world,
                pending.actor,
                format!("You now listen to Grapevine {}.", remote(&channel)),
            );
        }
        "channels/send" | "tells/send" => {
            if let Some(echo) = pending.echo.take() {
                tell(world, pending.actor, echo);
            }
        }
        "games/status" | "players/status" => {
            let game = payload["game"].as_str().unwrap_or("?");
            if !pending.filter.is_empty() && !game.eq_ignore_ascii_case(&pending.filter) {
                b.pending.insert(reference.into(), pending);
                return;
            }
            pending.replies += 1;
            if pending.replies <= 256 {
                let text = if event == "games/status" {
                    format!(
                        "[Grapevine] {} — {} ({} players)",
                        remote(game),
                        field(payload, "display_name"),
                        payload["players_online_count"]
                            .as_u64()
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "?".into())
                    )
                } else {
                    let names = payload["players"]
                        .as_array()
                        .map(|v| {
                            v.iter()
                                .take(128)
                                .filter_map(Value::as_str)
                                .map(remote)
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    format!(
                        "[Grapevine] {}: {}",
                        remote(game),
                        if names.is_empty() {
                            "no players reported".into()
                        } else {
                            names
                        }
                    )
                };
                tell(world, pending.actor, text);
            }
            b.pending.insert(reference.into(), pending);
        }
        _ => {}
    }
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
#[derive(Default)]
struct HistoryStore {
    records: VecDeque<Activity>,
    dirty: bool,
}
fn private_path(path: &str) -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(path);
    if !p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        || ["/mnt", "/proc", "/sys", "/dev"]
            .iter()
            .any(|prefix| p.starts_with(prefix))
    {
        return None;
    }
    let parent = p.parent()?;
    let canonical = parent.canonicalize().ok()?;
    if canonical != parent {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = parent.metadata().ok()?;
        let current = std::fs::metadata("/proc/self").ok()?.uid();
        if meta.uid() != current || meta.permissions().mode() & 0o077 != 0 {
            return None;
        }
    }
    Some(p)
}
fn valid_activity(record: &Activity) -> bool {
    record.at > 0
        && record.at <= now().saturating_add(5)
        && valid_channel(&record.channel)
        && record.speaker.len() <= 400
        && record.game.len() <= 400
        && record.message.len() <= 4096
}
fn load_history(path: &str) -> VecDeque<Activity> {
    let Some(path) = private_path(path) else {
        return VecDeque::new();
    };
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return VecDeque::new();
    };
    if !meta.is_file() || meta.len() > 256 * 1024 {
        return VecDeque::new();
    }
    let Ok(file) = std::fs::File::open(path) else {
        return VecDeque::new();
    };
    let mut bytes = Vec::new();
    if file.take(256 * 1024 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > 256 * 1024 {
        return VecDeque::new();
    }
    let Ok(records) = serde_json::from_slice::<VecDeque<Activity>>(&bytes) else {
        return VecDeque::new();
    };
    if records.len() > 200 || records.iter().any(|r| !valid_activity(r)) {
        return VecDeque::new();
    }
    records
}
fn save_history(path: &str, records: &VecDeque<Activity>) -> std::io::Result<()> {
    let path = private_path(path).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid private history path",
        )
    })?;
    let mut records = records.clone();
    let bytes = loop {
        let bytes = serde_json::to_vec(&records)?;
        if bytes.len() <= 256 * 1024 {
            break bytes;
        }
        records.pop_front();
    };
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // A stale temp file is never followed or overwritten; use a fresh unique suffix.
    let temp = if temp.exists() {
        path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()))
    } else {
        temp
    };
    let mut file = options.open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_data()?;
    drop(file);
    std::fs::rename(temp, path)
}
fn history_worker(path: String, store: Arc<Mutex<HistoryStore>>, events: SyncSender<Incoming>) {
    let records = load_history(&path);
    let _ = events.try_send(Incoming::History(records));
    loop {
        std::thread::sleep(Duration::from_millis(1000));
        let records = store.lock().ok().and_then(|mut state| {
            if state.dirty {
                state.dirty = false;
                Some(state.records.clone())
            } else {
                None
            }
        });
        if let Some(records) = records {
            if save_history(&path, &records).is_err() {
                if let Ok(mut state) = store.lock() {
                    state.dirty = true;
                }
            }
        }
        if Arc::strong_count(&store) == 1 {
            return;
        }
    }
}
fn mark_history(b: &mut Bridge) {
    if let Ok(mut state) = b.history_store.lock() {
        state.records = b.history.clone();
        state.dirty = true;
    }
}

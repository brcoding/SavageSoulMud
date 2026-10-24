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
fn unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn bounded(value: &str, limit: usize) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if out.len() + c.len_utf8() > limit {
            break;
        }
        out.push(c);
    }
    out
}
fn valid_record(record: &Activity, now: u64) -> bool {
    record.at > 0
        && record.at <= now
        && valid_channel(&record.channel)
        && record.speaker.len() <= 128
        && record.mud.len() <= 128
        && record.message.len() <= 1024
}
fn record_activity(history: &mut VecDeque<Activity>, kind: &str, p: &Value, now: u64) -> bool {
    if !matches!(kind, "channel_message" | "channel_emote") {
        return false;
    }
    let Some(channel) = p["channel"].as_str().filter(|c| valid_channel(c)) else {
        return false;
    };
    let Some(message) = p["message"].as_str() else {
        return false;
    };
    history.push_back(Activity {
        at: now,
        channel: channel.into(),
        speaker: bounded(p["from_user"].as_str().unwrap_or("?"), 128),
        mud: bounded(p["from_mud"].as_str().unwrap_or("?"), 128),
        message: bounded(message, 1024),
        emote: kind == "channel_emote",
    });
    while history.len() > ACTIVITY_LIMIT
        || serde_json::to_vec(history).map_or(true, |b| b.len() as u64 > ACTIVITY_BYTES)
    {
        history.pop_front();
    }
    true
}
fn activity_path(path: &str) -> Option<std::path::PathBuf> {
    if !path.starts_with("/home/") || path.split('/').any(|p| p == "..") {
        return None;
    }
    let path = std::path::PathBuf::from(path);
    let parent = path.parent()?.canonicalize().ok()?;
    if !parent.starts_with("/home") {
        return None;
    }
    Some(parent.join(path.file_name()?))
}
fn load_activity(path: &str) -> VecDeque<Activity> {
    let Some(path) = activity_path(path) else {
        return VecDeque::new();
    };
    let Ok(file) = std::fs::File::open(path) else {
        return VecDeque::new();
    };
    let mut bytes = Vec::new();
    if file
        .take(ACTIVITY_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > ACTIVITY_BYTES
    {
        return VecDeque::new();
    }
    let Ok(history) = serde_json::from_slice::<VecDeque<Activity>>(&bytes) else {
        return VecDeque::new();
    };
    let now = unix_time();
    if history.len() > ACTIVITY_LIMIT || history.iter().any(|r| !valid_record(r, now)) {
        return VecDeque::new();
    }
    history
}
fn save_activity(path: &str, history: &VecDeque<Activity>) -> std::io::Result<()> {
    if path.is_empty() {
        return Ok(());
    }
    let path = activity_path(path).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "activity_file must be inside an existing local /home directory",
        )
    })?;
    let temp = path.with_extension("tmp");
    let bytes = serde_json::to_vec(history)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_data()?;
    drop(file);
    std::fs::rename(temp, path)
}
fn age(at: u64, now: u64) -> String {
    let seconds = now.saturating_sub(at);
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86400)
    }
}
fn activity_command(bridge: &Bridge, arg: &str) -> Option<String> {
    let (verb, filter) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
    let filter = filter.trim();
    let now = unix_time();
    match verb.to_ascii_lowercase().as_str() {
        "recent" => {
            if !filter.is_empty() && !valid_channel(filter) {
                return Some("Use i3 recent [channel].".into());
            }
            let mut records: Vec<_> = bridge
                .activity
                .iter()
                .rev()
                .filter(|r| filter.is_empty() || r.channel.eq_ignore_ascii_case(filter))
                .take(20)
                .collect();
            records.reverse();
            if records.is_empty() {
                return Some("No observed channel messages yet. History starts when this MUD listens; private tells are excluded.".into());
            }
            let mut output = format!(
                "Recent Intermud: {} observed messages (oldest first).",
                records.len()
            );
            for r in records {
                output.push_str(&format!(
                    "\n[{}; {}] {}@{}{} {}",
                    plain(&r.channel),
                    age(r.at, now),
                    plain(&r.speaker),
                    plain(&r.mud),
                    if r.emote { " emotes:" } else { ":" },
                    plain(&r.message)
                ));
            }
            Some(output)
        }
        "activity" => {
            if !filter.is_empty() {
                return Some("Use i3 activity; use i3 recent <channel> for messages.".into());
            }
            let mut channels: HashMap<&str, (usize, u64)> = HashMap::new();
            for r in &bridge.activity {
                let item = channels.entry(&r.channel).or_default();
                item.0 += 1;
                item.1 = item.1.max(r.at);
            }
            let mut rows: Vec<_> = channels.into_iter().collect();
            rows.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then(a.0.cmp(b.0)));
            let mut output=format!("Observed channel activity: {} retained public messages (maximum 200; listened channels only).",bridge.activity.len());
            if rows.is_empty() {
                output.push_str(" No messages observed yet.");
            }
            for (channel, (count, at)) in rows {
                output.push_str(&format!(
                    "\n{}: {} retained; latest {}.",
                    plain(channel),
                    count,
                    age(at, now)
                ));
            }
            Some(output)
        }
        _ => None,
    }
}
/// Optional plugin: no network thread or credentials are opened unless configured.
pub struct IntermudPlugin;
impl Plugin for IntermudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start).add_systems(Update, tick.after(grim::SceneSystems::Staff));
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
fn send(stream: &mut TcpStream, id: u64, method: &str, params: Value) -> std::io::Result<()> {
    let mut bytes =
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
    bytes.push(b'\n');
    stream.write_all(&bytes)
}
fn worker(config: Config, requests: Receiver<Request>, events: SyncSender<Incoming>) {
    let addr = SocketAddr::new(config.host.parse().unwrap(), config.port);
    loop {
        // A disconnected bridge never replays messages from an earlier session.
        while requests.try_recv().is_ok() {}
        let outcome = (|| -> std::io::Result<()> {
            let key = std::fs::read_to_string(&config.api_key_file)?;
            let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
            stream.set_read_timeout(Some(Duration::from_millis(100)))?;
            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
            send(
                &mut stream,
                1,
                "authenticate",
                json!({"api_key":key.trim()}),
            )?;
            let mut authenticated = false;
            let began = Instant::now();
            let mut buffer = Vec::new();
            let mut bytes = [0; 8192];
            loop {
                if !authenticated && began.elapsed() > Duration::from_secs(8) {
                    return Err(std::io::ErrorKind::TimedOut.into());
                }
                match stream.read(&mut bytes) {
                    Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
                    Ok(n) => {
                        buffer.extend_from_slice(&bytes[..n]);
                        if buffer.len() > 1_048_576 {
                            return Err(std::io::ErrorKind::InvalidData.into());
                        }
                        while let Some(end) = buffer.iter().position(|&b| b == b'\n') {
                            let line: Vec<_> = buffer.drain(..=end).collect();
                            let packet: Value = serde_json::from_slice(&line)?;
                            if !authenticated {
                                if packet["id"] == 1 {
                                    if packet["result"]["status"] != "authenticated" {
                                        return Err(std::io::ErrorKind::PermissionDenied.into());
                                    }
                                    authenticated = true;
                                    if events.send(Incoming::Connected).is_err() {
                                        return Err(std::io::ErrorKind::BrokenPipe.into());
                                    }
                                }
                            } else {
                                forward_packet(&events,packet)?;
                            }
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(e) => return Err(e),
                }
                if authenticated {
                    loop {
                        match requests.try_recv() {
                            Ok(request) => {
                                send(&mut stream, request.id, &request.method, request.params)?
                            }
                            Err(mpsc::TryRecvError::Empty) => break,
                            Err(mpsc::TryRecvError::Disconnected) => {
                                return Err(std::io::ErrorKind::BrokenPipe.into())
                            }
                        }
                    }
                }
            }
        })();
        let _ = outcome;
        if events.send(Incoming::Disconnected).is_err() {
            return;
        }
        // Bounded retry and cancellation when the game's resource is dropped.
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(100));
            match requests.try_recv() {
                Err(mpsc::TryRecvError::Disconnected) => return,
                _ => {}
            }
        }
    }
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
fn request(bridge: &mut Bridge, name: &str, method: &str, params: Value) -> Result<(), String> {
    if !bridge.connected {
        return Err("Intermud gateway is disconnected. It will reconnect automatically.".into());
    }
    if bridge.pending.len() >= 64 {
        return Err("Intermud is busy. Try again shortly.".into());
    }
    bridge.next = bridge.next.wrapping_add(1).max(100);
    let id = bridge.next;
    let label = if method == "channel_list" && params["bulk_listen"] == true {
        "channel_list#listen_all".into()
    } else if method == "mudlist" {
        format!("mudlist#{}", params["local_filter"].as_str().unwrap_or(""))
    } else if method == "tell" {
        format!(
            "tell#{}@{}",
            params["target_user"].as_str().unwrap_or("?"),
            params["target_mud"].as_str().unwrap_or("?")
        )
    } else if method == "channel_send" {
        format!("channel_send#{}", params["channel"].as_str().unwrap_or("?"))
    } else {
        method.into()
    };
    bridge
        .tx
        .try_send(Request {
            id,
            method: method.into(),
            params,
        })
        .map_err(|_| "Intermud is busy. Try again shortly.".to_string())?;
    if !name.is_empty() || matches!(method, "channel_join" | "channel_leave") {
        bridge
            .pending
            .insert(id, (name.into(), label, Instant::now()));
    }
    Ok(())
}
fn load_subscriptions(path: &str) -> Option<Vec<String>> {
    let path = activity_path(path)?;
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(ACTIVITY_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > ACTIVITY_BYTES {
        return None;
    }
    let channels: Vec<String> = serde_json::from_slice(&bytes).ok()?;
    if channels.len() > 512 || channels.iter().any(|c| !valid_channel(c)) {
        return None;
    }
    Some(channels)
}
fn save_subscriptions(path: &str, channels: &[String]) -> std::io::Result<()> {
    if path.is_empty() {
        return Ok(());
    }
    let path = activity_path(path).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "subscriptions_file must be inside existing local /home state directory",
        )
    })?;
    let temp = path.with_extension("subscriptions.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(&serde_json::to_vec(channels)?)?;
    file.sync_data()?;
    drop(file);
    std::fs::rename(temp, path)
}
/// Router recovery is a control event: unlike excess chat, it cannot be dropped.
fn forward_packet(events:&SyncSender<Incoming>,packet:Value)->std::io::Result<()> {
    if packet["method"]=="gateway_reconnected" {
        return events.send(Incoming::Packet(packet)).map_err(|_|std::io::ErrorKind::BrokenPipe.into());
    }
    match events.try_send(Incoming::Packet(packet)) {
        Ok(()) | Err(mpsc::TrySendError::Full(_)) => Ok(()),
        Err(mpsc::TrySendError::Disconnected(_)) => Err(std::io::ErrorKind::BrokenPipe.into()),
    }
}
fn rejoin(bridge: &mut Bridge) {
    bridge.confirmed.clear();
    bridge.subscription_failures = 0;
    let leaves: Vec<_> = bridge.subscriptions.drain(..).filter(|(channel, join)| !join && !bridge.config.channels.contains(channel)).collect();
    bridge.subscriptions = bridge
        .config
        .channels
        .iter()
        .map(|c| (c.clone(), true))
        .chain(leaves)
        .collect();
}
fn pump_subscriptions(bridge: &mut Bridge) {
    if !bridge.connected || bridge.last_subscription.elapsed() < Duration::from_millis(1100) {
        return;
    }
    if let Some((channel, join)) = bridge.subscriptions.front().cloned() {
        if request(
            bridge,
            "",
            if join {
                "channel_join"
            } else {
                "channel_leave"
            },
            json!({"channel":channel}),
        )
        .is_ok()
        {
            bridge.subscriptions.pop_front();
            bridge.last_subscription = Instant::now();
        }
    }
}
fn queue_subscriptions(
    bridge: &mut Bridge,
    channels: Vec<String>,
    join: bool,
) -> Result<usize, String> {
    let mut desired = bridge.config.channels.clone();
    let mut jobs = bridge.subscriptions.clone();
    for channel in channels {
        if !valid_channel(&channel) {
            continue;
        }
        if join {
            if !desired.contains(&channel) {
                desired.push(channel.clone());
            }
        } else {
            desired.retain(|c| c != &channel);
        }
        jobs.retain(|(c, _)| c != &channel);
        jobs.push_back((channel, join));
    }
    if desired.len() > 512 || jobs.len() > 512 {
        return Err("Intermud subscription limit is 512 channels.".into());
    }
    save_subscriptions(&bridge.config.subscriptions_file, &desired)
        .map_err(|_| "Could not save Intermud subscriptions; no changes applied.".to_string())?;
    bridge.config.channels = desired;
    bridge.subscriptions = jobs;
    Ok(bridge.subscriptions.len())
}
fn public_channels(result: &Value) -> (Vec<String>, usize) {
    let mut channels = Vec::new();
    let mut skipped = 0;
    for c in result["channels"].as_array().into_iter().flatten() {
        let name = c["name"].as_str().unwrap_or("");
        if c["type"].as_i64() == Some(0) && valid_channel(name) {
            if !channels.iter().any(|v| v == name) {
                channels.push(name.to_string());
            }
        } else {
            skipped += 1;
        }
    }
    (channels, skipped)
}
fn subscription_command(bridge: &mut Bridge, arg: &str) -> Option<String> {
    let (verb, channel) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
    let channel = channel.trim();
    if verb.eq_ignore_ascii_case("listening") {
        return Some(format!("Intermud subscriptions: {} requested; {} gateway acknowledgements; {} queued; {} failed requests. Requested: {}",bridge.config.channels.len(),bridge.confirmed.len(),bridge.subscriptions.len(),bridge.subscription_failures,bridge.config.channels.iter().map(|c|plain(c)).collect::<Vec<_>>().join(", ")));
    }
    let join = verb.eq_ignore_ascii_case("listen");
    if !join && !verb.eq_ignore_ascii_case("unlisten") {
        return None;
    }
    if join && channel.eq_ignore_ascii_case("all") {
        return None;
    }
    let channels = if !join && channel.eq_ignore_ascii_case("all") {
        let mut all = bridge.config.channels.clone();
        for c in &bridge.confirmed {
            if !all.contains(c) {
                all.push(c.clone());
            }
        }
        all
    } else if valid_channel(channel) {
        vec![channel.into()]
    } else {
        return Some("Use i3 listen <channel|all> or i3 unlisten <channel|all>.".into());
    };
    Some(match queue_subscriptions(bridge, channels, join) {
        Ok(n) => format!(
            "Intermud: {} requests queued; subscriptions are paced. Use i3 listening for progress.",
            n
        ),
        Err(e) => e,
    })
}

fn tick(world: &mut World, mut cursor: Local<MessageCursor<EngineCommand>>) {
    let inputs: Vec<_> = cursor
        .read(world.resource::<Messages<EngineCommand>>())
        .filter_map(|m| match &m.command {
            Command::SavageSoul { verb, arg }
                if ["intermud", "i3", "imud"].contains(&verb.as_str()) =>
            {
                Some((m.client, verb.clone(), arg.clone()))
            }
            _ => None,
        })
        .collect();
    let Some(mut bridge) = world.remove_resource::<Bridge>() else {
        for (e, _, _) in inputs {
            if allowed(world, e, true) {
                reply(world, e, "Intermud is not enabled. See help intermud.");
            } else {
                reply(world, e, "Intermud requires wizard privileges.");
            }
        }
        return;
    };
    let events: Vec<_> = bridge.rx.lock().unwrap().try_iter().take(128).collect();
    for event in events {
        match event {
            Incoming::Connected => {
                bridge.connected = true;
                rejoin(&mut bridge);
                bridge.last_presence = Instant::now() - Duration::from_secs(60);
            }
            Incoming::Disconnected => {
                bridge.connected = false;
                bridge.pending.clear();
            }
            Incoming::Packet(packet) => deliver(world, &mut bridge, packet),
        }
    }
    for (actor, verb, arg) in inputs {
        if !allowed(world, actor, bridge.config.wizard_only) {
            reply(
                world,
                actor,
                "Intermud is currently available to wizards only.",
            );
            continue;
        }
        let name = world
            .get::<GrimName>(actor)
            .map(|n| n.0.clone())
            .unwrap_or_default();
        if bridge
            .cooldown
            .get(&name)
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            reply(
                world,
                actor,
                "Please wait a moment before another Intermud command.",
            );
            continue;
        }
        bridge.cooldown.insert(name.clone(), Instant::now());
        let arg = if verb == "imud" {
            format!("chat {arg}")
        } else {
            arg
        };
        if let Some(output) = activity_command(&bridge, &arg) {
            reply(world, actor, output);
            continue;
        }
        if let Some(output) = subscription_command(&mut bridge, &arg) {
            reply(world, actor, output);
            continue;
        }
        match parse(&name, &arg) {
            Ok((method, params)) => {
                if let Err(e) = request(&mut bridge, &name, &method, params) {
                    reply(world, actor, e)
                }
            }
            Err(e) => reply(world, actor, e),
        }
    }
    pump_subscriptions(&mut bridge);
    let users:Vec<_>=world.query::<(&GrimName,&Character,&Player)>().iter(world).filter(|(_,c,_)|c.is_admin()&&!c.is_hidden()).map(|(n,c,_)|json!({"name":n.0,"title":c.title.clone().unwrap_or_default(),"status":"online"})).collect();
    let presence = json!({"users":users});
    if bridge.connected
        && (presence != bridge.presence || bridge.last_presence.elapsed() > Duration::from_secs(20))
    {
        if request(&mut bridge, "", "presence_sync", presence.clone()).is_ok() {
            bridge.presence = presence;
            bridge.last_presence = Instant::now();
        }
    }
    bridge
        .pending
        .retain(|_, (_, _, t)| t.elapsed() < Duration::from_secs(30));
    bridge
        .cooldown
        .retain(|_, t| t.elapsed() < Duration::from_secs(60));
    if bridge.activity_dirty && bridge.last_activity_save.elapsed() >= Duration::from_secs(5) {
        if let Err(error) = save_activity(&bridge.config.activity_file, &bridge.activity) {
            warn!("Intermud activity could not be saved: {error}");
        }
        bridge.last_activity_save = Instant::now();
        bridge.activity_dirty = false;
    }
    world.insert_resource(bridge);
}
fn parse(name: &str, arg: &str) -> Result<(String, Value), String> {
    let (verb, rest) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
    let rest = rest.trim();
    let result=match verb.to_ascii_lowercase().as_str(){
        ""|"status"=>("status",json!({})),"muds"=>("mudlist",json!({"filter":{"status":"up"},"local_filter":rest})),"channels"=>("channel_list",json!({})),
        "listen" if rest.eq_ignore_ascii_case("all")=>("channel_list",json!({"bulk_listen":true})),
        "listen"|"unlisten" if valid_channel(rest)=>(if verb.eq_ignore_ascii_case("listen"){"channel_join"}else{"channel_leave"},json!({"channel":rest,"user_name":name})),
        "who" if !rest.is_empty()&&rest.len()<=100=>("who",json!({"target_mud":rest,"from_user":name})),
        "chat"=>{let(channel,message)=rest.split_once(' ').ok_or("Use intermud chat <channel> <message>.")?;if !valid_channel(channel)||message.trim().is_empty()||message.len()>1024{return Err("Use a channel name and a message of up to 1024 characters.".into());}("channel_send",json!({"channel":channel,"from_user":name,"message":message.trim()}))},
        "tell"=>{let(target,message)=rest.split_once(' ').ok_or("Use intermud tell <user>@<mud> <message>; mud names may contain spaces when separated with a colon: user@Mud Name: message.")?;
            let (target,message)=if let Some((target,message))=rest.split_once(": "){(target,message)}else{(target,message)};
            let(user,mud)=target.split_once('@').ok_or("Use intermud tell user@mud message.")?;if user.is_empty()||mud.is_empty()||message.trim().is_empty()||message.len()>1024{return Err("Use intermud tell user@mud message (maximum 1024 characters).".into());}("tell",json!({"target_user":user,"target_mud":mud,"from_user":name,"message":message.trim()}))},
        _=>return Err("Intermud: status, muds, channels, recent [channel], activity, listen <channel>, unlisten <channel>, chat <channel> <message>, tell user@mud message, who <mud>. See help intermud.".into())};
    Ok((result.0.into(), result.1))
}
fn valid_channel(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
fn recipients(world: &mut World, _wizard: bool, target: Option<&str>) -> Vec<Entity> {
    world
        .query::<(Entity, &GrimName, &Character, &Player)>()
        .iter(world)
        .filter(|(_, n, c, _)| c.is_admin() && target.is_none_or(|t| n.0.eq_ignore_ascii_case(t)))
        .map(|(e, _, _, _)| e)
        .collect()
}
fn deliver(world: &mut World, bridge: &mut Bridge, packet: Value) {
    if let Some(id) = packet["id"].as_u64() {
        let Some((name, method, _)) = bridge.pending.remove(&id) else {
            return;
        };
        let targets = recipients(world, bridge.config.wizard_only, Some(&name));
        let result = &packet["result"];
        if let Some(channel) = result["channel"].as_str().filter(|c| valid_channel(c)) {
            if result["status"] == "joined" && !bridge.confirmed.iter().any(|c| c == channel) {
                bridge.confirmed.push(channel.into());
            }
            if result["status"] == "left" {
                bridge.confirmed.retain(|c| c != channel);
            }
        }
        if packet.get("error").is_some()
            && matches!(method.as_str(), "channel_join" | "channel_leave")
        {
            bridge.subscription_failures += 1;
        }
        let (method, filter) = method.split_once('#').unwrap_or((&method, ""));
        if method == "channel_list" && filter == "listen_all" && packet.get("error").is_none() {
            // Discovery may outlive the requesting character's wizard privileges.
            if targets.is_empty() {
                return;
            }
            let (channels, skipped) = public_channels(result);
            let count = channels.len();
            let output=match queue_subscriptions(bridge,channels,true){Ok(queued)=>format!("Intermud: requested {} public channels; {} restricted/invalid channels skipped; {} paced requests queued. Router restrictions still apply. Use i3 listening for progress.",count,skipped,queued),Err(e)=>e};
            for e in targets {
                reply(world, e, output.clone());
            }
            return;
        }
        let output = if packet.get("error").is_some() {
            format!("Intermud: {}", text(&packet["error"], "message"))
        } else {
            match method {
                "status" => format!(
                    "Intermud: {} as {}.",
                    if result["connected"].as_bool() == Some(true) {
                        "connected to the router"
                    } else {
                        "gateway connected; router reconnecting"
                    },
                    text(result, "mud_name")
                ),
                "mudlist" => {
                    let filter = filter.to_lowercase();
                    let names: Vec<_> = result["muds"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|m| text(m, "name"))
                        .filter(|name| name.to_lowercase().contains(&filter))
                        .take(150)
                        .collect();
                    format!(
                        "Intermud MUDs ({} shown): {}",
                        names.len(),
                        names.join(", ")
                    )
                }
                "channel_list" => {
                    let names: Vec<_> = result["channels"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .take(150)
                        .map(|c| text(c, "name"))
                        .collect();
                    format!("Intermud channels: {}", names.join(", "))
                }
                "who" => "Intermud: requested remote who; waiting for its reply.".into(),
                "tell" | "channel_send" => format!(
                    "Intermud: {} to {} (gateway acceptance).",
                    text(result, "status"),
                    plain(filter)
                ),
                "channel_join" | "channel_leave" => {
                    format!(
                        "Intermud: {} {}.",
                        text(result, "status"),
                        text(result, "channel")
                    )
                }
                _ => return,
            }
        };
        for e in targets {
            reply(world, e, output.clone());
        }
        return;
    }
    let kind = packet["method"].as_str().unwrap_or("");
    if kind == "gateway_reconnected" {
        rejoin(bridge);
        bridge.last_presence = Instant::now() - Duration::from_secs(60);
        return;
    }
    let p = &packet["params"];
    if kind == "request_received" {
        if let Some(output) = request_notice(p) {
            // Bound discovery spam independently of player command cooldowns.
            let key = "\0request-audit".to_string();
            if bridge.cooldown.get(&key).is_some_and(|at| at.elapsed() < Duration::from_secs(1)) { return; }
            bridge.cooldown.insert(key, Instant::now());
            for actor in recipients(world, true, None) { reply(world, actor, output.clone()); }
        }
        return;
    }
    if matches!(kind, "channel_message" | "channel_emote")
        && !p["channel"].as_str().is_some_and(|channel| {
            bridge
                .config
                .channels
                .iter()
                .any(|c| c.eq_ignore_ascii_case(channel))
        })
    {
        return;
    }
    if record_activity(&mut bridge.activity, kind, p, unix_time()) {
        bridge.activity_dirty = true;
    }
    let directed = matches!(
        kind,
        "tell_received" | "emoteto_received" | "who_reply" | "error_occurred"
    );
    let target = if directed {
        p["to_user"].as_str().filter(|t| !t.is_empty())
    } else {
        None
    };
    if directed && target.is_none() {
        return;
    }
    let output = match kind {
        "tell_received" => format!(
            "[Intermud] {}@{} tells you: {}",
            text(p, "from_user"),
            text(p, "from_mud"),
            text(p, "message")
        ),
        "emoteto_received" => format!(
            "[Intermud] {}@{}: {}",
            text(p, "from_user"),
            text(p, "from_mud"),
            text(p, "message")
        ),
        "channel_message" | "channel_emote" => format!(
            "[I3 {}] {}@{}: {}",
            text(p, "channel"),
            text(p, "from_user"),
            text(p, "from_mud"),
            text(p, "message")
        ),
        "who_reply" => {
            let rows: Vec<_> = p["users"]
                .as_array()
                .into_iter()
                .flatten()
                .take(100)
                .map(|u| {
                    if let Some(row) = u.as_array() {
                        row.iter()
                            .filter_map(Value::as_str)
                            .map(plain)
                            .collect::<Vec<_>>()
                            .join(" ")
                    } else {
                        text(u, "name")
                    }
                })
                .collect();
            format!(
                "[Intermud] Who at {}: {}",
                text(p, "from_mud"),
                rows.join("; ")
            )
        }
        "error_occurred" => format!(
            "[Intermud] {}: {}",
            text(p, "error_code"),
            text(p, "error_message")
        ),
        _ => return,
    };
    for e in recipients(world, bridge.config.wizard_only, target) {
        reply(world, e, output.clone());
    }
}
fn request_notice(p: &Value) -> Option<String> {
    let action = match p["request"].as_str()? {
        "who-req" => "requests the who list",
        "finger-req" => "requests character information",
        _ => return None,
    };
    let user = p["from_user"].as_str().filter(|s| !s.is_empty()).map(plain).unwrap_or_else(|| "SYSTEM".into());
    Some(format!("SYSTEM <intermud> [{}@{} {action}]", user, text(p, "from_mud")))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn router_recovery_survives_a_saturated_event_queue() {
        let (tx,rx)=mpsc::sync_channel(1);
        forward_packet(&tx,json!({"method":"channel_message"})).unwrap();
        forward_packet(&tx,json!({"method":"channel_message"})).unwrap();
        let worker=std::thread::spawn(move ||forward_packet(&tx,json!({"method":"gateway_reconnected"})));
        let Incoming::Packet(first)=rx.recv_timeout(Duration::from_secs(2)).unwrap() else {panic!("packet expected")};
        assert_eq!(first["method"],"channel_message");
        let Incoming::Packet(control)=rx.recv_timeout(Duration::from_secs(2)).unwrap() else {panic!("control expected")};
        assert_eq!(control["method"],"gateway_reconnected");
        worker.join().unwrap().unwrap();
    }
    #[test]
    fn discovery_notice_is_local_sanitized_and_has_no_reply_data() {
        let p=json!({"request":"who-req","from_user":"Crawler","from_mud":"MUIndex","users":["private"]});
        assert_eq!(request_notice(&p).unwrap(), "SYSTEM <intermud> [Crawler@MUIndex requests the who list]");
        assert!(request_notice(&json!({"request":"tell"})).is_none());
        let p=json!({"request":"finger-req","from_user":"bad\n{R","from_mud":"remote\u{001b}"});
        let notice=request_notice(&p).unwrap();
        assert!(!notice.contains('\n')); assert!(!notice.contains('\u{001b}'));
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

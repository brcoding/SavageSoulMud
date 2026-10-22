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
fn header(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_graphic() && c != '|' || c == ' ')
}
fn encode(p: &Packet) -> Option<Vec<u8>> {
    let mut out = String::new();
    for (k, v) in &p.headers {
        if !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || k == "DATA" || !header(v) {
            return None;
        }
        let escaped = if !matches!(k.as_str(), "ID" | "UDP" | "EMOTE")
            && (v.starts_with('$') || v.parse::<i64>().is_ok())
        {
            format!("${v}")
        } else {
            v.clone()
        };
        out.push_str(&format!("{k}:{escaped}|"));
    }
    out.push_str("DATA:");
    if p.data.starts_with('$') || p.data.parse::<i64>().is_ok() {
        out.push('$');
    }
    out.push_str(&p.data);
    (out.len() <= MAX_PACKET).then(|| out.into_bytes())
}
fn decode(bytes: &[u8]) -> Option<Packet> {
    if bytes.len() > MAX_PACKET {
        return None;
    }
    // ASCII is the common interoperable encoding; Latin-1 peers remain legible.
    let s = String::from_utf8(bytes.to_vec())
        .unwrap_or_else(|_| bytes.iter().map(|b| char::from(*b)).collect());
    let mut p = Packet::default();
    let mut rest = s.as_str();
    let unquote = |v: &str| v.strip_prefix('$').unwrap_or(v).to_owned();
    while !rest.is_empty() {
        if let Some(data) = rest.strip_prefix("DATA:") {
            p.data = unquote(data);
            return Some(p);
        }
        let (part, next) = rest.split_once('|').unwrap_or((rest, ""));
        let (k, v) = part.split_once(':')?;
        if k.is_empty()
            || k.len() > 32
            || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || p.headers.len() >= 24
            || p.headers.contains_key(k)
        {
            return None;
        }
        let v = unquote(v);
        if !header(&v) {
            return None;
        }
        p.headers.insert(k.into(), v);
        rest = next;
    }
    Some(p)
}
struct Fragments {
    at: Instant,
    pieces: Vec<Option<Vec<u8>>>,
}
#[derive(Default)]
struct Assembly {
    groups: HashMap<(usize, String), Fragments>,
}
impl Assembly {
    fn receive(&mut self, peer: usize, bytes: &[u8]) -> Option<Packet> {
        self.groups
            .retain(|_, v| v.at.elapsed() < Duration::from_secs(15));
        if !bytes.starts_with(b"PKT:") {
            return decode(bytes);
        }
        let at = bytes.iter().position(|b| *b == b'|')?;
        let head = std::str::from_utf8(bytes.get(4..at)?).ok()?;
        let mut parts = head.rsplitn(3, ':');
        let (pos, total) = parts.next()?.split_once('/')?;
        let pos: usize = pos.parse().ok()?;
        let total: usize = total.parse().ok()?;
        let id = parts.next()?;
        let name = parts.next()?;
        if total == 0 || total > 32 || pos == 0 || pos > total || id.len() > 32 || !header(name) {
            return None;
        }
        let key = (peer, format!("{name}:{id}"));
        if self.groups.len() >= 16 && !self.groups.contains_key(&key) {
            return None;
        }
        let group = self.groups.entry(key.clone()).or_insert_with(|| Fragments {
            at: Instant::now(),
            pieces: vec![None; total],
        });
        if group.pieces.len() != total {
            return None;
        }
        group.pieces[pos - 1] = Some(bytes.get(at + 1..)?.to_vec());
        if group.pieces.iter().flatten().map(Vec::len).sum::<usize>() > MAX_PACKET {
            self.groups.remove(&key);
            return None;
        }
        if !group.pieces.iter().all(Option::is_some) {
            return None;
        }
        let bytes: Vec<u8> = group
            .pieces
            .iter()
            .flatten()
            .flat_map(|v| v.iter().copied())
            .collect();
        self.groups.remove(&key);
        decode(&bytes)
    }
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
    app.add_systems(Startup, start).add_systems(Update, tick);
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
fn worker(config: Config, rx: Receiver<Outgoing>, tx: SyncSender<Incoming>) {
    if !config.relay_host.is_empty() {
        relay_worker(config, rx, tx);
        return;
    }
    let socket = match UdpSocket::bind(&config.bind) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.try_send(Incoming::Failed(e.to_string()));
            return;
        }
    };
    if socket.set_nonblocking(true).is_err() {
        return;
    }
    let peers: Vec<Vec<SocketAddr>> = config
        .peers
        .iter()
        .map(|p| {
            (p.host.as_str(), p.port)
                .to_socket_addrs()
                .map(|v| v.take(8).collect())
                .unwrap_or_default()
        })
        .collect();
    let unresolved = peers.iter().filter(|p| p.is_empty()).count();
    let _=tx.try_send(Incoming::Ready(format!("UDP bound {}; {} configured peers, {unresolved} unresolved. Reachability requires an actual reply; unsolicited inbound traffic requires UDP port forwarding.",config.bind,peers.len())));
    let mut assembly = Assembly::default();
    let mut buffer = [0u8; 65536];
    loop {
        for _ in 0..16 {
            match rx.try_recv() {
                Ok(job) => {
                    if let (Some(address), Some(data)) = (
                        peers.get(job.peer).and_then(|p| p.first()),
                        encode(&job.packet),
                    ) {
                        // Keep datagrams under typical inetd limits. Fragmented packets are reassembled by peers.
                        if data.len() <= 900 {
                            let _ = socket.send_to(&data, address);
                        } else {
                            let chunks: Vec<_> = data.chunks(800).collect();
                            for (i, chunk) in chunks.iter().enumerate() {
                                let mut part = format!(
                                    "PKT:{}:{}:{}/{}|",
                                    config.mud_name,
                                    job.packet.get("ID"),
                                    i + 1,
                                    chunks.len()
                                )
                                .into_bytes();
                                part.extend_from_slice(chunk);
                                let _ = socket.send_to(&part, address);
                            }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => return,
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        for _ in 0..32 {
            let (n, source) = match socket.recv_from(&mut buffer) {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => return,
            };
            let Some(peer) = peers
                .iter()
                .position(|p| p.iter().any(|a| a.ip() == source.ip()))
            else {
                continue;
            };
            let Some(packet) = assembly.receive(peer, &buffer[..n]) else {
                continue;
            };
            if !packet
                .get("NAME")
                .eq_ignore_ascii_case(&config.peers[peer].name)
                || packet.get("UDP").parse::<u16>().ok() != Some(config.peers[peer].port)
            {
                continue;
            }
            let _ = tx.try_send(Incoming::Packet(peer, packet));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
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
/// Root command router calls this only for the zebedee / z2 verbs; authorization repeats here.
pub(crate) fn command(world: &mut World, actor: Entity, arg: &str) {
    if !allowed(world, actor) {
        say(world, actor, "Zebedee commands are wizard-only.");
        return;
    }
    let Some(mut bridge) = world.remove_resource::<Bridge>() else {
        say(world, actor, "Zebedee is disabled. See help zebedee.");
        return;
    };
    let result = command_inner(world, &mut bridge, actor, arg);
    if let Err(e) = result {
        say(world, actor, e);
    }
    world.insert_resource(bridge);
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
            say(world,actor,format!("Zebedee: {} {} pending requests. {} peers have replied this session. UDP does not authenticate identities cryptographically; only configured peers are accepted.",b.status,b.pending.len(),b.last_seen.len()));
            return Ok(());
        }
        "muds" => {
            let rows: Vec<_> = b
                .config
                .peers
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    format!(
                        "{} ({})",
                        plain(&p.name),
                        b.last_seen
                            .get(&i)
                            .map(|t| format!("reply {}s ago", t.elapsed().as_secs()))
                            .unwrap_or_else(|| "unverified".into())
                    )
                })
                .collect();
            say(
                world,
                actor,
                format!(
                    "Configured Zebedee peers: {}",
                    if rows.is_empty() {
                        "none; configure data/zebedee.json".into()
                    } else {
                        rows.join(", ")
                    }
                ),
            );
            return Ok(());
        }
        "channels" => {
            say(
                world,
                actor,
                format!(
                    "Configured channels: {}. Use zebedee listen <channel> or zebedee listen all.",
                    b.config.channels.join(", ")
                ),
            );
            return Ok(());
        }
        "listen" | "unlisten" => {
            let channels = if rest.eq_ignore_ascii_case("all") {
                b.config.channels.clone()
            } else if b
                .config
                .channels
                .iter()
                .any(|c| c.eq_ignore_ascii_case(rest))
            {
                vec![rest.to_ascii_lowercase()]
            } else {
                return Err("Choose a configured channel from zebedee channels.".into());
            };
            let subscribed = b.listeners.entry(actor).or_default();
            for c in channels {
                if verb.eq_ignore_ascii_case("listen") {
                    subscribed.insert(c);
                } else {
                    subscribed.remove(&c);
                }
            }
            say(
                world,
                actor,
                format!(
                    "Zebedee listening: {}. Subscriptions are local and last until logout/restart.",
                    subscribed.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            );
            return Ok(());
        }
        _ => {}
    }
    if b.cooldown
        .get(&actor)
        .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
    {
        return Err("Wait one second between Zebedee network requests.".into());
    }
    let name = world
        .get::<GrimName>(actor)
        .map(|n| n.0.clone())
        .ok_or("Character name unavailable.")?;
    let mut p = base(&b.config, verb);
    p.set("SND", name.to_ascii_lowercase());
    let peer_name=match verb.to_ascii_lowercase().as_str(){
        "ping"|"who"=>rest,
        "mudlist"=>{p.set("REQ","query");p.data="hosts-json".into();rest},
        "query"=>{let(peer,query)=rest.split_once(':').ok_or("Use zebedee query <mud>: <query>.")?;p.data=query.trim().into();peer.trim()},
        "finger"=>{let(user,mud)=rest.split_once('@').ok_or("Use zebedee finger <user>@<mud>.")?;p.data=user.trim().into();mud.trim()},
        "tell"=>{let(target,message)=rest.split_once(':').ok_or("Use zebedee tell <user>@<mud>: <message>.")?;let(user,mud)=target.split_once('@').ok_or("Use zebedee tell <user>@<mud>: <message>.")?;if !header(user.trim())||message.trim().is_empty()||message.len()>1024{return Err("Use a recipient and 1–1024 characters of text.".into());}p.set("RCPNT",user.trim().to_ascii_lowercase());p.data=message.trim().into();mud.trim()},
        "chat"=>{
            let(channel,message)=rest.split_once(' ').ok_or("Use zebedee chat <channel> <message>.")?;
            if !b.config.channels.iter().any(|c|c==channel)||message.trim().is_empty()||message.len()>1024{return Err("Choose a configured channel and 1–1024 characters of text.".into());}
            p.set("REQ","channel");p.set("CHANNEL",channel);p.data=message.trim().into();
            if b.config.peers.is_empty(){return Err("No configured Zebedee peers.".into());}
            for peer in 0..b.config.peers.len(){request(b,actor,peer,p.clone())?;}
            b.cooldown.insert(actor,Instant::now());say(world,actor,"Zebedee channel message queued; UDP delivery is not guaranteed.");return Ok(());
        },
        _=>return Err("Zebedee: status, muds, channels, listen/unlisten <channel|all>, ping <mud>, who <mud>, finger user@mud, mudlist <mud>, query <mud>: <query>, tell user@mud: message, chat channel message. See help zebedee.".into())
    };
    let peer = b
        .config
        .peers
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(peer_name))
        .ok_or("Unknown configured peer. Use zebedee muds; mud names can contain spaces.")?;
    p.set("REQ", p.get("REQ").to_ascii_lowercase());
    request(b, actor, peer, p)?;
    b.cooldown.insert(actor, Instant::now());
    say(world, actor, "Zebedee request queued.");
    Ok(())
}
fn online_wizards(world: &mut World) -> Vec<(Entity, String)> {
    world
        .query::<(Entity, &GrimName, &Character, &Player)>()
        .iter(world)
        .filter(|(_, _, c, _)| c.is_admin())
        .map(|(e, n, _, _)| (e, n.0.clone()))
        .collect()
}
fn tick(world: &mut World) {
    let Some(mut b) = world.remove_resource::<Bridge>() else {
        return;
    };
    let events: Vec<_> =
        b.rx.lock()
            .ok()
            .map(|rx| rx.try_iter().take(32).collect())
            .unwrap_or_default();
    for event in events {
        match event {
            Incoming::Ready(s) => {
                b.ready = true;
                b.status = s;
            }
            Incoming::Failed(s) => {
                b.ready = false;
                b.status = s;
            }
            Incoming::Packet(peer, p) => deliver(world, &mut b, peer, p),
        }
    }
    let expired: Vec<_> = b
        .pending
        .iter()
        .filter(|(_, v)| v.at.elapsed() > Duration::from_secs(5))
        .map(|(k, _)| *k)
        .collect();
    for id in expired {
        let Some(mut pending) = b.pending.remove(&id) else {
            continue;
        };
        // Never resend tell/channel messages, which could otherwise duplicate public communication.
        if pending.retries < 2
            && matches!(
                pending.packet.get("REQ"),
                "ping" | "query" | "who" | "finger"
            )
            && allowed(world, pending.actor)
        {
            let _ = send(&mut b, pending.peer, pending.packet.clone());
            pending.retries += 1;
            pending.at = Instant::now();
            b.pending.insert(id, pending);
        } else if allowed(world, pending.actor) {
            say(world,pending.actor,format!("Zebedee {} to {}: no acknowledgement received (UDP/firewall/NAT or remote service).",plain(pending.packet.get("REQ")),plain(&b.config.peers[pending.peer].name)));
        }
    }
    b.listeners.retain(|actor, _| allowed(world, *actor));
    b.cooldown
        .retain(|actor, t| allowed(world, *actor) && t.elapsed() < Duration::from_secs(60));
    b.seen.retain(|_, t| t.elapsed() < Duration::from_secs(60));
    world.insert_resource(b);
}
fn deliver(world: &mut World, b: &mut Bridge, peer: usize, p: Packet) {
    b.last_seen.insert(peer, Instant::now());
    let req = p.get("REQ");
    if req == "reply" {
        let Ok(id) = p.get("ID").parse::<u64>() else {
            return;
        };
        if !b.pending.get(&id).is_some_and(|v| v.peer == peer) {
            return;
        }
        let Some(pending) = b.pending.remove(&id) else {
            return;
        };
        if allowed(world, pending.actor) {
            say(
                world,
                pending.actor,
                format!(
                    "[Zebedee {}] {}",
                    plain(&b.config.peers[peer].name),
                    plain(&p.data)
                ),
            );
        }
        return;
    }
    if b.seen.len() >= 256 {
        return;
    }
    let key = (peer, p.get("ID").into(), req.into());
    if !p.get("ID").is_empty() && b.seen.insert(key, Instant::now()).is_some() {
        return;
    }
    let mut response = base(&b.config, "reply");
    response.set("ID", p.get("ID"));
    response.set("RCPNT", p.get("SND"));
    let wizards = online_wizards(world);
    let peer_name = plain(&b.config.peers[peer].name);
    response.data = match req {
        "ping" => format!("{} is alive.", b.config.mud_name),
        "who" => format!(
            "Online wizards: {}",
            wizards
                .iter()
                .map(|(_, n)| n.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        "finger" => {
            if let Some((_, name)) = wizards
                .iter()
                .find(|(_, n)| n.eq_ignore_ascii_case(p.data.trim()))
            {
                format!("{name}: wizard, online.")
            } else {
                "No public information available.".into()
            }
        }
        "query" => {
            response.set("QUERY", &p.data);
            match p.data.trim() {
                "commands" => "ping:query:who:finger:tell:channel".into(),
                "list" => "commands:list:info:version:encoding:ansi:hosts-json:users".into(),
                "info" => format!(
                    "{}; GRIM; development MUD; wizard-only intermud",
                    b.config.mud_name
                ),
                "version" => "GRIM Savage Soul".into(),
                "encoding" => "ascii".into(),
                "ansi" => "no".into(),
                "hosts-json" => "[]".into(),
                "users" => wizards
                    .iter()
                    .map(|(_, n)| n.clone())
                    .collect::<Vec<_>>()
                    .join(":"),
                _ => "Unsupported query.".into(),
            }
        }
        "tell" => {
            let targets: Vec<_> = wizards
                .iter()
                .filter(|(_, n)| n.eq_ignore_ascii_case(p.get("RCPNT")))
                .map(|(e, _)| *e)
                .collect();
            for target in &targets {
                say(
                    world,
                    *target,
                    format!(
                        "[Zebedee] {}@{} tells you: {}",
                        plain(p.get("SND")),
                        peer_name,
                        plain(&p.data)
                    ),
                );
            }
            if targets.is_empty() {
                "Recipient unavailable.".into()
            } else {
                "Message delivered.".into()
            }
        }
        "channel" => {
            let channel = p.get("CHANNEL");
            if !b.config.channels.iter().any(|c| c == channel) {
                return;
            }
            if !matches!(p.get("CMD"), "" | "emote") {
                response.data = "Channel history/list is not offered.".into();
                let _ = send(b, peer, response);
                return;
            }
            let targets: Vec<_> = wizards
                .iter()
                .filter(|(e, _)| {
                    b.listeners
                        .get(e)
                        .is_some_and(|channels| channels.contains(channel))
                })
                .map(|(e, _)| *e)
                .collect();
            for target in targets {
                say(
                    world,
                    target,
                    format!(
                        "[Zebedee:{}] {}@{}{} {}",
                        plain(channel),
                        plain(p.get("SND")),
                        peer_name,
                        if p.get("CMD") == "emote" { "" } else { ":" },
                        plain(&p.data)
                    ),
                );
            }
            "Channel message received.".into()
        }
        _ => return,
    };
    if !p.get("ID").is_empty() && !p.get("SND").is_empty() {
        let _ = send(b, peer, response);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
        let mut config = Config::default();
        config.relay_host = "127.0.0.1".into();
        config.relay_port = port;
        config.relay_key_file = keyfile.to_string_lossy().into();
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
fn relay_worker(config: Config, rx: Receiver<Outgoing>, tx: SyncSender<Incoming>) {
    let key = match std::fs::read_to_string(&config.relay_key_file) {
        Ok(k) if k.trim().len() == 64 => k.trim().to_owned(),
        _ => {
            let _ = tx.try_send(Incoming::Failed(
                "Windows UDP relay key unavailable or invalid.".into(),
            ));
            return;
        }
    };
    let peers: Vec<Vec<SocketAddr>> = config
        .peers
        .iter()
        .map(|p| {
            (p.host.as_str(), p.port)
                .to_socket_addrs()
                .map(|v| v.take(8).collect())
                .unwrap_or_default()
        })
        .collect();
    let address: SocketAddr = match format!("{}:{}", config.relay_host, config.relay_port).parse() {
        Ok(a) => a,
        Err(_) => {
            let _ = tx.try_send(Incoming::Failed(
                "relay_host must be a literal IP address".into(),
            ));
            return;
        }
    };
    loop {
        let connected = TcpStream::connect_timeout(&address, Duration::from_secs(2));
        if let Ok(mut stream) = connected {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(30)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
            let auth = serde_json::json!({"key":key}).to_string() + "\n";
            if stream.write_all(auth.as_bytes()).is_ok() {
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 8192];
                let mut ready = false;
                let started = Instant::now();
                let mut assembly = Assembly::default();
                'connected: loop {
                    if !ready && started.elapsed() > Duration::from_secs(3) {
                        break;
                    }
                    if ready {
                        for _ in 0..16 {
                            match rx.try_recv() {
                                Ok(job) => {
                                    let Some(peer) = config.peers.get(job.peer) else {
                                        continue;
                                    };
                                    let Some(data) = encode(&job.packet) else {
                                        continue;
                                    };
                                    let chunks: Vec<Vec<u8>> = if data.len() <= 900 {
                                        vec![data]
                                    } else {
                                        let count = data.len().div_ceil(800);
                                        data.chunks(800)
                                            .enumerate()
                                            .map(|(i, chunk)| {
                                                let mut out = format!(
                                                    "PKT:{}:{}:{}/{}|",
                                                    config.mud_name,
                                                    job.packet.get("ID"),
                                                    i + 1,
                                                    count
                                                )
                                                .into_bytes();
                                                out.extend_from_slice(chunk);
                                                out
                                            })
                                            .collect()
                                    };
                                    for chunk in chunks {
                                        let wire =
                                            serde_json::json!({"peer":peer.name,"data":chunk})
                                                .to_string()
                                                + "\n";
                                        if stream.write_all(wire.as_bytes()).is_err() {
                                            break 'connected;
                                        }
                                    }
                                }
                                Err(mpsc::TryRecvError::Disconnected) => return,
                                Err(mpsc::TryRecvError::Empty) => break,
                            }
                        }
                    }
                    match stream.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => bytes.extend_from_slice(&buffer[..n]),
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                            ) =>
                        {
                            continue
                        }
                        Err(_) => break,
                    }
                    if bytes.len() > 65536 {
                        break;
                    }
                    while let Some(end) = bytes.iter().position(|b| *b == b'\n') {
                        let line: Vec<u8> = bytes.drain(..=end).collect();
                        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&line) else {
                            break 'connected;
                        };
                        if value["ready"] == true {
                            ready = true;
                            let _=tx.try_send(Incoming::Ready(format!("Windows UDP relay connected; {} configured peers. Reachability requires peer replies.",config.peers.len())));
                            continue;
                        }
                        if !ready {
                            break 'connected;
                        }
                        let Some(ip) = value["source_ip"]
                            .as_str()
                            .and_then(|s| s.parse::<std::net::IpAddr>().ok())
                        else {
                            continue;
                        };
                        let Some(peer) = peers
                            .iter()
                            .position(|list| list.iter().any(|a| a.ip() == ip))
                        else {
                            continue;
                        };
                        let Ok(data) = serde_json::from_value::<Vec<u8>>(value["data"].clone())
                        else {
                            continue;
                        };
                        if data.len() > 12000 {
                            continue;
                        }
                        let Some(p) = assembly.receive(peer, &data) else {
                            continue;
                        };
                        if p.get("NAME").eq_ignore_ascii_case(&config.peers[peer].name)
                            && p.get("UDP").parse::<u16>().ok() == Some(config.peers[peer].port)
                        {
                            let _ = tx.try_send(Incoming::Packet(peer, p));
                        }
                    }
                }
            }
        }
        let _ = tx.try_send(Incoming::Failed(
            "Windows UDP relay disconnected; retrying. Messages are never replayed.".into(),
        ));
        for _ in 0..20 {
            // Drop unsent traffic on a broken connection; never replay public communications.
            loop {
                match rx.try_recv() {
                    Ok(_) => {}
                    Err(mpsc::TryRecvError::Disconnected) => return,
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

//! transport implementation, split from the parent subsystem.

use super::*;
mod session;
use session::{publish_presence, receive_packet, send_jobs};

pub(super) fn credentials(path: &str) -> Result<Credentials, &'static str> {
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

pub(super) fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .read_buffer_size(8192)
        .write_buffer_size(0)
        .max_write_buffer_size(128 * 1024)
        .max_message_size(Some(MAX_PACKET))
        .max_frame_size(Some(MAX_PACKET))
}

pub(super) fn auth(c: &Credentials, _snapshot: &Arc<Mutex<Snapshot>>) -> Value {
    // Subscribe with correlated refs after authentication; auth success alone does not confirm channels.
    let channels: Vec<String> = Vec::new();
    json!({"event":"authenticate","payload":{"client_id":c.client_id,"client_secret":c.client_secret,"supports":["channels","players","tells","games"],"channels":channels,"version":"2.3.0","user_agent":"The Savage Soul / Grim"}})
}

pub(super) fn set_timeout(stream: &mut MaybeTlsStream<TcpStream>, read: Duration) {
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

pub(super) fn connect() -> Result<WebSocket<MaybeTlsStream<TcpStream>>, &'static str> {
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

pub(super) fn emit(events: &SyncSender<Incoming>, ready: bool, generation: u64, text: &str) {
    let _ = events.try_send(Incoming::State {
        ready,
        generation,
        text: text.into(),
    });
}

pub(super) fn worker(
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

pub(super) fn run_connection<S: Read + Write>(
    socket: &mut WebSocket<S>,
    requests: &Receiver<Job>,
    events: &SyncSender<Incoming>,
    snapshot: &Arc<Mutex<Snapshot>>,
    reload: &Arc<AtomicBool>,
    generation: u64,
) -> Outcome {
    let mut session = WireSession {
        authenticated: false,
        last_heartbeat: Instant::now(),
        restart: None,
        published: HashSet::new(),
        bootstrap: HashMap::new(),
    };
    let started = Instant::now();
    let result = loop {
        if reload.load(Ordering::Relaxed) {
            break "Grapevine reloading private credentials.";
        }
        if !session.authenticated && started.elapsed() > Duration::from_secs(10) {
            break "Grapevine authentication timed out; check private credentials.";
        }
        if session.authenticated && session.last_heartbeat.elapsed() > Duration::from_secs(180) {
            break "Grapevine heartbeat timed out; reconnecting.";
        }
        if session.authenticated {
            if let Err(status) = publish_presence(socket, snapshot, &mut session) {
                break status;
            }
            if let Err(outcome) =
                send_jobs(socket, requests, events, snapshot, generation, &session)
            {
                return outcome;
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let packet: Value = match serde_json::from_str(text.as_str()) {
                    Ok(p) => p,
                    Err(_) => break "Grapevine sent invalid JSON; reconnecting.",
                };
                if let Err(status) =
                    receive_packet(socket, packet, events, snapshot, generation, &mut session)
                {
                    break status;
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
        authenticated: session.authenticated,
        retry_secs: session.restart,
        status: result,
    }
}

struct WireSession {
    authenticated: bool,
    last_heartbeat: Instant,
    restart: Option<u64>,
    published: HashSet<String>,
    bootstrap: HashMap<String, String>,
}

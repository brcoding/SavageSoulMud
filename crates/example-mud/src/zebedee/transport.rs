//! transport implementation, split from the parent subsystem.

use super::*;

pub(super) fn worker(config: Config, rx: Receiver<Outgoing>, tx: SyncSender<Incoming>) {
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
                            let fragment_id = fragment_id(&job.packet);
                            let chunks: Vec<_> = data.chunks(800).collect();
                            for (i, chunk) in chunks.iter().enumerate() {
                                let mut part = format!(
                                    "PKT:{}:{}:{}/{}|",
                                    config.mud_name,
                                    fragment_id,
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

pub(super) fn relay_worker(config: Config, rx: Receiver<Outgoing>, tx: SyncSender<Incoming>) {
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
                        match relay_send(&mut stream, &config, &rx) {
                            Ok(()) => {}
                            Err(RelayFailure::Stopped) => return,
                            Err(RelayFailure::Disconnected) => break 'connected,
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
                    if !relay_receive(&mut bytes, &config, &peers, &mut assembly, &mut ready, &tx) {
                        break 'connected;
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

enum RelayFailure {
    Stopped,
    Disconnected,
}
fn relay_send(
    stream: &mut TcpStream,
    config: &Config,
    rx: &Receiver<Outgoing>,
) -> Result<(), RelayFailure> {
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
                    let fragment_id = fragment_id(&job.packet);
                    let count = data.len().div_ceil(800);
                    data.chunks(800)
                        .enumerate()
                        .map(|(i, chunk)| {
                            let mut out = format!(
                                "PKT:{}:{}:{}/{}|",
                                config.mud_name,
                                fragment_id,
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
                        serde_json::json!({"peer":peer.name,"data":chunk}).to_string() + "\n";
                    if stream.write_all(wire.as_bytes()).is_err() {
                        return Err(RelayFailure::Disconnected);
                    }
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => return Err(RelayFailure::Stopped),
            Err(mpsc::TryRecvError::Empty) => break,
        }
    }
    Ok(())
}

fn relay_receive(
    bytes: &mut Vec<u8>,
    config: &Config,
    peers: &[Vec<SocketAddr>],
    assembly: &mut Assembly,
    ready: &mut bool,
    tx: &SyncSender<Incoming>,
) -> bool {
    while let Some(end) = bytes.iter().position(|b| *b == b'\n') {
        let line: Vec<u8> = bytes.drain(..=end).collect();
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&line) else {
            return false;
        };
        if value["ready"] == true {
            *ready = true;
            let _=tx.try_send(Incoming::Ready(format!("Windows UDP relay connected; {} configured peers. Reachability requires peer replies.",config.peers.len())));
            continue;
        }
        if !*ready {
            return false;
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
        let Ok(data) = serde_json::from_value::<Vec<u8>>(value["data"].clone()) else {
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
    true
}

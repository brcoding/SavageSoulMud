//! transport implementation, split from the parent subsystem.

use super::*;

pub(super) fn send(
    stream: &mut TcpStream,
    id: u64,
    method: &str,
    params: Value,
) -> std::io::Result<()> {
    let mut bytes =
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
    bytes.push(b'\n');
    stream.write_all(&bytes)
}

pub(super) fn worker(config: Config, requests: Receiver<Request>, events: SyncSender<Incoming>) {
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
                                forward_packet(&events, packet)?;
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
            if let Err(mpsc::TryRecvError::Disconnected) = requests.try_recv() {
                return;
            }
        }
    }
}

//! connection implementation, split from the parent subsystem.

use super::*;

/// Split `socket` into read/write tasks and register it under `conn_id`. Fresh
/// accepts negotiate optional client protocols immediately; adopted copyover
/// sockets reset and renegotiate them without touching password echo.
///
/// The read task enforces `limits` at framing, before any byte reaches Bevy:
/// lines past `max_line_len` are truncated (remainder discarded to the
/// newline, so a long line never splits into two commands); a line past
/// `max_buffer` without a newline, or more than `max_lines` inside
/// `rate_window_secs`, drops the connection with a [`GuardTrip`] reason.
pub(crate) fn register_connection(
    conn_id: usize,
    socket: TcpStream,
    to_bevy_tx: &std_mpsc::Sender<NetworkEvent>,
    conns: &Arc<Mutex<HashMap<usize, Conn>>>,
    handshake: bool,
    limits: TelnetLimits,
) {
    let raw_fd = socket.as_raw_fd();
    let (read_half, mut write_half) = tokio::io::split(socket);
    let (write_tx, mut write_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);

    let mut protocol = ProtocolState::default();
    let negotiation = protocol.handshake(!handshake);
    let protocol_write_tx = write_tx.clone();
    let read_handle = tokio::spawn(read_connection(
        conn_id,
        read_half,
        protocol_write_tx,
        to_bevy_tx.clone(),
        conns.clone(),
        protocol,
        limits,
    ));
    let write_handle = tokio::spawn(async move {
        // Negotiate immediately, even while the client hasn't sent a newline.
        // Adopted sockets reset optional protocols but leave ECHO untouched.
        if write_half.write_all(&negotiation).await.is_err() {
            return;
        }
        while let Some(data) = write_rx.recv().await {
            if write_half.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    conns.lock().unwrap().insert(
        conn_id,
        Conn {
            write_tx,
            read_handle,
            write_handle,
            raw_fd,
        },
    );
}

/// Own input framing separately from protocol negotiation and socket lifetime.
struct InputLines {
    line: Vec<u8>,
    bytes: usize,
    truncated: bool,
    times: VecDeque<tokio::time::Instant>,
    line_cap: usize,
    buffer_cap: usize,
    window: std::time::Duration,
    limits: TelnetLimits,
}
impl InputLines {
    fn new(limits: TelnetLimits) -> Self {
        Self {
            line: Vec::new(),
            bytes: 0,
            truncated: false,
            times: VecDeque::new(),
            line_cap: limits.max_line_len.max(1),
            buffer_cap: limits.buffer_cap(),
            window: std::time::Duration::from_secs(limits.rate_window_secs.max(1)),
            limits,
        }
    }
    fn emit(
        &mut self,
        conn_id: usize,
        sender: &std_mpsc::Sender<NetworkEvent>,
    ) -> Result<(), GuardTrip> {
        let text = frame_input(&self.line);
        let _ = sender.send(NetworkEvent::Input { conn_id, text });
        if rate_tripped(&mut self.times, self.window, self.limits.max_lines) {
            return Err(GuardTrip::RateExceeded {
                lines: self.limits.max_lines,
                window_secs: self.limits.rate_window_secs,
            });
        }
        Ok(())
    }
    fn accept(
        &mut self,
        byte: u8,
        conn_id: usize,
        sender: &std_mpsc::Sender<NetworkEvent>,
    ) -> Result<(), GuardTrip> {
        self.bytes = self.bytes.saturating_add(1);
        if byte == b'\n' {
            if !self.truncated {
                self.emit(conn_id, sender)?;
            }
            self.line.clear();
            self.bytes = 0;
            self.truncated = false;
        } else {
            if self.bytes > self.buffer_cap {
                return Err(GuardTrip::BufferExceeded {
                    bytes: self.buffer_cap,
                });
            }
            if !self.truncated {
                self.line.push(byte);
                if self.line.len() == self.line_cap {
                    self.emit(conn_id, sender)?;
                    self.truncated = true;
                    self.line.clear();
                }
            }
        }
        Ok(())
    }
    fn finish(&self, conn_id: usize, sender: &std_mpsc::Sender<NetworkEvent>) {
        if !self.truncated && !self.line.is_empty() {
            let text = frame_input(&self.line);
            let _ = sender.send(NetworkEvent::Input { conn_id, text });
        }
    }
}

async fn read_connection(
    conn_id: usize,
    read_half: tokio::io::ReadHalf<TcpStream>,
    protocol_write_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    to_bevy_tx: std_mpsc::Sender<NetworkEvent>,
    conns: Arc<Mutex<HashMap<usize, Conn>>>,
    mut protocol: ProtocolState,
    limits: TelnetLimits,
) {
    let mut protocol_times = VecDeque::new();
    let mut reason = None;
    let mut reader = read_half;
    let mut decoder = iac::Decoder::default();
    let mut lines = InputLines::new(limits);
    let mut bytes = [0u8; 4096];
    'socket: loop {
        let n = match reader.read(&mut bytes).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let frames = match decoder.feed(&bytes[..n]) {
            Ok(frames) => frames,
            Err(_) => {
                reason = Some(GuardTrip::BufferExceeded {
                    bytes: iac::MAX_SUBNEGOTIATION,
                });
                break;
            }
        };
        for frame in frames {
            if let iac::Frame::Data(byte) = frame {
                if let Err(trip) = lines.accept(byte, conn_id, &to_bevy_tx) {
                    reason = Some(trip);
                    break 'socket;
                }
            } else {
                // Bound out-of-band traffic separately; normal client
                // negotiation must not consume the player's line quota.
                if rate_tripped(&mut protocol_times, std::time::Duration::from_secs(1), 128) {
                    reason = Some(GuardTrip::RateExceeded {
                        lines: 128,
                        window_secs: 1,
                    });
                    break 'socket;
                }
                let before = protocol.capabilities.clone();
                let (reply, input) = protocol.accept(frame);
                if !reply.is_empty() && protocol_write_tx.try_send(reply).is_err() {
                    break 'socket;
                }
                if input.is_some() || before != protocol.capabilities {
                    let _ = to_bevy_tx.send(NetworkEvent::Protocol {
                        conn_id,
                        capabilities: protocol.capabilities.clone(),
                        input,
                    });
                }
            }
        }
    }
    if reason.is_none() {
        lines.finish(conn_id, &to_bevy_tx);
    }
    let _ = to_bevy_tx.send(NetworkEvent::Disconnected { conn_id, reason });
    conns.lock().unwrap().remove(&conn_id);
}

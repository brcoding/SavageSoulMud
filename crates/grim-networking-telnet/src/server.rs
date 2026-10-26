//! The TCP accept/serve loop and the tokio runtime thread that owns it. Startup
//! spawns a detached thread which either adopts an inherited listener + sockets
//! from a copyover predecessor or binds fresh, signals systemd readiness, then
//! runs the accept/command `select!` loop until the app exits.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::log::{error, info};
use bevy::prelude::*;
use grim_networking::{HandoverEntry, HandoverManifest};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpSocket, TcpStream};

use crate::bridge::{
    register_connection, Conn, CopyoverConn, NetworkBridge, NetworkCommand, NetworkEvent,
    TelnetPort,
};
use crate::copyover::{perform_handoff, receive_handoff, CopyoverDone, Handoff, COPYOVER_SOCK_ENV};
use crate::limits::TelnetLimits;
use crate::shed::{gate_accept, AcceptOutcome, ShedState};
mod proxy;
#[cfg(test)]
use proxy::parse_proxy_header;
use proxy::{bind_proxy_listener, read_proxy_header};

/// Shared per-serve state threaded through the accept loop: the next connection
/// id, the Bevy-bound event sender, and the live connection registry.
struct ServeState {
    next_id: AtomicUsize,
    to_bevy_tx: std_mpsc::Sender<NetworkEvent>,
    conns: Arc<Mutex<HashMap<usize, Conn>>>,
    limits: TelnetLimits,
    shed: Mutex<ShedState>,
}

/// The normal accept path: announce to Bevy and register the socket.
fn admit_connection(state: &ServeState, socket: TcpStream, addr: SocketAddr) {
    let conn_id = state.next_id.fetch_add(1, Ordering::Relaxed);
    let _ = state
        .to_bevy_tx
        .send(NetworkEvent::Connected { conn_id, addr });
    register_connection(
        conn_id,
        socket,
        &state.to_bevy_tx,
        &state.conns,
        true,
        state.limits.clone(),
    );
}

// ─── Startup: spawn the tokio network thread ────────────────────────

pub(crate) fn start_telnet_server(
    port: Res<TelnetPort>,
    limits: Res<TelnetLimits>,
    done: Res<CopyoverDone>,
    mut commands: Commands,
) {
    let port = port.0;
    let limits = limits.clone();

    let (to_bevy_tx, from_network) = std_mpsc::channel::<NetworkEvent>();
    let (to_network, to_tokio_rx) = tokio::sync::mpsc::channel::<NetworkCommand>(64);

    commands.insert_resource(NetworkBridge {
        to_network,
        from_network: Arc::new(Mutex::new(from_network)),
    });

    let copyover_done = done.0.clone();

    std::thread::spawn(move || {
        network_thread(port, limits, to_bevy_tx, to_tokio_rx, copyover_done)
    });
}

/// The detached network thread: receive any handoff (blocking std I/O, before
/// the runtime touches the fds), then build the tokio runtime and serve.
fn network_thread(
    port: u16,
    limits: TelnetLimits,
    to_bevy_tx: std_mpsc::Sender<NetworkEvent>,
    to_tokio_rx: tokio::sync::mpsc::Receiver<NetworkCommand>,
    copyover_done: Arc<AtomicBool>,
) {
    // Receive any handoff *before* building the runtime — blocking std I/O, and
    // the raw fds must be adopted before tokio touches them.
    let handoff = std::env::var(COPYOVER_SOCK_ENV)
        .ok()
        .map(|path| receive_handoff(&path));

    tokio::runtime::Runtime::new().unwrap().block_on(serve(
        port,
        limits,
        to_bevy_tx,
        to_tokio_rx,
        copyover_done,
        handoff,
    ));
}

/// Adopt the inherited listener + sockets (copyover) or bind fresh, signal
/// readiness, resume any carried connections, then run the accept loop.
async fn serve(
    port: u16,
    limits: TelnetLimits,
    to_bevy_tx: std_mpsc::Sender<NetworkEvent>,
    mut to_tokio_rx: tokio::sync::mpsc::Receiver<NetworkCommand>,
    copyover_done: Arc<AtomicBool>,
    handoff: Option<std::io::Result<Handoff>>,
) {
    let state = ServeState {
        next_id: AtomicUsize::new(1),
        to_bevy_tx,
        conns: Arc::new(Mutex::new(HashMap::new())),
        limits,
        shed: Mutex::new(ShedState::default()),
    };

    let (listener, resumed, ack) = adopt_or_bind(handoff, port).await;
    let listener_fd = listener.as_raw_fd();
    let proxy_listener = bind_proxy_listener();

    resume_connections(&state, resumed);
    signal_ready(ack);
    run_accept_loop(
        &state,
        listener,
        listener_fd,
        proxy_listener,
        &mut to_tokio_rx,
        copyover_done,
    )
    .await;
}

/// Adopt an inherited listener from a copyover handoff, or bind fresh. Returns
/// the listener, the sockets to resume, and the ack channel (copyover only).
async fn adopt_or_bind(
    handoff: Option<std::io::Result<Handoff>>,
    port: u16,
) -> (TcpListener, Vec<(RawFd, HandoverEntry)>, Option<UnixStream>) {
    match handoff {
        Some(Ok(h)) => {
            let listener = match TcpListener::from_std(h.listener) {
                Ok(l) => l,
                Err(e) => {
                    // Can't serve without the inherited listener — a detached
                    // network thread quietly dying would leave a listener-less
                    // zombie MUD, so fail the process.
                    error!("failed to adopt inherited listener: {e}");
                    std::process::exit(1);
                }
            };
            info!(
                "Telnet server resumed via copyover with {} connection(s)",
                h.conns.len()
            );
            (listener, h.conns, Some(h.ack))
        }
        Some(Err(e)) => {
            error!("copyover receive failed: {e}; binding fresh on {port}");
            (bind_fresh(port).await, Vec::new(), None)
        }
        None => (bind_fresh(port).await, Vec::new(), None),
    }
}

/// Bind the telnet listener fresh (the non-copyover path).
async fn bind_fresh(port: u16) -> TcpListener {
    let host = std::env::var("GRIM_BIND_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    match TcpListener::bind((host.as_str(), port)).await {
        Ok(listener) => {
            info!("Telnet server listening on {host}:{port}");
            listener
        }
        Err(e) => {
            // The listener lives on a detached thread, so a panic here would be
            // swallowed and leave the MUD running with no transport. Exit the
            // whole process instead (systemd `Restart=on-failure` handles the
            // real box; locally you get a clean, obvious crash).
            error!("failed to bind telnet listener on {host}:{port}: {e}");
            std::process::exit(1);
        }
    }
}

/// Re-adopt each socket carried across a copyover into a live connection and ask
/// the session layer to resume its character (no login, no banner).
fn resume_connections(state: &ServeState, resumed: Vec<(RawFd, HandoverEntry)>) {
    for (fd, entry) in resumed {
        // SAFETY: `fd` was just delivered to us via SCM_RIGHTS and is not owned
        // by anything else in this process.
        let std_stream = unsafe { std::net::TcpStream::from_raw_fd(fd) };
        if std_stream.set_nonblocking(true).is_err() {
            continue;
        }
        let addr = entry.peer_addr.unwrap_or_else(|| {
            std_stream
                .peer_addr()
                .unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap())
        });
        let Ok(socket) = TcpStream::from_std(std_stream) else {
            continue;
        };
        let conn_id = state.next_id.fetch_add(1, Ordering::Relaxed);
        register_connection(
            conn_id,
            socket,
            &state.to_bevy_tx,
            &state.conns,
            false,
            state.limits.clone(),
        );
        let _ = state.to_bevy_tx.send(NetworkEvent::Resumed {
            conn_id,
            addr,
            character: entry.character,
            echo_hidden: entry.echo_hidden,
        });
    }
}

/// Signal readiness to systemd. The unit is `Type=notify`, so even a fresh start
/// must send READY=1 or systemd times the service out. We also send our own
/// MAINPID; on a copyover the predecessor authoritatively re-sends MAINPID=<us>
/// before it exits (see `perform_handoff`), so supervision transfers regardless
/// of which notify systemd processes first. No-op outside systemd. Then ack the
/// predecessor (copyover only) so it can exit now that we are serving.
fn signal_ready(ack: Option<UnixStream>) {
    let _ = sd_notify::notify(&[
        sd_notify::NotifyState::MainPid(std::process::id()),
        sd_notify::NotifyState::Ready,
    ]);
    if let Some(mut ack) = ack {
        use std::io::Write;
        let _ = ack.write_all(&[1u8]);
    }
}

/// The accept/command `select!` loop. `accepting` gates new connections; it is
/// turned off for the brief window of a copyover handoff.
async fn run_accept_loop(
    state: &ServeState,
    listener: TcpListener,
    listener_fd: RawFd,
    proxy_listener: Option<TcpListener>,
    to_tokio_rx: &mut tokio::sync::mpsc::Receiver<NetworkCommand>,
    copyover_done: Arc<AtomicBool>,
) {
    let mut accepting = true;
    let proxy_slots = Arc::new(tokio::sync::Semaphore::new(64));
    let (proxy_tx, mut proxy_rx) = tokio::sync::mpsc::channel::<(TcpStream, SocketAddr)>(32);

    loop {
        tokio::select! {
            Ok((mut socket, peer)) = async {
                match &proxy_listener {
                    Some(listener) => listener.accept().await,
                    None => std::future::pending().await,
                }
            }, if accepting => {
                // Only this dedicated loopback listener is trusted. A bounded
                // handshake runs separately so slow headers cannot stall I/O.
                if !peer.ip().is_loopback() { continue; }
                let Ok(slot) = proxy_slots.clone().try_acquire_owned() else { continue; };
                let tx = proxy_tx.clone();
                tokio::spawn(async move {
                    let _slot = slot;
                    if let Some(addr) = read_proxy_header(&mut socket).await {
                        let _ = tx.try_send((socket, addr));
                    }
                });
            }
            Some((socket, addr)) = proxy_rx.recv(), if accepting => {
                match gate_accept(&state.limits, &state.shed) {
                    AcceptOutcome::Admit => admit_connection(state, socket, addr),
                    AcceptOutcome::AdmitWith(ev) => {
                        let _ = state.to_bevy_tx.send(ev);
                        admit_connection(state, socket, addr);
                    }
                    AcceptOutcome::Drop => {},
                    AcceptOutcome::DropWith(ev) => { let _ = state.to_bevy_tx.send(ev); }
                }
            }
            Ok((socket, addr)) = listener.accept(), if accepting => {
                match gate_accept(&state.limits, &state.shed) {
                    AcceptOutcome::Admit => admit_connection(state, socket, addr),
                    AcceptOutcome::AdmitWith(ev) => {
                        let _ = state.to_bevy_tx.send(ev);
                        admit_connection(state, socket, addr);
                    }
                    AcceptOutcome::Drop => {}
                    AcceptOutcome::DropWith(ev) => {
                        let _ = state.to_bevy_tx.send(ev);
                    }
                }
            }
            Some(cmd) = to_tokio_rx.recv() => {
                match cmd {
                    NetworkCommand::Send { conn_id, text } if !text.is_empty() => {
                        let guard = state.conns.lock().unwrap();
                        if let Some(tx) = guard.get(&conn_id) {
                            let _ = tx.write_tx.try_send(text.into_bytes());
                        }
                    }
                    NetworkCommand::SendRaw { conn_id, data } if !data.is_empty() => {
                        let guard = state.conns.lock().unwrap();
                        if let Some(tx) = guard.get(&conn_id) {
                            let _ = tx.write_tx.try_send(data);
                        }
                    }
                    NetworkCommand::Send { .. } | NetworkCommand::SendRaw { .. } => {}
                    NetworkCommand::Disconnect { conn_id } => {
                        if let Some(conn) = state.conns.lock().unwrap().remove(&conn_id) {
                            // Stop input immediately, but drain queued output before
                            // EOF so rejection/quit messages actually reach the client.
                            conn.read_handle.abort();
                            drop(conn.write_tx);
                            tokio::spawn(async move {
                                let mut writer = conn.write_handle;
                                if tokio::time::timeout(Duration::from_secs(2), &mut writer)
                                    .await
                                    .is_err()
                                {
                                    writer.abort();
                                }
                            });
                        }
                        // The aborted read task can no longer report EOF, so echo
                        // the close: without this the session layer never learns
                        // a server-initiated sever happened (idle sweep, ban kick,
                        // takeover) and the Client leaks (issue #141).
                        let _ = state.to_bevy_tx.send(NetworkEvent::Disconnected {
                            conn_id,
                            reason: None,
                        });
                    }
                    NetworkCommand::Copyover { conns: list } => {
                        // Stop accepting for the handoff window; resume only if it fails.
                        accepting =
                            handle_copyover(&list, &state.conns, listener_fd, &copyover_done).await;
                    }
                }
            }
            // All senders dropped (the app is exiting, e.g. right after a
            // copyover handoff) and we've stopped accepting — end the loop
            // cleanly instead of panicking on an all-branches-disabled select.
            else => break,
        }
    }
}

/// Join each requested connection with its raw fd, build the ordered manifest
/// (listener fd first, then one fd per entry), and notify each carried client.
fn collect_handoff(
    list: &[CopyoverConn],
    conns: &Arc<Mutex<HashMap<usize, Conn>>>,
    listener_fd: RawFd,
) -> (Vec<RawFd>, Vec<HandoverEntry>) {
    let mut fds = vec![listener_fd];
    let mut entries = Vec::new();
    let guard = conns.lock().unwrap();
    for c in list {
        if let Some(conn) = guard.get(&c.conn_id) {
            fds.push(conn.raw_fd);
            entries.push(HandoverEntry {
                character: c.character.clone(),
                echo_hidden: c.echo_hidden,
                peer_addr: c.peer_addr,
            });
            let _ = conn
                .write_tx
                .try_send(b"\r\n[SERVER] Reloading, hold on...\r\n".to_vec());
        }
    }
    (fds, entries)
}

/// Run a copyover handoff on a blocking task and report the resulting `accepting`
/// state: `false` on success (the app is about to exit), `true` on failure
/// (resume serving).
async fn handle_copyover(
    list: &[CopyoverConn],
    conns: &Arc<Mutex<HashMap<usize, Conn>>>,
    listener_fd: RawFd,
    copyover_done: &Arc<AtomicBool>,
) -> bool {
    let (fds, entries) = collect_handoff(list, conns, listener_fd);
    // Let the notices flush to the sockets before we hand the fds to the successor.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let manifest = HandoverManifest { entries };
    match tokio::task::spawn_blocking(move || perform_handoff(&manifest, &fds)).await {
        Ok(Ok(())) => {
            info!("copyover handoff complete; exiting");
            copyover_done.store(true, Ordering::SeqCst);
            false
        }
        Ok(Err(e)) => {
            error!("copyover handoff failed: {e}; resuming service");
            true
        }
        Err(e) => {
            error!("copyover task panicked: {e}; resuming service");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Instant;

    #[test]
    fn proxy_header_accepts_valid_v4_and_v6_only() {
        assert_eq!(
            parse_proxy_header(b"PROXY TCP4 192.0.2.9 127.0.0.1 32123 4001\r\n"),
            Some("192.0.2.9:32123".parse().unwrap())
        );
        assert_eq!(
            parse_proxy_header(b"PROXY TCP6 2001:db8::1 ::1 32123 4001\r\n"),
            Some("[2001:db8::1]:32123".parse().unwrap())
        );
        for invalid in [
            "PROXY UNKNOWN\r\n",
            "PROXY TCP4 ::1 ::1 10 4001\r\n",
            "PROXY TCP4 127.0.0.1 127.0.0.1 0 4001\r\n",
            "PROXY TCP4 127.0.0.1 127.0.0.1 100 70000\r\n",
            "PROXY TCP4 127.0.0.1 127.0.0.1 100 4001\n",
            "PROXY  TCP4 127.0.0.1 127.0.0.1 100 4001\r\n",
        ] {
            assert!(
                parse_proxy_header(invalid.as_bytes()).is_none(),
                "{invalid:?}"
            );
        }
        assert!(parse_proxy_header(&[b'x'; 109]).is_none());
    }

    #[tokio::test]
    async fn proxy_header_does_not_consume_telnet_bytes() {
        use tokio::io::AsyncWriteExt;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let write = tokio::spawn(async move {
            let mut client = TcpStream::connect(address).await.unwrap();
            client
                .write_all(b"PROXY TCP4 192.0.2.9 127.0.0.1 1234 4001\r\nlook\n")
                .await
                .unwrap();
        });
        let (mut socket, _) = listener.accept().await.unwrap();
        assert_eq!(
            read_proxy_header(&mut socket).await,
            Some("192.0.2.9:1234".parse().unwrap())
        );
        let mut command = [0; 5];
        socket.read_exact(&mut command).await.unwrap();
        assert_eq!(&command, b"look\n");
        write.await.unwrap();
    }

    fn test_state(limits: TelnetLimits) -> ServeState {
        let (tx, _rx) = std_mpsc::channel();
        ServeState {
            next_id: AtomicUsize::new(1),
            to_bevy_tx: tx,
            conns: Arc::new(Mutex::new(HashMap::new())),
            limits,
            shed: Mutex::new(ShedState::default()),
        }
    }

    fn tight_limits() -> TelnetLimits {
        TelnetLimits {
            max_connects_total: 3,
            total_window_secs: 60,
            shed_secs: 3600,
            ..TelnetLimits::default()
        }
    }

    #[test]
    fn rapid_accepts_trip_shed_then_drop() {
        let state = test_state(tight_limits());
        for _ in 0..3 {
            assert!(matches!(
                gate_accept(&state.limits, &state.shed),
                AcceptOutcome::Admit
            ));
        }
        // The tripping connection arrived before the shed: admitted, with
        // the entry event.
        assert!(matches!(
            gate_accept(&state.limits, &state.shed),
            AcceptOutcome::AdmitWith(NetworkEvent::Shed {
                active: true,
                refused: 0,
                ..
            })
        ));
        // Shedding: drops, heartbeat-suppressed (60 s interval).
        assert!(matches!(
            gate_accept(&state.limits, &state.shed),
            AcceptOutcome::Drop
        ));
        let shed = state.shed.lock().unwrap();
        assert!(shed.shed_until.is_some());
        assert_eq!(shed.refused, 1);
    }

    #[test]
    fn expired_shed_lifts_on_next_accept() {
        let state = test_state(TelnetLimits::default());
        // Pre-arm an already-expired shed: `until == now` never satisfies
        // `now < until`, so the next accept lifts deterministically.
        state.shed.lock().unwrap().shed_until = Some(Instant::now());
        state.shed.lock().unwrap().refused = 7;
        assert!(matches!(
            gate_accept(&state.limits, &state.shed),
            AcceptOutcome::AdmitWith(_)
        ));
        let shed = state.shed.lock().unwrap();
        assert!(shed.shed_until.is_none());
        assert_eq!(shed.refused, 0);
        assert_eq!(shed.accepts.len(), 1, "the lifting accept is counted");
    }

    #[test]
    fn zero_windows_floor_instead_of_failing_open() {
        let state = test_state(TelnetLimits {
            max_connects_total: 2,
            total_window_secs: 0,
            shed_secs: 0,
            ..TelnetLimits::default()
        });
        assert!(matches!(
            gate_accept(&state.limits, &state.shed),
            AcceptOutcome::Admit
        ));
        assert!(matches!(
            gate_accept(&state.limits, &state.shed),
            AcceptOutcome::Admit
        ));
        // Floored 1 s windows: the third trips and the shed holds.
        assert!(matches!(
            gate_accept(&state.limits, &state.shed),
            AcceptOutcome::AdmitWith(_)
        ));
        assert!(matches!(
            gate_accept(&state.limits, &state.shed),
            AcceptOutcome::Drop
        ));
    }
}

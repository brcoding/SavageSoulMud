//! Copyover (hot restart): swap the server binary without disconnecting players.
//! `SIGUSR2` flips a flag; the predecessor hands its live listener + in-game
//! client sockets to a freshly-spawned successor over a unix socket
//! (`SCM_RIGHTS`, via the `sendfd` crate), waits for the successor to ack, then
//! exits. This module owns the fd framing, the process-level handoff, the signal
//! plumbing, and the two Bevy systems that snapshot sessions and finish the exit.

use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bevy::log::{error, info, warn};
use bevy::prelude::*;
use grim_actor::{Character, Linkdead, Player};
use grim_core::components::{Client, ClientState, Name as GrimName};
use grim_core::events::CopyoverDue;
use grim_networking::{Connection, HandoverEntry, HandoverManifest};
use sendfd::{RecvWithFd, SendWithFd};

use crate::bridge::{CopyoverConn, NetworkBridge, NetworkCommand};

/// Env var carrying the unix-socket path a copyover successor uses to receive
/// the live listener + client sockets from its predecessor. Its presence on
/// startup means "you are the successor: adopt fds instead of binding fresh".
pub(crate) const COPYOVER_SOCK_ENV: &str = "GRIM_COPYOVER_SOCK";

/// Set by the `SIGUSR2` handler; drained by `poll_copyover_signal` on the next
/// tick (a signal handler can safely do little more than flip a flag).
#[derive(Resource, Clone)]
pub(crate) struct CopyoverSignal(pub(crate) Arc<AtomicBool>);

impl Default for CopyoverSignal {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}

/// Flipped by the tokio thread once the successor has acknowledged the handoff;
/// `finish_copyover` then exits this (predecessor) process cleanly.
#[derive(Resource, Clone)]
pub(crate) struct CopyoverDone(pub(crate) Arc<AtomicBool>);

impl Default for CopyoverDone {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}

/// Live sockets received from a predecessor during a copyover.
pub(crate) struct Handoff {
    /// The inherited listener, already set non-blocking.
    pub(crate) listener: std::net::TcpListener,
    /// Client sockets paired with the character to resume on each.
    pub(crate) conns: Vec<(RawFd, HandoverEntry)>,
    /// The handoff channel, kept open so we can acknowledge once we're serving.
    pub(crate) ack: UnixStream,
}

fn invalid_data(e: impl std::error::Error + Send + Sync + 'static) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e)
}

/// Path to exec for the copyover successor. Copyover exists to swap *to a new
/// binary*, so by the time it runs the old binary file has been replaced (the
/// deploy `mv`s it; a local `cargo build` relinks it). On Linux that unlinks the
/// running image's inode, and `current_exe()` then reports the path with a
/// trailing " (deleted)" — which does not exist, so spawning it fails. Strip that
/// marker to get the path now holding the *new* binary, which is exactly what we
/// want to exec.
fn current_exe_path() -> std::io::Result<std::path::PathBuf> {
    let exe = std::env::current_exe()?;
    if let Some(s) = exe.to_str() {
        if let Some(stripped) = s.strip_suffix(" (deleted)") {
            return Ok(std::path::PathBuf::from(stripped));
        }
    }
    Ok(exe)
}

/// Serialize `manifest` and send it together with `fds` (listener first) over a
/// unix socket via SCM_RIGHTS. The framing half of the predecessor handoff, split
/// out so it can be unit-tested without spawning a process.
pub(crate) fn write_handoff(
    stream: &UnixStream,
    manifest: &HandoverManifest,
    fds: &[RawFd],
) -> std::io::Result<()> {
    let json = serde_json::to_vec(manifest).map_err(invalid_data)?;
    stream.send_with_fd(&json, fds)?;
    Ok(())
}

/// Receive the manifest + fds sent by [`write_handoff`]. Returns the manifest,
/// the listener fd (`fds[0]`), and the client fds (`fds[1..]`, aligned with
/// `manifest.entries`). The framing half of the successor handoff.
pub(crate) fn read_handoff(
    stream: &UnixStream,
) -> std::io::Result<(HandoverManifest, RawFd, Vec<RawFd>)> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut fds = [0 as RawFd; 512];
    let (n, fd_count) = stream.recv_with_fd(&mut buf, &mut fds)?;
    if fd_count == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "copyover handoff carried no listener fd",
        ));
    }
    // fds delivered via SCM_RIGHTS arrive WITHOUT close-on-exec. Set it now, or
    // they'd be inherited by *this* process's own future copyover successor
    // (fork+exec) as leaked duplicates — a second fd on the same socket that
    // never closes, so the client never sees EOF across a chained copyover.
    for fd in &fds[..fd_count] {
        let borrowed = unsafe { std::os::fd::BorrowedFd::borrow_raw(*fd) };
        let _ = rustix::io::fcntl_setfd(borrowed, rustix::io::FdFlags::CLOEXEC);
    }
    let manifest: HandoverManifest = serde_json::from_slice(&buf[..n]).map_err(invalid_data)?;
    let listener_fd = fds[0];
    let conn_fds = fds[1..fd_count].to_vec();
    Ok((manifest, listener_fd, conn_fds))
}

/// Predecessor side of a copyover: spawn the successor, hand it the listener +
/// client fds and the manifest over a unix socket, and wait for its ack.
pub(crate) fn perform_handoff(manifest: &HandoverManifest, fds: &[RawFd]) -> std::io::Result<()> {
    use std::io::Read;
    let sock_path = std::env::temp_dir().join(format!("grim-copyover-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock_path);
    let listener = UnixListener::bind(&sock_path)?;

    let exe = current_exe_path()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.env(COPYOVER_SOCK_ENV, &sock_path);
    let mut child = cmd.spawn()?;

    // Everything after spawn can fail; on any failure the predecessor resumes
    // serving, so the half-started successor must not linger (dropping `Child`
    // does not kill it) holding dup'd sockets or the port.
    let outcome = (|| -> std::io::Result<()> {
        let (mut stream, _addr) = listener.accept()?;
        write_handoff(&stream, manifest, fds)?;

        // Wait for the successor to confirm it is serving before we let the
        // process exit — this keeps the fds (and the systemd MainPID handoff)
        // valid until the new instance has taken over.
        let mut ack = [0u8; 1];
        stream.read_exact(&mut ack)?;

        // Hand the systemd MainPID to the successor *from the current main
        // process* (us). systemd trusts a MAINPID change from the tracked main
        // most readily, and doing it here — before we exit — queues the
        // reassignment ahead of our exit rather than racing the successor's own
        // notify. The successor also sends MAINPID+READY; this is the
        // authoritative belt-and-suspenders. No-op outside systemd.
        let _ = sd_notify::notify(&[sd_notify::NotifyState::MainPid(child.id())]);
        Ok(())
    })();

    let _ = std::fs::remove_file(&sock_path);

    if let Err(e) = outcome {
        error!("copyover handoff failed ({e}); terminating half-started successor");
        let _ = child.kill();
        let _ = child.wait();
        return Err(e);
    }
    Ok(())
}

/// Successor side of a copyover: connect to the predecessor and receive the
/// listener + client fds plus the manifest. Blocking; run before the runtime.
pub(crate) fn receive_handoff(path: &str) -> std::io::Result<Handoff> {
    let stream = UnixStream::connect(path)?;
    let (manifest, listener_fd, conn_fds) = read_handoff(&stream)?;

    // SAFETY: each fd was just delivered via SCM_RIGHTS and is owned by no one
    // else in this process.
    let listener = unsafe { std::net::TcpListener::from_raw_fd(listener_fd) };
    listener.set_nonblocking(true)?;

    // fds[1..] pair with manifest entries in order; any entry without a matching
    // fd is dropped rather than mis-adopted.
    let conns = manifest
        .entries
        .into_iter()
        .enumerate()
        .filter_map(|(i, entry)| conn_fds.get(i).map(|&fd| (fd, entry)))
        .collect();

    Ok(Handoff {
        listener,
        conns,
        ack: stream,
    })
}

// ─── Systems ────────────────────────────────────────────────────────

/// Wire `SIGUSR2` to the copyover flag. Failure is logged, not fatal.
pub(crate) fn install_copyover_signal(signal: Res<CopyoverSignal>) {
    match signal_hook::flag::register(signal_hook::consts::SIGUSR2, signal.0.clone()) {
        Ok(_) => info!("SIGUSR2 will trigger a copyover (hot restart)"),
        Err(e) => warn!("failed to register SIGUSR2 handler: {e}"),
    }
}
/// An in-game `copyover` countdown has expired: flip the handoff flag so
/// `poll_copyover_signal` snapshots sessions on the next tick, exactly as if
/// `SIGUSR2` had fired. The countdown, warnings, and admin gate all ran
/// upstream; this only bridges the engine message to the latched flag.
pub(crate) fn trigger_copyover_on_due(
    mut due: MessageReader<CopyoverDue>,
    signal: Res<CopyoverSignal>,
) {
    let mut fire = false;
    for _ in due.read() {
        fire = true;
    }
    if fire {
        info!("copyover countdown expired: handing off to a successor");
        signal.0.store(true, Ordering::SeqCst);
    }
}

/// If `SIGUSR2` fired, snapshot the in-game connections and ask the tokio thread
/// to hand them to a successor process. Runs once — `started` latches so a
/// second signal mid-handoff is ignored.
#[allow(clippy::too_many_arguments)]
pub(crate) fn poll_copyover_signal(
    signal: Res<CopyoverSignal>,
    bridge: Res<NetworkBridge>,
    clients: Query<&Client>,
    characters: Query<&Character>,
    names: Query<&GrimName>,
    players: Query<&Player>,
    linkdead: Query<&Linkdead>,
    connections: Query<&Connection>,
    mut started: Local<bool>,
) {
    if !signal.0.swap(false, Ordering::SeqCst) {
        return;
    }
    if *started {
        return;
    }
    // Only actively-playing sessions carry across: in-game state, a bound
    // character, and not linkdead. Anyone at the login prompt or linkdead is
    // dropped and reconnects fresh.
    let mut list = Vec::new();
    for client in clients.iter() {
        if client.state != ClientState::InGame {
            continue;
        }
        let Some(char_entity) = client.character else {
            continue;
        };
        if linkdead.get(char_entity).is_ok() {
            continue;
        }
        // A bound, in-game character is a PC: confirm the `Character` being and
        // read its display name from the `Name` component (post-split, the name
        // no longer lives on `Character`).
        if characters.get(char_entity).is_err() {
            continue;
        }
        let Ok(name) = names.get(char_entity) else {
            continue;
        };
        // Takeover guard: after a takeover the old `Client` lingers until its
        // `ConnectionClosed` is processed, but the character's live `Player`
        // already points at the NEW connection. Only the session that actually
        // owns the character (its connection == the live `Player.connection`)
        // may carry across — otherwise a copyover in that window would hand off
        // BOTH connections for one character.
        let Ok(player) = players.get(char_entity) else {
            continue;
        };
        if player.connection != client.connection {
            continue;
        }
        let Ok(conn) = connections.get(client.connection) else {
            continue;
        };
        list.push(CopyoverConn {
            peer_addr: Some(conn.addr),
            conn_id: conn.id,
            character: name.0.clone(),
            echo_hidden: conn.echo_hidden,
        });
    }
    info!(
        "copyover requested: handing off {} connection(s)",
        list.len()
    );
    match bridge
        .to_network
        .try_send(NetworkCommand::Copyover { conns: list })
    {
        Ok(()) => *started = true,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            // Bounded bridge momentarily full: keep the flag raised so the
            // next tick retries instead of dropping the handoff. Don't latch
            // `started` until the request is actually queued.
            warn!("copyover handoff deferred: bridge full, retrying");
            signal.0.store(true, Ordering::SeqCst);
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            // Network thread gone: the handoff can never proceed. Stay up
            // rather than strand players on a false promise, and say so
            // loudly. Latch `started` — retrying a closed bridge is futile.
            error!("copyover handoff failed: network bridge closed");
            *started = true;
        }
    }
}

/// Once the tokio thread reports the successor has taken over, exit cleanly so
/// the predecessor process goes away.
pub(crate) fn finish_copyover(done: Res<CopyoverDone>, mut exit: MessageWriter<AppExit>) {
    if done.0.load(Ordering::SeqCst) {
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    /// Takeover/copyover regression: after a takeover the old `Client` lingers
    /// (its `ConnectionClosed` not yet processed) while the character's live
    /// `Player` already points at the NEW connection. A copyover firing in that
    /// window must hand off ONLY the new connection, not both.
    #[test]
    fn copyover_manifest_skips_taken_over_old_connection() {
        use crate::bridge::NetworkEvent;
        use grim_actor::{Character, Player};
        use grim_core::components::{Client, ClientState, Name as GrimName};
        use grim_core::GrimId;
        use grim_networking::Connection;
        use std::sync::Mutex;

        fn conn(id: usize) -> Connection {
            Connection {
                id,
                addr: ([127, 0, 0, 1], 40000 + id as u16).into(),
                echo_hidden: false,
            }
        }
        fn ingame(connection: Entity, character: Entity) -> Client {
            Client {
                state: ClientState::InGame,
                character: Some(character),
                ..Client::new(connection)
            }
        }

        let mut app = App::new();
        let signal = CopyoverSignal::default();
        signal.0.store(true, Ordering::SeqCst);
        app.insert_resource(signal);

        let (to_net_tx, mut to_net_rx) = tokio::sync::mpsc::channel::<NetworkCommand>(16);
        let (_from_tx, from_rx) = std::sync::mpsc::channel::<NetworkEvent>();
        app.insert_resource(NetworkBridge {
            to_network: to_net_tx,
            from_network: std::sync::Arc::new(Mutex::new(from_rx)),
        });
        app.add_systems(Update, poll_copyover_signal);

        let conn_old = app.world_mut().spawn(conn(1)).id();
        let conn_new = app.world_mut().spawn(conn(2)).id();
        // The character's live Player points at the NEW connection (post-takeover).
        let character = app
            .world_mut()
            .spawn((
                GrimName("Hero".into()),
                Character {
                    credits: 100,
                    id: GrimId::new(),
                    account_id: GrimId::new(),
                    created_at: chrono::Utc::now(),
                    last_room: None,
                    roles: Vec::new(),
                    class: String::new(),
                    title: None,
                    restrings: std::collections::HashMap::new(),
                    config: std::collections::HashMap::new(),
                    game_state: Default::default(),
                },
                Player {
                    connection: conn_new,
                },
            ))
            .id();
        // Old lingering session + new owning session, both bound to the character.
        app.world_mut().spawn(ingame(conn_old, character));
        app.world_mut().spawn(ingame(conn_new, character));

        app.update();

        let cmd = to_net_rx.try_recv().expect("a Copyover command was sent");
        let NetworkCommand::Copyover { conns } = cmd else {
            panic!("expected NetworkCommand::Copyover");
        };
        assert_eq!(conns.len(), 1, "only the owning connection carries across");
        assert_eq!(
            conns[0].conn_id, 2,
            "the new (owning) connection, not the old"
        );
    }

    /// The copyover framing round-trips the manifest and real fds over a unix
    /// socket in-process (no child spawn). Covers the `write_handoff`/`read_handoff`
    /// SCM_RIGHTS + serde path that the process-level integration test can't
    /// reliably measure (its successor is SIGKILLed).
    #[test]
    fn handoff_round_trips_manifest_and_fds() {
        use std::os::unix::net::UnixStream as StdUnix;

        let (tx, rx) = StdUnix::pair().unwrap();
        // Two throwaway sockets whose fds stand in for "listener" + one client.
        let (a, _a2) = StdUnix::pair().unwrap();
        let (b, _b2) = StdUnix::pair().unwrap();

        let manifest = HandoverManifest {
            entries: vec![HandoverEntry {
                character: "Alice".into(),
                echo_hidden: false,
                peer_addr: Some("127.0.0.1:4000".parse().unwrap()),
            }],
        };
        write_handoff(&tx, &manifest, &[a.as_raw_fd(), b.as_raw_fd()]).unwrap();

        let (got, listener_fd, conn_fds) = read_handoff(&rx).unwrap();
        assert_eq!(got.entries.len(), 1);
        assert_eq!(got.entries[0].character, "Alice");
        assert!(listener_fd >= 0, "listener fd delivered");
        assert_eq!(conn_fds.len(), 1, "one client fd, aligned with the entry");
        assert!(conn_fds[0] >= 0);

        // The received fds are dups owned by us now — wrap + drop to close them.
        let _l = unsafe { StdUnix::from_raw_fd(listener_fd) };
        let _c = unsafe { StdUnix::from_raw_fd(conn_fds[0]) };
    }

    /// A handoff carrying no fds is rejected rather than silently adopting a bad
    /// listener.
    #[test]
    fn read_handoff_rejects_empty_fd_set() {
        use std::os::unix::net::UnixStream as StdUnix;
        let (tx, rx) = StdUnix::pair().unwrap();
        write_handoff(&tx, &HandoverManifest::default(), &[]).unwrap();
        assert!(read_handoff(&rx).is_err(), "no listener fd → error");
    }

    /// An expired in-game `copyover` countdown raises the handoff flag, exactly
    /// like `SIGUSR2` — the next `poll_copyover_signal` performs the handoff.
    #[test]
    fn copyover_due_fires_handoff_flag() {
        let mut app = App::new();
        app.insert_resource(CopyoverSignal::default());
        app.add_message::<CopyoverDue>();
        app.add_systems(Update, trigger_copyover_on_due);
        app.world_mut().write_message(CopyoverDue);
        app.update();

        assert!(
            app.world()
                .resource::<CopyoverSignal>()
                .0
                .load(Ordering::SeqCst),
            "CopyoverDue must latch the handoff flag"
        );
    }

    /// Silence upstream means silence here: no message, no flag.
    #[test]
    fn no_due_leaves_flag_down() {
        let mut app = App::new();
        app.insert_resource(CopyoverSignal::default());
        app.add_message::<CopyoverDue>();
        app.add_systems(Update, trigger_copyover_on_due);
        app.update();

        assert!(
            !app.world()
                .resource::<CopyoverSignal>()
                .0
                .load(Ordering::SeqCst),
            "no CopyoverDue must leave the flag down"
        );
    }

    /// A full bridge must not drop the handoff: the flag stays raised and a
    /// later tick retries with the session list intact.
    #[test]
    fn full_bridge_rearms_and_retries_handoff() {
        use crate::bridge::NetworkEvent;
        use grim_core::components::{Client, ClientState, Name as GrimName};
        use grim_core::GrimId;
        use grim_networking::Connection;
        use std::sync::Mutex;

        let mut app = App::new();
        let signal = CopyoverSignal::default();
        signal.0.store(true, Ordering::SeqCst);
        app.insert_resource(signal);
        // Capacity 1, prefilled: the handoff send fails Full.
        let (to_net_tx, mut to_net_rx) = tokio::sync::mpsc::channel::<NetworkCommand>(1);
        to_net_tx
            .try_send(NetworkCommand::Copyover { conns: Vec::new() })
            .unwrap();
        let (_from_tx, from_rx) = std::sync::mpsc::channel::<NetworkEvent>();
        app.insert_resource(NetworkBridge {
            to_network: to_net_tx,
            from_network: std::sync::Arc::new(Mutex::new(from_rx)),
        });
        app.add_systems(Update, poll_copyover_signal);

        // One in-game session so the retried request has a manifest to carry.
        let conn = app
            .world_mut()
            .spawn(Connection {
                id: 7,
                addr: ([127, 0, 0, 1], 40007).into(),
                echo_hidden: false,
            })
            .id();
        let character = app
            .world_mut()
            .spawn((
                GrimName("Hero".into()),
                Character {
                    credits: 100,
                    id: GrimId::new(),
                    account_id: GrimId::new(),
                    created_at: chrono::Utc::now(),
                    last_room: None,
                    roles: Vec::new(),
                    class: String::new(),
                    title: None,
                    restrings: std::collections::HashMap::new(),
                    config: std::collections::HashMap::new(),
                    game_state: Default::default(),
                },
                Player { connection: conn },
            ))
            .id();
        app.world_mut().spawn(Client {
            state: ClientState::InGame,
            character: Some(character),
            ..Client::new(conn)
        });

        app.update();

        // Request preserved, not latched: flag re-armed, prefill untouched.
        assert!(
            app.world()
                .resource::<CopyoverSignal>()
                .0
                .load(Ordering::SeqCst),
            "Full bridge must re-arm the flag for a retry"
        );
        assert!(
            matches!(to_net_rx.try_recv(), Ok(NetworkCommand::Copyover { .. })),
            "only the prefill is queued; the handoff was not sent"
        );

        // Capacity freed: the retry queues the handoff with its session.
        app.update();
        let cmd = to_net_rx.try_recv().expect("handoff queued after retry");
        let NetworkCommand::Copyover { conns } = cmd else {
            panic!("expected NetworkCommand::Copyover");
        };
        assert_eq!(conns.len(), 1, "retried handoff carries the session");
        assert_eq!(conns[0].conn_id, 7);
    }

    /// A closed bridge can never hand off: latch `started` (no futile
    /// retry loop) and stay up. Observed via the flag: re-raising it is
    /// consumed by the early return instead of re-armed by a retry.
    #[test]
    fn closed_bridge_latches_without_retry() {
        use crate::bridge::NetworkEvent;
        use std::sync::Mutex;

        let mut app = App::new();
        let signal = CopyoverSignal::default();
        signal.0.store(true, Ordering::SeqCst);
        app.insert_resource(signal);
        let (to_net_tx, to_net_rx) = tokio::sync::mpsc::channel::<NetworkCommand>(16);
        drop(to_net_rx);
        let (_from_tx, from_rx) = std::sync::mpsc::channel::<NetworkEvent>();
        app.insert_resource(NetworkBridge {
            to_network: to_net_tx,
            from_network: std::sync::Arc::new(Mutex::new(from_rx)),
        });
        app.add_systems(Update, poll_copyover_signal);
        app.update();

        // Re-raise: a latched poll consumes the flag without re-arming.
        app.world()
            .resource::<CopyoverSignal>()
            .0
            .store(true, Ordering::SeqCst);
        app.update();
        assert!(
            !app.world()
                .resource::<CopyoverSignal>()
                .0
                .load(Ordering::SeqCst),
            "closed bridge must latch, not re-arm the flag"
        );
    }
}

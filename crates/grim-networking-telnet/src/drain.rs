//! Drain network events into Bevy: the `drain_network_events` system plus
//! one handler per event variant. Split out of `bridge.rs` under the module
//! line cap; `bridge` keeps the registry, the socket tasks, and the
//! Bevy-to-network direction.

use std::net::SocketAddr;

use bevy::log::info;
use bevy::prelude::*;
use grim_networking::{
    admin_log, Connection, ConnectionClosed, ConnectionEstablished, ConnectionInput,
    ConnectionResumed, WiznetAlert, WiznetCategory,
};

use crate::bridge::{NetworkBridge, NetworkCommand, NetworkEvent};
use crate::guard::GuardTrip;
use crate::iac;
use crate::protocols::TelnetInput;

/// Spawn the `Connection` entity for a fresh accept and announce it.
fn handle_connected(
    commands: &mut Commands,
    established: &mut MessageWriter<ConnectionEstablished>,
    alerts: &mut MessageWriter<WiznetAlert>,
    conn_id: usize,
    addr: SocketAddr,
) {
    info!("Connection from {} (id={})", addr, conn_id);
    let entity = commands
        .spawn(Connection {
            id: conn_id,
            addr,
            echo_hidden: false,
        })
        .id();
    established.write(ConnectionEstablished {
        connection: entity,
        addr,
    });
    admin_log!(alerts, WiznetCategory::Logins, "new connection from {addr}");
}

/// Spawn the `Connection` entity for a copyover re-adoption and ask the
/// session layer to resume it.
#[allow(clippy::too_many_arguments)]
fn handle_resumed(
    commands: &mut Commands,
    resumed: &mut MessageWriter<ConnectionResumed>,
    alerts: &mut MessageWriter<WiznetAlert>,
    conn_id: usize,
    addr: SocketAddr,
    character: String,
    echo_hidden: bool,
) {
    info!(
        "Connection {} resumed as '{}' (copyover)",
        conn_id, character
    );
    let entity = commands
        .spawn(Connection {
            id: conn_id,
            addr,
            echo_hidden,
        })
        .id();
    resumed.write(ConnectionResumed {
        connection: entity,
        character: character.clone(),
        addr,
    });
    admin_log!(
        alerts,
        WiznetCategory::Logins,
        "connection resumed as '{character}' from {addr}"
    );
}

/// Filter one input line to printable Unicode and forward it, restoring
/// terminal echo first when password mode hid it.
fn handle_input(
    bridge: &NetworkBridge,
    connections: &mut Query<(Entity, &mut Connection)>,
    input: &mut MessageWriter<ConnectionInput>,
    conn_id: usize,
    text: String,
) {
    if let Some((entity, mut conn)) = connections.iter_mut().find(|(_, c)| c.id == conn_id) {
        // UTF-8 is supported; reject C0/C1 terminal controls in all modes.
        let text: String = text.chars().filter(|&c| !c.is_control()).collect();

        // If echo was hidden (password mode), auto-restore on user input.
        if conn.echo_hidden {
            let _ = bridge.to_network.try_send(NetworkCommand::SendRaw {
                conn_id,
                data: iac::WONT_ECHO.to_vec(), // IAC WONT ECHO → visible
            });
            let _ = bridge.to_network.try_send(NetworkCommand::Send {
                conn_id,
                text: "\n".into(),
            });
            conn.echo_hidden = false;
        }
        input.write(ConnectionInput {
            connection: entity,
            text,
        });
    }
}

/// Tear down one socket: reasoned log line plus alert, then the closed
/// message.
fn handle_disconnected(
    connections: &Query<(Entity, &mut Connection)>,
    closed: &mut MessageWriter<ConnectionClosed>,
    alerts: &mut MessageWriter<WiznetAlert>,
    conn_id: usize,
    reason: Option<GuardTrip>,
) {
    if let Some((entity, conn)) = connections.iter().find(|(_, c)| c.id == conn_id) {
        match reason {
            Some(trip) => {
                info!(
                    "Connection {} ({}) disconnected: input guard tripped ({})",
                    conn_id, conn.addr, trip
                );
                admin_log!(
                    alerts,
                    WiznetCategory::Security,
                    "connection from {} dropped ({})",
                    conn.addr,
                    trip
                );
            }
            None => {
                info!("Connection {} disconnected", conn_id);
                admin_log!(
                    alerts,
                    WiznetCategory::Logins,
                    "connection from {} closed",
                    conn.addr
                );
            }
        }
        closed.write(ConnectionClosed { connection: entity });
        // Despawn is handled by save_on_disconnect in the persistence plugin
    }
}

/// Log a shed transition or heartbeat and alert it.
fn handle_shed(
    alerts: &mut MessageWriter<WiznetAlert>,
    active: bool,
    connects: u32,
    window_secs: u64,
    shed_secs: u64,
    refused: u64,
) {
    if active {
        if refused == 0 {
            info!(
                "Connection flood: {} connects in {}s — shedding new connections for {}s",
                connects, window_secs, shed_secs
            );
            admin_log!(
                alerts,
                WiznetCategory::Security,
                "connection flood: {connects} connects in {window_secs}s; shedding new connections for {shed_secs}s"
            );
        } else {
            info!(
                "Still shedding new connections ({} refused since the trip)",
                refused
            );
            admin_log!(
                alerts,
                WiznetCategory::Security,
                "still shedding new connections ({refused} refused)"
            );
        }
    } else {
        info!(
            "Connection shed lifted after refusing {}; accepting again",
            refused
        );
        admin_log!(
            alerts,
            WiznetCategory::Security,
            "connection shed lifted after refusing {refused}"
        );
    }
}

/// Events drained but not yet handled: `drain_creations` buffers everything
/// that needs a spawned `Connection` (`Input`, `Disconnected`, `Shed`
/// reports) for `drain_dependents`, which runs after the spawns apply.
/// Bounded in practice by one tick's channel backlog; drained every tick.
#[derive(Resource, Default)]
pub(crate) struct PendingDrain(pub Vec<NetworkEvent>);

/// First phase: spawn `Connection` entities for accepts and re-adoptions
/// and announce them; buffer everything else for phase two.
pub(crate) fn drain_creations(
    bridge: Res<NetworkBridge>,
    mut commands: Commands,
    mut established: MessageWriter<ConnectionEstablished>,
    mut resumed: MessageWriter<ConnectionResumed>,
    mut pending: ResMut<PendingDrain>,
    mut alerts: MessageWriter<WiznetAlert>,
) {
    let rx = bridge.from_network.lock().unwrap();
    while let Ok(ev) = rx.try_recv() {
        match ev {
            NetworkEvent::Connected { conn_id, addr } => {
                handle_connected(&mut commands, &mut established, &mut alerts, conn_id, addr);
            }
            NetworkEvent::Resumed {
                conn_id,
                addr,
                character,
                echo_hidden,
            } => {
                handle_resumed(
                    &mut commands,
                    &mut resumed,
                    &mut alerts,
                    conn_id,
                    addr,
                    character,
                    echo_hidden,
                );
            }
            // Dependent events wait for the next phase: the spawns above
            // apply when this system ends, so a same-pass lookup would miss
            // them and drop the first input or close.
            dependent => pending.0.push(dependent),
        }
    }
}

/// Second phase: the buffered dependent events, now that creation spawns
/// are applied and the `connections` query resolves them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_dependents(
    mut commands: Commands,
    bridge: Res<NetworkBridge>,
    mut input: MessageWriter<ConnectionInput>,
    mut protocol_input: MessageWriter<TelnetInput>,
    mut closed: MessageWriter<ConnectionClosed>,
    mut connections: Query<(Entity, &mut Connection)>,
    mut pending: ResMut<PendingDrain>,
    mut alerts: MessageWriter<WiznetAlert>,
) {
    for ev in std::mem::take(&mut pending.0) {
        match ev {
            NetworkEvent::Protocol {
                conn_id,
                capabilities,
                input: message,
            } => {
                if let Some((entity, _)) = connections.iter().find(|(_, c)| c.id == conn_id) {
                    if capabilities.width > 0 {
                        commands
                            .entity(entity)
                            .insert(grim_networking::TerminalSize {
                                columns: capabilities.width.clamp(20, 1000),
                                rows: capabilities.height,
                            });
                    }
                    commands.entity(entity).insert(capabilities);
                    if let Some((option, payload)) = message {
                        protocol_input.write(TelnetInput {
                            connection: entity,
                            option,
                            payload,
                        });
                    }
                }
            }
            NetworkEvent::Input { conn_id, text } => {
                handle_input(&bridge, &mut connections, &mut input, conn_id, text);
            }
            NetworkEvent::Disconnected { conn_id, reason } => {
                handle_disconnected(&connections, &mut closed, &mut alerts, conn_id, reason);
            }
            NetworkEvent::Shed {
                active,
                connects,
                window_secs,
                shed_secs,
                refused,
            } => {
                handle_shed(
                    &mut alerts,
                    active,
                    connects,
                    window_secs,
                    shed_secs,
                    refused,
                );
            }
            // Creations never buffer (handled in phase one); unreachable.
            NetworkEvent::Connected { .. } | NetworkEvent::Resumed { .. } => {}
        }
    }
}

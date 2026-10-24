//! session implementation, split from the parent subsystem.

use super::*;

pub(super) fn publish_presence<S: Read + Write>(
    socket: &mut WebSocket<S>,
    snapshot: &Arc<Mutex<Snapshot>>,
    session: &mut WireSession,
) -> Result<(), &'static str> {
    let current: HashSet<_> = snapshot
        .lock()
        .ok()
        .map(|s| s.actors.values().cloned().collect())
        .unwrap_or_default();
    for (event, names) in [
        (
            "players/sign-out",
            session
                .published
                .difference(&current)
                .cloned()
                .collect::<Vec<_>>(),
        ),
        (
            "players/sign-in",
            current
                .difference(&session.published)
                .cloned()
                .collect::<Vec<_>>(),
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
                return Err("Grapevine presence update failed; reconnecting.");
            }
        }
    }
    session.published = current;
    Ok(())
}

pub(super) fn send_jobs<S: Read + Write>(
    socket: &mut WebSocket<S>,
    requests: &Receiver<Job>,
    events: &SyncSender<Incoming>,
    snapshot: &Arc<Mutex<Snapshot>>,
    generation: u64,
    session: &WireSession,
) -> Result<(), Outcome> {
    for _ in 0..16 {
        let job = match requests.try_recv() {
            Ok(j) => j,
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(Outcome {
                    authenticated: session.authenticated,
                    retry_secs: Some(1),
                    status: "Grapevine stopped.",
                });
            }
            Err(mpsc::TryRecvError::Empty) => break,
        };
        let authorized = snapshot.lock().ok().is_some_and(|s| {
            s.actors
                .get(&job.actor)
                .is_some_and(|name| match job.packet["event"].as_str() {
                    Some("channels/send") => {
                        job.packet["payload"]["name"].as_str() == Some(name.as_str())
                    }
                    Some("tells/send") => {
                        job.packet["payload"]["from_name"].as_str() == Some(name.as_str())
                    }
                    _ => true,
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
            return Err(Outcome {
                authenticated: session.authenticated,
                retry_secs: session.restart,
                status: "Grapevine disconnected; queued messages will not be replayed.",
            });
        }
    }
    Ok(())
}

pub(super) fn receive_packet<S: Read + Write>(
    socket: &mut WebSocket<S>,
    packet: Value,
    events: &SyncSender<Incoming>,
    snapshot: &Arc<Mutex<Snapshot>>,
    generation: u64,
    session: &mut WireSession,
) -> Result<(), &'static str> {
    let event = packet["event"].as_str().unwrap_or("");
    if event == "authenticate" {
        if packet["status"] != "success" {
            return Err("Grapevine authentication rejected; check private credentials.");
        }
        session.authenticated = true;
        session.last_heartbeat = Instant::now();
        emit(events, true, generation, "Grapevine authenticated.");
        let channels: Vec<_> = snapshot
            .lock()
            .ok()
            .map(|s| s.channels.iter().take(32).cloned().collect())
            .unwrap_or_default();
        for channel in channels {
            let reference = uuid::Uuid::new_v4().to_string();
            let packet =
                json!({"event":"channels/subscribe","ref":reference,"payload":{"channel":channel}});
            if socket
                .send(Message::Text(packet.to_string().into()))
                .is_err()
            {
                return Err("Grapevine channel subscription failed; reconnecting.");
            }
            session.bootstrap.insert(reference, channel);
        }
        return Ok(());
    }
    if !session.authenticated {
        return Ok(());
    }
    if event == "channels/subscribe" {
        if let Some(channel) = packet["ref"]
            .as_str()
            .and_then(|r| session.bootstrap.remove(r))
        {
            let _ = events.try_send(Incoming::Subscribed {
                channel,
                success: packet["status"] != "failure",
            });
            return Ok(());
        }
    }
    if event == "heartbeat" {
        session.last_heartbeat = Instant::now();
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
            return Err("Grapevine heartbeat could not be sent; reconnecting.");
        }
        return Ok(());
    }
    if event == "restart" {
        session.restart = Some(
            packet["payload"]["downtime"]
                .as_u64()
                .unwrap_or(15)
                .clamp(1, 300),
        );
        let _ = events.try_send(Incoming::Packet(packet));
        return Ok(());
    }
    if events.try_send(Incoming::Packet(packet)).is_err() {
        return Err("Grapevine receive queue full; reconnecting without replay.");
    }
    Ok(())
}

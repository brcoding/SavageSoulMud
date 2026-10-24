//! delivery implementation, split from the parent subsystem.

use super::*;

pub(super) fn update_presence(world: &mut World, b: &mut Bridge) {
    let actors: HashMap<_, _> = world
        .query::<(Entity, &GrimName, &Character, &Player)>()
        .iter(world)
        .filter(|(_, _, c, _)| c.is_admin() && !c.is_hidden())
        .take(128)
        .map(|(e, n, _, _)| (e, bounded(&n.0, 100)))
        .collect();
    if let Ok(mut s) = b.snapshot.lock() {
        s.actors = actors;
    }
}

pub(super) fn tick(world: &mut World) {
    let Some(mut b) = world.remove_resource::<Bridge>() else {
        return;
    };
    update_presence(world, &mut b);
    let events: Vec<_> =
        b.rx.lock()
            .ok()
            .map(|rx| rx.try_iter().take(32).collect())
            .unwrap_or_default();
    for event in events {
        match event {
            Incoming::State {
                ready,
                generation,
                text,
            } => {
                b.ready = ready;
                b.generation = generation;
                b.status = text;
                if ready {
                    b.confirmed.clear();
                } else {
                    b.confirmed.clear();
                    let pending = std::mem::take(&mut b.pending);
                    for p in pending.into_values() {
                        if allowed(world, p.actor) {
                            tell(world,p.actor,"Grapevine disconnected. Pending requests were discarded; messages will not be replayed.");
                        }
                    }
                }
            }
            Incoming::Packet(packet) => deliver(world, &mut b, packet),
            Incoming::History(mut records) => {
                records.append(&mut b.history);
                while records.len() > 200 {
                    records.pop_front();
                }
                b.history = records;
                mark_history(&mut b);
            }
            Incoming::Subscribed { channel, success } => {
                if success {
                    b.confirmed.insert(channel);
                } else {
                    b.confirmed.remove(&channel);
                }
            }
            Incoming::Dropped(reference) => {
                if let Some(p) = b.pending.remove(&reference) {
                    if allowed(world, p.actor) {
                        tell(world,p.actor,"Grapevine request was discarded because the session changed or the connection ended.");
                    }
                }
            }
        }
    }
    let expired: Vec<_> = b
        .pending
        .iter()
        .filter(|(_, p)| p.at.elapsed() > Duration::from_secs(10))
        .map(|(r, _)| r.clone())
        .collect();
    for reference in expired {
        if let Some(p) = b.pending.remove(&reference) {
            if allowed(world, p.actor) {
                if p.replies == 0 {
                    tell(
                        world,
                        p.actor,
                        format!(
                            "Grapevine {}: no matching response within 10 seconds.",
                            remote(&p.event)
                        ),
                    );
                } else if matches!(p.event.as_str(), "games/status" | "players/status") {
                    tell(
                        world,
                        p.actor,
                        format!("Grapevine query complete: {} response(s).", p.replies),
                    );
                }
            }
        }
    }
    b.listeners.retain(|e, _| allowed(world, *e));
    b.cooldown
        .retain(|e, t| allowed(world, *e) && t.elapsed() < Duration::from_secs(60));
    world.insert_resource(b);
}

pub(super) fn deliver(world: &mut World, b: &mut Bridge, packet: Value) {
    let event = packet["event"].as_str().unwrap_or("");
    let payload = &packet["payload"];
    match event {
        "channels/broadcast" => {
            deliver_broadcast(world, b, payload);
            return;
        }
        "tells/receive" => {
            deliver_tell(world, payload);
            return;
        }
        "restart" => {
            b.status = "Grapevine announced a restart; automatic reconnect is ready.".into();
            return;
        }
        _ => {}
    }
    let Some(reference) = packet["ref"].as_str() else {
        return;
    };
    let Some(mut pending) = b.pending.remove(reference) else {
        return;
    };
    if pending.event != event || !allowed(world, pending.actor) {
        return;
    }
    if packet["status"] == "failure" {
        tell(world,pending.actor,format!("Grapevine {} failed. Check the channel, game, player, or private credentials; remote error details are suppressed.",remote(event)));
        return;
    }
    match event {
        "channels/subscribe" => {
            let channel = pending.filter.clone();
            b.confirmed.insert(channel.clone());
            if let Ok(mut s) = b.snapshot.lock() {
                s.channels.insert(channel.clone());
            }
            b.listeners
                .entry(pending.actor)
                .or_default()
                .insert(channel.clone());
            tell(
                world,
                pending.actor,
                format!("You now listen to Grapevine {}.", remote(&channel)),
            );
        }
        "channels/send" | "tells/send" => {
            if let Some(echo) = pending.echo.take() {
                tell(world, pending.actor, echo);
            }
        }
        "games/status" | "players/status" => {
            let game = payload["game"].as_str().unwrap_or("?");
            if !pending.filter.is_empty() && !game.eq_ignore_ascii_case(&pending.filter) {
                b.pending.insert(reference.into(), pending);
                return;
            }
            pending.replies += 1;
            if pending.replies <= 256 {
                let text = if event == "games/status" {
                    format!(
                        "[Grapevine] {} — {} ({} players)",
                        remote(game),
                        field(payload, "display_name"),
                        payload["players_online_count"]
                            .as_u64()
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "?".into())
                    )
                } else {
                    let names = payload["players"]
                        .as_array()
                        .map(|v| {
                            v.iter()
                                .take(128)
                                .filter_map(Value::as_str)
                                .map(remote)
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    format!(
                        "[Grapevine] {}: {}",
                        remote(game),
                        if names.is_empty() {
                            "no players reported".into()
                        } else {
                            names
                        }
                    )
                };
                tell(world, pending.actor, text);
            }
            b.pending.insert(reference.into(), pending);
        }
        _ => {}
    }
}

fn deliver_broadcast(world: &mut World, b: &mut Bridge, payload: &Value) {
    let Some(channel) = payload["channel"].as_str().filter(|s| valid_channel(s)) else {
        return;
    };
    if !b.confirmed.contains(channel) {
        return;
    }
    let (Some(message), Some(name), Some(game)) = (
        payload["message"].as_str(),
        payload["name"].as_str(),
        payload["game"].as_str(),
    ) else {
        return;
    };
    let record = Activity {
        at: now(),
        channel: channel.into(),
        speaker: bounded(name, 100),
        game: bounded(game, 100),
        message: bounded(message, 1024),
    };
    let text = format!(
        "[Grapevine:{}] {}@{}: {}",
        remote(channel),
        remote(name),
        remote(game),
        remote(message)
    );
    b.history.push_back(record);
    while b.history.len() > 200 {
        b.history.pop_front();
    }
    mark_history(b);
    let targets: Vec<_> = b
        .listeners
        .iter()
        .filter(|(e, c)| allowed(world, **e) && c.contains(channel))
        .map(|(e, _)| *e)
        .collect();
    for actor in targets {
        tell(world, actor, &text);
    }
}

fn deliver_tell(world: &mut World, payload: &Value) {
    let Some(target) = payload["to_name"].as_str() else {
        return;
    };
    let targets: Vec<_> = world
        .query::<(Entity, &GrimName, &Character, &Player)>()
        .iter(world)
        .filter(|(_, n, c, _)| c.is_admin() && n.0.eq_ignore_ascii_case(target))
        .map(|(e, _, _, _)| e)
        .collect();
    for actor in targets {
        tell(
            world,
            actor,
            format!(
                "[Grapevine] {}@{} tells you: {}",
                field(payload, "from_name"),
                field(payload, "from_game"),
                field(payload, "message")
            ),
        );
    }
}

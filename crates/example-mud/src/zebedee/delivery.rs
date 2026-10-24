//! delivery implementation, split from the parent subsystem.

use super::*;

pub(super) fn online_wizards(world: &mut World) -> Vec<(Entity, String)> {
    world
        .query::<(Entity, &GrimName, &Character, &Player)>()
        .iter(world)
        .filter(|(_, _, c, _)| c.is_admin() && !c.is_hidden())
        .map(|(e, n, _, _)| (e, n.0.clone()))
        .collect()
}

pub(super) fn tick(world: &mut World) {
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

pub(super) fn deliver(world: &mut World, b: &mut Bridge, peer: usize, p: Packet) {
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
        "query" => query_response(b, &p, &wizards, &mut response),
        "tell" => deliver_tell(world, &p, &wizards, &peer_name),
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

fn query_response(
    b: &Bridge,
    p: &Packet,
    wizards: &[(Entity, String)],
    response: &mut Packet,
) -> String {
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

fn deliver_tell(
    world: &mut World,
    p: &Packet,
    wizards: &[(Entity, String)],
    peer_name: &str,
) -> String {
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

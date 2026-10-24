//! delivery implementation, split from the parent subsystem.

use super::*;

pub(super) fn tick(world: &mut World, mut cursor: Local<MessageCursor<EngineCommand>>) {
    let inputs: Vec<_> = cursor
        .read(world.resource::<Messages<EngineCommand>>())
        .filter_map(|m| match &m.command {
            Command::SavageSoul { verb, arg }
                if ["intermud", "i3", "imud"].contains(&verb.as_str()) =>
            {
                Some((m.client, verb.clone(), arg.clone()))
            }
            _ => None,
        })
        .collect();
    let Some(mut bridge) = world.remove_resource::<Bridge>() else {
        for (e, _, _) in inputs {
            if allowed(world, e, true) {
                reply(world, e, "Intermud is not enabled. See help intermud.");
            } else {
                reply(world, e, "Intermud requires wizard privileges.");
            }
        }
        return;
    };
    let events: Vec<_> = bridge.rx.lock().unwrap().try_iter().take(128).collect();
    for event in events {
        match event {
            Incoming::Connected => {
                bridge.connected = true;
                rejoin(&mut bridge);
                bridge.last_presence = Instant::now() - Duration::from_secs(60);
            }
            Incoming::Disconnected => {
                bridge.connected = false;
                bridge.pending.clear();
            }
            Incoming::Packet(packet) => deliver(world, &mut bridge, packet),
        }
    }
    for (actor, verb, arg) in inputs {
        handle_input(world, &mut bridge, actor, &verb, arg);
    }
    pump_subscriptions(&mut bridge);
    let users:Vec<_>=world.query::<(&GrimName,&Character,&Player)>().iter(world).filter(|(_,c,_)|c.is_admin()&&!c.is_hidden()).map(|(n,c,_)|json!({"name":n.0,"title":c.title.clone().unwrap_or_default(),"status":"online"})).collect();
    let presence = json!({"users":users});
    if bridge.connected
        && (presence != bridge.presence || bridge.last_presence.elapsed() > Duration::from_secs(20))
        && request(&mut bridge, "", "presence_sync", presence.clone()).is_ok()
    {
        bridge.presence = presence;
        bridge.last_presence = Instant::now();
    }
    bridge
        .pending
        .retain(|_, (_, _, t)| t.elapsed() < Duration::from_secs(30));
    bridge
        .cooldown
        .retain(|_, t| t.elapsed() < Duration::from_secs(60));
    if bridge.activity_dirty && bridge.last_activity_save.elapsed() >= Duration::from_secs(5) {
        if let Err(error) = save_activity(&bridge.config.activity_file, &bridge.activity) {
            warn!("Intermud activity could not be saved: {error}");
        }
        bridge.last_activity_save = Instant::now();
        bridge.activity_dirty = false;
    }
    world.insert_resource(bridge);
}

pub(super) fn recipients(world: &mut World, _wizard: bool, target: Option<&str>) -> Vec<Entity> {
    world
        .query::<(Entity, &GrimName, &Character, &Player)>()
        .iter(world)
        .filter(|(_, n, c, _)| c.is_admin() && target.is_none_or(|t| n.0.eq_ignore_ascii_case(t)))
        .map(|(e, _, _, _)| e)
        .collect()
}

pub(super) fn deliver(world: &mut World, bridge: &mut Bridge, packet: Value) {
    if let Some(id) = packet["id"].as_u64() {
        deliver_response(world, bridge, &packet, id);
        return;
    }
    let kind = packet["method"].as_str().unwrap_or("");
    if kind == "gateway_reconnected" {
        rejoin(bridge);
        bridge.last_presence = Instant::now() - Duration::from_secs(60);
        return;
    }
    let p = &packet["params"];
    if kind == "request_received" {
        deliver_request_notice(world, bridge, p);
        return;
    }
    if matches!(kind, "channel_message" | "channel_emote")
        && !p["channel"].as_str().is_some_and(|channel| {
            bridge
                .config
                .channels
                .iter()
                .any(|c| c.eq_ignore_ascii_case(channel))
        })
    {
        return;
    }
    if record_activity(&mut bridge.activity, kind, p, unix_time()) {
        bridge.activity_dirty = true;
    }
    let directed = matches!(
        kind,
        "tell_received" | "emoteto_received" | "who_reply" | "error_occurred"
    );
    let target = if directed {
        p["to_user"].as_str().filter(|t| !t.is_empty())
    } else {
        None
    };
    if directed && target.is_none() {
        return;
    }
    let output = match kind {
        "tell_received" => format!(
            "[Intermud] {}@{} tells you: {}",
            text(p, "from_user"),
            text(p, "from_mud"),
            text(p, "message")
        ),
        "emoteto_received" => format!(
            "[Intermud] {}@{}: {}",
            text(p, "from_user"),
            text(p, "from_mud"),
            text(p, "message")
        ),
        "channel_message" | "channel_emote" => format!(
            "[I3 {}] {}@{}: {}",
            text(p, "channel"),
            text(p, "from_user"),
            text(p, "from_mud"),
            text(p, "message")
        ),
        "who_reply" => {
            let rows: Vec<_> = p["users"]
                .as_array()
                .into_iter()
                .flatten()
                .take(100)
                .map(|u| {
                    if let Some(row) = u.as_array() {
                        row.iter()
                            .filter_map(Value::as_str)
                            .map(plain)
                            .collect::<Vec<_>>()
                            .join(" ")
                    } else {
                        text(u, "name")
                    }
                })
                .collect();
            format!(
                "[Intermud] Who at {}: {}",
                text(p, "from_mud"),
                rows.join("; ")
            )
        }
        "error_occurred" => format!(
            "[Intermud] {}: {}",
            text(p, "error_code"),
            text(p, "error_message")
        ),
        _ => return,
    };
    for e in recipients(world, bridge.config.wizard_only, target) {
        reply(world, e, output.clone());
    }
}

pub(super) fn request_notice(p: &Value) -> Option<String> {
    let action = match p["request"].as_str()? {
        "who-req" => "requests the who list",
        "finger-req" => "requests character information",
        _ => return None,
    };
    let user = p["from_user"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(plain)
        .unwrap_or_else(|| "SYSTEM".into());
    Some(format!(
        "SYSTEM <intermud> [{}@{} {action}]",
        user,
        text(p, "from_mud")
    ))
}

fn handle_input(world: &mut World, bridge: &mut Bridge, actor: Entity, verb: &str, arg: String) {
    if !allowed(world, actor, bridge.config.wizard_only) {
        reply(
            world,
            actor,
            "Intermud is currently available to wizards only.",
        );
        return;
    }
    let name = world
        .get::<GrimName>(actor)
        .map(|n| n.0.clone())
        .unwrap_or_default();
    if bridge
        .cooldown
        .get(&name)
        .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
    {
        reply(
            world,
            actor,
            "Please wait a moment before another Intermud command.",
        );
        return;
    }
    bridge.cooldown.insert(name.clone(), Instant::now());
    let arg = if verb == "imud" {
        format!("chat {arg}")
    } else {
        arg
    };
    if let Some(output) = activity_command(bridge, &arg) {
        reply(world, actor, output);
        return;
    }
    if let Some(output) = subscription_command(bridge, &arg) {
        reply(world, actor, output);
        return;
    }
    match parse(&name, &arg) {
        Ok((method, params)) => {
            if let Err(e) = request(bridge, &name, &method, params) {
                reply(world, actor, e)
            }
        }
        Err(e) => reply(world, actor, e),
    }
}

fn deliver_response(world: &mut World, bridge: &mut Bridge, packet: &Value, id: u64) {
    let Some((name, method, _)) = bridge.pending.remove(&id) else {
        return;
    };
    let targets = recipients(world, bridge.config.wizard_only, Some(&name));
    let result = &packet["result"];
    if let Some(channel) = result["channel"].as_str().filter(|c| valid_channel(c)) {
        if result["status"] == "joined" && !bridge.confirmed.iter().any(|c| c == channel) {
            bridge.confirmed.push(channel.into());
        }
        if result["status"] == "left" {
            bridge.confirmed.retain(|c| c != channel);
        }
    }
    if packet.get("error").is_some() && matches!(method.as_str(), "channel_join" | "channel_leave")
    {
        bridge.subscription_failures += 1;
    }
    let (method, filter) = method.split_once('#').unwrap_or((&method, ""));
    if method == "channel_list" && filter == "listen_all" && packet.get("error").is_none() {
        // Discovery may outlive the requesting character's wizard privileges.
        if targets.is_empty() {
            return;
        }
        let (channels, skipped) = public_channels(result);
        let count = channels.len();
        let output=match queue_subscriptions(bridge,channels,true){Ok(queued)=>format!("Intermud: requested {} public channels; {} restricted/invalid channels skipped; {} paced requests queued. Router restrictions still apply. Use i3 listening for progress.",count,skipped,queued),Err(e)=>e};
        for e in targets {
            reply(world, e, output.clone());
        }
        return;
    }
    let output = if packet.get("error").is_some() {
        format!("Intermud: {}", text(&packet["error"], "message"))
    } else {
        match method {
            "status" => format!(
                "Intermud: {} as {}.",
                if result["connected"].as_bool() == Some(true) {
                    "connected to the router"
                } else {
                    "gateway connected; router reconnecting"
                },
                text(result, "mud_name")
            ),
            "mudlist" => {
                let filter = filter.to_lowercase();
                let names: Vec<_> = result["muds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|m| text(m, "name"))
                    .filter(|name| name.to_lowercase().contains(&filter))
                    .take(150)
                    .collect();
                format!(
                    "Intermud MUDs ({} shown): {}",
                    names.len(),
                    names.join(", ")
                )
            }
            "channel_list" => {
                let names: Vec<_> = result["channels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(150)
                    .map(|c| text(c, "name"))
                    .collect();
                format!("Intermud channels: {}", names.join(", "))
            }
            "who" => "Intermud: requested remote who; waiting for its reply.".into(),
            "tell" | "channel_send" => format!(
                "Intermud: {} to {} (gateway acceptance).",
                text(result, "status"),
                plain(filter)
            ),
            "channel_join" | "channel_leave" => {
                format!(
                    "Intermud: {} {}.",
                    text(result, "status"),
                    text(result, "channel")
                )
            }
            _ => return,
        }
    };
    for e in targets {
        reply(world, e, output.clone());
    }
}

fn deliver_request_notice(world: &mut World, bridge: &mut Bridge, p: &Value) {
    if let Some(output) = request_notice(p) {
        // Bound discovery spam independently of player command cooldowns.
        let key = "\0request-audit".to_string();
        if bridge
            .cooldown
            .get(&key)
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        bridge.cooldown.insert(key, Instant::now());
        for actor in recipients(world, true, None) {
            reply(world, actor, output.clone());
        }
    }
}

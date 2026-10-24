//! commands implementation, split from the parent subsystem.

use super::*;

pub(super) fn queue(
    b: &mut Bridge,
    actor: Entity,
    event: &str,
    payload: Value,
    filter: String,
    echo: Option<String>,
) -> Result<(), String> {
    if !b.ready {
        return Err(b.status.clone());
    }
    if b.pending.len() >= 64 {
        return Err("Grapevine is busy; wait for pending requests.".into());
    }
    let reference = uuid::Uuid::new_v4().to_string();
    let packet = json!({"event":event,"ref":reference,"payload":payload});
    b.tx.try_send(Job {
        actor,
        generation: b.generation,
        queued: Instant::now(),
        packet,
    })
    .map_err(|_| "Grapevine queue unavailable; try again.".to_owned())?;
    b.pending.insert(
        reference,
        Pending {
            actor,
            event: event.into(),
            filter,
            at: Instant::now(),
            replies: 0,
            echo,
        },
    );
    Ok(())
}

pub(crate) fn command(world: &mut World, actor: Entity, arg: &str) -> Result<(), String> {
    if !allowed(world, actor) {
        return Err("Grapevine commands are wizard-only.".into());
    }
    let Some(mut b) = world.remove_resource::<Bridge>() else {
        return Err("Grapevine is disabled; see help grapevine.".into());
    };
    // Update the worker's send authorization before enqueuing commands in this frame.
    update_presence(world, &mut b);
    let result = command_inner(world, &mut b, actor, arg);
    world.insert_resource(b);
    result
}

pub(super) fn command_inner(
    world: &mut World,
    b: &mut Bridge,
    actor: Entity,
    arg: &str,
) -> Result<(), String> {
    let (verb, rest) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
    let rest = rest.trim();
    match verb.to_ascii_lowercase().as_str() {
        "" | "status" => {
            tell(world,actor,format!("{} {} pending requests; {} channel messages retained locally. No credentials are displayed.",b.status,b.pending.len(),b.history.len()));
            return Ok(());
        }
        "reload" => {
            b.ready = false;
            b.reload.store(true, Ordering::Relaxed);
            tell(world,actor,"Grapevine will reconnect and reload private credentials. Unsent messages are discarded.");
            return Ok(());
        }
        "channels" => {
            let desired = b
                .snapshot
                .lock()
                .ok()
                .map(|s| {
                    let mut v: Vec<_> = s.channels.iter().cloned().collect();
                    v.sort();
                    v.join(", ")
                })
                .unwrap_or_default();
            let mut own: Vec<_> = b
                .listeners
                .get(&actor)
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            own.sort();
            tell(world,actor,format!("Grapevine channels: {desired}. You listen to: {}. Channel names contain 3–15 letters, _ or -.",if own.is_empty(){"none".into()}else{own.join(", ")}));
            return Ok(());
        }
        "recent" => return recent(world, b, actor, rest),
        "unlisten" => {
            if !valid_channel(rest) {
                return Err("Use grapevine unlisten <channel>.".into());
            }
            b.listeners.entry(actor).or_default().remove(rest);
            tell(
                world,
                actor,
                format!(
                    "You no longer listen to {}. Other wizards and recent history are unaffected.",
                    remote(rest)
                ),
            );
            return Ok(());
        }
        _ => {}
    }
    if b.cooldown
        .get(&actor)
        .is_some_and(|t| t.elapsed() < Duration::from_millis(750))
    {
        return Err("Wait a moment between Grapevine network requests.".into());
    }
    let name = world
        .get::<GrimName>(actor)
        .map(|n| n.0.clone())
        .ok_or("Character name unavailable.")?;
    match verb.to_ascii_lowercase().as_str(){
        "listen"=>{if !valid_channel(rest){return Err("Use grapevine listen <channel> (3–15 letters, _ or -).".into());}
if b.snapshot.lock().ok().is_some_and(|s|s.channels.len()>=32&&!s.channels.contains(rest)){return Err("At most 32 channels may be tracked.".into());}
            if b.confirmed.contains(rest){b.listeners.entry(actor).or_default().insert(rest.into());tell(world,actor,format!("You now listen to {}.",remote(rest)));return Ok(());}
            queue(b,actor,"channels/subscribe",json!({"channel":rest}),rest.into(),None)?;
        },
        "games"=>{queue(b,actor,"games/status",json!({}),String::new(),None)?;},
        "who"=>{if rest.len()>100{return Err("Game name is too long.".into());}queue(b,actor,"players/status",json!({}),rest.into(),None)?;},
        "chat"=>{let(channel,message)=rest.split_once(' ').ok_or("Use grapevine chat <channel> <message>.")?;if !valid_channel(channel)||!b.confirmed.contains(channel){return Err("Listen to that channel successfully before sending.".into());}
if message.trim().is_empty()||message.len()>1024{return Err("Use a message of 1–1024 bytes.".into());}queue(b,actor,"channels/send",json!({"channel":channel,"name":name,"message":message.trim()}),channel.into(),Some(format!("[Grapevine:{}] {}: {}",remote(channel),remote(&name),remote(message.trim()))))?;},
        "tell"=>{let(target,message)=rest.split_once(':').ok_or("Use grapevine tell <user>@<game>: <message>.")?;let(user,game)=target.trim().split_once('@').ok_or("Use grapevine tell <user>@<game>: <message>.")?;if user.is_empty()||game.is_empty()||user.len()>100||game.len()>100||message.trim().is_empty()||message.len()>1024{return Err("Use a recipient and a message of 1–1024 bytes.".into());}let sent_at=chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs,true);queue(b,actor,"tells/send",json!({"from_name":name,"to_game":game,"to_name":user,"sent_at":sent_at,"message":message.trim()}),String::new(),Some(format!("[Grapevine] You tell {}@{}: {}",remote(user),remote(game),remote(message.trim()))))?;},
        _=>return Err("Grapevine: status, reload, games, who [game], channels, listen/unlisten <channel>, chat <channel> <message>, tell user@game: message, recent [channel].".into())
    }
    b.cooldown.insert(actor, Instant::now());
    tell(world, actor, "Grapevine request queued.");
    Ok(())
}

fn recent(world: &mut World, b: &Bridge, actor: Entity, rest: &str) -> Result<(), String> {
    if !rest.is_empty() && !valid_channel(rest) {
        return Err("Use grapevine recent [channel].".into());
    }
    let rows: Vec<_> = b
        .history
        .iter()
        .rev()
        .filter(|h| rest.is_empty() || h.channel.eq_ignore_ascii_case(rest))
        .take(20)
        .map(|h| {
            format!(
                "[{}; {}s ago] {}@{}: {}",
                remote(&h.channel),
                now().saturating_sub(h.at),
                remote(&h.speaker),
                remote(&h.game),
                remote(&h.message)
            )
        })
        .collect();
    tell(
        world,
        actor,
        if rows.is_empty() {
            "No recent Grapevine channel activity.".into()
        } else {
            rows.into_iter().rev().collect::<Vec<_>>().join("\n")
        },
    );
    Ok(())
}

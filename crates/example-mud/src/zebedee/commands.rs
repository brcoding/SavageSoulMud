//! commands implementation, split from the parent subsystem.

use super::*;

/// Root command router calls this only for the zebedee / z2 verbs; authorization repeats here.
pub(crate) fn command(world: &mut World, actor: Entity, arg: &str) {
    if !allowed(world, actor) {
        say(world, actor, "Zebedee commands are wizard-only.");
        return;
    }
    let Some(mut bridge) = world.remove_resource::<Bridge>() else {
        say(world, actor, "Zebedee is disabled. See help zebedee.");
        return;
    };
    let result = command_inner(world, &mut bridge, actor, arg);
    if let Err(e) = result {
        say(world, actor, e);
    }
    world.insert_resource(bridge);
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
            say(world,actor,format!("Zebedee: {} {} pending requests. {} peers have replied this session. UDP does not authenticate identities cryptographically; only configured peers are accepted.",b.status,b.pending.len(),b.last_seen.len()));
            return Ok(());
        }
        "muds" => {
            let rows: Vec<_> = b
                .config
                .peers
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    format!(
                        "{} ({})",
                        plain(&p.name),
                        b.last_seen
                            .get(&i)
                            .map(|t| format!("reply {}s ago", t.elapsed().as_secs()))
                            .unwrap_or_else(|| "unverified".into())
                    )
                })
                .collect();
            say(
                world,
                actor,
                format!(
                    "Configured Zebedee peers: {}",
                    if rows.is_empty() {
                        "none; configure data/zebedee.json".into()
                    } else {
                        rows.join(", ")
                    }
                ),
            );
            return Ok(());
        }
        "channels" => {
            say(
                world,
                actor,
                format!(
                    "Configured channels: {}. Use zebedee listen <channel> or zebedee listen all.",
                    b.config.channels.join(", ")
                ),
            );
            return Ok(());
        }
        "listen" | "unlisten" => return subscription(world, b, actor, verb, rest),
        _ => {}
    }
    if b.cooldown
        .get(&actor)
        .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
    {
        return Err("Wait one second between Zebedee network requests.".into());
    }
    let name = world
        .get::<GrimName>(actor)
        .map(|n| n.0.clone())
        .ok_or("Character name unavailable.")?;
    let mut p = base(&b.config, verb);
    p.set("SND", name.to_ascii_lowercase());
    let peer_name=match verb.to_ascii_lowercase().as_str(){
        "ping"|"who"=>rest,
        "mudlist"=>{p.set("REQ","query");p.data="hosts-json".into();rest},
        "query"=>{let(peer,query)=rest.split_once(':').ok_or("Use zebedee query <mud>: <query>.")?;p.data=query.trim().into();peer.trim()},
        "finger"=>{let(user,mud)=rest.split_once('@').ok_or("Use zebedee finger <user>@<mud>.")?;p.data=user.trim().into();mud.trim()},
        "tell"=>{let(target,message)=rest.split_once(':').ok_or("Use zebedee tell <user>@<mud>: <message>.")?;let(user,mud)=target.split_once('@').ok_or("Use zebedee tell <user>@<mud>: <message>.")?;if !header(user.trim())||message.trim().is_empty()||message.len()>1024{return Err("Use a recipient and 1–1024 characters of text.".into());}p.set("RCPNT",user.trim().to_ascii_lowercase());p.data=message.trim().into();mud.trim()},
        "chat"=>{
            let(channel,message)=rest.split_once(' ').ok_or("Use zebedee chat <channel> <message>.")?;
            let channel=b.config.channels.iter().find(|c|c.eq_ignore_ascii_case(channel)).cloned().ok_or("Choose a configured channel.")?;
            if message.trim().is_empty()||message.len()>1024{return Err("Choose a configured channel and 1–1024 characters of text.".into());}
            p.set("REQ","channel");p.set("CHANNEL",channel);p.data=message.trim().into();
            if b.config.peers.is_empty(){return Err("No configured Zebedee peers.".into());}
            if !b.ready{return Err(b.status.clone());}
            let mut failed=false;
            for peer in 0..b.config.peers.len(){if send(b,peer,p.clone()).is_err(){failed=true;}}
            b.cooldown.insert(actor,Instant::now());
            if failed{return Err("Some Zebedee sends could not be queued; wait before retrying.".into());}
            say(world,actor,"Zebedee channel message queued; UDP delivery is not guaranteed.");return Ok(());
        },
        _=>return Err("Zebedee: status, muds, channels, listen/unlisten <channel|all>, ping <mud>, who <mud>, finger user@mud, mudlist <mud>, query <mud>: <query>, tell user@mud: message, chat channel message. See help zebedee.".into())
    };
    let peer = b
        .config
        .peers
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(peer_name))
        .ok_or("Unknown configured peer. Use zebedee muds; mud names can contain spaces.")?;
    p.set("REQ", p.get("REQ").to_ascii_lowercase());
    request(b, actor, peer, p)?;
    b.cooldown.insert(actor, Instant::now());
    say(world, actor, "Zebedee request queued.");
    Ok(())
}

fn subscription(
    world: &mut World,
    b: &mut Bridge,
    actor: Entity,
    verb: &str,
    rest: &str,
) -> Result<(), String> {
    let channels = if rest.eq_ignore_ascii_case("all") {
        b.config.channels.clone()
    } else if b
        .config
        .channels
        .iter()
        .any(|c| c.eq_ignore_ascii_case(rest))
    {
        vec![b
            .config
            .channels
            .iter()
            .find(|c| c.eq_ignore_ascii_case(rest))
            .unwrap()
            .clone()]
    } else {
        return Err("Choose a configured channel from zebedee channels.".into());
    };
    let subscribed = b.listeners.entry(actor).or_default();
    for c in channels {
        if verb.eq_ignore_ascii_case("listen") {
            subscribed.insert(c);
        } else {
            subscribed.remove(&c);
        }
    }
    say(
        world,
        actor,
        format!(
            "Zebedee listening: {}. Subscriptions are local and last until logout/restart.",
            subscribed.iter().cloned().collect::<Vec<_>>().join(", ")
        ),
    );
    Ok(())
}

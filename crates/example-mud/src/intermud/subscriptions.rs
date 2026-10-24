//! subscriptions implementation, split from the parent subsystem.

use super::*;

pub(super) fn load_subscriptions(path: &str) -> Option<Vec<String>> {
    let path = activity_path(path)?;
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(ACTIVITY_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > ACTIVITY_BYTES {
        return None;
    }
    let channels: Vec<String> = serde_json::from_slice(&bytes).ok()?;
    if channels.len() > 512 || channels.iter().any(|c| !valid_channel(c)) {
        return None;
    }
    Some(channels)
}

pub(super) fn save_subscriptions(path: &str, channels: &[String]) -> std::io::Result<()> {
    if path.is_empty() {
        return Ok(());
    }
    let path = activity_path(path).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "subscriptions_file must be inside existing local /home state directory",
        )
    })?;
    let temp = path.with_extension("subscriptions.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(&serde_json::to_vec(channels)?)?;
    file.sync_data()?;
    drop(file);
    std::fs::rename(temp, path)
}

/// Router recovery is a control event: unlike excess chat, it cannot be dropped.
pub(super) fn forward_packet(events: &SyncSender<Incoming>, packet: Value) -> std::io::Result<()> {
    if packet["method"] == "gateway_reconnected" {
        return events
            .send(Incoming::Packet(packet))
            .map_err(|_| std::io::ErrorKind::BrokenPipe.into());
    }
    match events.try_send(Incoming::Packet(packet)) {
        Ok(()) | Err(mpsc::TrySendError::Full(_)) => Ok(()),
        Err(mpsc::TrySendError::Disconnected(_)) => Err(std::io::ErrorKind::BrokenPipe.into()),
    }
}

pub(super) fn rejoin(bridge: &mut Bridge) {
    bridge.confirmed.clear();
    bridge.subscription_failures = 0;
    let leaves: Vec<_> = bridge
        .subscriptions
        .drain(..)
        .filter(|(channel, join)| !join && !bridge.config.channels.contains(channel))
        .collect();
    bridge.subscriptions = bridge
        .config
        .channels
        .iter()
        .map(|c| (c.clone(), true))
        .chain(leaves)
        .collect();
}

pub(super) fn pump_subscriptions(bridge: &mut Bridge) {
    if !bridge.connected || bridge.last_subscription.elapsed() < Duration::from_millis(1100) {
        return;
    }
    if let Some((channel, join)) = bridge.subscriptions.front().cloned() {
        if request(
            bridge,
            "",
            if join {
                "channel_join"
            } else {
                "channel_leave"
            },
            json!({"channel":channel}),
        )
        .is_ok()
        {
            bridge.subscriptions.pop_front();
            bridge.last_subscription = Instant::now();
        }
    }
}

pub(super) fn queue_subscriptions(
    bridge: &mut Bridge,
    channels: Vec<String>,
    join: bool,
) -> Result<usize, String> {
    let mut desired = bridge.config.channels.clone();
    let mut jobs = bridge.subscriptions.clone();
    for channel in channels {
        if !valid_channel(&channel) {
            continue;
        }
        if join {
            if !desired.contains(&channel) {
                desired.push(channel.clone());
            }
        } else {
            desired.retain(|c| c != &channel);
        }
        jobs.retain(|(c, _)| c != &channel);
        jobs.push_back((channel, join));
    }
    if desired.len() > 512 || jobs.len() > 512 {
        return Err("Intermud subscription limit is 512 channels.".into());
    }
    save_subscriptions(&bridge.config.subscriptions_file, &desired)
        .map_err(|_| "Could not save Intermud subscriptions; no changes applied.".to_string())?;
    bridge.config.channels = desired;
    bridge.subscriptions = jobs;
    Ok(bridge.subscriptions.len())
}

pub(super) fn public_channels(result: &Value) -> (Vec<String>, usize) {
    let mut channels = Vec::new();
    let mut skipped = 0;
    for c in result["channels"].as_array().into_iter().flatten() {
        let name = c["name"].as_str().unwrap_or("");
        if c["type"].as_i64() == Some(0) && valid_channel(name) {
            if !channels.iter().any(|v| v == name) {
                channels.push(name.to_string());
            }
        } else {
            skipped += 1;
        }
    }
    (channels, skipped)
}

pub(super) fn subscription_command(bridge: &mut Bridge, arg: &str) -> Option<String> {
    let (verb, channel) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
    let channel = channel.trim();
    if verb.eq_ignore_ascii_case("listening") {
        return Some(format!("Intermud subscriptions: {} requested; {} gateway acknowledgements; {} queued; {} failed requests. Requested: {}",bridge.config.channels.len(),bridge.confirmed.len(),bridge.subscriptions.len(),bridge.subscription_failures,bridge.config.channels.iter().map(|c|plain(c)).collect::<Vec<_>>().join(", ")));
    }
    let join = verb.eq_ignore_ascii_case("listen");
    if !join && !verb.eq_ignore_ascii_case("unlisten") {
        return None;
    }
    if join && channel.eq_ignore_ascii_case("all") {
        return None;
    }
    let channels = if !join && channel.eq_ignore_ascii_case("all") {
        let mut all = bridge.config.channels.clone();
        for c in &bridge.confirmed {
            if !all.contains(c) {
                all.push(c.clone());
            }
        }
        all
    } else if valid_channel(channel) {
        vec![channel.into()]
    } else {
        return Some("Use i3 listen <channel|all> or i3 unlisten <channel|all>.".into());
    };
    Some(match queue_subscriptions(bridge, channels, join) {
        Ok(n) => format!(
            "Intermud: {} requests queued; subscriptions are paced. Use i3 listening for progress.",
            n
        ),
        Err(e) => e,
    })
}

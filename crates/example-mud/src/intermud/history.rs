//! history implementation, split from the parent subsystem.

use super::*;

pub(super) fn unix_time() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn bounded(value: &str, limit: usize) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if out.len() + c.len_utf8() > limit {
            break;
        }
        out.push(c);
    }
    out
}

pub(super) fn valid_record(record: &Activity, now: u64) -> bool {
    record.at > 0
        && record.at <= now
        && valid_channel(&record.channel)
        && record.speaker.len() <= 128
        && record.mud.len() <= 128
        && record.message.len() <= 1024
}

pub(super) fn record_activity(
    history: &mut VecDeque<Activity>,
    kind: &str,
    p: &Value,
    now: u64,
) -> bool {
    if !matches!(kind, "channel_message" | "channel_emote") {
        return false;
    }
    let Some(channel) = p["channel"].as_str().filter(|c| valid_channel(c)) else {
        return false;
    };
    let Some(message) = p["message"].as_str() else {
        return false;
    };
    history.push_back(Activity {
        at: now,
        channel: channel.into(),
        speaker: bounded(p["from_user"].as_str().unwrap_or("?"), 128),
        mud: bounded(p["from_mud"].as_str().unwrap_or("?"), 128),
        message: bounded(message, 1024),
        emote: kind == "channel_emote",
    });
    while history.len() > ACTIVITY_LIMIT
        || serde_json::to_vec(history).map_or(true, |b| b.len() as u64 > ACTIVITY_BYTES)
    {
        history.pop_front();
    }
    true
}

pub(super) fn activity_path(path: &str) -> Option<std::path::PathBuf> {
    if !path.starts_with("/home/") || path.split('/').any(|p| p == "..") {
        return None;
    }
    let path = std::path::PathBuf::from(path);
    let parent = path.parent()?.canonicalize().ok()?;
    if !parent.starts_with("/home") {
        return None;
    }
    Some(parent.join(path.file_name()?))
}

pub(super) fn load_activity(path: &str) -> VecDeque<Activity> {
    let Some(path) = activity_path(path) else {
        return VecDeque::new();
    };
    let Ok(file) = std::fs::File::open(path) else {
        return VecDeque::new();
    };
    let mut bytes = Vec::new();
    if file
        .take(ACTIVITY_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > ACTIVITY_BYTES
    {
        return VecDeque::new();
    }
    let Ok(history) = serde_json::from_slice::<VecDeque<Activity>>(&bytes) else {
        return VecDeque::new();
    };
    let now = unix_time();
    if history.len() > ACTIVITY_LIMIT || history.iter().any(|r| !valid_record(r, now)) {
        return VecDeque::new();
    }
    history
}

pub(super) fn save_activity(path: &str, history: &VecDeque<Activity>) -> std::io::Result<()> {
    if path.is_empty() {
        return Ok(());
    }
    let path = activity_path(path).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "activity_file must be inside an existing local /home directory",
        )
    })?;
    let temp = path.with_extension("tmp");
    let bytes = serde_json::to_vec(history)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_data()?;
    drop(file);
    std::fs::rename(temp, path)
}

pub(super) fn age(at: u64, now: u64) -> String {
    let seconds = now.saturating_sub(at);
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86400)
    }
}

pub(super) fn activity_command(bridge: &Bridge, arg: &str) -> Option<String> {
    let (verb, filter) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
    let filter = filter.trim();
    let now = unix_time();
    match verb.to_ascii_lowercase().as_str() {
        "recent" => {
            if !filter.is_empty() && !valid_channel(filter) {
                return Some("Use i3 recent [channel].".into());
            }
            let mut records: Vec<_> = bridge
                .activity
                .iter()
                .rev()
                .filter(|r| filter.is_empty() || r.channel.eq_ignore_ascii_case(filter))
                .take(20)
                .collect();
            records.reverse();
            if records.is_empty() {
                return Some("No observed channel messages yet. History starts when this MUD listens; private tells are excluded.".into());
            }
            let mut output = format!(
                "Recent Intermud: {} observed messages (oldest first).",
                records.len()
            );
            for r in records {
                output.push_str(&format!(
                    "\n[{}; {}] {}@{}{} {}",
                    plain(&r.channel),
                    age(r.at, now),
                    plain(&r.speaker),
                    plain(&r.mud),
                    if r.emote { " emotes:" } else { ":" },
                    plain(&r.message)
                ));
            }
            Some(output)
        }
        "activity" => {
            if !filter.is_empty() {
                return Some("Use i3 activity; use i3 recent <channel> for messages.".into());
            }
            let mut channels: HashMap<&str, (usize, u64)> = HashMap::new();
            for r in &bridge.activity {
                let item = channels.entry(&r.channel).or_default();
                item.0 += 1;
                item.1 = item.1.max(r.at);
            }
            let mut rows: Vec<_> = channels.into_iter().collect();
            rows.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then(a.0.cmp(b.0)));
            let mut output=format!("Observed channel activity: {} retained public messages (maximum 200; listened channels only).",bridge.activity.len());
            if rows.is_empty() {
                output.push_str(" No messages observed yet.");
            }
            for (channel, (count, at)) in rows {
                output.push_str(&format!(
                    "\n{}: {} retained; latest {}.",
                    plain(channel),
                    count,
                    age(at, now)
                ));
            }
            Some(output)
        }
        _ => None,
    }
}

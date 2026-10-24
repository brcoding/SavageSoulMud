//! history implementation, split from the parent subsystem.

use super::*;

pub(super) fn private_path(path: &str) -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(path);
    if !p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        || ["/mnt", "/proc", "/sys", "/dev"]
            .iter()
            .any(|prefix| p.starts_with(prefix))
    {
        return None;
    }
    let parent = p.parent()?;
    let canonical = parent.canonicalize().ok()?;
    if canonical != parent {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = parent.metadata().ok()?;
        let current = std::fs::metadata("/proc/self").ok()?.uid();
        if meta.uid() != current || meta.permissions().mode() & 0o077 != 0 {
            return None;
        }
    }
    Some(p)
}

pub(super) fn valid_activity(record: &Activity) -> bool {
    record.at > 0
        && record.at <= now().saturating_add(5)
        && valid_channel(&record.channel)
        && record.speaker.len() <= 400
        && record.game.len() <= 400
        && record.message.len() <= 4096
}

pub(super) fn load_history(path: &str) -> VecDeque<Activity> {
    let Some(path) = private_path(path) else {
        return VecDeque::new();
    };
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return VecDeque::new();
    };
    if !meta.is_file() || meta.len() > 256 * 1024 {
        return VecDeque::new();
    }
    let Ok(file) = std::fs::File::open(path) else {
        return VecDeque::new();
    };
    let mut bytes = Vec::new();
    if file.take(256 * 1024 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > 256 * 1024 {
        return VecDeque::new();
    }
    let Ok(records) = serde_json::from_slice::<VecDeque<Activity>>(&bytes) else {
        return VecDeque::new();
    };
    if records.len() > 200 || records.iter().any(|r| !valid_activity(r)) {
        return VecDeque::new();
    }
    records
}

pub(super) fn save_history(path: &str, records: &VecDeque<Activity>) -> std::io::Result<()> {
    let path = private_path(path).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid private history path",
        )
    })?;
    let mut records = records.clone();
    let bytes = loop {
        let bytes = serde_json::to_vec(&records)?;
        if bytes.len() <= 256 * 1024 {
            break bytes;
        }
        records.pop_front();
    };
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // A stale temp file is never followed or overwritten; use a fresh unique suffix.
    let temp = if temp.exists() {
        path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()))
    } else {
        temp
    };
    let mut file = options.open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_data()?;
    drop(file);
    std::fs::rename(temp, path)
}

pub(super) fn history_worker(
    path: String,
    store: Arc<Mutex<HistoryStore>>,
    events: SyncSender<Incoming>,
) {
    let records = load_history(&path);
    let _ = events.try_send(Incoming::History(records));
    loop {
        std::thread::sleep(Duration::from_millis(1000));
        let records = store.lock().ok().and_then(|mut state| {
            if state.dirty {
                state.dirty = false;
                Some(state.records.clone())
            } else {
                None
            }
        });
        if let Some(records) = records {
            if save_history(&path, &records).is_err() {
                if let Ok(mut state) = store.lock() {
                    state.dirty = true;
                }
            }
        }
        if Arc::strong_count(&store) == 1 {
            return;
        }
    }
}

pub(super) fn mark_history(b: &mut Bridge) {
    if let Ok(mut state) = b.history_store.lock() {
        state.records = b.history.clone();
        state.dirty = true;
    }
}

//! codec implementation, split from the parent subsystem.

use super::*;

pub(super) fn header(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_graphic() && c != '|' || c == ' ')
}

pub(super) fn encode(p: &Packet) -> Option<Vec<u8>> {
    let mut out = String::new();
    for (k, v) in &p.headers {
        if !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || k == "DATA" || !header(v) {
            return None;
        }
        let escaped = if !matches!(k.as_str(), "ID" | "UDP" | "EMOTE")
            && (v.starts_with('$') || v.parse::<i64>().is_ok())
        {
            format!("${v}")
        } else {
            v.clone()
        };
        out.push_str(&format!("{k}:{escaped}|"));
    }
    out.push_str("DATA:");
    if p.data.starts_with('$') || p.data.parse::<i64>().is_ok() {
        out.push('$');
    }
    out.push_str(&p.data);
    (out.len() <= MAX_PACKET).then(|| out.into_bytes())
}

pub(super) fn decode(bytes: &[u8]) -> Option<Packet> {
    if bytes.len() > MAX_PACKET {
        return None;
    }
    // ASCII is the common interoperable encoding; Latin-1 peers remain legible.
    let s = String::from_utf8(bytes.to_vec())
        .unwrap_or_else(|_| bytes.iter().map(|b| char::from(*b)).collect());
    let mut p = Packet::default();
    let mut rest = s.as_str();
    let unquote = |v: &str| v.strip_prefix('$').unwrap_or(v).to_owned();
    while !rest.is_empty() {
        if let Some(data) = rest.strip_prefix("DATA:") {
            p.data = unquote(data);
            return Some(p);
        }
        let (part, next) = rest.split_once('|').unwrap_or((rest, ""));
        let (k, v) = part.split_once(':')?;
        if k.is_empty()
            || k.len() > 32
            || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || p.headers.len() >= 24
            || p.headers.contains_key(k)
        {
            return None;
        }
        let v = unquote(v);
        if !header(&v) {
            return None;
        }
        p.headers.insert(k.into(), v);
        rest = next;
    }
    Some(p)
}

impl Assembly {
    pub(super) fn receive(&mut self, peer: usize, bytes: &[u8]) -> Option<Packet> {
        self.groups
            .retain(|_, v| v.at.elapsed() < Duration::from_secs(15));
        if !bytes.starts_with(b"PKT:") {
            return decode(bytes);
        }
        let at = bytes.iter().position(|b| *b == b'|')?;
        let head = std::str::from_utf8(bytes.get(4..at)?).ok()?;
        let mut parts = head.rsplitn(3, ':');
        let (pos, total) = parts.next()?.split_once('/')?;
        let pos: usize = pos.parse().ok()?;
        let total: usize = total.parse().ok()?;
        let id = parts.next()?;
        let name = parts.next()?;
        if total == 0 || total > 32 || pos == 0 || pos > total || id.len() > 32 || !header(name) {
            return None;
        }
        let key = (peer, format!("{name}:{id}"));
        if self.groups.len() >= 16 && !self.groups.contains_key(&key) {
            return None;
        }
        let group = self.groups.entry(key.clone()).or_insert_with(|| Fragments {
            at: Instant::now(),
            pieces: vec![None; total],
        });
        if group.pieces.len() != total {
            return None;
        }
        group.pieces[pos - 1] = Some(bytes.get(at + 1..)?.to_vec());
        if group.pieces.iter().flatten().map(Vec::len).sum::<usize>() > MAX_PACKET {
            self.groups.remove(&key);
            return None;
        }
        if !group.pieces.iter().all(Option::is_some) {
            return None;
        }
        let bytes: Vec<u8> = group
            .pieces
            .iter()
            .flatten()
            .flat_map(|v| v.iter().copied())
            .collect();
        self.groups.remove(&key);
        decode(&bytes)
    }
}

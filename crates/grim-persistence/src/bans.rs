//! The persisted blocklist: admin bans by IP, account, or character.
//!
//! Stored as a single `bans.json` array under the persistence dir
//! (`data/bans.json`), loaded at Startup by [`PersistencePlugin`](crate::PersistencePlugin)
//! and saved synchronously on every `ban add` / `ban remove`, so a crash
//! between the command and the next tick can never lose one.

use bevy::prelude::*;
use chrono::{DateTime, Utc};
use grim_core::components::Account;
use grim_core::events::BanKind;
use serde::{Deserialize, Serialize};
use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

/// One blocklist entry: what is blocked, plus when and by whom it was banned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BanEntry {
    pub kind: BanKind,
    /// The blocked value: an IP pattern (`127.0.*`), an account identifier/id,
    /// or a character name. Account/character patterns store lowercase, so
    /// enforcement is case-insensitive by construction.
    pub pattern: String,
    pub created_at: DateTime<Utc>,
    /// Display name of the admin character that created the ban.
    pub created_by: String,
}

/// The live blocklist, loaded from `bans.json` at Startup.
#[derive(Resource, Debug, Clone, Default)]
pub struct BanList {
    entries: Vec<BanEntry>,
}

impl BanList {
    /// Canonicalize a pattern for storage: trimmed, lowercased for the
    /// exact-match kinds (account/character) so enforcement never depends on
    /// the casing the admin typed.
    pub fn normalize(kind: BanKind, pattern: &str) -> String {
        match kind {
            BanKind::Ip => pattern.trim().to_string(),
            BanKind::Account | BanKind::Character => pattern.trim().to_lowercase(),
        }
    }

    /// Whether `pattern` is bannable for `kind`: a non-empty exact value for
    /// account/character, or 1–4 dot parts each `*` or 0–255 (with at least
    /// one numeric octet). IPv6 is out of scope: only IPv4 is matched.
    pub fn valid_pattern(kind: BanKind, pattern: &str) -> bool {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            return false;
        }
        match kind {
            BanKind::Account | BanKind::Character => true,
            BanKind::Ip => {
                let mut numeric = false;
                let parts: Vec<&str> = trimmed.split('.').collect();
                if parts.len() > 4 {
                    return false;
                }
                for part in parts {
                    if part == "*" {
                        continue;
                    }
                    if part.parse::<u8>().is_ok() {
                        numeric = true;
                    } else {
                        return false;
                    }
                }
                numeric
            }
        }
    }

    /// Whether an IPv4 pattern matches an address: per-octet, where `*` (or a
    /// missing trailing part) skips the octet. Anything else — including any
    /// IPv6 address — matches nothing (fail closed).
    pub fn matches_ip(pattern: &str, ip: &IpAddr) -> bool {
        let IpAddr::V4(v4) = ip else {
            return false;
        };
        let mut parts = pattern.split('.');
        for octet in v4.octets() {
            match parts.next() {
                None => return true,
                Some("*") => continue,
                Some(part) => match part.parse::<u8>() {
                    Ok(n) if n == octet => continue,
                    _ => return false,
                },
            }
        }
        parts.next().is_none()
    }

    /// Insert a ban. Returns `false` (and stores nothing) when the pattern is
    /// invalid or the exact kind + pattern is already banned.
    pub fn add(&mut self, kind: BanKind, pattern: &str, created_by: &str) -> bool {
        if !Self::valid_pattern(kind, pattern) {
            return false;
        }
        let pattern = Self::normalize(kind, pattern);
        if self
            .entries
            .iter()
            .any(|e| e.kind == kind && e.pattern == pattern)
        {
            return false;
        }
        self.entries.push(BanEntry {
            kind,
            pattern,
            created_at: Utc::now(),
            created_by: created_by.to_string(),
        });
        true
    }

    /// Lift a ban. Returns whether one was removed.
    pub fn remove(&mut self, kind: BanKind, pattern: &str) -> bool {
        let pattern = Self::normalize(kind, pattern);
        match self
            .entries
            .iter()
            .position(|e| e.kind == kind && e.pattern == pattern)
        {
            Some(idx) => {
                self.entries.remove(idx);
                true
            }
            None => false,
        }
    }

    /// Every entry, optionally filtered by kind, in insertion order.
    pub fn list(&self, filter: Option<BanKind>) -> Vec<&BanEntry> {
        self.entries
            .iter()
            .filter(|e| filter.is_none_or(|k| e.kind == k))
            .collect()
    }

    /// Whether an address is IP-banned.
    pub fn is_ip_banned(&self, ip: &IpAddr) -> bool {
        self.entries
            .iter()
            .filter(|e| e.kind == BanKind::Ip)
            .any(|e| Self::matches_ip(&e.pattern, ip))
    }

    /// Whether an account is banned, by identifier or id (case-insensitive).
    pub fn is_account_banned(&self, account: &Account) -> bool {
        let identifier = account.identifier.to_lowercase();
        let id = account.id.to_string().to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.kind == BanKind::Account)
            .any(|e| e.pattern == identifier || e.pattern == id)
    }

    /// Whether a character name is banned (case-insensitive, exact).
    pub fn is_character_banned(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.kind == BanKind::Character)
            .any(|e| e.pattern == name)
    }

    /// The single-file store: `<dir>/bans.json`.
    pub fn bans_file(dir: &Path) -> PathBuf {
        dir.join("bans.json")
    }

    /// Load the store. A missing or corrupt file reads as empty (fail open at
    /// boot would lock everyone out on a typo'd edit; fail closed happens per
    /// pattern at enforcement instead).
    pub fn load(dir: &Path) -> Self {
        let entries = std::fs::read_to_string(Self::bans_file(dir))
            .ok()
            .and_then(|data| serde_json::from_str::<Vec<BanEntry>>(&data).ok())
            .unwrap_or_default();
        Self { entries }
    }

    /// Persist the store synchronously.
    pub fn save(&self, dir: &Path) -> io::Result<()> {
        let path = Self::bans_file(dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.entries)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_core::GrimId;

    fn v4(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn account(identifier: &str) -> Account {
        Account {
            testing: false,
            id: GrimId::new(),
            identifier: identifier.into(),
            password_hash: "hash".into(),
            characters: vec![],
            created_at: Utc::now(),
        }
    }

    #[test]
    fn ip_exact_and_wildcard_matching() {
        let ip = v4("127.0.0.1");
        assert!(BanList::matches_ip("127.0.0.1", &ip));
        assert!(BanList::matches_ip("127.0.*", &ip));
        assert!(BanList::matches_ip("127.*", &ip));
        assert!(BanList::matches_ip("*", &ip));
        assert!(!BanList::matches_ip("127.0.0.2", &ip));
        assert!(!BanList::matches_ip("128.*", &ip));
        assert!(!BanList::matches_ip("127.0.0.*", &v4("127.0.1.1")));
        // IPv6 is out of scope: nothing matches it, not even an exact literal.
        let v6: IpAddr = "::1".parse().unwrap();
        assert!(!BanList::matches_ip("::1", &v6));
        assert!(!BanList::matches_ip("*", &v6));
    }

    #[test]
    fn ip_invalid_patterns_match_nothing() {
        let ip = v4("10.1.2.3");
        for bad in [
            "",
            "10.1.2.3.4",
            "10.1.2.x",
            "10..2.3",
            "300.1.1.1",
            "10.1.2.",
        ] {
            assert!(!BanList::matches_ip(bad, &ip), "{bad} must not match");
            assert!(!BanList::valid_pattern(BanKind::Ip, bad), "{bad} invalid");
        }
    }

    #[test]
    fn ip_pattern_validation() {
        for good in ["1.2.3.4", "10.*", "10.0.*", "10.0.0.*"] {
            assert!(BanList::valid_pattern(BanKind::Ip, good), "{good} valid");
        }
        assert!(!BanList::valid_pattern(BanKind::Ip, ""));
        assert!(!BanList::valid_pattern(BanKind::Ip, "   "));
        // A bare `*` would ban every address including the admin's own: refuse
        // it rather than guessing intent.
        assert!(!BanList::valid_pattern(BanKind::Ip, "*"));
        // IPv6 is out of scope: literals are rejected and never match.
        assert!(!BanList::valid_pattern(BanKind::Ip, "::1"));
        assert!(!BanList::valid_pattern(BanKind::Ip, "2001:db8::1"));
        // Exact-match kinds accept any non-empty value.
        assert!(BanList::valid_pattern(BanKind::Account, "a@b.c"));
        assert!(BanList::valid_pattern(BanKind::Character, "Bob"));
        assert!(!BanList::valid_pattern(BanKind::Character, ""));
    }

    #[test]
    fn add_dedupes_and_normalizes() {
        let mut bans = BanList::default();
        assert!(bans.add(BanKind::Character, "Bob", "Admin"));
        assert!(!bans.add(BanKind::Character, "BOB", "Admin"));
        assert!(!bans.add(BanKind::Character, "bob", "Admin"));
        // Same value under another kind is a different ban.
        assert!(bans.add(BanKind::Account, "bob", "Admin"));
        // Invalid patterns are refused.
        assert!(!bans.add(BanKind::Ip, "999.1.1.1", "Admin"));
        assert_eq!(bans.list(None).len(), 2);
        assert_eq!(bans.list(Some(BanKind::Character)).len(), 1);
    }

    #[test]
    fn character_and_account_checks_are_case_insensitive() {
        let mut bans = BanList::default();
        assert!(bans.add(BanKind::Character, "Grimmok", "Admin"));
        assert!(bans.is_character_banned("grimmok"));
        assert!(bans.is_character_banned("GRIMMOK"));
        assert!(!bans.is_character_banned("grimm"));
        assert!(bans.add(BanKind::Account, "Player@Example.com", "Admin"));
        assert!(bans.is_account_banned(&account("player@example.com")));
        assert!(!bans.is_account_banned(&account("other@example.com")));
    }

    #[test]
    fn account_ban_matches_id_too() {
        let mut bans = BanList::default();
        let acct = account("someone@example.com");
        assert!(bans.add(BanKind::Account, &acct.id.to_string(), "Admin"));
        assert!(bans.is_account_banned(&acct));
    }

    #[test]
    fn remove_lifts_only_the_named_ban() {
        let mut bans = BanList::default();
        bans.add(BanKind::Ip, "10.*", "Admin");
        assert!(bans.is_ip_banned(&v4("10.9.9.9")));
        assert!(!bans.remove(BanKind::Ip, "11.*"));
        assert!(bans.remove(BanKind::Ip, "10.*"));
        assert!(!bans.is_ip_banned(&v4("10.9.9.9")));
        assert!(!bans.remove(BanKind::Ip, "10.*"));
    }

    fn unique_dir(tag: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!("grim-bans-{tag}-{}-{n}", std::process::id()))
    }

    #[test]
    fn save_load_round_trip_single_file() {
        let dir = unique_dir("roundtrip");
        let mut bans = BanList::default();
        bans.add(BanKind::Ip, "192.168.*", "Root");
        bans.add(BanKind::Character, "Villain", "Root");
        bans.save(&dir).unwrap();
        assert!(BanList::bans_file(&dir).is_file());
        let loaded = BanList::load(&dir);
        assert_eq!(loaded.list(None).len(), 2);
        assert!(loaded.is_ip_banned(&v4("192.168.5.5")));
        assert!(loaded.is_character_banned("villain"));
        let entry = loaded.list(Some(BanKind::Ip))[0];
        assert_eq!(entry.created_by, "Root");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_or_corrupt_reads_empty() {
        let dir = unique_dir("missing");
        assert!(BanList::load(&dir).list(None).is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(BanList::bans_file(&dir), "{not json").unwrap();
        assert!(BanList::load(&dir).list(None).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

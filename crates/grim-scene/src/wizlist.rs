//! The admin-filtered `wizlist`: online admin rows plus the offline admins from
//! a startup disk snapshot. Factored out of `who.rs` (file-length cap) — the
//! same one-file-per-list shape as `sockets.rs` and `finger.rs`. Row machinery
//! (`WhoRow`, the ordering keys, the collector) stays in `who.rs` and is shared.

use std::collections::HashSet;

use bevy::prelude::*;
use grim_actor::{Linkdead, Role, StoredCharacter};
use grim_persistence::{load_all_characters, PersistenceConfig};

use crate::params::{PlayerChars, SessionRes};
use crate::who::{
    collect_who_data, format_who_row, gender_char, who_order, WhoData, WhoKey, WhoRow,
};

/// Admin-flagged characters snapshotted from disk at startup, backing the
/// offline half of the `wizlist`. Short-term shape (per review): a reboot
/// refreshes it, but a role granted or revoked at runtime only takes effect on
/// the live rows until the next restart — the snapshot is not rewritten.
#[derive(Resource, Default)]
pub(crate) struct WizlistAdmins(pub Vec<StoredCharacter>);

/// Snapshot every admin-flagged character on disk into [`WizlistAdmins`].
/// Missing dir → empty (same fail-closed read as the other disk scans).
pub(crate) fn load_wizlist_admins(mut commands: Commands, config: Res<PersistenceConfig>) {
    let mut admins: Vec<StoredCharacter> = load_all_characters(&config)
        .into_iter()
        .filter(|stored| stored.roles.contains(&Role::Admin))
        .collect();
    admins.sort_by_key(|a| a.name.to_lowercase());
    commands.insert_resource(WizlistAdmins(admins));
}

/// Render the full WIZLIST from already-sorted rows: the same MUD-style rows
/// as WHO, but every admin — online rows plus `(offline)` snapshot rows.
pub(crate) fn format_wizlist_list(rows: &[WhoRow]) -> String {
    if rows.is_empty() {
        return "No wizards found.\n".into();
    }
    let mut out = format!("Wizards ({}):\n", rows.len());
    for row in rows {
        out.push_str(&format_who_row(row));
        out.push('\n');
    }
    out
}

/// The `wizlist`: every admin-flagged character — online rows (same MUD-style
/// rows as [`format_who`](crate::who::format_who), linkdead marked) plus the
/// offline admins from the startup [`WizlistAdmins`] snapshot, marked
/// `(offline)`. Online wins the name: a snapshot entry also in the world
/// renders once, live. Sorted alphabetical by name.
pub(crate) fn format_wizlist(
    viewer: Option<Entity>,
    player_chars: &PlayerChars,
    linkdead: &Query<&Linkdead>,
    afk_chars: &HashSet<Entity>,
    res: &SessionRes,
    admins: &WizlistAdmins,
) -> String {
    let mut data = collect_who_data(player_chars, linkdead, afk_chars, res, viewer);
    data.retain(|d| d.key.is_admin);
    let observer = viewer
        .and_then(|v| player_chars.get(v).ok())
        .and_then(|(_, _, _, _, c, _, _)| c);
    let filtered = WizlistAdmins(
        admins
            .0
            .iter()
            .filter(|s| {
                crate::staff::stored_character(&res.persistence, s).visible_to(observer, false)
            })
            .cloned()
            .collect(),
    );
    // Keep this snapshot alive while its rows borrow restrings below.
    data.extend(offline_admin_rows(player_chars, res, &filtered));
    data.sort_by(|a, b| who_order(&a.key, &b.key));
    let rows: Vec<WhoRow> = data.into_iter().map(|d| d.row).collect();
    format_wizlist_list(&rows)
}

/// One [`WhoData`] per snapshot admin absent from the world: the same columns
/// as a live row (always `IMM`, registry abbrevs resolved the same way),
/// marked `(offline)` at render. Borrows the snapshot, so the resource must
/// outlive the returned rows — it does (the whole command answer).
fn offline_admin_rows<'a>(
    player_chars: &PlayerChars,
    res: &SessionRes,
    admins: &'a WizlistAdmins,
) -> Vec<WhoData<'a>> {
    admins
        .0
        .iter()
        .filter(|stored| {
            !player_chars
                .iter()
                .any(|(_, n, _, _, _, _, _)| n.0.eq_ignore_ascii_case(&stored.name))
        })
        .map(|stored| WhoData {
            key: WhoKey {
                is_admin: true,
                level: stored.level,
                connected_at: stored.created_at,
                sort_name: stored.name.to_lowercase(),
            },
            row: WhoRow {
                level: "IMM".to_string(),
                gender: gender_char(stored.gender).to_string(),
                race: res
                    .races
                    .get(&stored.race)
                    .map(|r| r.abbrev.clone())
                    .unwrap_or_default(),
                class: res
                    .classes
                    .get(&stored.class)
                    .map(|c| c.abbrev.clone())
                    .unwrap_or_default(),
                guild: String::new(),
                name: stored.name.clone(),
                title: stored.title.clone(),
                restrings: &stored.restrings,
                linkdead: false,
                afk: false,
                offline: true,
            },
        })
        .collect()
}

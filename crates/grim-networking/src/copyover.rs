//! Copyover (hot restart) wire types: the [`ConnectionResumed`] message the
//! transport raises for a re-adopted socket, and the serde
//! [`HandoverManifest`] / [`HandoverEntry`] payload carried alongside the live
//! socket fds. See `docs/DEPLOY.md`.

use bevy::prelude::*;
use std::net::SocketAddr;

/// A connection re-adopted from a previous process across a copyover. The
/// transport rebuilt the live socket and spawned a fresh [`Connection`] entity;
/// the session layer places `character` straight back into the world (skipping
/// the login flow) at its persisted `last_room`. See `docs/DEPLOY.md`.
///
/// [`Connection`]: crate::Connection
#[derive(Message, Debug)]
pub struct ConnectionResumed {
    pub connection: Entity,
    /// The character name that was in-game on this socket before the restart.
    pub character: String,
    /// Peer address of the re-adopted socket. Carried on the message (not
    /// joined off the `Connection` entity) so the resume gate never depends
    /// on a spawn having applied yet.
    pub addr: SocketAddr,
}

/// One in-game socket carried across a copyover: the character bound to it and
/// whether its terminal echo was hidden (password entry). The order of
/// [`HandoverManifest::entries`] matches the order the file descriptors are sent,
/// so index *i* of the manifest pairs with the *i*-th connection fd — no
/// transport-side connection id needs to survive the restart.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct HandoverEntry {
    pub character: String,
    pub echo_hidden: bool,
    /// Original client address, including when a trusted TLS proxy terminates TCP.
    #[serde(default)]
    pub peer_addr: Option<SocketAddr>,
}

/// The out-of-band payload sent alongside the live socket fds during a copyover.
/// Only in-game connections are carried; sessions still at the login prompt are
/// dropped and reconnect fresh.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default)]
pub struct HandoverManifest {
    pub entries: Vec<HandoverEntry>,
}

//! The [`Connection`] component — a live transport-level link, owned by a
//! transport crate and carrying nothing about the game.

use bevy::prelude::*;
use std::net::SocketAddr;

/// Negotiated terminal geometry, independent of which transport supplied it.
#[derive(Component, Debug, Clone, Copy)]
pub struct TerminalSize {
    pub columns: u16,
    pub rows: u16,
}

/// A raw network connection. Spawned by a transport.
/// The `id` maps to the transport-side connection identifier.
#[derive(Component, Debug)]
pub struct Connection {
    pub id: usize,
    pub addr: SocketAddr,
    /// Whether the server has sent IAC WILL ECHO (hidden input) for this connection.
    /// Reset to false when the user sends their next input.
    pub echo_hidden: bool,
}

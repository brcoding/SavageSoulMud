//! Transport-agnostic networking primitives.
//!
//! Owns the `Connection` component and the wire events every transport
//! speaks in terms of. A transport crate (e.g. `grim-networking-telnet`)
//! drives the socket and translates to/from these; nothing here knows about
//! sessions, scenes, or the game.
//!
//! The tokio bridge itself still lives in the telnet transport for now — it
//! moves here once a second transport exists and the shared shape is real
//! rather than guessed (see ARCHITECTURE.md §5.1).
//!
//! Concerns split into modules: [`connection`] (the `Connection` component),
//! [`messages`] (the bidirectional wire message/event types), [`copyover`]
//! (the hot-restart handover types), [`wiznet`] (admin alerts), and
//! [`plugin`] (message registration).

mod connection;
mod copyover;
mod messages;
mod plugin;
mod wiznet;

pub use connection::{Connection, TerminalSize};
pub use copyover::{ConnectionResumed, HandoverEntry, HandoverManifest};
pub use messages::{
    ConnectionClosed, ConnectionEstablished, ConnectionInput, ConnectionOutput, DisconnectRequest,
};
pub use plugin::GrimNetworkingPlugin;
pub use wiznet::{WiznetAlert, WiznetCategory};

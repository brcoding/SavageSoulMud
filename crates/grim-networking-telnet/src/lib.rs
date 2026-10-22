//! Telnet transport for GRIM.
//!
//! `TelnetPlugin` runs the TCP accept loop on a dedicated tokio runtime thread,
//! bridges it to Bevy's synchronous schedule over channels, negotiates the
//! negotiated Telnet options and structured client protocols, renders
//! colour codes to ANSI on the way out, and performs copyover (hot restart) fd
//! handoff to a successor process over a unix socket via `SCM_RIGHTS`.
//!
//! Modules, by concern:
//! - [`server`] — the TCP accept/serve loop and the tokio runtime thread.
//! - [`bridge`] — the tokio↔Bevy channel bridge, connection registry, and the
//!   two Bevy systems draining events and routing outbound messages.
//! - [`iac`] — bounded streaming Telnet frame decoding and IAC escaping.
//! - [`protocols`] — negotiated capabilities and out-of-band protocol messages.
//! - [`render`] — the ANSI render path for outbound text.
//! - [`copyover`] — `SIGUSR2` hot restart: listener + socket handoff.

mod bridge;
mod copyover;
mod drain;
mod guard;
mod iac;
mod limits;
mod plugin;
mod protocols;
mod render;
mod server;
mod shed;

pub use limits::TelnetLimits;
pub use plugin::TelnetPlugin;
pub use protocols::{
    TelnetCapabilities, TelnetInput, TelnetOutput, ATCP, CHARSET, EOR, GMCP, MSDP, MSP, MSSP, MXP,
    NAWS, NEW_ENVIRON, RAW_MARKUP, TTYPE,
};

pub use plugin::TelnetSystems;

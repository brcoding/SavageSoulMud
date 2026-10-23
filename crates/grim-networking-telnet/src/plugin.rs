//! The `TelnetPlugin`: registers the transport wire messages and resources, and
//! schedules the startup + update systems that own the tokio bridge and the
//! copyover lifecycle.

use bevy::prelude::*;
use grim_networking::{
    ConnectionClosed, ConnectionEstablished, ConnectionInput, ConnectionOutput, ConnectionResumed,
    DisconnectRequest, WiznetAlert,
};

use crate::bridge::{send_network_commands, send_protocol_commands, TelnetPort};
use crate::copyover::{
    finish_copyover, install_copyover_signal, poll_copyover_signal, trigger_copyover_on_due,
    CopyoverDone, CopyoverSignal,
};
use crate::drain::{drain_creations, drain_dependents, PendingDrain};
use crate::limits::TelnetLimits;
use crate::protocols::{TelnetInput, TelnetOutput};
use crate::server::start_telnet_server;

#[derive(SystemSet,Debug,Clone,PartialEq,Eq,Hash)]
pub enum TelnetSystems {Send}

pub struct TelnetPlugin {
    pub port: u16,
    pub limits: TelnetLimits,
}

impl TelnetPlugin {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            limits: TelnetLimits::default(),
        }
    }

    pub fn with_limits(mut self, limits: TelnetLimits) -> Self {
        self.limits = limits;
        self
    }
}

impl Plugin for TelnetPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ConnectionEstablished>()
            .add_message::<TelnetInput>()
            .add_message::<TelnetOutput>()
            .add_message::<ConnectionInput>()
            .add_message::<ConnectionClosed>()
            .add_message::<ConnectionOutput>()
            .add_message::<ConnectionResumed>()
            .add_message::<DisconnectRequest>()
            .add_message::<WiznetAlert>()
            .add_message::<grim_core::events::CopyoverDue>()
            .insert_resource(TelnetPort(self.port))
            .insert_resource(self.limits.clone())
            .init_resource::<CopyoverSignal>()
            .init_resource::<CopyoverDone>()
            .init_resource::<PendingDrain>()
            .add_systems(Startup, (install_copyover_signal, start_telnet_server))
            .add_systems(
                Update,
                (
                    // Two-phase drain: creations spawn first (applied before
                    // the next system), dependents resolve after.
                    drain_creations,
                    drain_dependents,
                    send_protocol_commands.in_set(TelnetSystems::Send),
                    send_network_commands.in_set(TelnetSystems::Send),
                    // Bridge an expired in-game `copyover` countdown into the
                    // same latched flag `SIGUSR2` raises; `poll` picks it up
                    // below (order is non-critical — the flag latches).
                    trigger_copyover_on_due,
                    poll_copyover_signal,
                    finish_copyover,
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telnet_plugin_new() {
        let plugin = TelnetPlugin::new(8080);
        assert_eq!(plugin.port, 8080);
        assert_eq!(plugin.limits.max_line_len, 1024);

        let plugin2 = TelnetPlugin::new(9090);
        assert_eq!(plugin2.port, 9090);
    }

    #[test]
    fn telnet_plugin_with_limits() {
        let limits = TelnetLimits {
            max_line_len: 512,
            ..TelnetLimits::default()
        };
        let plugin = TelnetPlugin::new(8080).with_limits(limits);
        assert_eq!(plugin.port, 8080);
        assert_eq!(plugin.limits.max_line_len, 512);
        assert_eq!(plugin.limits.max_buffer, 65536);
    }
}

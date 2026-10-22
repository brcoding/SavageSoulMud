//! The connection-established greeter: the entry point to the pre-game flow.
//! Spawns the session's [`Client`], prints the login banner, and issues the
//! first prompt.

use bevy::prelude::*;
use grim_core::components::Client;
use grim_networking::{
    admin_log, ConnectionEstablished, ConnectionOutput, DisconnectRequest, WiznetAlert,
    WiznetCategory,
};
use grim_persistence::BanList;
use grim_text::tr;

use crate::throttle::{ReconnectLimits, ReconnectThrottle};

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_connection_established(
    mut established: MessageReader<ConnectionEstablished>,
    mut commands: Commands,
    mut outputs: MessageWriter<ConnectionOutput>,
    bans: Res<BanList>,
    limits: Res<ReconnectLimits>,
    mut throttle: ResMut<ReconnectThrottle>,
    time: Res<Time>,
    mut disconnect: MessageWriter<DisconnectRequest>,
    mut alerts: MessageWriter<WiznetAlert>,
) {
    for ev in established.read() {
        // Banned IPs never get a session: refuse with the ban message and
        // sever the socket before any `Client` exists to drive it.
        if bans.is_ip_banned(&ev.addr.ip()) {
            outputs.write(ConnectionOutput::new(ev.connection, tr!("ban.banned.ip")));
            disconnect.write(DisconnectRequest {
                connection: ev.connection,
            });
            admin_log!(
                alerts,
                WiznetCategory::Security,
                "refused banned IP {}",
                ev.addr.ip()
            );
            continue;
        }
        // Reconnect floods never get a session either: same gate, one stage
        // down. Throttle state is per-IP attempt timestamps, so a distributed
        // flood passes here and meets the global shed instead.
        if throttle.check(&ev.addr.ip(), time.elapsed_secs_f64(), &limits) {
            outputs.write(ConnectionOutput::new(
                ev.connection,
                tr!("throttle.reconnect"),
            ));
            disconnect.write(DisconnectRequest {
                connection: ev.connection,
            });
            admin_log!(
                alerts,
                WiznetCategory::Security,
                "throttled reconnect flood from {}",
                ev.addr.ip()
            );
            continue;
        }
        commands.spawn(Client::new(ev.connection));
        // The connection banner is game branding. Character-specific accessible
        // output and its onboarding question remain at authenticated world entry.
        let banner = grim_color::ansi(include_str!("../../../assets/login-banner.txt"));
        let text = format!("{}\n\n{}", banner, tr!("login.prompt"));
        outputs.write(ConnectionOutput {
            echo: None,
            ..ConnectionOutput::new(ev.connection, text)
        });
    }
}

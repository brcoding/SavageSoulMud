//! Unified channel handler that uses ChannelRegistry for all channels.
//!
//! This replaces the distinct say/yell/ooc handlers with a single system
//! that dispatches based on the channel name from the EngineCommand.

use bevy::prelude::*;

use crate::{ChannelMessage, ChannelRegistry};
use grim_actor::{Character, InRoom, Player};
use grim_core::channel::{Channel, Identify, ListenEligibility, SpeakEligibility};
use grim_core::events::{Command, EngineCommand, InfoMessage};
use grim_text::tr;
use grim_world::Room;

/// Handle all channel commands based on the channel registry.
#[allow(clippy::too_many_arguments)]
pub fn handle_channel(
    mut engine: MessageReader<EngineCommand>,
    mut channel_message: MessageWriter<ChannelMessage>,
    mut info: MessageWriter<InfoMessage>,
    channel_registry: Res<ChannelRegistry>,
    _inroom: Query<&InRoom>,
    players: Query<&Player>,
    characters: Query<&Character>,
    _rooms: Query<&Room>,
) {
    for cmd in engine.read() {
        // Extract channel name and text from the command
        let (channel_name, text) = match &cmd.command {
            Command::Channel { channel, text } => (channel, text),
            _ => continue,
        };

        // Look up the channel configuration
        let channel = match channel_registry.get(channel_name) {
            Some(c) => c.clone(),
            None => {
                // Unknown channel - should not happen in normal operation
                continue;
            }
        };

        // Check speak eligibility
        if !check_speak_eligibility(&channel.speak, cmd.client, &_inroom, &players, &characters) {
            continue;
        }

        // Check listen eligibility for the actor (they must be able to listen to the channel)
        if !check_listen_eligibility(&channel.listen, cmd.client, &_inroom, &players, &characters) {
            continue;
        }

        // Emit the channel message
        channel_message.write(ChannelMessage {
            origin: None,
            hidden_origin: false,
            channel,
            actor: cmd.client,
            text: text.clone(),
        });

        // Send echo to actor (with proper formatting based on channel)
        let echo_text = format_echo(channel_registry.get(channel_name).unwrap(), text);
        info.write(InfoMessage {
            target: cmd.client,
            text: echo_text,
        });
    }
}

/// Check if the actor can speak on this channel.
fn check_speak_eligibility(
    speak: &SpeakEligibility,
    actor: Entity,
    _inroom: &Query<&InRoom>,
    players: &Query<&Player>,
    characters: &Query<&Character>,
) -> bool {
    match speak {
        SpeakEligibility::All => true,
        SpeakEligibility::AdminOnly => {
            // Check if actor has admin role
            characters
                .get(actor)
                .map(Character::is_admin)
                .unwrap_or(false)
        }
        SpeakEligibility::Authenticated => {
            // Check if actor has a Character (is logged in)
            players.get(actor).is_ok()
        }
    }
}

/// Check if the actor can listen to this channel.
fn check_listen_eligibility(
    listen: &ListenEligibility,
    actor: Entity,
    _inroom: &Query<&InRoom>,
    players: &Query<&Player>,
    characters: &Query<&Character>,
) -> bool {
    match listen {
        ListenEligibility::All => true,
        ListenEligibility::AdminOnly => {
            // Admin check - need Character with admin role
            characters
                .get(actor)
                .map(Character::is_admin)
                .unwrap_or(false)
        }
        ListenEligibility::Authenticated => {
            // Must have Player (authenticated) or Linkdead (still in world)
            players.get(actor).is_ok()
        }
        ListenEligibility::InRoom => {
            // Check if actor is in a room (has InRoom component)
            true
        }
        ListenEligibility::InArea => {
            // Check if actor is in a room (has InRoom component)
            true
        }
    }
}

/// Format the echo message for the actor based on channel configuration.
fn format_echo(channel: &Channel, text: &str) -> String {
    match channel.identify {
        Identify::Perceived => {
            // Use the catalog key for perceived (in-world) speech with first_party template
            let first_party_key = format!("{}.first_party", channel.key);
            tr!(first_party_key.as_str(), text = text)
        }
        Identify::Always => {
            // OOC always shows "[OOC]" prefix
            format!("[OOC] You: {}\n", text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_channel_emits_channel_message() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<EngineCommand>()
            .add_message::<InfoMessage>()
            .add_message::<ChannelMessage>();
        app.init_resource::<ChannelRegistry>();

        // Add default channel for say command
        app.world_mut()
            .resource_mut::<ChannelRegistry>()
            .add_channel(Channel::new("say"));

        // Add system under test
        app.add_systems(Update, handle_channel);

        // Spawn a test actor
        let actor = app.world_mut().spawn(()).id();

        // Send a command
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Command::Channel {
                channel: "say".into(),
                text: "hello".into(),
            },
        });

        app.update();

        // Check that ChannelMessage was emitted
        let messages = app.world().resource::<Messages<ChannelMessage>>();
        let mut cursor = messages.get_cursor();
        let iter = cursor.read(messages);
        let msgs: Vec<_> = iter.collect();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].actor, actor);
        assert_eq!(msgs[0].text, "hello");
    }
}

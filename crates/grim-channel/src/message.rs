//! Channel message event for all player-speech channels.
//!
//! This is the single shared event that replaces the distinct SayEvent,
//! YellEvent, and OocEvent. The channel configuration (scope, audience,
//! formatting) is determined by the channel the message was sent on.

use bevy::prelude::*;

use crate::Channel;

/// A message sent on a channel.
#[derive(Message, Debug)]
pub struct ChannelMessage {
    pub origin: Option<Entity>,
    pub hidden_origin: bool,
    /// The channel the message was sent on.
    pub channel: Channel,
    /// The actor who sent the message.
    pub actor: Entity,
    /// The message text.
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_message_implements_message() {
        fn assert_message<T: bevy::prelude::Message>() {}
        assert_message::<ChannelMessage>();
    }

    #[test]
    fn channel_message_implements_debug() {
        let channel = Channel::new("test");
        let msg = ChannelMessage {
            origin: None,
            hidden_origin: false,
            channel,
            actor: Entity::from_raw_u32(1).unwrap(),
            text: "hello".to_string(),
        };
        let _ = format!("{:?}", msg);
    }
}

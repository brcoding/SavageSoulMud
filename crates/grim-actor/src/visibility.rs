//! Shared room visibility for look, maps and game-owned combat context.
use bevy::prelude::*;
use grim_core::components::CarriedBy;
use grim_world::{RoomDarkness,LightSource};
use crate::InRoom;

#[derive(Component)]
pub struct DarkVision;

pub fn can_see(dark:bool,night_vision:bool,lit:bool)->bool {!dark||night_vision||lit}
#[derive(bevy::ecs::system::SystemParam)]
pub struct Visibility<'w,'s> {
    dark:Query<'w,'s,(),With<RoomDarkness>>,
    vision:Query<'w,'s,(),With<DarkVision>>,
    lights:Query<'w,'s,(Option<&'static InRoom>,Option<&'static CarriedBy>),With<LightSource>>,
    locations:Query<'w,'s,&'static InRoom>,
}
impl Visibility<'_,'_> {
    pub fn can_see(&self,actor:Entity,room:Entity)->bool {
        can_see(self.dark.get(room).is_ok(),self.vision.get(actor).is_ok(),self.lights.iter().any(|(placed,held)|placed.is_some_and(|p|p.room==room)||held.is_some_and(|h|h.carrier==actor||self.locations.get(h.carrier).is_ok_and(|p|p.room==room))))
    }
}

//! Room-transition watcher: match scripted creatures, run triggers, route output.
//!
//! For each transition event, every [`Creature`] in the affected room — except
//! the mover itself — fires its triggers for that moment in blueprint order.
//! Script speech goes out as a `say` [`ChannelMessage`] from the mob (rendered
//! by the existing channel path, so mobs sound exactly like players), with one
//! addition: attempt-time speech also reaches the mover directly, because the
//! room broadcast renders after placement, when the mover is already gone
//! (leave) or not yet there (enter). Any failure — a Lua error, or a missing
//! `say` channel — pages every online admin via [`InfoMessage`] and the server
//! log, and never disables anything.
use bevy::prelude::*;
use grim_actor::{
    in_room, AttemptEnter, AttemptLeave, AttemptWalk, Character, Creature, Enter, InRoom, Leave,
    Player,
};
use grim_channel::{ChannelMessage, ChannelRegistry};
use grim_core::components::Name as GrimName;
use grim_core::events::InfoMessage;
use grim_text::tr;

use crate::runtime::run_trigger;
use crate::trigger::{ScriptTriggers, TriggerKind};

/// The transition observers. One per moment (observers are concrete
/// functions), all funneling into [`fire`] so the matching rule lives in
/// exactly one place. Attempt observers run *synchronously* during the
/// movement pipeline — a `deny()` latches onto the attempt before anything
/// moves. Fact observers run post-placement.
#[allow(clippy::too_many_arguments)]
pub(crate) fn on_attempt_walk(
    mut trigger: On<AttemptWalk>,
    creatures: Query<(Entity, &ScriptTriggers, &InRoom), With<Creature>>,
    mob_names: Query<&GrimName>,
    admins: Query<(Entity, &Character, Option<&Player>)>,
    registry: Option<Res<ChannelRegistry>>,
    mut speech: MessageWriter<ChannelMessage>,
    mut info: MessageWriter<InfoMessage>,
) {
    let attempt = *trigger.event();
    if fire(
        TriggerKind::AttemptWalk,
        attempt.actor,
        attempt.room,
        &creatures,
        &mob_names,
        &admins,
        registry.as_deref(),
        &mut speech,
        &mut info,
    ) {
        trigger.event_mut().denied = true;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn on_attempt_leave(
    mut trigger: On<AttemptLeave>,
    creatures: Query<(Entity, &ScriptTriggers, &InRoom), With<Creature>>,
    mob_names: Query<&GrimName>,
    admins: Query<(Entity, &Character, Option<&Player>)>,
    registry: Option<Res<ChannelRegistry>>,
    mut speech: MessageWriter<ChannelMessage>,
    mut info: MessageWriter<InfoMessage>,
) {
    let attempt = *trigger.event();
    if fire(
        TriggerKind::AttemptLeave,
        attempt.actor,
        attempt.room,
        &creatures,
        &mob_names,
        &admins,
        registry.as_deref(),
        &mut speech,
        &mut info,
    ) {
        trigger.event_mut().denied = true;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn on_attempt_enter(
    mut trigger: On<AttemptEnter>,
    creatures: Query<(Entity, &ScriptTriggers, &InRoom), With<Creature>>,
    mob_names: Query<&GrimName>,
    admins: Query<(Entity, &Character, Option<&Player>)>,
    registry: Option<Res<ChannelRegistry>>,
    mut speech: MessageWriter<ChannelMessage>,
    mut info: MessageWriter<InfoMessage>,
) {
    let attempt = *trigger.event();
    if fire(
        TriggerKind::AttemptEnter,
        attempt.actor,
        attempt.room,
        &creatures,
        &mob_names,
        &admins,
        registry.as_deref(),
        &mut speech,
        &mut info,
    ) {
        trigger.event_mut().denied = true;
    }
}

pub(crate) fn on_leave(
    trigger: On<Leave>,
    creatures: Query<(Entity, &ScriptTriggers, &InRoom), With<Creature>>,
    mob_names: Query<&GrimName>,
    admins: Query<(Entity, &Character, Option<&Player>)>,
    registry: Option<Res<ChannelRegistry>>,
    mut speech: MessageWriter<ChannelMessage>,
    mut info: MessageWriter<InfoMessage>,
) {
    let fact = *trigger.event();
    fire(
        TriggerKind::Leave,
        fact.actor,
        fact.room,
        &creatures,
        &mob_names,
        &admins,
        registry.as_deref(),
        &mut speech,
        &mut info,
    );
}

pub(crate) fn on_enter(
    trigger: On<Enter>,
    creatures: Query<(Entity, &ScriptTriggers, &InRoom), With<Creature>>,
    mob_names: Query<&GrimName>,
    admins: Query<(Entity, &Character, Option<&Player>)>,
    registry: Option<Res<ChannelRegistry>>,
    mut speech: MessageWriter<ChannelMessage>,
    mut info: MessageWriter<InfoMessage>,
) {
    let fact = *trigger.event();
    fire(
        TriggerKind::Enter,
        fact.actor,
        fact.room,
        &creatures,
        &mob_names,
        &admins,
        registry.as_deref(),
        &mut speech,
        &mut info,
    );
}

/// Fire every trigger for `on` on each scripted creature standing in `room`,
/// skipping the mover itself (a mob never greets its own steps). Speech goes
/// out on the `say` channel; facts fire a tick after placement, so their
/// greetings land in a later flush than the arrival they react to.
#[allow(clippy::too_many_arguments)]
fn fire(
    on: TriggerKind,
    mover: Entity,
    room: Entity,
    creatures: &Query<(Entity, &ScriptTriggers, &InRoom), With<Creature>>,
    mob_names: &Query<&GrimName>,
    admins: &Query<(Entity, &Character, Option<&Player>)>,
    registry: Option<&ChannelRegistry>,
    speech: &mut MessageWriter<ChannelMessage>,
    info: &mut MessageWriter<InfoMessage>,
) -> bool {
    let channel = registry.and_then(|r| r.get("say")).cloned();
    // renders after placement, when the mover is already gone (leave) or not
    // yet there (enter). Facts need no catch-up — whoever is present hears them.
    let catch_up_mover = matches!(
        on,
        TriggerKind::AttemptWalk | TriggerKind::AttemptLeave | TriggerKind::AttemptEnter
    );
    let mut denied = false;
    for mob in in_room(
        room,
        creatures.iter().map(|(mob, _, ir)| (mob, ir.room)),
        Some(mover),
    ) {
        let Ok((_, triggers, _)) = creatures.get(mob) else {
            continue;
        };
        let mob_name = mob_names_or_entity(mob_names, mob);
        for trigger in &triggers.0 {
            if trigger.on != on {
                continue;
            }
            match run_trigger(&trigger.bytecode, on) {
                Ok(outcome) => {
                    if outcome.denied {
                        denied = true;
                    }
                    match &channel {
                        Some(say) => {
                            for text in outcome.said {
                                speech.write(ChannelMessage {
                                    origin: Some(mover),
                                    hidden_origin: admins.get(mover).is_ok_and(|(_,c,_)|c.is_hidden()),
                                    channel: say.clone(),
                                    actor: mob,
                                    text: text.clone(),
                                });
                                if catch_up_mover {
                                    // Same third-party framing the room hears
                                    // (`channel.say.third_party`), delivered as a
                                    // direct line since the mover is mid-transition.
                                    info.write(InfoMessage {
                                        target: mover,
                                        text: tr!(
                                            &format!("{}.third_party", say.key),
                                            speaker = mob_name,
                                            text = text
                                        ),
                                    });
                                }
                            }
                        }
                        None => report(
                            &mob_name,
                            on,
                            "the `say` channel is not registered",
                            admins,
                            info,
                        ),
                    }
                }
                Err(error) => report(&mob_name, on, &error, admins, info),
            }
        }
    }
    denied
}

/// A trigger failed: log it for the operator and page every online admin.
/// Nothing is disabled — the next transition fires the script again.
fn report(
    mob_name: &str,
    on: TriggerKind,
    error: &str,
    admins: &Query<(Entity, &Character, Option<&Player>)>,
    info: &mut MessageWriter<InfoMessage>,
) {
    bevy::log::error!(
        "script trigger failed: {mob_name} ({}): {error}",
        on.as_str()
    );
    let text = tr!(
        "script.trigger.failed",
        name = mob_name,
        trigger = on.as_str(),
        error = error
    );
    for (admin, character, player) in admins.iter() {
        if character.is_admin() && player.is_some() {
            info.write(InfoMessage {
                target: admin,
                text: text.clone(),
            });
        }
    }
}

/// The mob's display name, or its entity id when nameless (never empty — the
/// admin page must always identify the culprit).
fn mob_names_or_entity(names: &Query<&GrimName>, mob: Entity) -> String {
    names
        .get(mob)
        .map(|n| n.0.clone())
        .unwrap_or_else(|_| format!("entity:{}", mob.index()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trigger::{compile, CompiledTrigger};

    fn trigger(on: TriggerKind, script: &str) -> CompiledTrigger {
        CompiledTrigger {
            on,
            bytecode: compile(script).expect("fixture must compile"),
        }
    }

    fn scripted_mob(app: &mut App, room: Entity, triggers: Vec<CompiledTrigger>) -> Entity {
        app.world_mut()
            .spawn((
                Creature,
                GrimName("Grimmok".into()),
                InRoom { room },
                ScriptTriggers(triggers),
            ))
            .id()
    }

    fn spawned_character(app: &mut App, roles: Vec<grim_actor::Role>) -> Entity {
        let conn = app.world_mut().spawn_empty().id();
        let actor = app
            .world_mut()
            .spawn((
                Character {
                    credits: 100,
                    id: grim_core::id::GrimId::new(),
                    account_id: grim_core::id::GrimId::new(),
                    created_at: chrono::Utc::now(),
                    last_room: None,
                    roles,
                    class: String::new(),
                    title: None,
                    restrings: Default::default(),
                    config: Default::default(),
                    game_state: Default::default(),
                },
                GrimName("Root".into()),
                Player { connection: conn },
            ))
            .id();
        actor
    }

    fn said(app: &mut App) -> Vec<(Entity, String)> {
        let messages = app.world().resource::<Messages<ChannelMessage>>();
        let mut cursor = messages.get_cursor();
        cursor
            .read(messages)
            .map(|m| (m.actor, m.text.clone()))
            .collect()
    }

    fn infos(app: &mut App) -> Vec<(Entity, String)> {
        let messages = app.world().resource::<Messages<InfoMessage>>();
        let mut cursor = messages.get_cursor();
        cursor
            .read(messages)
            .map(|m| (m.target, m.text.clone()))
            .collect()
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<ChannelMessage>()
            .add_message::<InfoMessage>()
            .init_resource::<ChannelRegistry>()
            .add_observer(on_attempt_walk)
            .add_observer(on_attempt_leave)
            .add_observer(on_attempt_enter)
            .add_observer(on_leave)
            .add_observer(on_enter);
        app.world_mut()
            .resource_mut::<ChannelRegistry>()
            .add_channel(grim_channel::Channel {
                name: "say".to_string(),
                scope: grim_core::channel::Scope::Room,
                identify: grim_core::channel::Identify::Perceived,
                toggleable: false,
                speak: grim_core::channel::SpeakEligibility::All,
                listen: grim_core::channel::ListenEligibility::All,
                key: "channel.say".to_string(),
            });
        app
    }

    #[test]
    fn enter_fires_matching_trigger_in_arrival_room() {
        let mut app = test_app();
        let dest = app.world_mut().spawn_empty().id();
        let mob = scripted_mob(
            &mut app,
            dest,
            vec![trigger(TriggerKind::Enter, "self.say('hi')")],
        );
        let mover = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(Enter {
            actor: mover,
            room: dest,
        });
        app.update();
        assert_eq!(said(&mut app), [(mob, "hi".to_string())]);
    }

    #[test]
    fn wrong_room_and_wrong_moment_stay_silent() {
        let mut app = test_app();
        let here = app.world_mut().spawn_empty().id();
        let elsewhere = app.world_mut().spawn_empty().id();
        scripted_mob(
            &mut app,
            elsewhere,
            vec![trigger(TriggerKind::Enter, "self.say('hi')")],
        );
        scripted_mob(
            &mut app,
            here,
            vec![trigger(TriggerKind::Leave, "self.say('bye')")],
        );
        let mover = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(Enter {
            actor: mover,
            room: here,
        });
        app.update();
        assert!(said(&mut app).is_empty());
    }

    #[test]
    fn mover_never_fires_its_own_triggers() {
        let mut app = test_app();
        let room = app.world_mut().spawn_empty().id();
        let mover = app
            .world_mut()
            .spawn((
                Creature,
                GrimName("Restless".into()),
                InRoom { room },
                ScriptTriggers(vec![trigger(TriggerKind::Enter, "self.say('me')")]),
            ))
            .id();
        app.world_mut().trigger(Enter { actor: mover, room });
        app.update();
        assert!(said(&mut app).is_empty());
    }

    #[test]
    fn attempt_leave_fires_in_departure_room() {
        let mut app = test_app();
        let src = app.world_mut().spawn_empty().id();
        let mob = scripted_mob(
            &mut app,
            src,
            vec![trigger(TriggerKind::AttemptLeave, "self.say('bye')")],
        );
        let mover = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(AttemptLeave {
            actor: mover,
            room: src,
            denied: false,
        });
        app.update();
        assert_eq!(said(&mut app), [(mob, "bye".to_string())]);
    }

    #[test]
    fn attempt_speech_also_reaches_the_mover() {
        let mut app = test_app();
        let src = app.world_mut().spawn_empty().id();
        scripted_mob(
            &mut app,
            src,
            vec![trigger(TriggerKind::AttemptLeave, "self.say('bye')")],
        );
        let mover = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(AttemptLeave {
            actor: mover,
            room: src,
            denied: false,
        });
        app.update();
        let pages = infos(&mut app);
        assert_eq!(pages.len(), 1, "mover gets one direct line");
        assert_eq!(pages[0].0, mover);
        assert!(
            pages[0].1.contains("Grimmok"),
            "framed as speech: {}",
            pages[0].1
        );
        assert!(
            pages[0].1.contains("bye"),
            "carries the text: {}",
            pages[0].1
        );
    }

    #[test]
    fn walk_attempt_speech_also_reaches_the_mover() {
        let mut app = test_app();
        let src = app.world_mut().spawn_empty().id();
        scripted_mob(
            &mut app,
            src,
            vec![trigger(TriggerKind::AttemptWalk, "self.say('Hold!')")],
        );
        let mover = app.world_mut().spawn_empty().id();
        let mut attempt = AttemptWalk {
            actor: mover,
            room: src,
            direction: grim_core::cardinal::Cardinal::North,
            denied: false,
        };
        app.world_mut().trigger_ref(&mut attempt);
        app.update();
        let pages = infos(&mut app);
        assert_eq!(pages.len(), 1, "mover gets one direct line");
        assert_eq!(pages[0].0, mover);
        assert!(
            pages[0].1.contains("Hold!"),
            "carries the text: {}",
            pages[0].1
        );
    }

    #[test]
    fn committed_leave_does_not_page_the_mover() {
        let mut app = test_app();
        let src = app.world_mut().spawn_empty().id();
        scripted_mob(
            &mut app,
            src,
            vec![trigger(TriggerKind::Leave, "self.say('bye')")],
        );
        let mover = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(Leave {
            actor: mover,
            room: src,
        });
        app.update();
        assert!(infos(&mut app).is_empty(), "facts speak to the room only");
    }

    #[test]
    fn failing_script_pages_online_admins() {
        let mut app = test_app();
        let room = app.world_mut().spawn_empty().id();
        scripted_mob(&mut app, room, vec![trigger(TriggerKind::Enter, "nope()")]);
        let admin = spawned_character(&mut app, vec![grim_actor::Role::Admin]);
        let mover = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(Enter { actor: mover, room });
        app.update();
        let pages = infos(&mut app);
        assert_eq!(pages.len(), 1, "exactly one admin page");
        assert_eq!(pages[0].0, admin, "paged to the admin");
        assert!(
            pages[0].1.contains("Grimmok"),
            "names the mob: {}",
            pages[0].1
        );
        assert!(
            pages[0].1.contains("enter"),
            "names the moment: {}",
            pages[0].1
        );
    }

    #[test]
    fn linkdead_admin_and_players_get_no_page() {
        let mut app = test_app();
        let room = app.world_mut().spawn_empty().id();
        scripted_mob(&mut app, room, vec![trigger(TriggerKind::Enter, "nope()")]);
        // Admin without a `Player` (linkdead): the page targets live sessions.
        app.world_mut().spawn((
            GrimName("Gone".into()),
            Character {
                credits: 100,
                id: grim_core::id::GrimId::new(),
                account_id: grim_core::id::GrimId::new(),
                created_at: chrono::Utc::now(),
                last_room: None,
                roles: vec![grim_actor::Role::Admin],
                class: String::new(),
                title: None,
                restrings: Default::default(),
                config: Default::default(),
                game_state: Default::default(),
            },
        ));
        spawned_character(&mut app, Vec::new());
        let mover = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(Enter { actor: mover, room });
        app.update();
        assert!(infos(&mut app).is_empty());
    }

    #[test]
    fn lua_deny_latches_the_attempt() {
        let mut app = test_app();
        let src = app.world_mut().spawn_empty().id();
        scripted_mob(
            &mut app,
            src,
            vec![trigger(
                TriggerKind::AttemptLeave,
                "self.say('Hold!') event:deny()",
            )],
        );
        let mover = app.world_mut().spawn_empty().id();
        let mut attempt = AttemptLeave {
            actor: mover,
            room: src,
            denied: false,
        };
        app.world_mut().trigger_ref(&mut attempt);
        assert!(attempt.denied, "script deny() must latch the attempt");
        let texts: Vec<String> = said(&mut app).into_iter().map(|(_, t)| t).collect();
        assert_eq!(texts, ["Hold!"], "refusal echo still goes out");
    }
}

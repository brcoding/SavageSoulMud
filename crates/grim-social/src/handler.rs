//! The social handler: resolve `Command::Social` against the actor's
//! roommates and render per-recipient [`InfoMessage`]s — the actor's wording,
//! the target's (when there is one), and the room's. The same primitive `tell`
//! uses; the future `act()` extraction (#58) absorbs this fan-out.

use bevy::prelude::*;
use grim_actor::{in_room, Actor, InRoom};
use grim_command::CommandRegistry;
use grim_core::character::Gender;
use grim_core::components::Name;
use grim_core::events::{Command, EngineCommand, InfoMessage};
use grim_target::rank_match;

use crate::social::SocialRegistry;

/// A social was performed and rendered. A fact for logging/moderation
/// observers; nothing reads it yet. A future mute/gag veto would hang its
/// attempt off this point.
#[derive(Message, Debug)]
pub struct SocialPerformed {
    pub actor: Entity,
    pub name: String,
    pub target: Option<Entity>,
}

#[allow(clippy::too_many_arguments)] // reason: Bevy system params; bundling would hide the queries
pub(crate) fn handle_social(
    mut engine: MessageReader<EngineCommand>,
    registry: Res<SocialRegistry>,
    commands: Option<Res<CommandRegistry<Command>>>,
    inroom: Query<&InRoom>,
    occupants: Query<(Entity, &InRoom, &Name)>,
    names: Query<&Name>,
    beings: Query<&Actor>,
    visibility: grim_actor::visibility::Visibility,
    mut info: MessageWriter<InfoMessage>,
    mut performed: MessageWriter<SocialPerformed>,
) {
    for cmd in engine.read() {
        if matches!(cmd.command, Command::SocialList) {
            // Absent registry (a partial composition without ScenePlugin):
            // header-only rather than a panic. Production always has it.
            let social_names = commands
                .as_ref()
                .map(|r| r.names_in_section("social"))
                .unwrap_or_default();
            let grid = grim_text::column_grid(&social_names, 80);
            send(
                &mut info,
                cmd.client,
                &("Available socials:\n".to_string() + &grid),
            );
            continue;
        }
        let Command::Social { name, target } = &cmd.command else {
            continue;
        };
        let actor = cmd.client;
        let Some(def) = registry.get(name) else {
            send(
                &mut info,
                actor,
                &grim_text::tr("error.unknown_command", &[]),
            );
            continue;
        };
        let Ok(actor_room) = inroom.get(actor).map(|ir| ir.room) else {
            continue;
        };
        let Ok(actor_name) = names.get(actor).map(|n| n.0.clone()) else {
            continue;
        };
        let actor_gender = gender_of(&beings, actor);
        let here: Vec<_> = roommates(&occupants, actor_room)
            .into_iter()
            .filter(|(e, _)| visibility.sees_actor(actor, *e) && visibility.sees_actor(*e, actor))
            .collect();
        let word = target.as_deref().and_then(|t| t.split_whitespace().next());
        let Some(word) = word else {
            emit_solo(&mut info, def, actor, &actor_name, actor_gender, &here);
            performed.write(SocialPerformed {
                actor,
                name: name.clone(),
                target: None,
            });
            continue;
        };
        let Some(recipient) = resolve_target(&here, word, actor) else {
            send(
                &mut info,
                actor,
                &grim_text::tr("social.error.unknown_target", &[]),
            );
            continue;
        };
        if recipient == actor {
            emit_self(&mut info, def, actor, &actor_name, actor_gender, &here);
        } else {
            let target_name = names
                .get(recipient)
                .map(|n| n.0.clone())
                .unwrap_or_default();
            emit_other(
                &mut info,
                def,
                actor,
                &actor_name,
                actor_gender,
                recipient,
                &target_name,
                gender_of(&beings, recipient),
                &here,
            );
        }
        performed.write(SocialPerformed {
            actor,
            name: name.clone(),
            target: Some(recipient),
        });
    }
}

/// Everyone standing in `room`: entity + display name. Funnels through the
/// shared [`in_room`](grim_actor::in_room) helper (mapped from this handler's
/// narrow tuple) instead of a local filter.
fn roommates(occupants: &Query<(Entity, &InRoom, &Name)>, room: Entity) -> Vec<(Entity, String)> {
    let here = in_room(room, occupants.iter().map(|(e, ir, _)| (e, ir.room)), None);
    here.into_iter()
        .filter_map(|e| occupants.get(e).map(|(_, _, n)| (e, n.0.clone())).ok())
        .collect()
}

/// `self` names the actor; otherwise rank-match the word among roommates
/// (exact beats prefix, ties to the lowest entity — the `tell` rule).
fn resolve_target(here: &[(Entity, String)], word: &str, actor: Entity) -> Option<Entity> {
    if word.eq_ignore_ascii_case("self") {
        return Some(actor);
    }
    let want = word.to_lowercase();
    here.iter()
        .filter_map(|(e, n)| {
            rank_match(n, &[], std::slice::from_ref(&want)).map(|rank| (rank, e.to_bits(), *e))
        })
        .min()
        .map(|(_, _, e)| e)
}

/// Solo: the actor's wording to them, the room's to everyone else here.
fn emit_solo(
    info: &mut MessageWriter<InfoMessage>,
    def: &crate::SocialDef,
    actor: Entity,
    actor_name: &str,
    actor_gender: Gender,
    here: &[(Entity, String)],
) {
    let target_gender = Gender::default();
    send(
        info,
        actor,
        &render(
            def,
            "solo",
            "actor",
            actor_name,
            actor_gender,
            "",
            target_gender,
        ),
    );
    for (entity, _) in here {
        if *entity != actor {
            send(
                info,
                *entity,
                &render(
                    def,
                    "solo",
                    "room",
                    actor_name,
                    actor_gender,
                    "",
                    target_gender,
                ),
            );
        }
    }
}

/// Self-targeted: the self case, with the same actor/room split.
fn emit_self(
    info: &mut MessageWriter<InfoMessage>,
    def: &crate::SocialDef,
    actor: Entity,
    actor_name: &str,
    actor_gender: Gender,
    here: &[(Entity, String)],
) {
    let target_gender = Gender::default();
    send(
        info,
        actor,
        &render(
            def,
            "self",
            "actor",
            actor_name,
            actor_gender,
            "",
            target_gender,
        ),
    );
    for (entity, _) in here {
        if *entity != actor {
            send(
                info,
                *entity,
                &render(
                    def,
                    "self",
                    "room",
                    actor_name,
                    actor_gender,
                    "",
                    target_gender,
                ),
            );
        }
    }
}

/// Targeted at another: actor, target, and room wordings.
#[allow(clippy::too_many_arguments)] // reason: per-audience render needs both parties' names + genders
fn emit_other(
    info: &mut MessageWriter<InfoMessage>,
    def: &crate::SocialDef,
    actor: Entity,
    actor_name: &str,
    actor_gender: Gender,
    recipient: Entity,
    target_name: &str,
    target_gender: Gender,
    here: &[(Entity, String)],
) {
    send(
        info,
        actor,
        &render(
            def,
            "other",
            "actor",
            actor_name,
            actor_gender,
            target_name,
            target_gender,
        ),
    );
    send(
        info,
        recipient,
        &render(
            def,
            "other",
            "target",
            actor_name,
            actor_gender,
            target_name,
            target_gender,
        ),
    );
    for (entity, _) in here {
        if *entity != actor && *entity != recipient {
            send(
                info,
                *entity,
                &render(
                    def,
                    "other",
                    "room",
                    actor_name,
                    actor_gender,
                    target_name,
                    target_gender,
                ),
            );
        }
    }
}

/// Four pronoun forms for one party, read off their gender. Templates are
/// authored male-assumed (`%{actor.him}`) and translated here.
struct Pronouns {
    subj: &'static str,
    obj: &'static str,
    poss: &'static str,
    refl: &'static str,
}

fn pronouns(gender: Gender) -> Pronouns {
    match gender {
        Gender::Male => Pronouns {
            subj: "he",
            obj: "him",
            poss: "his",
            refl: "himself",
        },
        Gender::Female => Pronouns {
            subj: "she",
            obj: "her",
            poss: "her",
            refl: "herself",
        },
        Gender::Neutral => Pronouns {
            subj: "they",
            obj: "them",
            poss: "their",
            refl: "themselves",
        },
    }
}

/// A being's gender, or Neutral when it has no `Actor` (fail soft — a missing
/// being component must never break rendering).
fn gender_of(beings: &Query<&Actor>, entity: Entity) -> Gender {
    beings.get(entity).map(|a| a.gender).unwrap_or_default()
}

fn render(
    def: &crate::SocialDef,
    case: &str,
    audience: &str,
    actor: &str,
    actor_gender: Gender,
    target: &str,
    target_gender: Gender,
) -> String {
    let a = pronouns(actor_gender);
    let t = pronouns(target_gender);
    grim_text::render(
        &def.template(case, audience),
        &[
            ("actor", actor),
            ("target", target),
            ("actor.he", a.subj),
            ("actor.him", a.obj),
            ("actor.his", a.poss),
            ("actor.self", a.refl),
            ("target.he", t.subj),
            ("target.him", t.obj),
            ("target.his", t.poss),
            ("target.self", t.refl),
        ],
    )
}

fn send(info: &mut MessageWriter<InfoMessage>, target: Entity, text: &str) {
    info.write(InfoMessage {
        target,
        text: text.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use grim_actor::{Actor, Character, Player};
    use grim_command::CommandRegistry;
    use grim_core::character::Gender;
    use grim_core::events::Command as Cmd;
    use grim_core::GrimId;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<EngineCommand>();
        app.add_message::<InfoMessage>();
        app.add_message::<SocialPerformed>();
        app.insert_resource(SocialRegistry::default());
        app.insert_resource(CommandRegistry::<Cmd>::new());
        app.add_systems(Update, handle_social);
        app
    }

    fn character() -> Character {
        Character {
            credits: 100,
            id: GrimId::new(),
            account_id: GrimId::new(),
            created_at: chrono::Utc::now(),
            last_room: None,
            roles: Vec::new(),
            class: String::new(),
            title: None,
            restrings: std::collections::HashMap::new(),
            config: std::collections::HashMap::new(),
            game_state: Default::default(),
        }
    }

    fn spawn_being(app: &mut App, name: &str, room: Entity, gender: Gender) -> Entity {
        app.world_mut()
            .spawn((
                Name(name.into()),
                Actor {
                    race: "human".into(),
                    level: 1,
                    gender,
                },
                character(),
                Player {
                    connection: Entity::PLACEHOLDER,
                },
                InRoom { room },
            ))
            .id()
    }

    /// Two rooms: Alice/Bob/Carol together, Dave elsewhere.
    fn setup() -> (App, Entity, Entity, Entity) {
        let mut app = test_app();
        let room = app.world_mut().spawn_empty().id();
        let elsewhere = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<SocialRegistry>()
            .insert(crate::SocialDef::new("grin"));
        let alice = spawn_being(&mut app, "Alice", room, Gender::Neutral);
        let bob = spawn_being(&mut app, "Bob", room, Gender::Male);
        let carol = spawn_being(&mut app, "Carol", room, Gender::Female);
        let _dave = spawn_being(&mut app, "Dave", elsewhere, Gender::Neutral);
        (app, alice, bob, carol)
    }

    #[test]
    fn social_list_shows_section_grid() {
        let (mut app, alice, _bob, _carol) = setup();
        {
            let mut reg = app.world_mut().resource_mut::<CommandRegistry<Cmd>>();
            reg.register("say", |_| Some(Cmd::Say { text: "x".into() }));
            reg.register("grin", |_| None);
            reg.set_section("grin", "social");
        }
        app.world_mut().write_message(EngineCommand {
            client: alice,
            command: Cmd::SocialList,
        });
        app.update();
        assert_eq!(by_target(&app, alice), vec!["Available socials:\ngrin\n"]);
    }

    fn emit(app: &mut App, actor: Entity, name: &str, target: Option<&str>) {
        app.world_mut().write_message(EngineCommand {
            client: actor,
            command: Cmd::Social {
                name: name.into(),
                target: target.map(str::to_string),
            },
        });
        app.update();
    }

    fn infos(app: &App) -> Vec<(Entity, String)> {
        let m = app.world().resource::<Messages<InfoMessage>>();
        let mut c = m.get_cursor();
        c.read(m).map(|i| (i.target, i.text.clone())).collect()
    }

    fn by_target(app: &App, target: Entity) -> Vec<String> {
        infos(app)
            .into_iter()
            .filter(|(e, _)| *e == target)
            .map(|(_, t)| t)
            .collect()
    }

    #[test]
    fn solo_reaches_actor_and_room_only() {
        let (mut app, alice, bob, carol) = setup();
        emit(&mut app, alice, "grin", None);
        assert_eq!(by_target(&app, alice), vec!["You grin.\n"]);
        assert_eq!(by_target(&app, bob), vec!["Alice grins.\n"]);
        assert_eq!(by_target(&app, carol), vec!["Alice grins.\n"]);
        // Dave is in another room: nothing. Three messages total.
        assert_eq!(infos(&app).len(), 3);
    }

    #[test]
    fn other_target_reaches_all_three_audiences() {
        let (mut app, alice, bob, carol) = setup();
        emit(&mut app, alice, "grin", Some("bob"));
        assert_eq!(by_target(&app, alice), vec!["You grin at Bob.\n"]);
        assert_eq!(by_target(&app, bob), vec!["Alice grins at you.\n"]);
        assert_eq!(by_target(&app, carol), vec!["Alice grins at Bob.\n"]);
    }

    #[test]
    fn self_keyword_uses_self_case() {
        let (mut app, alice, bob, _carol) = setup();
        emit(&mut app, alice, "grin", Some("self"));
        assert_eq!(by_target(&app, alice), vec!["You grin to yourself.\n"]);
        assert_eq!(by_target(&app, bob), vec!["Alice grins to themselves.\n"]);
    }

    #[test]
    fn pronouns_cover_all_genders() {
        assert_eq!(pronouns(Gender::Male).refl, "himself");
        assert_eq!(pronouns(Gender::Female).refl, "herself");
        assert_eq!(pronouns(Gender::Neutral).refl, "themselves");
        let f = pronouns(Gender::Female);
        assert_eq!((f.subj, f.obj, f.poss), ("she", "her", "her"));
        let m = pronouns(Gender::Male);
        assert_eq!((m.subj, m.obj, m.poss), ("he", "him", "his"));
    }

    #[test]
    fn self_room_uses_actor_gender() {
        let (mut app, _alice, bob, carol) = setup();
        // Bob is male, Carol female; the room wording follows each actor.
        emit(&mut app, bob, "grin", Some("self"));
        emit(&mut app, carol, "grin", Some("self"));
        assert!(by_target(&app, _alice).contains(&"Bob grins to himself.\n".to_string()));
        assert!(by_target(&app, _alice).contains(&"Carol grins to herself.\n".to_string()));
    }

    #[test]
    fn file_template_target_pronouns_translate() {
        let (mut app, alice, bob, carol) = setup();
        app.world_mut()
            .resource_mut::<SocialRegistry>()
            .insert(crate::SocialDef {
                name: "prod".into(),
                overrides: crate::social::SocialFile {
                    with_target: crate::social::OtherCase {
                        room: Some("%{actor} prods %{target.him}.\n".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            });
        // Bob is male: the bystander sees "him"; retarget Carol for "her".
        emit(&mut app, alice, "prod", Some("bob"));
        emit(&mut app, alice, "prod", Some("carol"));
        let to_carol = by_target(&app, carol);
        assert!(
            to_carol.contains(&"Alice prods him.\n".to_string()),
            "{to_carol:?}"
        );
        let to_bob = by_target(&app, bob);
        assert!(
            to_bob.contains(&"Alice prods her.\n".to_string()),
            "{to_bob:?}"
        );
    }
    #[test]
    fn own_name_resolves_to_self_case() {
        let (mut app, alice, _bob, _carol) = setup();
        emit(&mut app, alice, "grin", Some("alice"));
        assert_eq!(by_target(&app, alice), vec!["You grin to yourself.\n"]);
    }

    #[test]
    fn target_prefix_matches_case_insensitively() {
        let (mut app, alice, bob, _carol) = setup();
        emit(&mut app, alice, "grin", Some("BO"));
        assert_eq!(by_target(&app, bob), vec!["Alice grins at you.\n"]);
    }

    #[test]
    fn unknown_target_messages_actor_only() {
        let (mut app, alice, bob, carol) = setup();
        emit(&mut app, alice, "grin", Some("xyzzy"));
        assert_eq!(
            by_target(&app, alice),
            vec!["You don't see anyone by that name here.\n"]
        );
        assert!(by_target(&app, bob).is_empty());
        assert!(by_target(&app, carol).is_empty());
    }

    #[test]
    fn unknown_social_answers_unknown_command() {
        let (mut app, alice, _bob, _carol) = setup();
        emit(&mut app, alice, "dinner", None);
        assert_eq!(
            by_target(&app, alice),
            vec![grim_text::tr("error.unknown_command", &[])]
        );
    }

    #[test]
    fn actor_names_cannot_inject_colour() {
        let (mut app, alice, bob, _carol) = setup();
        app.world_mut()
            .entity_mut(alice)
            .insert(Name("{REve".into()));
        emit(&mut app, alice, "grin", None);
        // `{` doubles on escape, so the markup arrives inert.
        assert_eq!(by_target(&app, bob), vec!["{{REve grins.\n"]);
    }

    #[test]
    fn performed_fact_fires_per_case() {
        let (mut app, alice, bob, _carol) = setup();
        emit(&mut app, alice, "grin", None);
        emit(&mut app, alice, "grin", Some("self"));
        emit(&mut app, alice, "grin", Some("bob"));
        let m = app.world().resource::<Messages<SocialPerformed>>();
        let mut c = m.get_cursor();
        let facts: Vec<(String, Option<Entity>)> =
            c.read(m).map(|f| (f.name.clone(), f.target)).collect();
        assert_eq!(
            facts,
            vec![
                ("grin".into(), None),
                ("grin".into(), Some(alice)),
                ("grin".into(), Some(bob)),
            ]
        );
    }

    #[test]
    fn closed_over_factory_parses_solo_and_targeted() {
        // The plugin's Startup registration closes over each social name;
        // prove the closed-over factory parses the same way here.
        let mut reg = CommandRegistry::<Cmd>::new();
        for name in ["grin", "smile"] {
            let owned = name.to_string();
            reg.register(name, move |rest| {
                Some(Cmd::Social {
                    name: owned.clone(),
                    target: rest.split_whitespace().next().map(str::to_string),
                })
            });
            reg.deprioritize(name);
        }
        assert_eq!(
            reg.resolve("grin", ""),
            Some(Cmd::Social {
                name: "grin".into(),
                target: None
            })
        );
        assert_eq!(
            reg.resolve("grin", "bob loudly"),
            Some(Cmd::Social {
                name: "grin".into(),
                target: Some("bob".into())
            })
        );
    }
}

//! End-to-end scenarios for example-mud, driven through the headless harness.
//!
//! Each test boots a fresh MUD (isolated temp data dir) and drives it as a
//! telnet user would — creating accounts, characters, moving, talking — then
//! asserts on the recorded output. Multiple related steps live in one test on
//! purpose: the flow IS the thing under test.

mod harness;
use grim::components::Gender;
use grim::IdleConfig;
use grim::Role;
use grim::TriggerKind;
use harness::{Mud, Session};

/// A password that satisfies validation (short ones are rejected — see
/// `password_must_be_valid`).
const PW: &str = "secretpw";

/// Map glyph colours (`grim_world::render_map` markup, asserted literally —
/// the harness reads pre-render `ConnectionOutput` text).
const ME: &str = "{R@@{x";
const RM: &str = "{go{x";
const HL: &str = "{w-{x";
const VL: &str = "{w|{x";

/// Create a brand-new account + character and enter the world. Leaves the
/// session in-game, standing in the starting room.
fn create_char(mud: &mut Mud, email: &str, name: &str) -> Session {
    let (s, _) = mud.connect();
    let _ = mud.send(s, email); // unknown email → offered account creation
    let _ = mud.send(s, "y"); //   confirm create
    let _ = mud.send(s, PW); //    choose password → account created, character menu
    let _ = mud.send(s, "c"); //   create a character
    let _ = mud.send(s, name); //  name it → gender picker
    let _ = mud.send(s, "1"); //   gender: Male (menu index)
    let _ = mud.send(s, "human"); // race: by slug
    let _ = mud.send(s, "warrior"); // class: by slug → MOTD
                                    // Press enter at the MOTD → enter the world. Assert we actually landed in
                                    // a room, so a broken login/creation step can't return a bogus Session.
    mud.send(s, "").assert_contains("Exits:");
    s
}

#[test]
fn privy_door_blocks_until_opened() {
    // #143 slice: the tavern's east exit hides behind a closed privy door.
    // Walking east is refused, `open east` flips both sides and announces
    // per-recipient, then the walk lands in the privy.
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");

    // Closed: the walk is refused and Alice stays in the tavern.
    mud.send(alice, "east").assert_contains("is closed");
    mud.send(alice, "look")
        .assert_contains("Exits: {gnorth{x {gsouth{x {gup{x  Doors: {geast{x");

    // Bob waits in the tavern: he witnesses Alice's open.
    let opened = mud.send(alice, "open east");
    opened.assert_contains("You open the privy door to the east.");
    mud.send(bob, "look")
        .assert_contains("Alice opens the privy door to the east.");

    // An opened door rejoins the Exits line and leaves the Doors line.
    mud.send(alice, "look")
        .assert_contains("Exits: {gnorth{x {geast{x {gsouth{x {gup{x  Doors: none");

    // Now the walk lands in the privy; closing from inside announces west.
    mud.send(alice, "east").assert_contains("The Privy");
    mud.send(alice, "close west")
        .assert_contains("You close the privy door to the west.");
}

#[test]
fn secret_cellar_door_hides_until_guessed() {
    // The tavern's down exit is a closed secret: walking it refuses like a
    // non-exit (no leak), `look` hides it, and `open down` still works —
    // discovery is by guessing, then walking through the opened door.
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    // No leak: closed-secret refusal is byte-identical to a non-exit.
    mud.send(alice, "down")
        .assert_contains("You can't go that way.");
    mud.send(alice, "look")
        .assert_contains("Exits: {gnorth{x {gsouth{x {gup{x  Doors: {geast{x")
        .assert_excludes("down");

    // Guessing `open down` operates the hidden door normally, and the opened
    // secret joins the Exits line like any open door.
    mud.send(alice, "open down")
        .assert_contains("You open the cellar door to the down.");
    mud.send(alice, "look")
        .assert_contains("{gdown{x  Doors: {geast");

    // The walk lands in the cellar; the guessed-open mirrored both sides, so
    // the visible up door lists under Exits.
    mud.send(alice, "down").assert_contains("Dusty Cellar");
    mud.send(alice, "look")
        .assert_contains("Exits: {gup{x  Doors: none");
    // And the loft above is a plain exit walk away.
    mud.send(alice, "up").assert_contains("The Rusted Anvil");
    mud.send(alice, "up").assert_contains("Tavern Loft");
}

#[test]
fn connect_shows_login_banner_and_prompt() {
    let mut mud = Mud::new();
    let (_s, banner) = mud.connect();
    banner.assert_contains("Character name or account email");
}

#[test]
fn account_creation_places_character_in_the_world() {
    let mut mud = Mud::new();
    assert!(!mud.character_names().contains(&"Alice".to_string()));

    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    // The character now exists and is standing in the seeded starting room.
    // The room name alone proves placement; the listing format is pinned by
    // the look/door scenario tests, not here.
    assert!(mud.character_names().contains(&"Alice".to_string()));
    mud.send(alice, "look").assert_contains("The Rusted Anvil");
}

#[test]
fn character_creation_records_gender_race_and_class_at_level_one() {
    let mut mud = Mud::new();
    let (s, _) = mud.connect();
    let _ = mud.send(s, "gwen@example.com");
    let _ = mud.send(s, "y");
    let _ = mud.send(s, PW);
    let _ = mud.send(s, "c");
    let _ = mud.send(s, "Gwen"); // → gender picker

    // Mix input styles: gender by name prefix, race by index, class by slug.
    mud.send(s, "fem").assert_contains("Choose a race"); // Female by prefix
    mud.send(s, "2").assert_contains("Choose a class"); // Race index 2 → Elf
    let _ = mud.send(s, "mage"); // Class by slug → MOTD
    mud.send(s, "").assert_contains("Exits:"); // enter the world

    let gwen = mud.character("Gwen").expect("Gwen is in the world");
    assert_eq!(gwen.gender, Gender::Female);
    assert_eq!(gwen.race, "elf");
    assert_eq!(gwen.class, "mage");
    assert_eq!(gwen.level, 1);
}

#[test]
fn invalid_creation_pick_reprompts_without_advancing() {
    let mut mud = Mud::new();
    let (s, _) = mud.connect();
    let _ = mud.send(s, "ivy@example.com");
    let _ = mud.send(s, "y");
    let _ = mud.send(s, PW);
    let _ = mud.send(s, "c");
    let _ = mud.send(s, "Ivy"); // → gender picker

    // Out-of-range index is rejected and the gender menu is shown again.
    mud.send(s, "9")
        .assert_contains("Please choose one of the options")
        .assert_contains("Choose a gender");
    // A valid pick then advances.
    mud.send(s, "1").assert_contains("Choose a race");
    // A tier-2 class is NOT offered/creatable: its slug does not resolve.
    let _ = mud.send(s, "human");
    mud.send(s, "champion")
        .assert_contains("Please choose one of the options")
        .assert_contains("Choose a class");
    let _ = mud.send(s, "warrior"); // valid tier-1 → MOTD
    mud.send(s, "").assert_contains("Exits:");

    let ivy = mud.character("Ivy").expect("Ivy is in the world");
    assert_eq!(ivy.class, "warrior");
}

#[test]
fn legacy_character_is_routed_through_the_picker_at_login() {
    let mut mud = Mud::new();
    // Create Nomad normally, then log out so the character lives only on disk.
    let nomad = create_char(&mut mud, "nomad@example.com", "Nomad");
    let _ = mud.send(nomad, "quit");
    mud.disconnect(nomad);

    // Simulate a character created before races/classes existed: clear its
    // on-disk race/class. The account still owns it.
    mud.make_character_legacy("Nomad");

    // Log back in and open the menu.
    let (again, _) = mud.connect();
    let _ = mud.send(again, "nomad@example.com");
    mud.send(again, PW).assert_contains("Nomad");

    // Selecting the legacy character does NOT drop into the world — it opens the
    // gender picker instead.
    mud.send(again, "1").assert_contains("Choose a gender");
    mud.send(again, "male").assert_contains("Choose a race");
    mud.send(again, "human").assert_contains("Choose a class");
    // Picking the class backfills the build and enters the world (MOTD → room).
    let _ = mud.send(again, "warrior");
    mud.send(again, "").assert_contains("Exits:");

    // The build is now recorded, and level is still 1 (no XP system).
    let nomad = mud.character("Nomad").expect("Nomad is in the world");
    assert_eq!(nomad.gender, Gender::Male);
    assert_eq!(nomad.race, "human");
    assert_eq!(nomad.class, "warrior");
    assert_eq!(nomad.level, 1);
}

#[test]
fn movement_walks_between_seeded_rooms() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    // Room names alone prove the walks; the listing format is pinned by the
    // look/door scenario tests, not here.
    mud.send(alice, "north").assert_contains("Town Square");
    mud.send(alice, "south").assert_contains("The Rusted Anvil");
}

#[test]
fn recall_returns_to_the_town_square() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    // Walk north into the Town Square; recall there is a no-op with a reply.
    mud.send(alice, "north").assert_contains("Town Square");
    mud.send(alice, "recall")
        .assert_contains("You are already there.");
    // Walk away, then recall back: the arrival room shows, not the tavern.
    mud.send(alice, "east").assert_contains("Grimmok's Forge");
    let back = mud.send(alice, "recall");
    let text = back.text();
    assert!(
        text.contains("Town Square"),
        "recall should arrive in the Town Square:\n{text}"
    );
    assert!(
        !text.contains("Rusted Anvil"),
        "recall should leave the tavern:\n{text}"
    );
}

#[test]
fn recall_echoes_departure_and_arrival() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");
    let carol = create_char(&mut mud, "carol@example.com", "Carol");

    // Bob waits in the Town Square; Carol stays in the tavern with Alice.
    mud.send(bob, "north").assert_contains("Town Square");

    mud.send(alice, "recall").assert_contains("Town Square");

    // Carol saw the attempt, then the disappearance, back in the tavern.
    let left = mud.send(carol, "look").text().to_string();
    let attempt = left.find("closes their eyes").expect("attempt echo");
    let vanish = left.find("disappears").expect("vanish echo");
    assert!(attempt < vanish, "attempt precedes disappearance:\n{left}");
    // Bob saw the recall-marked arrival in the square, not a walk-in.
    mud.send(bob, "look")
        .assert_contains("appears in a flash of light");
}

#[test]
fn can_walk_from_tavern_to_bear_cavern() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    // Haven's east road crosses into Whisperwood; the cave mouth hides off
    // the dead-end clearing, south of the main trail — an explorer's find.
    // The walk proves every link wires, including the cross-area road.
    mud.send(alice, "north").assert_contains("Town Square");
    mud.send(alice, "east").assert_contains("Grimmok's Forge");
    mud.send(alice, "east").assert_contains("East Road");
    mud.send(alice, "east").assert_contains("Forest Edge");
    mud.send(alice, "east").assert_contains("Forest Heart");
    mud.send(alice, "east").assert_contains("Forest Clearing");
    mud.send(alice, "south").assert_contains("Bear Cavern");
    mud.send(alice, "look").assert_contains("bear");
    // And the way back is wired too.
    mud.send(alice, "north").assert_contains("Forest Clearing");
}

#[test]
fn grimmok_greets_after_the_room_loads() {
    let mut mud = Mud::new();
    mud.set_mob_triggers(
        "Grimmok Ironhand",
        &[(TriggerKind::Enter, "self.say(\"Hello there adventurer.\")")],
    );
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    // Leave Grimmok's room, then come back: the enter trigger is post-state,
    // so the arrival description renders before the hello reacts to it.
    mud.send(alice, "north").assert_contains("Town Square");
    let back = mud.send(alice, "south");
    let text = back.text();
    let room = text.find("The Rusted Anvil").expect("arrival description");
    let hello = text.find("Hello there adventurer").expect("enter greeting");
    assert!(room < hello, "room loads before the hello:\n{text}");
}

#[test]
fn grimmok_farewell_precedes_departure() {
    let mut mud = Mud::new();
    mud.set_mob_triggers(
        "Grimmok Ironhand",
        &[(TriggerKind::AttemptLeave, "self.say(\"See you later.\")")],
    );
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");

    // The attempt-leave trigger is pre-state: the farewell renders before
    // the arrival it precedes — for Alice, who is mid-transition, and for
    // Bob, who stays and hears the room broadcast.
    let out = mud.send(alice, "north");
    let text = out.text();
    let bye = text.find("See you later").expect("farewell");
    let square = text.find("Town Square").expect("arrival description");
    assert!(bye < square, "farewell precedes the arrival:\n{text}");
    mud.send(bob, "look").assert_contains("See you later");
}

/// Log an existing account's first character back in (after quit or reboot):
/// email → password → character menu → select → MOTD → room.
fn login_again(mud: &mut Mud, email: &str) -> Session {
    let (s, _) = mud.connect();
    mud.send(s, email).assert_contains("Password");
    let _ = mud.send(s, PW); // → character menu
    let _ = mud.send(s, "1"); // select → MOTD
    mud.send(s, "").assert_contains("The Rusted Anvil");
    s
}

/// Count non-overlapping occurrences of `needle` in output.
fn count_in(output: &harness::Output, needle: &str) -> usize {
    output.text().matches(needle).count()
}

#[test]
fn pack_survives_quit_and_relogin() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    mud.send(alice, "get lantern")
        .assert_contains("You pick up brass lantern");
    let _ = mud.send(alice, "quit");
    mud.disconnect(alice);

    // Back in with the lantern still in hand, and none on the ground.
    let alice = login_again(&mut mud, "alice@example.com");
    mud.send(alice, "inventory")
        .assert_contains("You are carrying:")
        .assert_contains("brass lantern");
    mud.send(alice, "look")
        .assert_excludes("brass lantern rests here");
}

#[test]
fn reboot_regrows_seed_while_pack_restores() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    mud.send(alice, "get lantern")
        .assert_contains("You pick up brass lantern");
    let _ = mud.send(alice, "quit");
    mud.disconnect(alice);

    // Reboot: the seed regrows one lantern AND the pack restores one — two
    // instances is correct state.
    mud.reboot();
    let alice = login_again(&mut mud, "alice@example.com");
    mud.send(alice, "inventory")
        .assert_contains("brass lantern");
    mud.send(alice, "look")
        .assert_contains("A brass lantern rests here, its glass dusty but intact.");

    // Dropping the carried copy leaves two on the ground — also correct.
    mud.send(alice, "drop lantern")
        .assert_contains("You drop brass lantern");
    let room = mud.send(alice, "look");
    assert_eq!(
        count_in(&room, "brass lantern rests here"),
        2,
        "two ground copies; got:\n{}",
        room.text()
    );

    // Another reboot wipes ground extras: one blueprint copy remains, and the
    // pack (saved empty at quit) restores empty.
    let _ = mud.send(alice, "quit");
    mud.disconnect(alice);
    mud.reboot();
    let alice = login_again(&mut mud, "alice@example.com");
    let room = mud.send(alice, "look");
    assert_eq!(
        count_in(&room, "brass lantern rests here"),
        1,
        "single regrown copy; got:\n{}",
        room.text()
    );
    mud.send(alice, "inventory")
        .assert_contains("You are carrying nothing.");
}

#[test]
fn give_steal_and_look_pack_echo_all_parties() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");
    let cara = create_char(&mut mud, "cara@example.com", "Cara");

    mud.send(alice, "get lantern")
        .assert_contains("You pick up brass lantern");

    // Give: giver, recipient, and watcher each see a named line.
    mud.send(alice, "give lantern bob")
        .assert_contains("You give brass lantern to Bob");
    mud.recv(bob)
        .assert_contains("Alice gives you brass lantern");
    mud.recv(cara)
        .assert_contains("Alice gives brass lantern to Bob");
    mud.send(bob, "inventory").assert_contains("brass lantern");

    // Looking at Bob shows his pack below his description.
    mud.send(alice, "look bob")
        .assert_contains("Bob is carrying:")
        .assert_contains("brass lantern");

    // Creatures refuse gifts.
    mud.send(bob, "give lantern grimmok")
        .assert_contains("They don't want that item.");

    // Steal: thief, victim, and watcher each see a named line.
    mud.send(cara, "steal lantern bob")
        .assert_contains("You steal brass lantern from Bob");
    mud.recv(bob)
        .assert_contains("Cara steals your brass lantern");
    mud.recv(alice)
        .assert_contains("Cara steals brass lantern from Bob");
    mud.send(cara, "inventory").assert_contains("brass lantern");
    mud.send(bob, "inventory")
        .assert_contains("You are carrying nothing.");

    // Stealing what they don't hold, or from thin air, explains itself.
    mud.send(cara, "steal lantern bob")
        .assert_contains("They aren't carrying that.");
    mud.send(cara, "steal lantern nobody")
        .assert_contains("They aren't here.");
}

#[test]
fn desc_edit_types_saves_and_shows() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    // Entry shows the help footer; typing appends silently.
    mud.send(alice, "desc edit")
        .assert_contains("Editing description.")
        .assert_contains("1. A new adventurer.")
        .assert_contains("@save");
    let _ = mud.send(alice, "A tall traveler cloaked in dust.");
    let _ = mud.send(alice, "Eyes like chipped flint.");
    // Commands still parse as editor input, not verbs: no movement happens.
    let _ = mud.send(alice, "north");
    mud.send(alice, "@save")
        .assert_contains("Your description has been saved.");

    // Saved paragraphs (seed line included — it was preloaded, not replaced
    // by typing) show on self-look, and the stray "north" never moved her.
    mud.send(alice, "look self")
        .assert_contains("A tall traveler cloaked in dust.")
        .assert_contains("Eyes like chipped flint.")
        .assert_contains("A new adventurer.");
    mud.send(alice, "look").assert_contains("The Rusted Anvil");
}

#[test]
fn desc_edit_exit_discards_and_clear_empties() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    mud.send(alice, "desc edit")
        .assert_contains("Editing description.");
    let _ = mud.send(alice, "Some junk never saved.");
    mud.send(alice, "@exit")
        .assert_contains("Edit cancelled, description unchanged.");
    mud.send(alice, "look self")
        .assert_excludes("Some junk never saved.");

    // @clear empties the buffer, then @save stores the empty description.
    mud.send(alice, "desc edit")
        .assert_contains("Editing description.");
    mud.send(alice, "@clear").assert_contains("Buffer cleared.");
    mud.send(alice, "@save")
        .assert_contains("Your description has been saved.");
    mud.send(alice, "look self")
        .assert_excludes("A new adventurer.");
}

#[test]
fn objects_can_be_picked_up_listed_and_dropped() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");

    // Empty hands report nothing carried.
    mud.send(alice, "inventory")
        .assert_contains("You are carrying nothing.");

    // The seeded lantern lists under the room description.
    let room = mud.send(alice, "look");
    let body = room.text();
    let grimmok = body
        .find("{GGrimmok Ironhand{x stands here, hammering metal.")
        .expect("creature line");
    let lantern = body
        .find("A brass lantern rests here, its glass dusty but intact.")
        .expect("object line");
    assert!(grimmok < lantern, "objects list under creatures");

    // Pickup: actor sees first-party, the room sees third-party.
    mud.send(alice, "get lantern")
        .assert_contains("You pick up brass lantern");
    mud.recv(bob)
        .assert_contains("Alice picks up brass lantern");

    // Carried now: inventory lists the short, the room no longer shows it.
    mud.send(alice, "inventory")
        .assert_contains("You are carrying:")
        .assert_contains("brass lantern");
    mud.send(alice, "look")
        .assert_excludes("brass lantern rests here");
    // A second pickup misses: it is in Alice's hands, not the room.
    mud.send(alice, "get lantern")
        .assert_contains("You don't see that here.");

    // Drop reverses the words for both sides, and the lantern is back.
    mud.send(alice, "drop lantern")
        .assert_contains("You drop brass lantern");
    mud.recv(bob).assert_contains("Alice drops brass lantern");
    mud.send(alice, "look")
        .assert_contains("A brass lantern rests here, its glass dusty but intact.");
    mud.send(alice, "drop lantern")
        .assert_contains("You aren't carrying that.");
}

#[test]
fn speech_is_heard_by_others_in_the_room() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");

    // Actor sees the first-party echo.
    mud.send(alice, "say hello there")
        .assert_contains("You say")
        .assert_contains("hello there");

    // Bob, in the same room, receives it passively as third-party speech.
    mud.recv(bob)
        .assert_contains("Alice")
        .assert_contains("hello there");
}

#[test]
fn socials_render_per_audience_from_builtins() {
    // Temp data dir holds no data/socials: the built-in set applies.
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");

    // Solo: actor sees the actor wording, the room the room wording.
    mud.send(alice, "grin").assert_contains("You grin.");
    mud.recv(bob).assert_contains("Alice grins.");

    // Targeted: all three audiences.
    mud.send(alice, "grin bob")
        .assert_contains("You grin at Bob.");
    mud.recv(bob).assert_contains("Alice grins at you.");

    // Self: the self case, not the targeted one. Alice was created Male,
    // so the room sees "himself".
    mud.send(alice, "grin self")
        .assert_contains("You grin to yourself.");
    mud.recv(bob).assert_contains("Alice grins to himself.");

    // Unknown target explains itself to the actor only.
    mud.send(alice, "grin xyzzy")
        .assert_contains("You don't see anyone by that name here.");
}

#[test]
fn commands_hides_socials_and_admin_verbs() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    // Socials live under `socials`; admin verbs stay hidden from non-admins.
    mud.send(alice, "commands")
        .assert_contains("look")
        .assert_contains("socials")
        .assert_excludes("grin")
        .assert_excludes("shutdown")
        .assert_excludes("sockets");
}

#[test]
fn socials_lists_the_data_driven_verbs() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    mud.send(alice, "socials")
        .assert_contains("grin")
        .assert_contains("smile");
}

#[test]
fn ooc_is_global_and_reaches_a_distant_player() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");

    // Bob walks away to another room.
    mud.send(bob, "north").assert_contains("Town Square");

    // OOC is global, so Alice's message still reaches Bob.
    mud.send(alice, "ooc anyone around?")
        .assert_contains("anyone around?");
    mud.recv(bob).assert_contains("anyone around?");
}

#[test]
fn quit_saves_and_unloads_then_reconnect_logs_in_fresh() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    // `quit` is an intentional logout: it saves and UNLOADS the character from
    // the world. It must NOT go linkdead (that is only for an unexpected socket
    // drop). The character lives only on disk now — no in-world entity remains.
    let _ = mud.send(alice, "quit");
    mud.disconnect(alice);
    assert!(
        !mud.character_names().contains(&"Alice".to_string()),
        "quit must unload the character from the world (disk-only when logged out)"
    );

    // Reconnect is a normal login, not a linkdead reconnect: existing account →
    // password → character menu (Alice listed, not "linkdead") → select → MOTD →
    // enter the world.
    let (again, _) = mud.connect();
    mud.send(again, "alice@example.com")
        .assert_contains("Password");
    mud.send(again, PW)
        .assert_contains("Alice")
        .assert_excludes("linkdead");
    let _ = mud.send(again, "1"); // select from the menu → MOTD
    mud.send(again, "").assert_contains("The Rusted Anvil");
}

#[test]
fn a_new_account_cannot_see_another_accounts_characters() {
    let mut mud = Mud::new();
    // Account A owns Alice.
    let _alice = create_char(&mut mud, "alice@example.com", "Alice");

    // Account B is created fresh; its character menu must not leak Alice.
    let (b, _) = mud.connect();
    let _ = mud.send(b, "bob@example.com");
    let _ = mud.send(b, "y");
    mud.send(b, PW)
        .assert_contains("no characters")
        .assert_excludes("Alice");
}

#[test]
fn title_command_sets_clears_and_shows_in_who() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    // Setting a title confirms, and clearing it confirms too.
    mud.send(alice, "title the Bold")
        .assert_contains("the Bold");
    assert_eq!(
        mud.character("Alice").unwrap().title.as_deref(),
        Some("the Bold")
    );
    mud.send(alice, "title").assert_contains("cleared");
    assert!(mud.character("Alice").unwrap().title.is_none());

    // An over-length title (61 chars) is rejected and nothing is stored.
    let long = "z".repeat(61);
    mud.send(alice, &format!("title {long}"))
        .assert_contains("at most 60 characters");
    assert!(mud.character("Alice").unwrap().title.is_none());
}

#[test]
fn who_list_is_ordered_and_formatted_mud_style() {
    let mut mud = Mud::new();
    // Three characters enter in order: Alice, then Bob, then Carol. Creation
    // order fixes the connect-time tiebreak (Bob connected before Carol).
    let _alice = create_char(&mut mud, "alice@example.com", "Alice");
    let _bob = create_char(&mut mud, "bob@example.com", "Bob");
    let carol = create_char(&mut mud, "carol@example.com", "Carol");

    // Alice is an immortal with a title (human warrior, level irrelevant → IMM).
    mud.edit_character("Alice", |c| {
        c.roles.push(Role::Admin);
        c.title = Some("the Great".into());
    });
    // Bob: level 10 elf mage with a title.
    mud.edit_character("Bob", |c| {
        c.level = 10;
        c.race = "elf".into();
        c.class = "mage".into();
        c.title = Some("the Wise".into());
    });
    // Carol: level 10 human warrior, no title. Same level as Bob, so the
    // connect-time tiebreak (Bob first) decides their order.
    mud.edit_character("Carol", |c| c.level = 10);

    let out = mud.send(carol, "who");
    let text = out.text();

    // Exact MUD-style rows: `LLL G RRRRR CCC GGGGG Name Title`.
    let alice_row = "IMM M Human War       Alice the Great";
    let bob_row = " 10 M Elf   Mag       Bob the Wise";
    let carol_row = " 10 M Human War       Carol";
    out.assert_contains("Players online (3):")
        .assert_contains(alice_row)
        .assert_contains(bob_row)
        .assert_contains(carol_row);

    // Ordering: admin first (alpha), then level-10 by connect time (Bob<Carol).
    let ai = text.find(alice_row).unwrap();
    let bi = text.find(bob_row).unwrap();
    let ci = text.find(carol_row).unwrap();
    assert!(ai < bi && bi < ci, "WHO order wrong:\n{text}");
}

#[test]
fn who_excludes_linkdead_but_look_shows_them() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");

    // Both online: the list counts two.
    mud.send(alice, "who")
        .assert_contains("Players online (2):");

    // Bob's socket drops without `quit`: he goes linkdead (body stays in
    // the room, no session drives him).
    mud.disconnect(bob);

    // `who` is the live-session list: Bob vanishes entirely, not marked. The
    // same tick also delivers his room linkdead announce, so scope the
    // assertions to the list itself.
    let out = mud.send(alice, "who");
    let text = out.text();
    let list = &text[text.find("Players online").expect("who list")..];
    assert!(list.contains("Players online (1):"), "count wrong:\n{text}");
    assert!(list.contains("Alice"), "missing Alice:\n{text}");
    assert!(!list.contains("Bob"), "linkdead Bob listed:\n{text}");
    assert!(
        !list.contains("(Linkdead)"),
        "linkdead marker in who:\n{text}"
    );

    // The room still shows his body with the linkdead marker.
    mud.send(alice, "look")
        .assert_contains("Bob (Linkdead) is standing here.");
}

#[test]
fn wizlist_shows_online_and_offline_admins_only() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");
    mud.edit_character("Alice", |c| {
        c.roles.push(Role::Admin);
    });

    // Online: only the admin appears; the ordinary player is excluded.
    mud.send(bob, "wizlist")
        .assert_contains("Wizards (1):")
        .assert_contains("Alice")
        .assert_excludes("Bob");

    // Persist both, then reboot: nobody is in the world, but the startup
    // snapshot reloads Alice from disk — marked offline — and Bob (no admin
    // flag on disk) stays excluded.
    mud.disconnect(alice);
    mud.disconnect(bob);
    mud.reboot();
    let carol = create_char(&mut mud, "carol@example.com", "Carol");
    mud.send(carol, "wizlist")
        .assert_contains("Wizards (1):")
        .assert_contains("Alice (offline)")
        .assert_excludes("Bob")
        .assert_excludes("Carol");
}

#[test]
fn password_must_be_valid() {
    let mut mud = Mud::new();
    let (s, _) = mud.connect();
    let _ = mud.send(s, "alice@example.com");
    let _ = mud.send(s, "y");
    // Too-short password is rejected with a validation error, not accepted;
    // the flow stays on the password prompt and no account is created.
    mud.send(s, "pw")
        .assert_contains("at least 6 characters")
        .assert_excludes("Characters");
    assert!(mud.character_names().is_empty());

    // And nothing was persisted: a fresh connection with that email is still
    // offered account creation, not an existing-account password prompt.
    let (s2, _) = mud.connect();
    mud.send(s2, "alice@example.com")
        .assert_contains("create an account")
        .assert_excludes("Password");
}

#[test]
fn admin_reboot_expires_to_nonzero_exit() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    mud.edit_character("Alice", |c| {
        c.roles.push(Role::Admin);
    });

    // Warned countdown expires at once (`0s`); the exit must be exactly one
    // non-zero request, so the service manager restarts the server cold.
    let outcome = mud.send_shutdown(alice, "reboot 0");
    outcome.output.assert_contains("restarting now");
    assert_eq!(
        outcome.exits,
        vec![grim::AppExit::from_code(1)],
        "reboot must fire one cold-restart exit"
    );
    assert_eq!(outcome.copyover_due, 0, "reboot must not ask for a handoff");
}

#[test]
fn admin_copyover_expires_to_handoff_without_exit() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    mud.edit_character("Alice", |c| {
        c.roles.push(Role::Admin);
    });

    // Same warnings, but expiry asks the transport to hand off instead of
    // exiting (headless has no transport, so assert the request itself: fired
    // exactly once, with no exit alongside).
    let outcome = mud.send_shutdown(alice, "copyover 0");
    outcome.output.assert_contains("restarting now");
    assert_eq!(
        outcome.copyover_due, 1,
        "copyover must fire exactly one handoff"
    );
    assert!(outcome.exits.is_empty(), "copyover must not exit");
}

#[test]
fn map_centers_self_repeats_stably_and_recenters_on_move() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    // The tavern sits mid-world now: the Haven farm chain runs north to the
    // foothills, the south slope drops south, and the east road runs along
    // the square's row all the way to the Whisperwood clearing (bear cavern
    // tucked south of it, level with the tavern's canvas row).
    let first_out = mud.send(alice, "map");
    let first = first_out.text();
    let second_out = mud.send(alice, "map");
    assert_eq!(first, second_out.text(), "map must not flicker");
    let rows: Vec<&str> = first.lines().collect();
    assert_eq!(rows.len(), 20);
    assert_eq!(
        rows[10],
        format!("{:40}{ME}{HL}{HL}{RM}           {RM}", "")
    );
    // The loft's `,` up-marker draws at the row's left edge (the secret
    // down exit stays hidden, so no `'` marker appears for it).
    assert_eq!(
        rows[9],
        "                                       {Y,{x{w|{x              {w|{x"
    );
    assert_eq!(
        rows[8],
        format!(
            "{:37}{RM}{HL}{HL}{RM}{HL}{HL}{RM}{HL}{HL}{RM}{HL}{HL}{RM}{HL}{HL}{RM}{HL}{HL}{RM}",
            ""
        )
    );

    // Walking north recenters the canvas on the square: west slope and forge
    // flank it, the tavern lies south, the farm chain continues north.
    mud.send(alice, "north").assert_contains("Town Square");
    let moved_out = mud.send(alice, "map");
    let rows: Vec<&str> = moved_out.text().lines().collect();
    assert_eq!(
        rows[10],
        format!(
            "{:37}{RM}{HL}{HL}{ME}{HL}{HL}{RM}{HL}{HL}{RM}{HL}{HL}{RM}{HL}{HL}{RM}{HL}{HL}{RM}",
            ""
        )
    );
    // The loft marker rides along: recentered on the square, the tavern's
    // up-link still draws its `,` marker on this row.
    assert_eq!(
        rows[11],
        "                                       {Y,{x{w|{x              {w|{x"
    );
    assert_eq!(
        rows[12],
        format!("{:40}{RM}{HL}{HL}{RM}           {RM}", "")
    );
}

#[test]
fn look_staples_minimap_left_of_room_text() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    // Tavern minimap (9x7): the farm chain runs north past the title row, the
    // square's row shows west slope through forge with the east-road stub,
    // `@` centered on the looker's row, 9-wide gutter + two spaces throughout.
    let out = mud.send(alice, "look");
    let lines: Vec<&str> = out.text().lines().collect();
    assert_eq!(lines[0], format!("    {VL}      {{BThe Rusted Anvil{{x"));
    assert!(
        lines[1].starts_with(&format!(" {RM}{HL}{HL}{RM}{HL}{HL}{RM}{HL}  ")),
        "square row:\n{}",
        lines[1]
    );
    assert!(
        lines[3].starts_with(&format!("    {ME}{HL}{HL}{RM}")),
        "self row (privy east):\n{}",
        lines[3]
    );
    // Past the 7-row canvas the gutter runs blank (11 spaces).
    let exits = lines
        .iter()
        .find(|l| l.contains("Exits: {gnorth"))
        .expect("exits line");
    assert!(exits.starts_with("           "), "blank gutter:\n{exits}");
}

#[test]
fn config_minimap_toggles_look_map_persists_and_rejects() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");

    // On by default: the stapled self row is in the look output.
    let out = mud.send(alice, "look");
    assert!(
        out.text()
            .lines()
            .any(|l| l.starts_with(&format!("    {ME}{HL}{HL}{RM}"))),
        "minimap on by default (privy east):\n{}",
        out.text()
    );
    // Bare `config` lists the setting with its valid values.
    mud.send(alice, "config")
        .assert_contains("minimap: on - [on|off]");
    // Bare key cycles off, naming the previous value.
    mud.send(alice, "config minimap")
        .assert_contains("minimap set to off. (was previously on)");

    // Look loses the map: title unguttered, no self row anywhere.
    let out = mud.send(alice, "look");
    let lines: Vec<&str> = out.text().lines().collect();
    assert_eq!(lines[0], "{BThe Rusted Anvil{x");
    assert!(
        !lines.iter().any(|l| l.contains(ME)),
        "no minimap rows:\n{}",
        out.text()
    );

    // Rejections name the valid options; nothing is stored.
    mud.send(alice, "config minimap sideways")
        .assert_contains("Invalid value")
        .assert_contains("on, off");
    mud.send(alice, "config frobnicate")
        .assert_contains("Unknown config option");

    // The off choice survives logout and login.
    let _ = mud.send(alice, "quit");
    mud.disconnect(alice);
    let (again, _) = mud.connect();
    let _ = mud.send(again, "alice@example.com");
    let _ = mud.send(again, PW);
    let _ = mud.send(again, "1"); // select → MOTD
    mud.send(again, "").assert_contains("Exits:"); // enter the world
    let out = mud.send(again, "look");
    assert_eq!(out.text().lines().next(), Some("{BThe Rusted Anvil{x"));

    // And back on again through the explicit set.
    mud.send(again, "config minimap on")
        .assert_contains("minimap set to on.");
    let out = mud.send(again, "look");
    assert!(
        out.text()
            .lines()
            .any(|l| l.starts_with(&format!("    {ME}{HL}{HL}{RM}"))),
        "minimap back on (privy east):\n{}",
        out.text()
    );
}

#[test]
fn afk_command_sets_marker_and_next_line_clears() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");

    // Explicit set: the notice, and Bob's `who` shows the marker —
    // Bob's own line only clears Bob.
    mud.send(alice, "afk").assert_contains("You are now AFK");
    mud.send(bob, "who").assert_contains("Alice (AFK)");

    // Alice's next line clears her flag: Bob's `who` is clean again.
    let _ = mud.send(alice, "look");
    mud.send(bob, "who").assert_excludes("(AFK)");
}

#[test]
fn quiet_login_limbo_severs_while_ingame_idles_forever() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    // A second socket that never logs in: login limbo.
    let (limbo, _) = mud.connect();
    // Small thresholds, set after setup so the setup pumps (8s each) don't
    // pre-age anyone past them. In-game kick stays opted out (default).
    mud.set_idle_config(IdleConfig {
        afk_after_secs: 5,
        disconnect_after_secs: 36,
        warn_secs_before_disconnect: 8,
        disconnect_ingame_idle: false,
    });
    // Reset Alice's clock, then let her go quiet. Each interaction pumps 8s,
    // so the reset send itself crosses the 5s AFK mark — the notice lands in
    // its own output. Alice flags but stays connected.
    mud.send(alice, "look").assert_contains("AFK");
    // The limbo socket keeps quiet past warn_at (28s) and the 36s timeout:
    // the sweep asks the transport to sever it — and only it.
    let _ = mud.recv(alice);
    let _ = mud.recv(alice);
    let _ = mud.recv(alice);
    let severed = mud.drain_disconnects();
    assert!(
        !severed.is_empty() && severed.iter().all(|c| *c == limbo.conn),
        "expected only the limbo socket severed, got {severed:?}",
    );
}

#[test]
fn wiznet_lists_toggles_masks_and_reports_quits() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");
    mud.edit_character("Alice", |c| {
        c.roles.push(Role::Admin);
    });

    // List through the engine round trip; admins see the verb, others don't.
    mud.send(alice, "wiznet")
        .assert_contains("Wiznet options:")
        .assert_contains("wiznet.logins: off");
    mud.send(alice, "commands").assert_contains("wiznet");
    mud.send(alice, "wiznet logins")
        .assert_contains("wiznet.logins set to on.");

    // Non-admin: masked as unknown, hidden from the command list.
    mud.send(bob, "wiznet").assert_contains("Unknown command");
    mud.send(bob, "commands").assert_excludes("wiznet");

    // Bob quits; the opted-in admin sees the logins line.
    let _ = mud.send(bob, "quit");
    mud.recv(alice).assert_contains("[wiznet:logins] Bob quit");
}

#[test]
fn announces_reach_only_the_subjects_room() {
    let mut mud = Mud::new();
    let alice = create_char(&mut mud, "alice@example.com", "Alice");
    let bob = create_char(&mut mud, "bob@example.com", "Bob");

    // Same room at entry: Alice hears Bob connect.
    mud.recv(alice).assert_contains("Bob has connected");

    // Bob leaves for another room; his quit must not reach Alice.
    mud.send(bob, "north").assert_contains("Exits:");
    let _ = mud.send(bob, "quit");
    mud.recv(alice).assert_excludes("Bob has disconnected");
}

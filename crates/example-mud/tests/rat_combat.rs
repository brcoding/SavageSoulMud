mod harness;
use harness::Mud;

#[test]
fn ordinary_rat_fights_are_brief_and_player_grammar_is_correct() {
    let (mut mud, s) = rat_hunter();
    mud.send(s, "combatlog brief").assert_contains("Saved");
    let mut transcript = mud.send(s, "kill rat").text().to_string();
    for _ in 0..40 {
        transcript.push_str(mud.send(s, "look").text());
    }
    assert!(transcript.contains("Combat begins"));
    assert!(transcript.contains("Rat killed. XP +5."), "{transcript}");
    assert!(transcript.contains("You trample rat"), "{transcript}");
    assert!(!transcript.contains("Initiative:"), "{transcript}");
    assert!(!transcript.contains("wound level"), "{transcript}");
    assert_eq!(
        mud.saved_character("Hunter").game_state["savage_soul"]["total_xp"],
        605
    );
    assert!(!transcript.contains("flees"), "{transcript}");
    assert!(!transcript.contains("You attacks"), "{transcript}");
    assert!(!transcript.contains("You activates"), "{transcript}");
    assert!(!transcript.contains("you is"), "{transcript}");
    mud.send(s, "look")
        .assert_contains("The corpse of the rat lies here.")
        .assert_excludes("a rat is here.");
    mud.send(s, "look corpse")
        .assert_contains("corpse of the rat")
        .assert_contains("Condition: fresh.")
        .assert_excludes("belongings");
    mud.send(s, "get all from corpse")
        .assert_contains("corpse is empty")
        .assert_excludes("desk");
    mud.send(s, "get food from rat corpse")
        .assert_excludes("desk");
    mud.send(s, "get all from chest")
        .assert_contains("don't see that container")
        .assert_excludes("desk");
    mud.send(s, "get corpse")
        .assert_contains("You pick up corpse of the rat");
    mud.send(s, "get all from corpse")
        .assert_contains("corpse is empty");
    mud.send(s, "inventory")
        .assert_contains("corpse of the rat");
    mud.send(s, "prompt compact").assert_contains("Saved");
    let saved = mud.saved_character("Hunter");
    let body = saved
        .inventory
        .iter()
        .find(|o| o.name == "corpse of the rat")
        .unwrap();
    let clock = body.condition.as_ref().unwrap().decay.as_ref().unwrap();
    assert_eq!(clock.expires_at - clock.started_at, 60);
    mud.send(s, "drop corpse")
        .assert_contains("You drop corpse of the rat");
    mud.send(s, "kill rat").assert_excludes("Combat begins");
    assert_eq!(
        mud.saved_character("Hunter").game_state["savage_soul"]["total_xp"],
        605
    );
    mud.send(s, "get corpse").assert_contains("You pick up");
    mud.send(s, "put corpse on table")
        .assert_contains("medical table is in Royal Oak Med Room");
    for line in [
        "west",
        "west",
        "south",
        "west",
        "west",
        "west",
        "west",
        "southwest",
        "west",
        "west",
    ] {
        let _ = mud.send(s, line);
    }
    mud.send(s, "look").assert_contains("Royal Oak Med Room");
    mud.send(s, "place corpse on table")
        .assert_contains("regenerates the rat")
        .assert_contains("Threat detected")
        .assert_excludes("Your body regenerates");
    mud.send(s, "sit on table")
        .assert_contains("restores your wounds");
    mud.send(s, "lie on table")
        .assert_contains("restores your wounds");
    mud.send(s, "lay on table")
        .assert_contains("restores your wounds");
    mud.send(s, "east").assert_contains("Stand up first");
    mud.send(s, "stand").assert_contains("You stand up");
    mud.send(s, "east").assert_contains("Medical Center Hub");
    mud.send(s, "west").assert_contains("Royal Oak Med Room");
    mud.send(s, "inventory")
        .assert_excludes("corpse of the rat");
    mud.send(s, "put corpse on table")
        .assert_contains("not carrying");
    assert_rat_prototypes();
}

fn rat_hunter() -> (Mud, harness::Session) {
    let mut mud = Mud::with_savage_soul();
    let (s, _) = mud.connect();
    for line in [
        "rat@example.com",
        "y",
        "secretpw",
        "c",
        "Hunter",
        "1",
        "human",
        "",
    ] {
        let _ = mud.send(s, line);
    }
    for line in [
        "east",
        "east",
        "northeast",
        "east",
        "east",
        "east",
        "east",
        "north",
        "east",
        "east",
    ] {
        let _ = mud.send(s, line);
    }
    (mud, s)
}

fn assert_rat_prototypes() {
    let a: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/areas/savsoul.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for room in a["rooms"].as_array().unwrap() {
        if let Some(mobs) = room["npcs"].as_array() {
            for rat in mobs.iter().filter(|n| n["name"] == "rat") {
                let soul: example_mud::savage_rules::SoulData =
                    serde_json::from_value(rat["soul"].clone()).unwrap();
                assert_eq!(soul.size, 0);
                assert_eq!(soul.mass, 0);
                assert_eq!(soul.derived().base_dr, 1);
            }
        }
    }
}

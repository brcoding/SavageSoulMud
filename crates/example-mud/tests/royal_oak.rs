mod harness;
use harness::Mud;

#[test]
fn dagger_quest_unlocks_the_south_gate_and_saves_rewards_once() {
    let mut mud = Mud::with_savage_soul();
    let (s, _) = mud.connect();
    for line in ["quest@example.com", "y", "secretpw", "c", "Hero", "1", ""] {
        let _ = mud.send(s, line);
    }
    mud.send(s, "east")
        .assert_contains("Sue gives you a Soulboy");
    mud.send(s, "activate soulboy").assert_contains("Soulboy");
    for line in [
        "east",
        "north",
        "north",
        "north",
        "north",
        "east",
        "up",
        "west",
        "north",
        "disarm drawer",
        "open desk",
        "get all from desk",
    ] {
        let _ = mud.send(s, line);
    }
    for line in [
        "south",
        "east",
        "down",
        "west",
        "south",
        "south",
        "south",
        "south",
        "northeast",
    ] {
        let _ = mud.send(s, line);
    }
    for _ in 0..9 {
        let _ = mud.send(s, "east");
    }
    mud.send(s, "look").assert_contains("archwraith");
    mud.send(s, "attack wraith").assert_contains("Wield dagger");
    mud.send(s, "wield dagger")
        .assert_contains("wield mist dagger");
    mud.send(s, "attack wraith")
        .assert_contains("completed Savior of Royal Oak");
    let saved = mud.saved_character("Hero");
    assert_eq!(saved.game_state["savage_soul"]["total_xp"], 2600);
    assert_eq!(saved.game_state["quest_points"], 10);
    assert!(saved.inventory.iter().any(|o| o.name == "metal gate key"));
    assert!(!saved.inventory.iter().any(|o| o.name == "mist dagger"));
    // Return to the city through the exact source graph.
    for _ in 0..9 {
        let _ = mud.send(s, "west");
    }
    for line in ["southwest", "south", "open gate"] {
        let _ = mud.send(s, line);
    }
    mud.send(s, "south")
        .assert_contains("Soulboy beeps")
        .assert_contains("Royal Oak South Road");
    let _ = mud.send(s, "quit");
    mud.disconnect(s);
    mud.reboot();
    let (s, _) = mud.connect();
    for line in ["quest@example.com", "secretpw", "1", ""] {
        let _ = mud.send(s, line);
    }
    mud.send(s, "activate soulboy")
        .assert_contains("Savior of Royal Oak");
    assert_eq!(mud.saved_character("Hero").game_state["quest_points"], 10);
    for line in ["open gate", "north", "north", "northeast"] {
        let _ = mud.send(s, line);
    }
    for _ in 0..9 {
        let _ = mud.send(s, "east");
    }
    mud.send(s, "attack wraith")
        .assert_contains("already banished");
    assert_eq!(
        mud.saved_character("Hero").game_state["savage_soul"]["total_xp"],
        2600
    );
}

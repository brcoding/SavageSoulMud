mod harness;
use harness::Mud;

#[test]
fn preferences_save_restore_and_survive_login_with_inventory() {
    let mut mud = Mud::with_savage_soul();
    let (s, _) = mud.connect();
    for input in [
        "prefs@example.com",
        "y",
        "secretpw",
        "c",
        "Reader",
        "1",
        "human",
        "",
    ] {
        let _ = mud.send(s, input);
    }
    for input in ["east", "east", "north", "north", "west"] {
        let _ = mud.send(s, input);
    }
    mud.send(s, "buy water bottle").assert_contains("You buy");
    mud.send(s, "look bottle")
        .assert_contains("Condition: excellent.");
    let pack = mud.saved_character("Reader").inventory.len();
    mud.send(s, "combatlog detailed").assert_contains("Saved");
    mud.send(s, "prompt status").assert_contains("Saved");
    mud.send(s, "screenreader on").assert_contains("Saved");
    let saved = mud.saved_character("Reader");
    for (key, value) in [
        ("screenreader", "on"),
        ("combatlog", "brief"),
        ("minimap", "off"),
        ("colors", "off"),
        ("prompt", "accessible"),
    ] {
        assert_eq!(saved.config.get(key).map(String::as_str), Some(value));
    }
    assert_eq!(saved.inventory.len(), pack);
    mud.send(s, "map").assert_contains("Exits:");
    mud.send(s, "screenreader off").assert_contains("Saved");
    let saved = mud.saved_character("Reader");
    assert_eq!(saved.config["combatlog"], "detailed");
    assert_eq!(saved.config["prompt"], "status");
    mud.send(s, "prompt custom {name} {xp} XP {credits}cr>")
        .assert_contains("Saved");
    mud.send(s, "prompt custom {bad}")
        .assert_contains("Unknown prompt token");
    let _ = mud.send(s, "config combatlog brief");
    assert_eq!(mud.saved_character("Reader").config["combatlog"], "brief");
    let _ = mud.send(s, "quit");
    mud.disconnect(s);
    mud.reboot();
    let (s, _) = mud.connect();
    for input in ["prefs@example.com", "secretpw", "1", ""] {
        let _ = mud.send(s, input);
    }
    let saved = mud.character("Reader").unwrap();
    assert_eq!(saved.config["prompt"], "custom");
    assert_eq!(
        saved.game_state["prompt_template"],
        "{name} {xp} XP {credits}cr>"
    );
    mud.send(s, "drink water").assert_contains("You drink");
}

mod harness;
use harness::Mud;

#[test]
fn south_gate_is_visible_and_opening_does_not_bypass_security() {
    let mut mud = Mud::with_savage_soul();
    let (s, _) = mud.connect();
    for line in [
        "gate@example.com",
        "y",
        "secretpw",
        "c",
        "Traveler",
        "1",
        "human",
        "",
    ] {
        let _ = mud.send(s, line);
    }
    for line in ["east", "east", "south"] {
        let _ = mud.send(s, line);
    }
    mud.send(s, "look")
        .assert_contains("Royal Oak Wall Gate")
        .assert_contains("south");
    mud.send(s, "look gate").assert_contains("chain link fence");
    mud.send(s, "open gate").assert_contains("open");
    mud.send(s, "south").assert_contains("gate refuses to open");
    assert_eq!(
        mud.saved_character("Traveler").last_room.unwrap().room,
        "romainstwallgate"
    );
    let _ = mud.send(s, "drop soulboy");
    mud.edit_character("Traveler", |c| {
        c.game_state.insert(
            "quests".into(),
            serde_json::json!(["Escape from Royal Oak"]),
        );
    });
    mud.send(s, "south")
        .assert_contains("scans your wrist for a Soulboy");
    mud.send(s, "west")
        .assert_contains("Royal Oak Gate Controls");
    mud.send(s, "look terminal").assert_contains("terminal");
    mud.send(s, "east").assert_contains("Royal Oak Wall Gate");
}

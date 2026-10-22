mod harness;
use harness::Mud;

#[test]
fn desk_disarm_loot_and_read_survive_reconnect_without_refilling() {
    let mut mud = Mud::with_savage_soul();
    let (s, _) = mud.connect();
    for line in ["desk@example.com", "y", "secretpw", "c", "Visitor", "1", ""] {
        let _ = mud.send(s, line);
    }
    mud.send(s, "disarm drawer")
        .assert_contains("no trapped desk");
    for line in [
        "east", "east", "north", "north", "north", "north", "east", "up", "west", "north",
    ] {
        let _ = mud.send(s, line);
    }
    mud.send(s, "look desk").assert_contains("trap is armed");
    mud.send(s, "get all from desk")
        .assert_contains("desk is closed");
    mud.send(s, "disarm drawer")
        .assert_contains("successfully disarm");
    mud.send(s, "open desk")
        .assert_contains("open the desk")
        .assert_excludes("pricked");
    mud.send(s, "look in desk")
        .assert_contains("mist-bladed dagger");
    mud.send(s, "get all from desk")
        .assert_contains("mist dagger")
        .assert_contains("dagger note");
    mud.send(s, "read note")
        .assert_contains("Beware the Archwraith");
    mud.send(s, "wield dagger")
        .assert_contains("wield mist dagger");
    let saved = mud.saved_character("Visitor");
    assert_eq!(saved.inventory.iter().filter(|o|o.name=="mist dagger"||o.name=="dagger note").count(), 2);
    let _ = mud.send(s, "quit");
    mud.disconnect(s);
    mud.reboot();
    let (s, _) = mud.connect();
    for line in ["desk@example.com", "secretpw", "1", ""] {
        let _ = mud.send(s, line);
    }
    mud.send(s, "get all from desk")
        .assert_contains("no matching item left");
    mud.send(s, "read note")
        .assert_contains("Beware the Archwraith");
}

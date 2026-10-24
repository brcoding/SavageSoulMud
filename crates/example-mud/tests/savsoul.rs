mod harness;
use harness::Mud;

#[test]
fn savsoul_door_routes_signs_scenery_and_return() {
    let mut mud = Mud::with_savsoul();
    let (s, _) = mud.connect();
    for input in [
        "visitor@example.com",
        "y",
        "secretpw",
        "c",
        "Visitor",
        "1",
        "human",
        "warrior",
        "",
    ] {
        let _ = mud.send(s, input);
    }
    mud.send(s, "look").assert_contains("Royal Oak Med Room");
    for input in ["east", "east", "south", "open east", "east"] {
        let _ = mud.send(s, input);
    }
    let _ = mud.send(s, "close west");
    mud.send(s, "look").assert_contains("The Rusted Anvil");
    mud.send(s, "west").assert_contains("is closed");
    mud.send(s, "open west")
        .assert_contains("You open the Savsoul door");
    mud.send(s, "west").assert_contains("Royal Oak Wall Gate");
    mud.send(s, "north")
        .assert_contains("South Royal Oak Main Street");
    mud.send(s, "west").assert_contains("Medical Center Hub");
    mud.send(s, "look sign")
        .assert_contains("For injuries or purchases");
    mud.send(s, "read sign")
        .assert_contains("For injuries or purchases");
    mud.send(s, "west").assert_contains("Royal Oak Med Room");
    for command in ["look sign", "read sign"] {
        mud.send(s, command)
            .assert_contains("sit table")
            .assert_contains("creator")
            .assert_contains("10,000 credits");
    }
    mud.send(s, "look table").assert_contains("stainless steel");
    mud.send(s, "get table").assert_excludes("You get");
    mud.send(s, "down").assert_contains("A polite young woman");
    mud.send(s, "look jennybot").assert_contains("android");
    mud.send(s, "west").assert_contains("Travel Concourse");
    mud.send(s, "east").assert_contains("A polite young woman");
    mud.send(s, "up").assert_contains("Royal Oak Med Room");
    mud.send(s, "east").assert_contains("Medical Center Hub");
    mud.send(s, "east")
        .assert_contains("South Royal Oak Main Street");
    mud.send(s, "northeast").assert_contains("Faded Trail");
    for _ in 0..4 {
        mud.send(s, "east").assert_contains("Faded Trail");
    }
    for _ in 0..4 {
        let _ = mud.send(s, "west");
    }
    let _ = mud.send(s, "southwest");
    mud.send(s, "look")
        .assert_contains("South Royal Oak Main Street");
    for _ in 0..3 {
        let _ = mud.send(s, "north");
    }
    mud.send(s, "north")
        .assert_contains("Royal Oak Bar N Grill");
    mud.send(s, "east").assert_contains("Royal Oak Bar N Grill");
    mud.send(s, "up").assert_contains("Second Floor");
    mud.send(s, "west").assert_contains("Second Floor");
    mud.send(s, "west").assert_contains("Second Floor");
    for input in [
        "east", "east", "down", "west", "south", "south", "south", "south", "south",
    ] {
        let _ = mud.send(s, input);
    }
    mud.send(s, "look").assert_contains("Royal Oak Wall Gate");
    mud.send(s, "close east")
        .assert_contains("You close the Savsoul door");
    mud.send(s, "east").assert_contains("is closed");
    mud.send(s, "open east")
        .assert_contains("You open the Savsoul door");
    mud.send(s, "east").assert_contains("The Rusted Anvil");
    mud.send(s, "west").assert_contains("Royal Oak Wall Gate");
}

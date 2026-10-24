mod harness;
use harness::Mud;

#[test]
fn shop_trades_save_credits_and_inventory_together() {
    let mut mud = Mud::with_savage_soul();
    let (s, _) = mud.connect();
    for input in [
        "shopper@example.com",
        "y",
        "secretpw",
        "c",
        "Shopper",
        "1",
        "human",
        "",
    ] {
        let _ = mud.send(s, input);
    }
    mud.send(s, "credits").assert_contains("1000 credits");
    mud.send(s, "list").assert_contains("no shopkeeper");
    for input in ["east", "east", "north", "north", "west"] {
        let _ = mud.send(s, input);
    }
    mud.send(s, "list")
        .assert_contains("shovel — buy 20, sell 10");
    mud.send(s, "buy 2*shovel")
        .assert_contains("960 credits remaining");
    let saved = mud.saved_character("Shopper");
    assert_eq!(saved.credits, 960);
    assert_eq!(saved.inventory.iter().filter(|item| item.name == "shovel").count(), 2);
    mud.send(s, "buy 100*shovel")
        .assert_contains("You need 2000 credits");
    assert_eq!(mud.saved_character("Shopper").credits, 960);
    mud.send(s, "sell 2.shovel")
        .assert_contains("970 credits remaining");
    mud.send(s, "sell all shovel")
        .assert_contains("980 credits remaining");
    mud.send(s, "buy ration")
        .assert_contains("975 credits remaining");
    mud.send(s, "buy nonexistent")
        .assert_contains("does not stock");
    assert_eq!(mud.saved_character("Shopper").credits, 975);
    let _ = mud.send(s, "quit");
    mud.disconnect(s);
    let (s, _) = mud.connect();
    for input in ["shopper@example.com", "secretpw", "1", ""] {
        let _ = mud.send(s, input);
    }
    mud.send(s, "credits").assert_contains("975 credits");
    mud.send(s, "inventory")
        .assert_contains("ration pack")
        .assert_excludes("shovel");
}

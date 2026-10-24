mod harness;
use harness::Mud;

#[test]
fn player_can_browse_read_and_search_help() {
    let mut mud = Mud::with_savage_soul();
    let (session, _) = mud.connect();
    for line in [
        "help@example.com",
        "y",
        "secretpw",
        "c",
        "Reader",
        "1",
        "human",
        "",
    ] {
        let _ = mud.send(session, line);
    }
    mud.send(session, "help")
        .assert_contains("Savsoul help")
        .assert_excludes("copyover");
    mud.send(session, "help drink").assert_contains("bottle");
    mud.send(session, "help search armor")
        .assert_contains("Matching help topics");
    mud.send(session, "help no-such-topic")
        .assert_contains("No such help topic");
    mud.send(session, "help copyover")
        .assert_contains("No such help topic");
}

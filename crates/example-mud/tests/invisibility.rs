mod harness;
use harness::{Mud, Session};

fn create(mud: &mut Mud, email: &str, name: &str) -> Session {
    let (session, _) = mud.connect();
    for line in [email, "y", "secretpw", "c", name, "1", "human", ""] {
        let _ = mud.send(session, line);
    }
    mud.send(session, "look")
        .assert_contains("Royal Oak Med Room");
    session
}

#[test]
fn test_accounts_hide_actions_presence_targets_and_relogin_without_admin_rights() {
    let mut mud = Mud::with_savage_soul();
    let wizard = create(&mut mud, "staff@example.com", "Supervisor");
    mud.edit_character("Supervisor", |c| c.roles.push(grim::Role::Admin));
    let observer = create(&mut mud, "player@example.com", "Observer");
    let tester = create(&mut mud, "testing@example.com", "Probe");
    let _ = mud.recv(observer);
    mud.send(wizard, "testaccount Probe on")
        .assert_contains("Test account enabled");
    mud.send(observer, "who").assert_excludes("Probe");
    mud.send(observer, "look").assert_excludes("Probe");
    mud.send(observer, "where").assert_excludes("Probe");
    mud.send(observer, "finger Probe")
        .assert_excludes("Name: Probe");
    mud.send(observer, "look Probe")
        .assert_contains("don't see");
    mud.send(observer, "tell Probe visible-secret")
        .assert_contains("No one named");
    mud.send(wizard, "who").assert_contains("Probe");
    mud.send(wizard, "look Probe").assert_contains("Probe");
    let _ = mud.recv(observer);
    let _ = mud.recv(wizard);
    mud.send(tester, "say hidden-speech")
        .assert_contains("hidden-speech");
    mud.send(tester, "grin").assert_contains("grin");
    mud.send(tester, "tell Observer hidden-tell")
        .assert_contains("No one named");
    mud.send(tester, "wizinvis off")
        .assert_contains("Unknown command");
    mud.send(tester, "testaccount Probe off")
        .assert_contains("Unknown command");
    mud.send(tester, "goto savsoul:start")
        .assert_contains("Unknown command");
    mud.send(tester, "i3 status")
        .assert_contains("Unknown command");
    mud.send(tester, "east").assert_excludes("can't go");
    mud.recv(observer)
        .assert_excludes("Probe")
        .assert_excludes("hidden-speech")
        .assert_excludes("hidden-tell");
    mud.recv(wizard).assert_contains("hidden-speech");
    let _ = mud.send(tester, "west");
    let _ = mud.send(tester, "quit");
    mud.disconnect(tester);
    mud.recv(observer).assert_excludes("Probe");
    let (tester, _) = mud.connect();
    for line in ["testing@example.com", "secretpw", "1", ""] {
        let _ = mud.send(tester, line);
    }
    mud.recv(observer).assert_excludes("Probe");
    mud.send(observer, "who").assert_excludes("Probe");
    assert!(mud
        .saved_character("Probe")
        .roles
        .contains(&grim::Role::Tester));
    assert!(!mud
        .saved_character("Probe")
        .roles
        .contains(&grim::Role::Admin));
    let _ = mud.send(tester, "quit");
    mud.disconnect(tester);
    mud.send(observer, "finger Probe")
        .assert_excludes("Name: Probe");
    mud.send(wizard, "testaccount Probe off")
        .assert_contains("Test account disabled");
}

#[test]
fn wizard_invisibility_toggles_persists_and_does_not_leak_to_ordinary_users() {
    let mut mud = Mud::with_savage_soul();
    let wizard = create(&mut mud, "wizard@example.com", "Invisible");
    mud.edit_character("Invisible", |c| c.roles.push(grim::Role::Admin));
    let observer = create(&mut mud, "normal@example.com", "Normal");
    mud.send(wizard, "wizinvis")
        .assert_contains("invisibility on");
    mud.send(observer, "who").assert_excludes("Invisible");
    mud.send(observer, "wizlist").assert_excludes("Invisible");
    mud.send(observer, "look").assert_excludes("Invisible");
    let _ = mud.recv(observer);
    let _ = mud.send(wizard, "say wizard-secret");
    let _ = mud.send(wizard, "quit");
    mud.disconnect(wizard);
    mud.recv(observer)
        .assert_excludes("Invisible")
        .assert_excludes("wizard-secret");
    let (wizard, _) = mud.connect();
    for line in ["wizard@example.com", "secretpw", "1", ""] {
        let _ = mud.send(wizard, line);
    }
    mud.recv(observer).assert_excludes("Invisible");
    mud.send(observer, "who").assert_excludes("Invisible");
    mud.send(wizard, "wizinvis off")
        .assert_contains("invisibility off");
    mud.send(observer, "who").assert_contains("Invisible");
    mud.send(wizard, "wizinvis")
        .assert_contains("invisibility on");
    mud.edit_character("Invisible", |c| c.roles.clear());
    mud.send(observer, "who").assert_contains("Invisible");
    mud.send(wizard, "wizinvis")
        .assert_contains("Unknown command");
}

#[test]
fn account_status_covers_new_characters_and_copyover_with_stale_character_roles() {
    let mut mud = Mud::with_savage_soul();
    let wizard = create(&mut mud, "manager@example.com", "Manager");
    mud.edit_character("Manager", |c| c.roles.push(grim::Role::Admin));
    let observer = create(&mut mud, "witness@example.com", "Witness");
    let tester = create(&mut mud, "automation@example.com", "Automation");
    let _ = mud.recv(observer);
    mud.send(wizard, "testaccount Automation on")
        .assert_contains("enabled");
    let _ = mud.send(tester, "quit");
    mud.disconnect(tester);
    let (tester, _) = mud.connect();
    for line in [
        "automation@example.com",
        "secretpw",
        "c",
        "Newprobe",
        "1",
        "human",
        "",
    ] {
        let _ = mud.send(tester, line);
    }
    mud.send(tester, "look")
        .assert_contains("Royal Oak Med Room");
    mud.recv(observer)
        .assert_excludes("Newprobe")
        .assert_excludes("Automation");
    mud.send(observer, "who").assert_excludes("Newprobe");
    for session in [wizard, observer, tester] {
        let _ = mud.send(session, "quit");
        mud.disconnect(session);
    }
    // Simulate a pre-marker character file. The account must still control it.
    mud.edit_saved_character("Newprobe", |c| c.roles.retain(|r| *r != grim::Role::Tester));
    mud.reboot();
    let observer = mud.resume("Witness");
    let tester = mud.resume("Newprobe");
    mud.send(tester, "look")
        .assert_contains("Royal Oak Med Room");
    mud.send(observer, "who").assert_excludes("Newprobe");
    mud.send(observer, "look").assert_excludes("Newprobe");
    mud.send(tester, "wizinvis off")
        .assert_contains("Unknown command");
}

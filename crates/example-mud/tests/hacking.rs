mod harness;
use harness::Mud;

#[test]
fn terminal_progress_survives_abort_and_reconnect_and_reward_is_once() {
    let mut mud = Mud::with_savage_soul();
    let (s, _) = mud.connect();
    for line in [
        "hacker@example.com",
        "y",
        "secretpw",
        "c",
        "Hacker",
        "1",
        "human",
        "",
    ] {
        let _ = mud.send(s, line);
    }
    for line in ["east", "east", "south", "west"] {
        let _ = mud.send(s, line);
    }
    mud.send(s, "hack terminal")
        .assert_contains("Codes:")
        .assert_contains("4 attempts left");
    let saved = mud.saved_character("Hacker");
    let state = &saved.game_state["hacking"];
    let key = state["active"].as_str().unwrap().to_string();
    let answer = state["devices"][&key]["answer"]
        .as_str()
        .unwrap()
        .to_string();
    let wrong = state["devices"][&key]["words"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .find(|w| *w != answer)
        .unwrap()
        .to_string();
    let xp = saved.game_state["savage_soul"]["total_xp"]
        .as_u64()
        .unwrap();
    mud.send(s, "hack try WRONG")
        .assert_contains("no attempt used");
    mud.send(s, &format!("hack try {wrong}"))
        .assert_contains("3 attempts left");
    mud.send(s, &format!("hack try {wrong}"))
        .assert_contains("already tried");
    mud.send(s, "hack abort").assert_contains("saved");
    mud.send(s, "hack terminal")
        .assert_contains("3 attempts left");
    let _ = mud.send(s, "east");
    mud.send(s, &format!("hack try {answer}"))
        .assert_contains("Return to the terminal");
    let _ = mud.send(s, "west");
    let _ = mud.send(s, "quit");
    let (s, _) = mud.connect();
    for line in ["hacker@example.com", "secretpw", "1", ""] {
        let _ = mud.send(s, line);
    }
    mud.send(s, "hack status")
        .assert_contains("3 attempts left");
    mud.send(s, &format!("hack try {answer}"))
        .assert_contains("Access granted")
        .assert_contains("XP +2000")
        .assert_contains("gate slides open");
    mud.send(s, "hack terminal")
        .assert_contains("no additional reward");
    let saved = mud.saved_character("Hacker");
    assert_eq!(
        saved.game_state["savage_soul"]["total_xp"].as_u64(),
        Some(xp + 2000)
    );
    assert_eq!(saved.game_state["quest_points"], 10);
    assert!(saved.game_state["quests"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "Escape from Royal Oak"));
    let _ = mud.send(s, "east");
    let _ = mud.send(s, "drop soulboy");
    mud.send(s, "south")
        .assert_contains("scans your wrist for a Soulboy");
    let _ = mud.send(s, "get soulboy");
    mud.send(s, "south").assert_contains("Royal Oak South Road");
}

#[test]
fn failed_challenge_locks_and_cannot_be_reset_by_reconnecting_to_device() {
    let mut mud = Mud::with_savage_soul();
    let (s, _) = mud.connect();
    for line in [
        "locked@example.com",
        "y",
        "secretpw",
        "c",
        "Locked",
        "1",
        "human",
        "",
    ] {
        let _ = mud.send(s, line);
    }
    for line in ["east", "east", "south", "west", "hack terminal"] {
        let _ = mud.send(s, line);
    }
    let saved = mud.saved_character("Locked");
    let state = &saved.game_state["hacking"];
    let key = state["active"].as_str().unwrap();
    let c = &state["devices"][key];
    let wrong: Vec<_> = c["words"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| **w != c["answer"])
        .take(4)
        .filter_map(|w| w.as_str())
        .map(str::to_string)
        .collect();
    for w in wrong {
        let _ = mud.send(s, &format!("hack try {w}"));
    }
    mud.send(s, "hack terminal")
        .assert_contains("Terminal locked");
    let _ = mud.send(s, "hack abort");
    mud.send(s, "hack terminal")
        .assert_contains("Terminal locked");
    assert_eq!(
        mud.saved_character("Locked").game_state["hacking"]["devices"][key]["remaining"],
        0
    );
}

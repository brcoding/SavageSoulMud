//! Telnet rendering, per-character prompts, ANSI and CRLF.
use grim_actor::Character;
use grim_color::ansi;

#[cfg(test)]
pub(crate) fn render_output(text: &str, ingame: bool, afk: bool, prepend: bool) -> String {
    render_output_for_character(text, ingame, afk, prepend, None)
}

pub(crate) fn render_output_for_character(
    text: &str,
    ingame: bool,
    afk: bool,
    prepend: bool,
    character: Option<(&Character, &str)>,
) -> String {
    let mut text = text.to_string();
    if prepend && !text.is_empty() {
        text.insert(0, '\n');
    }
    if ingame && !text.is_empty() {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&prompt(character, afk));
    }
    let colored = ansi(&grim_color::convert_16color(&text));
    let plain = character.is_some_and(|(c, _)| {
        c.config.get("screenreader").is_some_and(|v| v == "on")
            || c.config.get("colors").is_some_and(|v| v == "off")
    });
    let output = if plain { strip_ansi(&colored) } else { colored };
    output.replace("\r\n", "\n").replace('\n', "\r\n")
}

fn prompt(character: Option<(&Character, &str)>, afk: bool) -> String {
    let Some((character, name)) = character else {
        return if afk { "<AFK> " } else { "> " }.to_string();
    };
    let preset = character
        .config
        .get("prompt")
        .map(String::as_str)
        .unwrap_or("compact");
    if preset == "off" {
        return String::new();
    }
    if character
        .config
        .get("screenreader")
        .is_some_and(|v| v == "on")
        || preset == "accessible"
    {
        return if afk { "Away. Ready. " } else { "Ready. " }.to_string();
    }
    let soul = character.game_state.get("savage_soul");
    let field = |key: &str| soul.and_then(|s| s.get(key));
    let mut condition = field("injury")
        .and_then(|v| v.as_str())
        .unwrap_or("uninjured")
        .replace('_', " ");
    for key in ["unconscious", "stunned", "winded", "fatigued"] {
        if field(key).and_then(|v| v.as_bool()) == Some(true) {
            condition.push_str(", ");
            condition.push_str(key);
        }
    }
    let number = |key: &str| {
        field(key)
            .and_then(|v| v.as_i64())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "-".to_string())
    };
    let xp = number("total_xp");
    let mana = number("mana");
    let maxmana = character
        .game_state
        .get("prompt_maxmana")
        .and_then(|v| v.as_i64())
        .map(|n| n.to_string())
        .unwrap_or_else(|| "-".to_string());
    let opponent = character
        .game_state
        .get("combat_opponent")
        .and_then(|v| v.as_str())
        .unwrap_or("none");
    let injury = field("injury")
        .and_then(|v| v.as_str())
        .unwrap_or("uninjured");
    let code = match injury {
        "uninjured" => "OK",
        "barely_injured" => "BI",
        "injured" => "I",
        "seriously_injured" => "SI",
        "critically_injured" => "CI",
        "dead" => "DEAD",
        "destroyed" => "DEST",
        "obliterated" => "OBL",
        _ => "?",
    };
    let mut codes = vec![code];
    for (key, code) in [
        ("unconscious", "U"),
        ("stunned", "S"),
        ("winded", "W"),
        ("fatigued", "F"),
    ] {
        if field(key).and_then(|v| v.as_bool()) == Some(true) {
            codes.push(code);
        }
    }
    let compact = format!("[{}] > ", codes.join(" "));
    let template = match preset {
        "classic" => "> ",
        "compact" => compact.as_str(),
        "status" => "[{condition} | MP {mana}/{maxmana} | {xp} XP | {credits} cr] > ",
        "combat" => "[{condition} | foe: {opponent}] > ",
        "custom" => character
            .game_state
            .get("prompt_template")
            .and_then(|v| v.as_str())
            .unwrap_or("> "),
        _ => "> ",
    };
    let credits = character.credits.to_string();
    let values = [
        ("name", name),
        ("condition", condition.as_str()),
        ("xp", xp.as_str()),
        ("credits", credits.as_str()),
        ("mana", mana.as_str()),
        ("maxmana", maxmana.as_str()),
        ("opponent", opponent),
    ];
    render_prompt_template(template, &values, afk)
}

fn render_prompt_template(template: &str, values: &[(&str, &str)], afk: bool) -> String {
    // Scan once: inserted values cannot introduce tokens or colour markup.
    let mut rendered = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        rendered.push_str(&rest[..start]);
        let Some(relative_end) = rest[start..].find('}') else {
            rendered.push_str(&grim_color::escape_codes(&rest[start..]));
            rest = "";
            break;
        };
        let end = start + relative_end;
        if let Some((_, value)) = values
            .iter()
            .find(|(token, _)| *token == &rest[start + 1..end])
        {
            rendered.push_str(&grim_color::escape_codes(value));
        }
        rest = &rest[end + 1..];
    }
    rendered.push_str(rest);
    if afk {
        rendered.insert_str(0, "<AFK> ");
    }
    rendered
}

/// Strip terminal controls after both markup families render. CSI and OSC are
/// handled too, preventing embedded ANSI from bypassing plain mode.
fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.peek() == Some(&'\\')) {
                            if c == '\u{1b}' {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !ch.is_control() || matches!(ch, '\n' | '\r' | '\t') {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn afk_prompt_replaces_default() {
        assert_eq!(render_output("hi", true, false, false), "hi\r\n> ");
        assert_eq!(render_output("hi", true, true, false), "hi\r\n<AFK> ");
        assert_eq!(render_output("hi", false, true, false), "hi");
    }
    fn character() -> Character {
        Character {
            id: grim_core::id::GrimId::new(),
            account_id: grim_core::id::GrimId::new(),
            credits: 80,
            created_at: chrono::Utc::now(),
            last_room: None,
            roles: vec![],
            class: String::new(),
            title: None,
            restrings: Default::default(),
            config: Default::default(),
            game_state: Default::default(),
        }
    }
    #[test]
    fn compact_prompt_distinguishes_conditions_from_classic() {
        let mut c = character();
        assert_eq!(prompt(Some((&c, "Sam")), false), "[OK] > ");
        c.game_state.insert(
            "savage_soul".into(),
            serde_json::json!({"injury": "barely_injured", "winded": true}),
        );
        assert_eq!(prompt(Some((&c, "Sam")), false), "[BI W] > ");
        c.config.insert("prompt".into(), "classic".into());
        assert_eq!(prompt(Some((&c, "Sam")), false), "> ");
    }
    #[test]
    fn accessible_plain_prompt_and_custom_status_values() {
        let mut c = character();
        c.config.insert("screenreader".into(), "on".into());
        assert_eq!(
            render_output_for_character("{RHit.{x", true, false, false, Some((&c, "Sam"))),
            "Hit.\r\nReady. "
        );
        c.config.insert("prompt".into(), "off".into());
        assert_eq!(
            render_output_for_character("Hit.", true, false, false, Some((&c, "Sam"))),
            "Hit.\r\n"
        );
        c.config.insert("screenreader".into(), "off".into());
        c.config.insert("prompt".into(), "custom".into());
        c.game_state.insert(
            "prompt_template".into(),
            serde_json::json!("{name} {xp} XP {credits}cr {condition}> "),
        );
        c.game_state.insert(
            "savage_soul".into(),
            serde_json::json!({"total_xp": 610, "injury": "barely_injured"}),
        );
        assert_eq!(
            render_output_for_character("Done.", true, false, false, Some((&c, "Sam"))),
            "Done.\r\nSam 610 XP 80cr barely injured> "
        );
    }
    #[test]
    fn strips_both_color_families_and_embedded_ansi() {
        assert_eq!(
            strip_ansi(&ansi(&grim_color::convert_16color(
                "{Rred{x @xF00red@r \u{1b}[31mred\u{1b}[0m"
            ))),
            "red red red"
        );
    }
}

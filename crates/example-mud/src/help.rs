//! Editable, immediately reloaded player documentation.
use bevy::prelude::*;
use grim::events::{Command, EngineCommand, InfoMessage};
use serde::Deserialize;
use std::path::{Component, Path, PathBuf};

#[derive(Resource)]
pub struct HelpDir(pub PathBuf);
pub struct HelpPlugin;
impl Plugin for HelpPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().contains_resource::<HelpDir>() {
            app.insert_resource(HelpDir(PathBuf::from("data/help")));
        }
        app.add_systems(Update, answer_help);
    }
}

#[derive(Deserialize)]
struct Topic {
    id: String,
    title: String,
    category: String,
    file: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    wizard: bool,
}
impl Topic {
    fn names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.id.as_str()).chain(self.aliases.iter().map(String::as_str))
    }
}

fn read_file(root: &Path, file: &str) -> Result<String, String> {
    let relative = Path::new(file);
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Invalid help filename.".into());
    }
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|_| "Help file unavailable.")?;
    if !path.starts_with(root) {
        return Err("Help file is outside the help directory.".into());
    }
    if std::fs::metadata(&path)
        .map_err(|_| "Help file unavailable.")?
        .len()
        > 256 * 1024
    {
        return Err("Help file is too large.".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "Help file unavailable.")?;
    String::from_utf8(bytes).map_err(|_| "Help file must be UTF-8.".into())
}

fn render(dir: &Path, query: &str, wizard: bool) -> Result<String, String> {
    let root = dir
        .canonicalize()
        .map_err(|_| "Help is temporarily unavailable.")?;
    let index = read_file(&root, "index.json")?;
    let mut topics: Vec<Topic> = serde_json::from_str(&index).map_err(|error| {
        warn!("Help index: {error}");
        "Help index needs repair.".to_string()
    })?;
    topics.retain(|t| !t.wizard || wizard);
    topics.sort_by(|a, b| (&a.category, &a.id).cmp(&(&b.category, &b.id)));
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() || matches!(query.as_str(), "topics" | "commands") {
        let mut out =
            String::from("Savsoul help\nUse: help <topic>, help search <words>, commands\n\n");
        let mut category = "";
        for topic in &topics {
            if category != topic.category {
                category = &topic.category;
                out.push_str(&format!("\n{category}:\n"));
            }
            out.push_str(&format!("  {} - {}\n", topic.id, topic.title));
        }
        return Ok(out);
    }
    if query == "search" {
        return Ok("Use: help search <words>\nExample: help search armor".into());
    }
    if let Some(words) = query.strip_prefix("search ") {
        let words: Vec<_> = words.split_whitespace().collect();
        if words.is_empty() {
            return Ok("Use: help search <words>".into());
        }
        let mut hits = Vec::new();
        for topic in &topics {
            let body = read_file(&root, &topic.file).unwrap_or_default();
            let text = format!(
                "{} {} {} {}",
                topic.id,
                topic.title,
                topic.aliases.join(" "),
                body
            )
            .to_ascii_lowercase();
            if words.iter().all(|word| text.contains(word)) {
                hits.push(format!("  {} - {}", topic.id, topic.title));
            }
        }
        return Ok(if hits.is_empty() {
            "No matching help topics. Type help to browse.".into()
        } else {
            format!(
                "Matching help topics:\n{}\nUse help <topic> to read one.",
                hits.join("\n")
            )
        });
    }
    let exact: Vec<_> = topics
        .iter()
        .filter(|t| t.names().any(|n| n.eq_ignore_ascii_case(&query)))
        .collect();
    let matches = if exact.is_empty() {
        topics
            .iter()
            .filter(|t| {
                t.names()
                    .any(|n| n.to_ascii_lowercase().starts_with(&query))
            })
            .collect::<Vec<_>>()
    } else {
        exact
    };
    match matches.as_slice() {
        [topic] => Ok(format!(
            "{}\n\n{}\n",
            topic.title,
            read_file(&root, &topic.file)?
        )),
        [] => Ok("No such help topic. Type help to browse, or help search <words>.".into()),
        _ => Ok(format!(
            "Choose a more specific topic: {}",
            matches
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn answer_help(
    mut requests: MessageReader<EngineCommand>,
    players: Query<&grim::Character, With<grim::Player>>,
    dir: Res<HelpDir>,
    mut replies: MessageWriter<InfoMessage>,
) {
    for request in requests.read() {
        let Command::Help { topic } = &request.command else {
            continue;
        };
        let Ok(character) = players.get(request.client) else {
            continue;
        };
        let text = render(&dir.0, topic, character.is_admin()).unwrap_or_else(|error| error);
        replies.write(InfoMessage {
            target: request.client,
            text,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reloads_text_and_filters_wizard_topics_and_paths() {
        let dir = std::env::temp_dir().join(format!("grim-help-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.json"), r#"[
          {"id":"drink","title":"Drinking","category":"Items","file":"drink.md","aliases":["water"]},
          {"id":"copyover","title":"Hot restart","category":"Wizard","file":"copyover.md","wizard":true},
          {"id":"bad","title":"Bad path","category":"Items","file":"../outside.md"}
        ]"#).unwrap();
        std::fs::write(dir.join("drink.md"), "Drink water.").unwrap();
        std::fs::write(dir.join("copyover.md"), "Restart live connections.").unwrap();
        assert!(render(&dir, "WATER", false)
            .unwrap()
            .contains("Drink water."));
        std::fs::write(dir.join("drink.md"), "Updated immediately.").unwrap();
        assert!(render(&dir, "drink", false)
            .unwrap()
            .contains("Updated immediately."));
        assert!(!render(&dir, "", false).unwrap().contains("copyover"));
        assert!(!render(&dir, "search restart", false)
            .unwrap()
            .contains("copyover"));
        assert!(render(&dir, "copyover", true)
            .unwrap()
            .contains("Restart live"));
        assert!(render(&dir, "bad", false).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

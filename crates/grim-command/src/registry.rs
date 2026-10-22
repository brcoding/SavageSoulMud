//! The registry itself: registration, priority ordering, and exact-then-prefix
//! resolution.

use bevy::prelude::Resource;

mod contest;

pub use contest::Contest;

/// A registry of named command factories with explicit priority.
///
/// Usable as a Bevy resource once instantiated with a concrete `C`
/// (`CommandRegistry<MyCommand>`).
#[derive(Resource)]
pub struct CommandRegistry<C: Send + Sync + 'static> {
    /// Entries in registration order; indices are stable once assigned.
    entries: Vec<Entry<C>>,
    /// Indices into `entries`, ordered most-preferred first.
    priority: Vec<usize>,
}

/// A command factory: the rest of the line in, the command (or a rejection)
/// out. Boxed so data-driven commands (socials) can close over their name;
/// static commands pass the same non-capturing closures as before.
pub type Factory<C> = Box<dyn Fn(&str) -> Option<C> + Send + Sync>;

struct Entry<C> {
    name: String,
    factory: Factory<C>,
    /// Listing section (`commands` vs `socials`). `None` = the default
    /// section. Data-driven sets tag their names so listings can split.
    section: Option<String>,
}

impl<C: Send + Sync + 'static> Default for CommandRegistry<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Send + Sync + 'static> CommandRegistry<C> {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            priority: Vec::new(),
        }
    }

    /// Register `name` with a factory. The factory receives the rest of the
    /// line (everything after the command word, trimmed) and returns the
    /// command, or `None` to reject the input (e.g. `say` with no text).
    ///
    /// The new command starts at the front of the priority ordering.
    /// Call [`CommandRegistry::deprioritize`] after registering a bulk
    /// data-driven set (socials) so static commands keep their prefixes.
    pub fn register(
        &mut self,
        name: &str,
        factory: impl Fn(&str) -> Option<C> + Send + Sync + 'static,
    ) {
        let idx = self.entries.len();
        self.entries.push(Entry {
            name: name.to_ascii_lowercase(),
            factory: Box::new(factory),
            section: None,
        });
        self.priority.insert(0, idx);
    }

    /// Whether `name` is already registered (case-insensitive). Data-driven
    /// sets check this first so a dynamic name can never shadow a static
    /// command's exact match.
    pub fn contains(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        self.entries.iter().any(|e| e.name == name)
    }

    /// Replace a registered factory without changing its priority or section.
    /// Returns false when the command is not registered.
    pub fn replace(
        &mut self,
        name: &str,
        factory: impl Fn(&str) -> Option<C> + Send + Sync + 'static,
    ) -> bool {
        let Some(idx) = self.priority.iter().copied().find(|&i| self.entries[i].name.eq_ignore_ascii_case(name)) else {
            return false;
        };
        self.entries[idx].factory = Box::new(factory);
        true
    }

    /// Tag `name` into a listing section (case-insensitive, no-op when
    /// unregistered). Sections split listings: `commands` shows the default
    /// section, `socials` shows the `"social"` section.
    pub fn set_section(&mut self, name: &str, section: &str) {
        let name = name.to_ascii_lowercase();
        if let Some(entry) = self.entries.iter_mut().find(|e| e.name == name) {
            entry.section = Some(section.to_string());
        }
    }

    /// Every registered name in `section`, sorted.
    pub fn names_in_section(&self, section: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .entries
            .iter()
            .filter(|e| e.section.as_deref() == Some(section))
            .map(|e| e.name.clone())
            .collect();
        names.sort();
        names
    }

    /// Resolve a command `word` (case-insensitive) to a command, passing `rest`
    /// to the matched factory. Returns `None` when nothing matches, or when the
    /// matched factory rejects `rest`.
    pub fn resolve(&self, word: &str, rest: &str) -> Option<C> {
        let word = word.to_ascii_lowercase();

        // 1. exact match wins outright, highest priority among exacts.
        if let Some(idx) = self
            .priority
            .iter()
            .copied()
            .find(|&i| self.entries[i].name == word)
        {
            return (self.entries[idx].factory)(rest);
        }

        // 2. otherwise the highest-priority prefix match.
        let idx = self
            .priority
            .iter()
            .copied()
            .find(|&i| self.entries[i].name.starts_with(&word))?;
        (self.entries[idx].factory)(rest)
    }

    /// Every registered name, sorted, for listings (`commands`). Includes
    /// data-driven names registered at Startup.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.entries.iter().map(|e| e.name.clone()).collect();
        names.sort();
        names
    }

    /// Move a command to the front of the priority ordering (highest). No-op if
    /// the name is not registered.
    pub fn prioritize(&mut self, name: &str) {
        self.reorder(name, true);
    }

    /// Move a command to the back of the priority ordering (lowest). No-op if
    /// the name is not registered.
    pub fn deprioritize(&mut self, name: &str) {
        self.reorder(name, false);
    }

    fn reorder(&mut self, name: &str, front: bool) {
        let name = name.to_ascii_lowercase();
        let Some(idx) = self.entries.iter().position(|e| e.name == name) else {
            return;
        };
        self.priority.retain(|&i| i != idx);
        if front {
            self.priority.insert(0, idx);
        } else {
            self.priority.push(idx);
        }
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! Shared test command type + factory, reused by the resolution, reorder,
    //! and contest test submodules.

    #[derive(Debug, PartialEq, Eq)]
    pub(crate) enum Cmd {
        North,
        Note(String),
        Nordic(String),
        Look(Option<String>),
    }

    pub(crate) fn look(rest: &str) -> Option<Cmd> {
        Some(Cmd::Look((!rest.is_empty()).then(|| rest.to_string())))
    }
}

#[cfg(test)]
mod resolution_tests {
    use super::fixture::{look, Cmd};
    use super::*;

    #[test]
    fn replace_updates_resolved_duplicate_and_preserves_priority() {
        let mut registry = CommandRegistry::new();
        registry.register("look", look);
        registry.register("look", |_| Some(Cmd::North));
        registry.register("long", |_| Some(Cmd::Nordic(String::new())));
        assert!(registry.replace("LOOK", |_| Some(Cmd::Note("replacement".into()))));
        assert_eq!(registry.resolve("look", ""), Some(Cmd::Note("replacement".into())));
        assert_eq!(registry.resolve("l", ""), Some(Cmd::Nordic(String::new())));
        assert!(!registry.replace("missing", look));
    }

    #[test]
    fn exact_match_beats_higher_priority_prefix() {
        let mut r = CommandRegistry::new();
        r.register("look", look);
        // "l" registered later => higher priority, but "look" is an exact hit.
        r.register("l", look);
        assert_eq!(r.resolve("look", ""), Some(Cmd::Look(None)));
        // "l" is exact for the shorthand.
        assert_eq!(r.resolve("l", ""), Some(Cmd::Look(None)));
    }

    #[test]
    fn last_registered_wins_a_prefix_by_default() {
        let mut r = CommandRegistry::new();
        r.register("note", |rest| Some(Cmd::Note(rest.to_string())));
        r.register("nordic", |rest| Some(Cmd::Nordic(rest.to_string())));
        r.register("north", |_| Some(Cmd::North));
        // all three share "n"; north registered last => wins
        assert_eq!(r.resolve("n", ""), Some(Cmd::North));
        assert_eq!(r.resolve("no", ""), Some(Cmd::North));
        assert_eq!(r.resolve("nor", ""), Some(Cmd::North));
        // unambiguous longer prefixes still reach their own command
        assert_eq!(r.resolve("nord", ""), Some(Cmd::Nordic(String::new())));
        assert_eq!(r.resolve("not", ""), Some(Cmd::Note(String::new())));
    }

    #[test]
    fn resolve_is_case_insensitive() {
        let mut r = CommandRegistry::new();
        r.register("north", |_| Some(Cmd::North));
        assert_eq!(r.resolve("NORTH", ""), Some(Cmd::North));
        assert_eq!(r.resolve("N", ""), Some(Cmd::North));
    }

    #[test]
    fn factory_rejection_returns_none() {
        let mut r = CommandRegistry::new();
        r.register("say", |rest| {
            (!rest.is_empty()).then(|| Cmd::Note(rest.to_string()))
        });
        assert_eq!(r.resolve("say", ""), None);
        assert_eq!(r.resolve("say", "hi"), Some(Cmd::Note("hi".into())));
    }

    #[test]
    fn unknown_word_resolves_to_none() {
        let mut r = CommandRegistry::new();
        r.register("north", |_| Some(Cmd::North));
        assert_eq!(r.resolve("xyzzy", ""), None);
    }

    #[test]
    fn capturing_closure_factory_resolves() {
        let mut r = CommandRegistry::new();
        for name in ["grin", "smile"] {
            let owned = name.to_string();
            r.register(name, move |rest| Some(Cmd::Note(format!("{owned}:{rest}"))));
            r.deprioritize(name);
        }
        assert_eq!(r.resolve("grin", "bob"), Some(Cmd::Note("grin:bob".into())));
        assert_eq!(r.resolve("SMILE", ""), Some(Cmd::Note("smile:".into())));
    }

    #[test]
    fn contains_is_case_insensitive() {
        let mut r = CommandRegistry::new();
        r.register("say", |_| Some(Cmd::North));
        assert!(r.contains("say"));
        assert!(r.contains("SAY"));
        assert!(!r.contains("yell"));
    }

    #[test]
    fn sections_split_listings() {
        let mut r = CommandRegistry::new();
        r.register("say", |_| Some(Cmd::North));
        r.register("grin", |_| Some(Cmd::North));
        r.set_section("grin", "social");
        r.set_section("missing", "social");
        assert_eq!(r.names_in_section("social"), vec!["grin"]);
        assert_eq!(r.names(), vec!["grin", "say"]);
    }

    #[test]
    fn names_lists_sorted_unique_entries() {
        let mut r = CommandRegistry::new();
        r.register("say", |_| Some(Cmd::North));
        r.register("grin", |_| Some(Cmd::North));
        r.register("look", look);
        assert_eq!(r.names(), vec!["grin", "look", "say"]);
    }
}

#[cfg(test)]
mod reorder_tests {
    use super::fixture::Cmd;
    use super::*;

    #[test]
    fn prioritize_moves_to_front() {
        let mut r = CommandRegistry::new();
        r.register("note", |rest| Some(Cmd::Note(rest.to_string())));
        r.register("north", |_| Some(Cmd::North));
        assert_eq!(r.resolve("n", ""), Some(Cmd::North)); // north last => wins
        r.prioritize("note");
        assert_eq!(r.resolve("n", ""), Some(Cmd::Note(String::new())));
    }

    #[test]
    fn deprioritize_moves_to_back() {
        let mut r = CommandRegistry::new();
        r.register("note", |rest| Some(Cmd::Note(rest.to_string())));
        r.register("north", |_| Some(Cmd::North));
        r.deprioritize("north");
        assert_eq!(r.resolve("n", ""), Some(Cmd::Note(String::new())));
    }

    #[test]
    fn reorder_unknown_name_is_noop() {
        let mut r = CommandRegistry::new();
        r.register("north", |_| Some(Cmd::North));
        r.prioritize("nope");
        r.deprioritize("nope");
        assert_eq!(r.resolve("n", ""), Some(Cmd::North));
    }
}

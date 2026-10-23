use bevy::prelude::Resource;

/// Chooses the character creation menus without changing existing characters.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterBuildMode {
    /// The engine's gender, race and class menus.
    #[default]
    Legacy,
    /// Name, gender and race; no class picker; the game owns attribute and Peculiarity purchases.
    SavageSoul,
}

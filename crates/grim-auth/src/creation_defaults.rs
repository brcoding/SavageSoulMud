//! Game-owned initial state, keyed by the race selected during creation.
use bevy::prelude::Resource;
use std::collections::HashMap;

/// Initial game-state values for brand-new characters. Existing saves and
/// legacy backfill never receive these defaults. The author validates them.
#[derive(Resource, Default, Clone, Debug)]
pub struct CharacterCreationDefaults(pub HashMap<String, HashMap<String, serde_json::Value>>);

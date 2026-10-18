//! NullAbility (abilities.md §13.3): an instant ability with no effect.

use super::prelude::*;

/// Module state (one per ability object).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct NullAbility;

/// Objects this module spawns.
pub type Spawned = NoSpawn;

impl AbilityImpl for NullAbility {
    fn new(_w: &World, _def: &AbilityDef) -> Self {
        NullAbility
    }
}

//! Random (abilities.md §13.1): resolved at spawn; never castable. Its
//! enter would exit immediately (the framework default).

use super::prelude::*;

/// Module state (one per ability object).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Random;

/// Objects this module spawns.
pub type Spawned = NoSpawn;

impl AbilityImpl for Random {
    fn new(_w: &World, _def: &AbilityDef) -> Self {
        Random
    }

    fn is_castable(&self, _w: &World, _fw: &super::Framework, _player: i32) -> bool {
        false
    }
}

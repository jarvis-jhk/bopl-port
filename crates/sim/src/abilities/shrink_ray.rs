//! Shrink ray (`docs/spec/abilities/shrink-ray.md`).
//!
//! The Growth ray configuration with the opposite signs: GunTransform
//! (blink-gun.md) + ShootScaleChange / ScaleChanger (growth-ray.md, module
//! `growth_ray`), all values read from the `Shrink ray` prefab.

use super::NoSpawn;
use super::blink_gun::GunAbility;
use super::growth_ray::ShootScaleChange;

/// The Shrink ray ability.
pub type ShrinkRay = GunAbility<ShootScaleChange>;

/// Objects this module spawns (its `ScaleChanger`s are growth-ray objects).
pub type Spawned = NoSpawn;

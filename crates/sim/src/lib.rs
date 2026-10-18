//! `bopl-sim`: the deterministic simulation of Bopl Battle.
//!
//! See `docs/sim-design.md` for the architecture and `docs/spec/` for the
//! behaviour it reproduces.

// Formulas keep the spec's explicit `a = a · b` grouping.
#![allow(clippy::assign_op_pattern)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::large_enum_variant)]

pub mod abilities;
pub mod anim;
pub mod bopl;
pub mod consts;
pub mod curve;
pub mod data;
pub mod fix;
pub mod flow;
pub mod input;
pub mod lut;
pub mod object;
pub mod physics;
pub mod platform;
pub mod player;
pub mod rng;
pub mod session;
pub mod sort;
pub mod surface;
pub mod tutorial;
pub mod vec2;
pub mod view;
pub mod world;

pub use fix::Fix;
pub use input::PlayerInput;
pub use vec2::Vec2;
pub use world::World;

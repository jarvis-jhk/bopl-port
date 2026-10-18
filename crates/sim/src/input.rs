//! Per-tick player input (player.md §2).
//!
//! [`PlayerInput`] is exactly what the original samples per tick and sends
//! over the network: the direction byte, the six buttons (jump, three
//! abilities, start, select) and W/A/S/D. It is a 3-byte `Pod`, so it can go
//! straight into GGRS / replays.

use std::sync::OnceLock;

use bytemuck::{Pod, Zeroable};

use crate::consts::DEADZONE;
use crate::fix::Fix;
use crate::vec2::Vec2;

/// One player's input for one tick.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct PlayerInput {
    /// `joystickAngleByte`: 0 = neutral, 1…255 = direction (1 = +x, counter-clockwise).
    pub angle: u8,
    /// Button bits, see the `BTN_*` constants.
    pub buttons: u8,
    /// W/A/S/D bits (`KEY_*`), only read for keyboard-and-mouse players.
    pub keys: u8,
}

pub const BTN_JUMP: u8 = 1 << 0;
/// Ability slot 0 (gamepad West / left mouse).
pub const BTN_ABILITY0: u8 = 1 << 1;
/// Ability slot 1 (gamepad East / right mouse).
pub const BTN_ABILITY1: u8 = 1 << 2;
/// Ability slot 2 (gamepad North / middle mouse).
pub const BTN_ABILITY2: u8 = 1 << 3;
pub const BTN_START: u8 = 1 << 4;
pub const BTN_SELECT: u8 = 1 << 5;

pub const KEY_W: u8 = 1 << 0;
pub const KEY_A: u8 = 1 << 1;
pub const KEY_S: u8 = 1 << 2;
pub const KEY_D: u8 = 1 << 3;

impl PlayerInput {
    /// Input with a direction byte and buttons.
    pub const fn new(angle: u8, buttons: u8) -> PlayerInput {
        PlayerInput {
            angle,
            buttons,
            keys: 0,
        }
    }

    #[inline]
    pub fn jump(&self) -> bool {
        self.buttons & BTN_JUMP != 0
    }

    /// Ability button of slot `k` (0, 1, 2).
    #[inline]
    pub fn ability(&self, k: usize) -> bool {
        k < 3 && self.buttons & (BTN_ABILITY0 << k) != 0
    }

    /// A direction byte for a vector of any length outside the deadzone
    /// (helper for scripted input).
    pub fn with_direction(mut self, v: Vec2) -> PlayerInput {
        self.angle = angle_byte_from_direction(v);
        self
    }
}

/// `InputVectorTable`: byte → unit vector (interpolated trig).
pub fn input_vector_table() -> &'static [Vec2; 256] {
    static T: OnceLock<[Vec2; 256]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [Vec2::ZERO; 256];
        for (b, e) in t.iter_mut().enumerate().skip(1) {
            let a = Fix::PI_TIMES_2 * (Fix::from_int(b as i32 - 1) / Fix::from_int(255));
            *e = Vec2::new(a.cos(), a.sin());
        }
        t
    })
}

/// `JoyStickAngleByteFromVector` (literal: the input path always passes a
/// normalised vector; for other vectors `NormalizedVectorAngle` reduces `x`
/// modulo `One + ε` and the byte is meaningless, as in the original — use
/// [`angle_byte_from_direction`]).
pub fn angle_byte_from_vector(v: Vec2) -> u8 {
    if v.sqr_magnitude() < DEADZONE * DEADZONE {
        return 0;
    }
    let a = v.normalized_vector_angle() / Fix::PI_TIMES_2 * Fix::from_int(255);
    ((a.to_int().rem_euclid(255)) + 1) as u8
}

/// The byte of a direction of any length: the device side's
/// normalisation, then [`angle_byte_from_vector`] (0 inside the deadzone).
pub fn angle_byte_from_direction(v: Vec2) -> u8 {
    if v.sqr_magnitude() < DEADZONE * DEADZONE {
        return 0;
    }
    angle_byte_from_vector(v.normalized_safe())
}

/// The per-player input profile the simulation reads (current profile).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct InputProfile {
    pub input: PlayerInput,
    pub uses_mouse: bool,
    pub jump_down_this_frame: bool,
    pub jump_down_last_frame: bool,
}

impl InputProfile {
    /// Tick-start edge detection (engine.md §4 step 3).
    pub fn update_edges(&mut self) {
        let j = self.input.jump();
        self.jump_down_this_frame = !self.jump_down_last_frame && j;
        self.jump_down_last_frame = j;
    }

    /// `InputVector()`: WASD sum for mouse players, else the table vector.
    pub fn input_vector(&self) -> Vec2 {
        if self.uses_mouse {
            let k = self.input.keys;
            let mut v = Vec2::ZERO;
            if k & KEY_W != 0 {
                v.y += Fix::ONE;
            }
            if k & KEY_S != 0 {
                v.y -= Fix::ONE;
            }
            if k & KEY_D != 0 {
                v.x += Fix::ONE;
            }
            if k & KEY_A != 0 {
                v.x -= Fix::ONE;
            }
            v
        } else {
            input_vector_table()[self.input.angle as usize]
        }
    }

    /// `AimVector()`: always the table vector.
    pub fn aim_vector(&self) -> Vec2 {
        input_vector_table()[self.input.angle as usize]
    }
}

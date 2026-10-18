//! The simulation RNG: Unity.Mathematics.Random (32-bit xorshift) and the
//! game's wrappers (fixmath.md §11).

use crate::fix::Fix;
use crate::vec2::Vec2;

/// The xorshift generator. `Default` is the static instance's initial state
/// (seed 37, state 9479367).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rng {
    pub state: u32,
}

impl Default for Rng {
    fn default() -> Rng {
        Rng::new(37)
    }
}

impl Rng {
    /// `new Random(seed)`.
    pub fn new(seed: u32) -> Rng {
        let mut r = Rng { state: seed };
        r.next_state();
        r
    }

    /// `InitSeed(seed)`.
    pub fn init_seed(&mut self, seed: u32) {
        self.state = seed;
        self.next_state();
    }

    /// Returns the current state, then advances it.
    #[inline]
    pub fn next_state(&mut self) -> u32 {
        let s = self.state;
        let mut t = s;
        t ^= t << 13;
        t ^= t >> 17;
        t ^= t << 5;
        self.state = t;
        s
    }

    #[inline]
    pub fn next_uint(&mut self, min: u32, max: u32) -> u32 {
        let range = max.wrapping_sub(min);
        ((((self.next_state() as u64) * (range as u64)) >> 32) as u32).wrapping_add(min)
    }

    #[inline]
    pub fn next_int(&mut self, min: i32, max: i32) -> i32 {
        let range = max.wrapping_sub(min) as u32;
        ((((self.next_state() as u64) * (range as u64)) >> 32) as i32).wrapping_add(min)
    }

    /// `RandomInt(from, to)`: `[from, to)`.
    #[inline]
    pub fn random_int(&mut self, from: i32, to: i32) -> i32 {
        self.next_int(from, to)
    }

    /// `RandomFix(from, to)` with the original's fraction quirk.
    pub fn random_fix(&mut self, from: Fix, to: Fix) -> Fix {
        let ff = from.floor();
        let tf = to.floor();
        let from_frac = (from.0 & 0xFFFF_FFFF) as u32;
        if ff == tf {
            let u = self.next_uint(from_frac, (to.0 & 0xFFFF_FFFF) as u32);
            Fix(ff.0.wrapping_add(u as i64))
        } else {
            let i = self.next_int(ff.to_int(), tf.to_int());
            let u = self.next_uint(from_frac, 0xFFFF_FFFF);
            Fix(((i as i64) << 32).wrapping_add(u as i64))
        }
    }

    /// `RandomUnitVector() = Vec2(RandomFix(0, 2π))`.
    pub fn random_unit_vector(&mut self) -> Vec2 {
        Vec2::from_angle(self.random_fix(Fix::ZERO, Fix::PI_TIMES_2))
    }

    /// `RandomInsideUnitCircle()`: angle first, then a uniform radius.
    pub fn random_inside_unit_circle(&mut self) -> Vec2 {
        let v = Vec2::from_angle(self.random_fix(Fix::ZERO, Fix::PI_TIMES_2));
        v * self.random_fix(Fix::ZERO, Fix::ONE)
    }
}

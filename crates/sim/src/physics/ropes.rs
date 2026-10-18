//! DetPhysics rope bodies: the pool, the rope-vs-rounded-rect sweep and the
//! Verlet rope simulation (physics.md §6.3, §11.1; abilities/grappling-hook.md
//! "Rope body", "Rope simulation"). Game-side rope logic (rope objects,
//! hooks, anchors, body coupling) is in `abilities/rope.rs`.

use std::cmp::Ordering;

use super::shapes::{Aabb, ShapeKind};
use super::{Physics, is_active};
use crate::fix::Fix;
use crate::object::Objects;
use crate::sort::introsort;
use crate::vec2::Vec2;

/// Pool size (`RopeBodyList`).
pub const MAX_ROPES: usize = 128;
/// Points per rope.
pub const MAX_SEGMENTS: usize = 40;
/// Colliding rounded rects recorded per rope and tick.
pub const MAX_RECTS: usize = 32;

/// `(Fix)0.1` (double).
const F_0_1D: Fix = Fix(429_496_729);
/// `(Fix)0.5f`.
const HALF: Fix = Fix(2_147_483_648);

/// One pooled rope body.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RopeBody {
    pub segment: [Vec2; MAX_SEGMENTS],
    pub prev: [Vec2; MAX_SEGMENTS],
    pub count: usize,
    pub enabled: bool,
    pub disabled_this_frame: bool,
    pub detached: bool,
    /// Owner id (stored as a byte).
    pub owner: u8,
    pub number: u32,
    pub index: usize,
    pub force_start: Vec2,
    pub force_end: Vec2,
    pub time_in_air_start: Fix,
    pub time_in_air_end: Fix,
    pub mass_start: Fix,
    pub mass_end: Fix,
    /// `< 0` = hook not arrived; "arrived" means `> 0`.
    pub time_since_hook_arrived: Fix,
    pub separation: Fix,
    /// Rounded-rect list indices collected by the sweep this tick.
    pub rects: [u16; MAX_RECTS],
    pub n_rects: u8,
}

impl RopeBody {
    fn new(index: usize) -> RopeBody {
        RopeBody {
            segment: [Vec2::ZERO; MAX_SEGMENTS],
            prev: [Vec2::ZERO; MAX_SEGMENTS],
            count: 0,
            enabled: false,
            disabled_this_frame: false,
            detached: false,
            owner: 0,
            number: 0,
            index,
            force_start: Vec2::ZERO,
            force_end: Vec2::ZERO,
            time_in_air_start: Fix::ZERO,
            time_in_air_end: Fix::ZERO,
            mass_start: Fix::ZERO,
            mass_end: Fix::ZERO,
            time_since_hook_arrived: Fix::ZERO,
            separation: Fix::ZERO,
            rects: [0; MAX_RECTS],
            n_rects: 0,
        }
    }

    /// `hookHasArrived`.
    pub fn hook_arrived(&self) -> bool {
        self.time_since_hook_arrived > Fix::ZERO
    }

    /// Index of the end a body drives (`top` = segment 0).
    pub fn end(&self, top: bool) -> usize {
        if top { 0 } else { self.count - 1 }
    }

    /// `SetMass01(m, end, s)`: `Clamp01((m + s · 0.1) − 0.1)`.
    pub fn set_mass01(&mut self, m: Fix, top: bool, s: Fix) {
        let v = ((m + s * F_0_1D) - F_0_1D).clamp01();
        if top {
            self.mass_start = v;
        } else {
            self.mass_end = v;
        }
    }

    pub fn set_force(&mut self, f: Vec2, top: bool) {
        if top {
            self.force_start = f;
        } else {
            self.force_end = f;
        }
    }

    pub fn set_time_in_air(&mut self, t: Fix, top: bool) {
        if top {
            self.time_in_air_start = t;
        } else {
            self.time_in_air_end = t;
        }
    }

    /// `Dettach(end)`: detached; the opposite end's mass := 0.5.
    pub fn dettach(&mut self, top: bool) {
        self.detached = true;
        if top {
            self.mass_end = HALF;
        } else {
            self.mass_start = HALF;
        }
    }

    /// `SpawnAdditionalSegment(p)`.
    pub fn spawn_additional_segment(&mut self, p: Vec2) -> bool {
        if self.count >= MAX_SEGMENTS {
            return false;
        }
        self.count += 1;
        let last = self.count - 1;
        self.segment[last] = self.segment[last - 1];
        self.prev[last] = self.prev[last - 1];
        self.segment[last - 1] = p;
        self.prev[last - 1] = p;
        true
    }

    /// `ReelInSegment(min)`.
    pub fn reel_in_segment(&mut self, min: usize) -> bool {
        if self.count <= min {
            return false;
        }
        let last = self.count - 1;
        self.segment[last - 1] = self.segment[last];
        self.prev[last - 1] = self.prev[last];
        self.count -= 1;
        true
    }

    /// AABB over the interior points (`ComputeBoundingBox(RopeBody)`).
    fn aabb(&self) -> Aabb {
        let mut min = self.segment[1];
        let mut max = self.segment[1];
        for i in 2..self.count.saturating_sub(1) {
            let v = self.segment[i];
            max.x = if v.x > max.x { v.x } else { max.x };
            max.y = if v.y > max.y { v.y } else { max.y };
            min.x = if v.x < min.x { v.x } else { min.x };
            min.y = if v.y < min.y { v.y } else { min.y };
        }
        Aabb {
            min,
            max,
            id: self.index as i32,
            layer: 12,
            shape: ShapeKind::Rope as u8,
        }
    }

    /// `SimulateRope` (one rope).
    fn simulate(&mut self, p: &Physics, dt: Fix, space: bool) {
        let (fs, fe, ms, me) = if self.hook_arrived() {
            (
                self.force_start,
                self.force_end,
                self.mass_start,
                self.mass_end,
            )
        } else {
            (Vec2::ZERO, Vec2::ZERO, Fix::ONE, Fix::ONE)
        };
        let g = Vec2::DOWN * p.params.rope_gravity * if space { Fix::HALF } else { Fix::ONE };
        let n = self.count - 1;
        let two = Fix::TWO;
        let s0 = self.segment[0];
        self.segment[0] = two * self.segment[0] - self.prev[0] + dt * fs;
        self.prev[0] = s0;
        for i in 1..n {
            let s = self.segment[i];
            self.segment[i] = two * self.segment[i] - self.prev[i] + dt * g;
            self.prev[i] = s;
        }
        let sn = self.segment[n];
        self.segment[n] = two * self.segment[n] - self.prev[n] + dt * fe;
        self.prev[n] = sn;
        let c = self.count;
        for _ in 0..p.params.rope_iterations {
            let d = self.segment[1] - self.segment[0];
            let l = d.magnitude();
            if l == Fix::ZERO {
                continue;
            }
            let k = -l;
            let u = d / l;
            self.segment[1] += k * u * ms;
            self.segment[0] -= k * u * (Fix::ONE - ms);
            let mut j = 1;
            while j + 2 < c {
                let d = self.segment[j] - self.segment[j + 1];
                let l = d.magnitude();
                if l != Fix::ZERO {
                    let k = self.separation - l;
                    let u = d / l;
                    self.segment[j] += k * u * HALF;
                    self.segment[j + 1] -= k * u * HALF;
                }
                j += 1;
            }
            let d = self.segment[c - 2] - self.segment[c - 1];
            let l = d.magnitude();
            if l != Fix::ZERO {
                let k = -l;
                let u = d / l;
                self.segment[c - 2] += k * u * me;
                self.segment[c - 1] -= k * u * (Fix::ONE - me);
            }
        }
        let stretch = p.params.platform_stretchiness / (self.separation * self.separation);
        for &ri in &self.rects[..self.n_rects as usize] {
            let Some(rr) = p.rrs.shapes.get(ri as usize) else {
                continue;
            };
            for m in 1..c - 1 {
                if rr.contains(self.segment[m]) {
                    self.segment[m] = rr.closest_point(self.segment[m]);
                    if self.mass_end != HALF && self.mass_start != HALF {
                        let t = ((self.segment[m] - self.segment[m - 1]).sqr_magnitude() * stretch)
                            .clamp01()
                            * F_0_5D;
                        self.segment[m] = self.segment[m].lerp(self.segment[m - 1], t);
                        self.segment[m] = rr.closest_point(self.segment[m]);
                    }
                }
            }
        }
        self.n_rects = 0;
    }
}

/// `(Fix)0.5` (double).
const F_0_5D: Fix = Fix(2_147_483_648);

/// The rope pool (`RopeBodyList`) plus the global rope counter.
///
/// The pool grows lazily up to [`MAX_ROPES`]; a slot index is the same as
/// in the original's fixed array (allocation takes the first free slot).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Ropes {
    pub bodies: Vec<RopeBody>,
    /// `Rope.ropeNumber` (static; never reset during a session).
    pub counter: u32,
    /// Space level (rope gravity halved).
    pub space: bool,
    /// `SimDeltaTimeThisFrame`.
    pub sim_dt: Fix,
}

impl Ropes {
    /// `AddRope(n)`: first slot neither enabled nor disabled this frame.
    pub fn allocate(&mut self, n: usize) -> Option<usize> {
        let i = match self
            .bodies
            .iter()
            .position(|r| !r.enabled && !r.disabled_this_frame)
        {
            Some(i) => i,
            None if self.bodies.len() < MAX_ROPES => {
                self.bodies.push(RopeBody::new(self.bodies.len()));
                self.bodies.len() - 1
            }
            None => return None,
        };
        let r = &mut self.bodies[i];
        r.count = n;
        r.enabled = true;
        r.index = i;
        r.force_end = Vec2::ZERO;
        r.force_start = Vec2::ZERO;
        r.disabled_this_frame = false;
        r.detached = false;
        r.separation = Fix::ZERO;
        r.time_in_air_end = Fix::ZERO;
        r.time_in_air_start = Fix::ZERO;
        r.time_since_hook_arrived = Fix::ZERO;
        Some(i)
    }

    /// `RemoveRope(r)`: disabled, not reusable until the end of the tick.
    pub fn remove(&mut self, i: usize) {
        if let Some(r) = self.bodies.get_mut(i) {
            r.enabled = false;
            r.disabled_this_frame = true;
        }
    }

    pub fn get(&self, i: usize) -> Option<&RopeBody> {
        self.bodies.get(i)
    }

    pub fn get_mut(&mut self, i: usize) -> Option<&mut RopeBody> {
        self.bodies.get_mut(i)
    }

    /// An enabled rope.
    pub fn enabled(&self, i: usize) -> Option<&RopeBody> {
        self.bodies.get(i).filter(|r| r.enabled)
    }

    /// `VeryLateRopeUpdate` (tick step 12).
    pub fn very_late_update(&mut self) {
        for r in &mut self.bodies {
            r.disabled_this_frame = false;
        }
    }

    pub fn any_enabled(&self) -> bool {
        self.bodies.iter().any(|r| r.enabled)
    }
}

impl Physics {
    /// `SimulateRope(rope, null)`: one simulation step of a single rope with
    /// this frame's rope time step and no platform list (MachoThrow2 after
    /// re-anchoring a carried rope).
    pub fn simulate_single_rope(&mut self, i: usize) {
        let dt = self.ropes.sim_dt;
        let space = self.ropes.space;
        let Some(mut r) = self.ropes.bodies.get(i).cloned() else {
            return;
        };
        r.n_rects = 0;
        r.simulate(self, dt, space);
        self.ropes.bodies[i] = r;
    }

    /// Rope sweep (§6.3) and `SimulateRopes` (§11.1), Simulate step 8.
    pub(super) fn simulate_ropes(&mut self, objs: &Objects, dt: Fix) {
        let dt = if dt == Fix::ZERO {
            crate::consts::FIXED_DT * F_0_1D
        } else {
            dt
        };
        self.ropes.sim_dt = dt;
        if !self.ropes.any_enabled() {
            return;
        }
        // The rope array: active rounded rects (list order), then ropes.
        let mut bb: Vec<Aabb> = Vec::new();
        let mut sum = Vec2::ZERO;
        let mut count = 0i32;
        for i in 0..self.rrs.len() {
            if is_active(objs, self.rrs.parents[i].obj) {
                bb.push(self.rrs.shapes[i].aabb(i as i32));
                sum += self.rrs.shapes[i].b.center;
                count += 1;
            }
        }
        for i in 0..self.circles.len() {
            if is_active(objs, self.circles.parents[i].obj) {
                sum += self.circles.shapes[i].center;
                count += 1;
            }
        }
        for i in 0..self.boxes.len() {
            if is_active(objs, self.boxes.parents[i].obj) {
                sum += self.boxes.shapes[i].center;
                count += 1;
            }
        }
        for r in &self.ropes.bodies {
            if r.enabled {
                bb.push(r.aabb());
            }
        }
        let mean = sum / Fix::from_int(count.max(1));
        let mut var = Vec2::ZERO;
        for s in &self.rrs.shapes {
            let d = mean - s.b.center;
            var += Vec2::new(d.x * d.x, d.y * d.y);
        }
        let along_x = var.x > var.y;
        let key = |a: &Aabb| {
            if along_x {
                (a.min.x, a.max.x)
            } else {
                (a.min.y, a.max.y)
            }
        };
        let other = |a: &Aabb| {
            if along_x {
                (a.min.y, a.max.y)
            } else {
                (a.min.x, a.max.x)
            }
        };
        introsort(&mut bb, |a, b| {
            let s = (key(a).0 - key(b).0).sign();
            if s != 0 {
                return s.cmp(&0);
            }
            let s = a.shape as i32 - b.shape as i32;
            if s != 0 {
                return s.cmp(&0);
            }
            match (a.id - b.id).cmp(&0) {
                Ordering::Equal => Ordering::Equal,
                o => o,
            }
        });
        let rr = ShapeKind::RoundedRect as u8;
        let rope = ShapeKind::Rope as u8;
        if bb.len() >= 2 {
            let mut active: Vec<usize> = vec![0];
            for i in 1..bb.len() {
                let bi = bb[i];
                let mut k = 0;
                while k < active.len() {
                    let a = bb[active[k]];
                    if key(&a).1 <= key(&bi).0 {
                        active.remove(k);
                        continue;
                    }
                    if other(&a).1 > other(&bi).0 && other(&a).0 < other(&bi).1 {
                        let (rect, rp) = if a.shape == rr && bi.shape == rope {
                            (a.id, bi.id)
                        } else if a.shape == rope && bi.shape == rr {
                            (bi.id, a.id)
                        } else {
                            k += 1;
                            continue;
                        };
                        if let Some(r) = self.ropes.bodies.get_mut(rp as usize)
                            && (r.n_rects as usize) < MAX_RECTS
                        {
                            r.rects[r.n_rects as usize] = rect as u16;
                            r.n_rects += 1;
                        }
                    }
                    k += 1;
                }
                active.push(i);
            }
        }
        let space = self.ropes.space;
        let mut bodies = std::mem::take(&mut self.ropes.bodies);
        for r in bodies.iter_mut().filter(|r| r.enabled) {
            r.simulate(self, dt, space);
        }
        self.ropes.bodies = bodies;
    }
}

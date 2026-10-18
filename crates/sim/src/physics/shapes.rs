//! DetPhysics shapes, narrow phase, ray casts and point helpers
//! (physics.md §1.1, §6.1, §7, §8.3, §9.2, §9.4). Pure functions on values.

use crate::consts::{POS_CORRECTION_STR, SLOP};
use crate::fix::Fix;
use crate::vec2::Vec2;

/// Shape tags (`AABB.shape`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum ShapeKind {
    Circle = 0,
    Box = 1,
    RoundedRect = 2,
    Rope = 3,
}

/// A circle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Circle {
    pub center: Vec2,
    pub radius: Fix,
    pub layer: i32,
}

/// An oriented box; `right`/`up` are the half-extent vectors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BoxShape {
    pub center: Vec2,
    pub right: Vec2,
    pub up: Vec2,
    pub ie: Vec2,
    pub layer: i32,
}

/// A rounded rectangle: inner box grown by `radius`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RoundedRect {
    pub b: BoxShape,
    pub radius: Fix,
    pub layer: i32,
}

/// Contact manifold; the normal points from A to B.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Manifold {
    pub penetration: Fix,
    pub normal: Vec2,
    pub contact: Vec2,
}

/// Broad-phase box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Aabb {
    pub min: Vec2,
    pub max: Vec2,
    pub id: i32,
    pub layer: i32,
    pub shape: u8,
}

impl BoxShape {
    #[inline]
    pub fn tr(&self) -> Vec2 {
        self.center + self.right + self.up
    }
    #[inline]
    pub fn br(&self) -> Vec2 {
        self.center + self.right - self.up
    }
    #[inline]
    pub fn tl(&self) -> Vec2 {
        self.center - self.right + self.up
    }
    #[inline]
    pub fn bl(&self) -> Vec2 {
        self.center - self.right - self.up
    }

    /// `[tr, br, tl, bl]`.
    #[inline]
    pub fn corners(&self) -> [Vec2; 4] {
        [self.tr(), self.br(), self.tl(), self.bl()]
    }

    /// `SetRotation(a)`: half extents recomputed from the inverse extents.
    pub fn set_rotation(&mut self, a: Fix) {
        let u = Vec2::from_angle(a);
        self.right = (Vec2::RIGHT / self.ie.x).complex_mul(u);
        self.up = (Vec2::UP / self.ie.y).complex_mul(u);
    }

    /// `UpdateExtents(e)`.
    pub fn update_extents(&mut self, e: Vec2) {
        self.right = self.right * self.ie.x;
        self.up = self.up * self.ie.y;
        self.ie = Vec2::new(Fix::ONE / e.x, Fix::ONE / e.y);
        self.right = self.right * e.x;
        self.up = self.up * e.y;
    }

    /// AABB (§6.1).
    pub fn aabb(&self, id: i32) -> Aabb {
        let h = Vec2::new(
            self.right.x.abs() + self.up.x.abs(),
            self.right.y.abs() + self.up.y.abs(),
        );
        Aabb {
            min: self.center - h,
            max: self.center + h,
            id,
            layer: self.layer,
            shape: ShapeKind::Box as u8,
        }
    }

    /// As a rounded rect of radius 0.
    #[inline]
    pub fn as_rr(&self) -> RoundedRect {
        RoundedRect {
            b: *self,
            radius: Fix::ZERO,
            layer: self.layer,
        }
    }

    /// Point containment (§9.4).
    pub fn contains(&self, p: Vec2) -> bool {
        let d = self.center - p;
        let a = d.dot(self.right * self.ie.x) * self.ie.x;
        let b = d.dot(self.up * self.ie.y) * self.ie.y;
        a.abs() <= Fix::ONE && b.abs() <= Fix::ONE
    }

    /// `NormalAtPoint(pos, box)`.
    pub fn normal_at(&self, pos: Vec2) -> Vec2 {
        let d = self.center - pos;
        let a = (-d.dot(self.right * self.ie.x)) * self.ie.x;
        let b = (-d.dot(self.up * self.ie.y)) * self.ie.y;
        let k = Fix(4_294_967); // 0.001f
        let aa = a.abs().max(k);
        let bb = b.abs().max(k);
        if b / aa >= Fix::ONE {
            self.up * self.ie.y
        } else if a / bb >= Fix::ONE {
            self.right * self.ie.x
        } else if a / bb <= -Fix::ONE {
            -(self.right * self.ie.x)
        } else if b / aa <= -Fix::ONE {
            -(self.up * self.ie.y)
        } else {
            Vec2::UP
        }
    }
}

impl Circle {
    pub fn aabb(&self, id: i32) -> Aabb {
        let r = Vec2::new(self.radius, self.radius);
        Aabb {
            min: self.center - r,
            max: self.center + r,
            id,
            layer: self.layer,
            shape: ShapeKind::Circle as u8,
        }
    }

    pub fn contains(&self, p: Vec2) -> bool {
        (self.center - p).sqr_magnitude() < self.radius * self.radius
    }

    pub fn normal_at(&self, pos: Vec2) -> Vec2 {
        (pos - self.center).normalized_safe()
    }
}

impl RoundedRect {
    pub fn aabb(&self, id: i32) -> Aabb {
        let b = &self.b;
        let h = Vec2::new(
            b.right.x.abs() + b.up.x.abs() + self.radius,
            b.right.y.abs() + b.up.y.abs() + self.radius,
        );
        Aabb {
            min: b.center - h,
            max: b.center + h,
            id,
            layer: self.layer,
            shape: ShapeKind::RoundedRect as u8,
        }
    }

    /// Point containment (§9.4).
    pub fn contains(&self, p: Vec2) -> bool {
        let b = &self.b;
        let d = b.center - p;
        let a = (d.dot(b.right * b.ie.x) * b.ie.x).abs();
        let bb = (d.dot(b.up * b.ie.y) * b.ie.y).abs();
        let lx = Fix::ONE + self.radius * b.ie.x;
        let ly = Fix::ONE + self.radius * b.ie.y;
        if a > lx || bb > ly {
            return false;
        }
        if (a <= Fix::ONE && bb <= ly) || (a <= lx && bb <= Fix::ONE) {
            return true;
        }
        let rr = self.radius * self.radius;
        b.corners().iter().any(|&c| (c - p).sqr_magnitude() < rr)
    }

    /// `GetClosestPoint(p)` (on the rounded outline).
    pub fn closest_point(&self, p: Vec2) -> Vec2 {
        let bx = &self.b;
        let c = bx.center;
        let r = self.radius;
        let ux = bx.right * bx.ie.x;
        let uy = bx.up * bx.ie.y;
        let a = (p - c).dot(ux);
        let b = (p - c).dot(uy);
        let aa = a.sign2_fix() * a * a;
        let bb = b.sign2_fix() * b * b;
        let ex2 = bx.right.dot(bx.right);
        let ey2 = bx.up.dot(bx.up);
        let corner = |k: Vec2| k + (p - k).normalized() * r;
        if aa <= -ex2 {
            if bb >= ey2 {
                return corner(c - bx.right + bx.up);
            }
            if bb <= -ey2 {
                return corner(c - bx.right - bx.up);
            }
            c - bx.right - ux * r + uy * b
        } else if aa >= ex2 {
            if bb >= ey2 {
                return corner(c + bx.right + bx.up);
            }
            if bb <= -ey2 {
                return corner(c + bx.right - bx.up);
            }
            c + bx.right + ux * r + uy * b
        } else if b >= Fix::ZERO {
            c + bx.up + uy * r + ux * a
        } else {
            c - bx.up - uy * r + ux * a
        }
    }

    /// `NormalAtPoint(pos, rr)` (side normals are half-extent vectors).
    pub fn normal_at(&self, pos: Vec2) -> Vec2 {
        let bx = &self.b;
        let d = bx.center - pos;
        let a = (-d.dot(bx.right * bx.ie.x)) * bx.ie.x;
        let b = (-d.dot(bx.up * bx.ie.y)) * bx.ie.y;
        if b.abs() <= Fix::ONE {
            return bx.right * a.sign2_fix();
        }
        if a.abs() <= Fix::ONE {
            return bx.up * b.sign2_fix();
        }
        let corner = if a >= Fix::ONE && b >= Fix::ONE {
            Some(bx.tr())
        } else if a >= Fix::ONE && b <= -Fix::ONE {
            Some(bx.br())
        } else if a <= -Fix::ONE && b >= Fix::ONE {
            Some(bx.tl())
        } else if a <= -Fix::ONE && b <= -Fix::ONE {
            Some(bx.bl())
        } else {
            None
        };
        match corner {
            Some(k) => (pos - k).normalized_safe(),
            None => Vec2::UP,
        }
    }
}

// ---------------------------------------------------------------------------
// Narrow phase

/// §7.1 circle–circle.
pub fn circle_circle(a: &Circle, b: &Circle) -> Option<Manifold> {
    let r = a.radius + b.radius;
    let d = b.center - a.center;
    let q = d.sqr_magnitude();
    if q > r * r {
        return None;
    }
    let s = q.sqrt();
    Some(if s.0 != 0 {
        let n = d / s;
        Manifold {
            penetration: a.radius + b.radius - s,
            normal: n,
            contact: a.center + n * a.radius,
        }
    } else {
        Manifold {
            penetration: a.radius + b.radius,
            normal: Vec2::RIGHT,
            contact: a.center,
        }
    })
}

/// Boolean circle–circle (strict).
pub fn circle_circle_bool(a: &Circle, b: &Circle) -> bool {
    let r = a.radius + b.radius;
    (b.center - a.center).sqr_magnitude() < r * r
}

/// `ClosestPointOnLine`.
fn closest_point_on_line(a: Vec2, b: Vec2, x: Vec2, inv_len: Fix) -> Vec2 {
    let v = b - a;
    let da = a.dot(v);
    let db = b.dot(v);
    let dx = x.dot(v);
    if dx > db {
        return b;
    }
    if dx < da {
        return a;
    }
    let v = v * inv_len;
    a + v * (dx - da) * inv_len
}

struct BoxClosest {
    best: Fix,
    vec: Vec2,
    inside: bool,
    qs: [Fix; 4],
}

/// §7.2 for a box and circle centre `p`.
fn box_closest(bx: &BoxShape, p: Vec2) -> BoxClosest {
    let tr = bx.tr();
    let br = bx.br();
    let tl = bx.tl();
    let half = Fix::HALF;
    let p_r = closest_point_on_line(br, tr, p, half * bx.ie.y);
    let p_t = closest_point_on_line(tl, tr, p, half * bx.ie.x);
    let p_l = p_r - bx.right - bx.right;
    let p_b = p_t - bx.up - bx.up;
    let ds = [p_r - p, p_l - p, p_t - p, p_b - p];
    let qs = [
        ds[0].sqr_magnitude(),
        ds[1].sqr_magnitude(),
        ds[2].sqr_magnitude(),
        ds[3].sqr_magnitude(),
    ];
    let mut best = qs[0];
    let mut side = 0;
    for (i, &q) in qs.iter().enumerate().skip(1) {
        if q < best {
            best = q;
            side = i;
        }
    }
    let inside = p_r != tr && p_r != br && p_t != tl && p_t != tr;
    BoxClosest {
        best,
        vec: ds[side],
        inside,
        qs,
    }
}

/// §7.3 circle–rounded-rect.
pub fn circle_rr(c: &Circle, rr: &RoundedRect) -> Option<Manifold> {
    let r = c.radius + rr.radius;
    let p = c.center;
    let k = box_closest(&rr.b, p);
    let (pen, normal) = if k.inside {
        if k.best.0 == 0 {
            (r, -(rr.b.center - p).normalized())
        } else {
            let s = k.best.sqrt();
            (s + r, (-k.vec) / s)
        }
    } else {
        if k.best > r * r {
            return None;
        }
        if k.best.0 == 0 {
            (r, (rr.b.center - p).normalized())
        } else {
            let s = k.best.sqrt();
            (r - s, k.vec / s)
        }
    };
    Some(Manifold {
        penetration: pen,
        normal,
        contact: p + normal * (r - rr.radius),
    })
}

/// §7.4 circle–box.
pub fn circle_box(c: &Circle, bx: &BoxShape) -> Option<Manifold> {
    let p = c.center;
    let r = c.radius;
    let k = box_closest(bx, p);
    // Face normal of the winning side, replaced in R, L, T, B order.
    let mut n = bx.right * bx.ie.x;
    let mut best = k.qs[0];
    if k.qs[1] < best {
        best = k.qs[1];
        n = -n;
    }
    if k.qs[2] < best {
        best = k.qs[2];
        n = bx.up * bx.ie.y;
    }
    if k.qs[3] < best {
        n = (-bx.up) * bx.ie.y;
    }
    let pen = if k.inside {
        k.best.sqrt() + r
    } else {
        if k.best > r * r {
            return None;
        }
        r - k.best.sqrt()
    };
    let normal = -n;
    Some(Manifold {
        penetration: pen,
        normal,
        contact: p + normal * r,
    })
}

/// Boolean circle–box.
pub fn circle_box_bool(c: &Circle, bx: &BoxShape) -> bool {
    let k = box_closest(bx, c.center);
    let m = k.qs[0].min(k.qs[1]).min(k.qs[2]).min(k.qs[3]);
    m < c.radius * c.radius || k.inside
}

/// Boolean circle–rounded-rect.
pub fn circle_rr_bool(c: &Circle, rr: &RoundedRect) -> bool {
    circle_box_bool(
        &Circle {
            center: c.center,
            radius: c.radius + rr.radius,
            layer: c.layer,
        },
        &rr.b,
    )
}

#[inline]
fn min_max(axis: Vec2, pts: &[Vec2]) -> (Fix, Fix) {
    let mut lo = axis.dot(pts[0]);
    let mut hi = lo;
    for p in &pts[1..] {
        let d = axis.dot(*p);
        if d < lo {
            lo = d;
        }
        if d > hi {
            hi = d;
        }
    }
    (lo, hi)
}

struct Sat {
    min_pen: Fix,
    min_dir: Vec2,
}

impl Sat {
    /// `SATRR`; false when separated.
    fn rr(&mut self, m1: (Fix, Fix), m2: (Fix, Fix), axis: Vec2) -> bool {
        let o1 = m1.1 - m2.0;
        let o2 = m2.1 - m1.0;
        if o1 < Fix::ZERO || o2 < Fix::ZERO {
            return false;
        }
        if o1 < self.min_pen && o1 < o2 {
            self.min_pen = o1;
            self.min_dir = axis;
        }
        if o2 < self.min_pen {
            self.min_pen = o2;
            self.min_dir = -axis;
        }
        true
    }

    /// `SATobb`; false when separated.
    fn obb(&mut self, m1: (Fix, Fix), m2: (Fix, Fix), axis: Vec2, inv_mag: Fix) -> bool {
        let o1 = m1.1 - m2.0;
        let o2 = m2.1 - m1.0;
        if o1 < Fix::ZERO || o2 < Fix::ZERO {
            return false;
        }
        let o1 = o1 * inv_mag;
        let o2 = o2 * inv_mag;
        if o1 < self.min_pen {
            self.min_pen = o1;
            self.min_dir = axis * inv_mag;
        }
        if o2 < self.min_pen {
            self.min_pen = o2;
            self.min_dir = (-axis) * inv_mag;
        }
        true
    }
}

#[inline]
fn widen(m: (Fix, Fix), r: Fix) -> (Fix, Fix) {
    (m.0 - r, m.1 + r)
}

/// `CollisionFace`: `((A, B), normal)`.
fn collision_face(c: &[Vec2; 4], p: Vec2, ie: Vec2) -> ((Vec2, Vec2), Vec2) {
    let [tr, br, tl, bl] = *c;
    let h = ie * Fix(2_147_483_648);
    let x = p.dot(tr);
    let y = p.dot(br);
    let z = p.dot(tl);
    let w = p.dot(bl);
    let pick = |na: Vec2, fa: (Vec2, Vec2), nb: Vec2, fb: (Vec2, Vec2)| {
        if na.dot(p).abs() > nb.dot(p).abs() {
            (fa, na)
        } else {
            (fb, nb)
        }
    };
    if x >= y && x >= z && x >= w {
        let na = (br - tr).mul_by_i() * h.y;
        let nb = (tr - tl).mul_by_i() * h.x;
        pick(na, (tr, br), nb, (tl, tr))
    } else if y >= x && y >= z && y >= w {
        let na = (br - tr).mul_by_i() * h.y;
        let nb = (bl - br).mul_by_i() * h.x;
        pick(na, (tr, br), nb, (br, bl))
    } else if z >= x && z >= y && z >= w {
        let na = (tl - bl).mul_by_i() * h.y;
        let nb = (tr - tl).mul_by_i() * h.x;
        pick(na, (bl, tl), nb, (tl, tr))
    } else {
        let na = (tl - bl).mul_by_i() * h.y;
        let nb = (bl - br).mul_by_i() * h.x;
        pick(na, (bl, tl), nb, (br, bl))
    }
}

/// `ClipIncidentFace`.
fn clip_incident(rf: (Vec2, Vec2), inc: (Vec2, Vec2), ref_n: Vec2, pen_n: Vec2) -> Vec2 {
    let t = ref_n.mul_by_neg_i();
    let (mut ia, mut ib) = inc;
    let sa = t.dot(ia);
    let sb = t.dot(ib);
    let lo = t.dot(rf.0);
    let hi = t.dot(rf.1);
    if sa < lo {
        ia += (lo - sa) * t;
    } else if sa > hi {
        ia -= (sa - hi) * t;
    }
    if sb < lo {
        ib += (lo - sb) * t;
    } else if sb > hi {
        ib -= (sb - hi) * t;
    }
    if pen_n.dot(ia) > pen_n.dot(ib) {
        ia
    } else {
        ib
    }
}

/// §7.5 rounded-rect–rounded-rect (also box–rounded-rect with radius 0).
pub fn rr_rr(s1: &RoundedRect, s2: &RoundedRect) -> Option<Manifold> {
    let b1 = &s1.b;
    let b2 = &s2.b;
    let c1 = b1.corners();
    let c2 = b2.corners();
    let (r1, r2) = (s1.radius, s2.radius);
    let ax1 = b1.right * b1.ie.x;
    let mut sat = Sat {
        min_pen: Fix::MAX,
        min_dir: ax1,
    };
    if !sat.rr(
        widen(min_max(ax1, &[c1[0], c1[2]]), r1),
        widen(min_max(ax1, &c2), r2),
        ax1,
    ) {
        return None;
    }
    let ax2 = b1.up * b1.ie.y;
    if !sat.rr(
        widen(min_max(ax2, &[c1[0], c1[1]]), r1),
        widen(min_max(ax2, &c2), r2),
        ax2,
    ) {
        return None;
    }
    let ax3 = b2.right * b2.ie.x;
    if !sat.rr(
        widen(min_max(ax3, &c1), r1),
        widen(min_max(ax3, &[c2[0], c2[2]]), r2),
        ax3,
    ) {
        return None;
    }
    let ax4 = b2.up * b2.ie.y;
    if !sat.rr(
        widen(min_max(ax4, &c1), r1),
        widen(min_max(ax4, &[c2[0], c2[1]]), r2),
        ax4,
    ) {
        return None;
    }
    let ((mut a1, mut bb1), n1) = collision_face(&c1, sat.min_dir, b1.ie);
    let ((mut a2, mut bb2), n2) = collision_face(&c2, -sat.min_dir, b2.ie);
    let mut best_q = Fix::MAX;
    let mut d = Vec2::ZERO;
    for cand in [a1 - a2, a1 - bb2, bb1 - bb2, bb1 - a2] {
        let q = cand.sqr_magnitude();
        if q < best_q {
            best_q = q;
            d = cand;
        }
    }
    let ax5 = d / best_q.sqrt();
    if !sat.rr(
        widen(min_max(ax5, &c1), r1),
        widen(min_max(ax5, &c2), r2),
        ax5,
    ) {
        return None;
    }
    let md = sat.min_dir;
    a1 += md * r1;
    bb1 += md * r1;
    a2 -= md * r2;
    bb2 -= md * r2;
    let contact = if n1.dot(md).abs() > n2.dot(md).abs() {
        clip_incident((a1, bb1), (a2, bb2), n1, -md)
    } else {
        clip_incident((a2, bb2), (a1, bb1), n2, md)
    };
    Some(Manifold {
        penetration: sat.min_pen,
        normal: md,
        contact,
    })
}

/// Boolean rounded-rect–rounded-rect (the same five SAT steps).
pub fn rr_rr_bool(s1: &RoundedRect, s2: &RoundedRect) -> bool {
    rr_rr_sat_only(s1, s2)
}

fn rr_rr_sat_only(s1: &RoundedRect, s2: &RoundedRect) -> bool {
    let b1 = &s1.b;
    let b2 = &s2.b;
    let c1 = b1.corners();
    let c2 = b2.corners();
    let (r1, r2) = (s1.radius, s2.radius);
    let ax1 = b1.right * b1.ie.x;
    let mut sat = Sat {
        min_pen: Fix::MAX,
        min_dir: ax1,
    };
    let ax2 = b1.up * b1.ie.y;
    let ax3 = b2.right * b2.ie.x;
    let ax4 = b2.up * b2.ie.y;
    if !sat.rr(
        widen(min_max(ax1, &[c1[0], c1[2]]), r1),
        widen(min_max(ax1, &c2), r2),
        ax1,
    ) || !sat.rr(
        widen(min_max(ax2, &[c1[0], c1[1]]), r1),
        widen(min_max(ax2, &c2), r2),
        ax2,
    ) || !sat.rr(
        widen(min_max(ax3, &c1), r1),
        widen(min_max(ax3, &[c2[0], c2[2]]), r2),
        ax3,
    ) || !sat.rr(
        widen(min_max(ax4, &c1), r1),
        widen(min_max(ax4, &[c2[0], c2[1]]), r2),
        ax4,
    ) {
        return false;
    }
    let ((a1, bb1), _) = collision_face(&c1, sat.min_dir, b1.ie);
    let ((a2, bb2), _) = collision_face(&c2, -sat.min_dir, b2.ie);
    let mut best_q = Fix::MAX;
    let mut d = Vec2::ZERO;
    for cand in [a1 - a2, a1 - bb2, bb1 - bb2, bb1 - a2] {
        let q = cand.sqr_magnitude();
        if q < best_q {
            best_q = q;
            d = cand;
        }
    }
    let ax5 = d / best_q.sqrt();
    sat.rr(
        widen(min_max(ax5, &c1), r1),
        widen(min_max(ax5, &c2), r2),
        ax5,
    )
}

/// Box–rounded-rect: the box as a rounded rect of radius 0.
pub fn box_rr(b: &BoxShape, rr: &RoundedRect) -> Option<Manifold> {
    rr_rr(&b.as_rr(), rr)
}

pub fn box_rr_bool(b: &BoxShape, rr: &RoundedRect) -> bool {
    rr_rr_bool(&b.as_rr(), rr)
}

/// §7.6 box–box.
pub fn box_box(b1: &BoxShape, b2: &BoxShape) -> Option<Manifold> {
    let c1 = b1.corners();
    let c2 = b2.corners();
    let mut sat = Sat {
        min_pen: Fix::MAX,
        min_dir: b1.right,
    };
    if !sat.obb(
        min_max(b1.right, &[c1[0], c1[2]]),
        min_max(b1.right, &c2),
        b1.right,
        b1.ie.x,
    ) || !sat.obb(
        min_max(b1.up, &[c1[0], c1[1]]),
        min_max(b1.up, &c2),
        b1.up,
        b1.ie.y,
    ) || !sat.obb(
        min_max(b2.right, &c1),
        min_max(b2.right, &[c2[0], c2[2]]),
        b2.right,
        b2.ie.x,
    ) || !sat.obb(
        min_max(b2.up, &c1),
        min_max(b2.up, &[c2[0], c2[1]]),
        b2.up,
        b2.ie.y,
    ) {
        return None;
    }
    let md = sat.min_dir;
    let ((a1, bb1), n1) = collision_face(&c1, md, b1.ie);
    let ((a2, bb2), n2) = collision_face(&c2, -md, b2.ie);
    let contact = if n1.dot(md).abs() > n2.dot(md).abs() {
        clip_incident((a1, bb1), (a2, bb2), n1, -md)
    } else {
        clip_incident((a2, bb2), (a1, bb1), n2, md)
    };
    Some(Manifold {
        penetration: sat.min_pen.abs(),
        normal: md,
        contact,
    })
}

/// Boolean box–box (unnormalised axes).
pub fn box_box_bool(b1: &BoxShape, b2: &BoxShape) -> bool {
    let c1 = b1.corners();
    let c2 = b2.corners();
    let sep = |m1: (Fix, Fix), m2: (Fix, Fix)| m2.0 > m1.1 || m1.0 > m2.1;
    !(sep(min_max(b1.right, &[c1[0], c1[2]]), min_max(b1.right, &c2))
        || sep(min_max(b1.up, &[c1[0], c1[1]]), min_max(b1.up, &c2))
        || sep(min_max(b2.right, &c1), min_max(b2.right, &[c2[0], c2[2]]))
        || sep(min_max(b2.up, &c1), min_max(b2.up, &[c2[0], c2[1]])))
}

/// §8.3 positional correction: `(a, b)` to add to A's and B's centres.
pub fn positional_correction(m: &Manifold, im_a: Fix, im_b: Fix, factor: Fix) -> (Vec2, Vec2) {
    let p = (m.penetration - SLOP).max(Fix::ZERO);
    let c = (p / (im_a + im_b)) * factor * m.normal;
    ((-im_a) * c, im_b * c)
}

/// The standard correction strength (0.3f).
pub fn positional_correction_std(m: &Manifold, im_a: Fix, im_b: Fix) -> (Vec2, Vec2) {
    positional_correction(m, im_a, im_b, POS_CORRECTION_STR)
}

// ---------------------------------------------------------------------------
// Ray casts (§9.2); the shape is translated into the ray-origin frame.

/// Ray vs circle; `(short, long)` on a hit.
pub fn ray_circle(origin: Vec2, dir: Vec2, c: &Circle) -> Option<(Fix, Fix)> {
    ray_circle_at(dir, c.center - origin, c.radius)
}

fn ray_circle_at(dir: Vec2, v: Vec2, r: Fix) -> Option<(Fix, Fix)> {
    let f = v.dot(dir);
    let rr = r * r;
    let h = rr - (dir * f - v).sqr_magnitude();
    if h < Fix::ZERO {
        return None;
    }
    let h = h.sqrt();
    if rr > v.sqr_magnitude() {
        return Some((Fix::ZERO, f + h));
    }
    if f < Fix::ZERO {
        return None;
    }
    Some((f - h, f + h))
}

/// Ray vs rounded rect; `(near, far)` on a hit.
pub fn ray_rr(origin: Vec2, dir: Vec2, rr: &RoundedRect) -> Option<(Fix, Fix)> {
    let b = &rr.b;
    let r = rr.radius;
    let x = b.right * b.ie.x;
    let y = x.mul_by_neg_i();
    let c = b.center - origin;
    let cx = x.cross(dir);
    let cy = y.cross(dir);
    let mut near = Fix::MAX;
    let mut far = Fix(1);
    let mut hits = 0;
    let mut face = |t: Fix, s: Fix, lo: Fix, hi: Fix, near: &mut Fix, far: &mut Fix| -> bool {
        if t > Fix::ZERO && lo < s && s < hi {
            if hits == 0 {
                *near = t;
                *far = t;
                hits = 1;
            } else {
                *far = t.max(*far);
                *near = t.min(*near);
                return true;
            }
        }
        false
    };
    let (right, up) = (b.right, b.up);
    if cx.0 != 0 {
        let v = c - right + up - y * r;
        let w = c + right + up - y * r;
        let t = (-w.dot(y)) / cx;
        if face(t, (dir * t).dot(x), v.dot(x), w.dot(x), &mut near, &mut far) {
            return Some((near, far));
        }
    }
    if cy.0 != 0 {
        let v = c + right + up + x * r;
        let w = c + right - up + x * r;
        let t = v.dot(x) / cy;
        if face(t, (dir * t).dot(y), v.dot(y), w.dot(y), &mut near, &mut far) {
            return Some((near, far));
        }
    }
    if cx.0 != 0 {
        let v = c + right - up + y * r;
        let w = c - right - up + y * r;
        let t = (-v.dot(y)) / cx;
        let nx = -x;
        if face(
            t,
            (dir * t).dot(nx),
            v.dot(nx),
            w.dot(nx),
            &mut near,
            &mut far,
        ) {
            return Some((near, far));
        }
    }
    if cy.0 != 0 {
        let v = c - right - up - x * r;
        let w = c - right + up - x * r;
        let t = w.dot(x) / cy;
        let ny = -y;
        if face(
            t,
            (dir * t).dot(ny),
            v.dot(ny),
            w.dot(ny),
            &mut near,
            &mut far,
        ) {
            return Some((near, far));
        }
    }
    let mut any = hits > 0;
    for k in [
        c - right + up,
        c + right + up,
        c - right - up,
        c + right - up,
    ] {
        if let Some((s, l)) = ray_circle_at(dir, k, r) {
            far = far.max(l);
            near = near.min(s);
            any = true;
        }
    }
    let local = RoundedRect {
        b: BoxShape { center: c, ..*b },
        radius: r,
        layer: rr.layer,
    };
    if local.contains(Vec2::ZERO) {
        return Some((Fix::ZERO, far));
    }
    if any { Some((near, far)) } else { None }
}

/// Ray vs box; `(near, far)` on a hit.
pub fn ray_box(origin: Vec2, dir: Vec2, b: &BoxShape) -> Option<(Fix, Fix)> {
    let x = b.right * b.ie.x;
    let y = x.mul_by_neg_i();
    let c = b.center - origin;
    let cx = x.cross(dir);
    let cy = y.cross(dir);
    let tl = c - b.right + b.up;
    let tr = c + b.right + b.up;
    let br = c + b.right - b.up;
    let bl = c - b.right - b.up;
    let mut near = Fix::MAX;
    let mut far = Fix(1);
    let mut hits = 0;
    let mut test = |t: Fix, s: Fix, lo: Fix, hi: Fix| -> bool {
        if t > Fix::ZERO && lo < s && s < hi {
            if hits == 0 {
                near = t;
                far = t;
                hits = 1;
            } else {
                far = t.max(far);
                near = t.min(near);
                hits = 2;
                return true;
            }
        }
        false
    };
    let nx = -x;
    let ny = -y;
    let done = (cx.0 != 0 && {
        let t = (-tr.dot(y)) / cx;
        test(t, (dir * t).dot(x), tl.dot(x), tr.dot(x))
    }) || (cy.0 != 0 && {
        let t = tr.dot(x) / cy;
        test(t, (dir * t).dot(y), tr.dot(y), br.dot(y))
    }) || (cx.0 != 0 && {
        let t = (-br.dot(y)) / cx;
        test(t, (dir * t).dot(nx), br.dot(nx), bl.dot(nx))
    }) || (cy.0 != 0 && {
        let t = tl.dot(x) / cy;
        test(t, (dir * t).dot(ny), bl.dot(ny), tl.dot(ny))
    });
    if done {
        return Some((near, far));
    }
    if hits == 1 {
        return Some((Fix::ZERO, far));
    }
    None
}

/// `LineToLineIntersection` (beams): `(tA, tB)` and whether it hits.
pub fn line_line(
    pa: Vec2,
    da: Vec2,
    len_a: Fix,
    pb: Vec2,
    db: Vec2,
    len_b: Fix,
) -> Option<(Fix, Fix)> {
    let pb = pb - pa;
    let nb = db.mul_by_i();
    let w = nb * pb.dot(nb);
    let k = w.dot(da);
    if k.0 == 0 {
        return None;
    }
    let ta = w.dot(w) / k;
    let tb = (da * ta - pb).dot(db);
    if Fix::ZERO < ta && ta < len_a && Fix::ZERO < tb && tb < len_b {
        Some((ta, tb))
    } else {
        None
    }
}

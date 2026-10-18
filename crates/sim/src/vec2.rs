//! `Vec2` on `Fix` and the small `Tools` helpers (fixmath.md §8).

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use crate::fix::Fix;

/// A 2D vector of [`Fix`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Vec2 {
    pub x: Fix,
    pub y: Fix,
}

impl std::fmt::Debug for Vec2 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({:?}, {:?})", self.x, self.y)
    }
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2::new(Fix::ZERO, Fix::ZERO);
    pub const UP: Vec2 = Vec2::new(Fix::ZERO, Fix::ONE);
    pub const DOWN: Vec2 = Vec2::new(Fix::ZERO, Fix(-(1 << 32)));
    pub const LEFT: Vec2 = Vec2::new(Fix(-(1 << 32)), Fix::ZERO);
    pub const RIGHT: Vec2 = Vec2::new(Fix::ONE, Fix::ZERO);

    #[inline]
    pub const fn new(x: Fix, y: Fix) -> Vec2 {
        Vec2 { x, y }
    }

    /// From raw bits.
    #[inline]
    pub const fn raw(x: i64, y: i64) -> Vec2 {
        Vec2::new(Fix(x), Fix(y))
    }

    /// `Vec2(angle) = (FastCos, FastSin)`.
    #[inline]
    pub fn from_angle(a: Fix) -> Vec2 {
        Vec2::new(a.fast_cos(), a.fast_sin())
    }

    #[inline]
    pub fn dot(self, o: Vec2) -> Fix {
        self.x * o.x + self.y * o.y
    }

    #[inline]
    pub fn cross(self, o: Vec2) -> Fix {
        self.x * o.y - self.y * o.x
    }

    /// `CrossProduct2(s, v) = ((−s) · v.y, s · v.x)`.
    #[inline]
    pub fn cross_sv(s: Fix, v: Vec2) -> Vec2 {
        Vec2::new((-s) * v.y, s * v.x)
    }

    /// `CrossProduct2(v, s) = (s · v.y, (−s) · v.x)`.
    #[inline]
    pub fn cross_vs(v: Vec2, s: Fix) -> Vec2 {
        Vec2::new(s * v.y, (-s) * v.x)
    }

    #[inline]
    pub fn sqr_magnitude(self) -> Fix {
        self.x * self.x + self.y * self.y
    }

    #[inline]
    pub fn magnitude(self) -> Fix {
        self.sqr_magnitude().sqrt()
    }

    /// `v / |v|` (faults for the zero vector).
    #[inline]
    pub fn normalized(self) -> Vec2 {
        self / self.magnitude()
    }

    /// `v` if its magnitude is 0, else `v / |v|`.
    #[inline]
    pub fn normalized_safe(self) -> Vec2 {
        let m = self.magnitude();
        if m.0 == 0 { self } else { self / m }
    }

    #[inline]
    pub fn distance(self, o: Vec2) -> Fix {
        (self - o).magnitude()
    }

    /// Rotate +90°: `(−y, x)`.
    #[inline]
    pub fn mul_by_i(self) -> Vec2 {
        Vec2::new(-self.y, self.x)
    }

    /// Rotate −90°: `(y, −x)`.
    #[inline]
    pub fn mul_by_neg_i(self) -> Vec2 {
        Vec2::new(self.y, -self.x)
    }

    #[inline]
    pub fn complex_mul(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x * o.x - self.y * o.y, self.y * o.x + self.x * o.y)
    }

    #[inline]
    pub fn lerp(self, o: Vec2, t: Fix) -> Vec2 {
        Vec2::new(self.x.lerp(o.x, t), self.y.lerp(o.y, t))
    }

    /// Componentwise multiplication `(a.x · b.x, a.y · b.y)`.
    #[inline]
    pub fn scale(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x * o.x, self.y * o.y)
    }

    /// Angle of a unit vector in `(0, 2π]`.
    pub fn normalized_vector_angle(self) -> Fix {
        let x = self.x.slow_mod(Fix::ONE + Fix(1));
        let a = Vec2::new(x, self.y).dot(Vec2::RIGHT).acos();
        if self.y > Fix::ZERO {
            a
        } else {
            Fix::PI_TIMES_2 - a
        }
    }

    /// `NormalizedSafe(Lerp(v, u, t))`.
    pub fn nlerp(self, o: Vec2, t: Fix) -> Vec2 {
        self.lerp(o, t).normalized_safe()
    }

    /// Spherical interpolation.
    pub fn slerp(self, o: Vec2, s: Fix) -> Vec2 {
        let theta = self
            .dot(o)
            .clamp(-Fix::ONE, Fix::ONE)
            .min(Fix(4_294_537_728))
            .acos();
        let mut st = theta.sin();
        if st.0 == 0 {
            st = Fix(42_949);
        }
        let k = Fix::ONE / st;
        self * ((Fix::ONE - s) * theta).sin() * k + o * (s * theta).sin() * k
    }

    /// `ApproachNormalizedVector(v, target, inc)`.
    pub fn approach_normalized(self, target: Vec2, inc: Fix) -> Vec2 {
        let c = self.cross(target);
        let w = Vec2::from_angle(if c < Fix::ZERO { -inc } else { inc });
        let v2 = self.complex_mul(w);
        if c * v2.cross(target) <= Fix::ZERO {
            target
        } else {
            v2
        }
    }

    /// `Tools.Reflect(v, n)`.
    pub fn reflect(self, n: Vec2) -> Vec2 {
        let m = n.mul_by_neg_i();
        n * (-self).dot(n) + m * m.dot(self)
    }

    /// `Tools.Angle2(a, b)` (inherits Atan2's inaccuracy).
    pub fn angle2(a: Vec2, b: Vec2) -> Fix {
        let p = a * b.magnitude();
        let q = b * a.magnitude();
        Fix::TWO * Fix::atan2((p - q).magnitude(), (p + q).magnitude())
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    #[inline]
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for Vec2 {
    type Output = Vec2;
    #[inline]
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}
impl Neg for Vec2 {
    type Output = Vec2;
    #[inline]
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}
impl AddAssign for Vec2 {
    #[inline]
    fn add_assign(&mut self, o: Vec2) {
        *self = *self + o;
    }
}
impl SubAssign for Vec2 {
    #[inline]
    fn sub_assign(&mut self, o: Vec2) {
        *self = *self - o;
    }
}
/// `v · s` = `(v.x · s, v.y · s)`.
impl Mul<Fix> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn mul(self, s: Fix) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}
/// `s · v`.
impl Mul<Vec2> for Fix {
    type Output = Vec2;
    #[inline]
    fn mul(self, v: Vec2) -> Vec2 {
        Vec2::new(self * v.x, self * v.y)
    }
}
impl Div<Fix> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn div(self, s: Fix) -> Vec2 {
        Vec2::new(self.x / s, self.y / s)
    }
}

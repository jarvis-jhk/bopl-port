//! `Fix`: the game's Q32.32 fixed-point number, bit-exact (fixmath.md).
//!
//! Every operation follows the spec literally, including wrapping overflow,
//! floor (not round) multiplication, the rounding division and the
//! approximate trigonometry. Operations that throw in the original
//! (division by zero, `Sqrt`/`Acos`/`Log2` domain errors, `MinValue % −1`)
//! return 0 here and raise a **fault** ([`take_fault`]); the world turns a
//! fault into the original online host's behaviour (the round ends as a
//! draw) instead of panicking.

use std::cell::Cell;
use std::fmt;
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, Sub, SubAssign};

use crate::lut::{LUT_SIZE, luts};

/// What raised a fault (the exception the original would throw).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fault {
    /// `DivideByZeroException`.
    DivideByZero,
    /// `OverflowException` (`MinValue % −1`).
    Overflow,
    /// `ArgumentOutOfRangeException` (`Sqrt`, `Acos`, `Log2` domain).
    Argument,
    /// A table index out of range (cannot happen for reduced inputs).
    Index,
}

thread_local! {
    static FAULT: Cell<Option<Fault>> = const { Cell::new(None) };
}

/// Records a fault (the first one sticks until [`take_fault`]).
#[cold]
pub fn raise(f: Fault) {
    FAULT.with(|c| {
        if c.get().is_none() {
            c.set(Some(f));
        }
    });
}

/// Returns and clears the pending fault of this thread.
pub fn take_fault() -> Option<Fault> {
    FAULT.with(|c| c.take())
}

/// A Q32.32 fixed-point number (`value = raw / 2^32`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct Fix(pub i64);

const ONE_RAW: i64 = 1 << 32;
const FRAC_MASK: i64 = 0xFFFF_FFFF;

impl Fix {
    pub const ONE: Fix = Fix(ONE_RAW);
    pub const ZERO: Fix = Fix(0);
    pub const MAX: Fix = Fix(i64::MAX);
    pub const MIN: Fix = Fix(i64::MIN);
    /// Smallest positive value (the game's `minValue`).
    pub const EPSILON: Fix = Fix(1);
    pub const PI: Fix = Fix(13_493_037_704);
    pub const PI_OVER_2: Fix = Fix(6_746_518_852);
    pub const PI_TIMES_2: Fix = Fix(26_986_075_409);
    pub const PI_INV: Fix = Fix(1_367_130_551);
    pub const PI_OVER_2_INV: Fix = Fix(2_734_261_102);
    pub const LN2: Fix = Fix(2_977_044_471);
    pub const LOG2_MAX: Fix = Fix(133_143_986_176);
    pub const LOG2_MIN: Fix = Fix(-137_438_953_472);
    pub const LUT_INTERVAL: Fix = Fix(562_946_081_331_096);
    /// 3π/2 as used by `Cos`.
    pub const THREE_HALF_PI: Fix = Fix(20_239_556_556);
    /// 0.28 (decimal) used by `Atan2`.
    pub const ATAN2_K: Fix = Fix(1_202_590_842);
    pub const TWO: Fix = Fix(2 * ONE_RAW);
    pub const THREE: Fix = Fix(3 * ONE_RAW);
    pub const HALF: Fix = Fix(ONE_RAW / 2);

    /// From the raw bits.
    #[inline]
    pub const fn from_raw(raw: i64) -> Fix {
        Fix(raw)
    }

    /// The raw bits.
    #[inline]
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// `(Fix)n` for an `int`.
    #[inline]
    pub const fn from_int(n: i32) -> Fix {
        Fix((n as i64) << 32)
    }

    /// `(Fix)n` for a `long` (wrapping).
    #[inline]
    pub const fn from_long(n: i64) -> Fix {
        Fix(n.wrapping_shl(32))
    }

    /// `(long)x` = floor.
    #[inline]
    pub const fn to_long(self) -> i64 {
        self.0 >> 32
    }

    /// `(int)x` (via `long`, low 32 bits).
    #[inline]
    pub const fn to_int(self) -> i32 {
        (self.0 >> 32) as i32
    }

    /// `(Fix)f` for a float literal: `trunc(f · 2^32)` (exact).
    /// For constants and tests; the simulation itself never converts floats.
    pub fn from_f32(f: f32) -> Fix {
        Fix::from_f64(f as f64)
    }

    /// `(Fix)d` for a double literal: `trunc(d · 2^32)` (exact).
    pub fn from_f64(d: f64) -> Fix {
        let v = d * 4_294_967_296.0;
        // .NET's double→long conversion of out-of-range values yields MinValue.
        if v.is_nan() || v >= 9_223_372_036_854_775_808.0 || v < -9_223_372_036_854_775_808.0 {
            return Fix(i64::MIN);
        }
        Fix(v as i64)
    }

    /// `(Fix)m` for a decimal literal given as a string (exact truncation).
    pub fn from_decimal_str(s: &str) -> Option<Fix> {
        let (neg, s) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let (int, frac) = s.split_once('.').unwrap_or((s, ""));
        let i: i128 = if int.is_empty() { 0 } else { int.parse().ok()? };
        let mut f: i128 = 0;
        let mut den: i128 = 1;
        for c in frac.chars().take(30) {
            f = f * 10 + c.to_digit(10)? as i128;
            den *= 10;
        }
        let mag = (i << 32) + (f << 32) / den;
        let v = if neg { -mag } else { mag };
        Some(Fix(v.clamp(i64::MIN as i128, i64::MAX as i128) as i64))
    }

    /// `(float)x`: presentation only.
    #[inline]
    pub fn to_f32(self) -> f32 {
        (self.0 as f32) / 4_294_967_296.0
    }

    /// `(double)x`: presentation only.
    #[inline]
    pub fn to_f64(self) -> f64 {
        (self.0 as f64) / 4_294_967_296.0
    }

    // -- arithmetic ---------------------------------------------------------

    /// `SlowAdd`: saturating addition.
    #[inline]
    pub const fn slow_add(self, o: Fix) -> Fix {
        Fix(self.0.saturating_add(o.0))
    }

    /// `SlowSub`: saturating subtraction.
    #[inline]
    pub const fn slow_sub(self, o: Fix) -> Fix {
        Fix(self.0.saturating_sub(o.0))
    }

    /// `SlowMul`: saturating multiplication.
    #[inline]
    pub const fn slow_mul(self, o: Fix) -> Fix {
        let p = ((self.0 as i128) * (o.0 as i128)) >> 32;
        Fix(if p > i64::MAX as i128 {
            i64::MAX
        } else if p < i64::MIN as i128 {
            i64::MIN
        } else {
            p as i64
        })
    }

    /// The ordinary (wrapping, flooring) multiplication.
    #[inline]
    pub const fn mul(self, o: Fix) -> Fix {
        Fix((((self.0 as i128) * (o.0 as i128)) >> 32) as i64)
    }

    /// The ordinary division (§4.1); division by zero faults and returns 0.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn div(self, o: Fix) -> Fix {
        let a = self.0;
        let b = o.0;
        if b == 0 {
            raise(Fault::DivideByZero);
            return Fix(0);
        }
        let neg = (a ^ b) < 0;
        let ua = a.unsigned_abs() as u128;
        let ub = b.unsigned_abs() as u128;
        let t = (ua << 33) / ub;
        if t >> 64 != 0 {
            return if neg { Fix::MIN } else { Fix::MAX };
        }
        let r = ((t as u64).wrapping_add(1) >> 1) as i64;
        Fix(if neg { r.wrapping_neg() } else { r })
    }

    /// `%`: truncated remainder; zero divisor or `MinValue % −1` fault.
    #[allow(clippy::should_implement_trait)]
    #[inline]
    pub fn rem(self, o: Fix) -> Fix {
        if o.0 == 0 {
            raise(Fault::DivideByZero);
            return Fix(0);
        }
        if self.0 == i64::MIN && o.0 == -1 {
            raise(Fault::Overflow);
            return Fix(0);
        }
        Fix(self.0 % o.0)
    }

    /// `SlowMod`: like `%` but `MinValue % −1 = 0`.
    #[inline]
    pub fn slow_mod(self, o: Fix) -> Fix {
        if o.0 == 0 {
            raise(Fault::DivideByZero);
            return Fix(0);
        }
        Fix(self.0.wrapping_rem(o.0))
    }

    /// `Abs` (wrapping: `Abs(MinValue) = MinValue`).
    #[inline]
    pub const fn abs(self) -> Fix {
        Fix(self.0.wrapping_abs())
    }

    /// `SlowAbs` (`MinValue → MaxValue`).
    #[inline]
    pub const fn slow_abs(self) -> Fix {
        Fix(self.0.saturating_abs())
    }

    /// `Sign` as an integer.
    #[inline]
    pub const fn sign(self) -> i32 {
        self.0.signum() as i32
    }

    /// `(Fix)Sign(x)`: ±One or 0.
    #[inline]
    pub const fn sign_fix(self) -> Fix {
        Fix((self.0.signum()) << 32)
    }

    /// `Sign2`: −1 if negative, else +1.
    #[inline]
    pub const fn sign2(self) -> i32 {
        if self.0 < 0 { -1 } else { 1 }
    }

    /// `(Fix)Sign2(x)`.
    #[inline]
    pub const fn sign2_fix(self) -> Fix {
        if self.0 < 0 {
            Fix(-ONE_RAW)
        } else {
            Fix(ONE_RAW)
        }
    }

    /// `Min(a, b)`: `a` if `a < b` else `b`.
    #[inline]
    pub const fn min(self, o: Fix) -> Fix {
        if self.0 < o.0 { self } else { o }
    }

    /// `Max(a, b)`: `a` if `a > b` else `b`.
    #[inline]
    pub const fn max(self, o: Fix) -> Fix {
        if self.0 > o.0 { self } else { o }
    }

    /// `Clamp(v, lo, hi) = Min(hi, Max(v, lo))`.
    #[inline]
    pub const fn clamp(self, lo: Fix, hi: Fix) -> Fix {
        hi.min(self.max(lo))
    }

    /// `Clamp01`.
    #[inline]
    pub const fn clamp01(self) -> Fix {
        Fix::ONE.min(self.max(Fix::ZERO))
    }

    /// `Lerp(a, b, t) = a · (One − Clamp01(t)) + b · t` (second term unclamped).
    #[inline]
    pub const fn lerp(self, b: Fix, t: Fix) -> Fix {
        Fix(self
            .mul(Fix(ONE_RAW.wrapping_sub(t.clamp01().0)))
            .0
            .wrapping_add(b.mul(t).0))
    }

    /// Integer comparison helper: `raw` against `n · 2^32`.
    #[inline]
    pub const fn cmp_int(self, n: i64) -> std::cmp::Ordering {
        let r = n.wrapping_mul(ONE_RAW);
        if self.0 < r {
            std::cmp::Ordering::Less
        } else if self.0 > r {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }

    /// `Floor`.
    #[inline]
    pub const fn floor(self) -> Fix {
        Fix(self.0 & !FRAC_MASK)
    }

    /// `Ceiling`.
    #[inline]
    pub const fn ceiling(self) -> Fix {
        if self.0 & FRAC_MASK == 0 {
            self
        } else {
            self.floor().slow_add(Fix::ONE)
        }
    }

    /// `Round` (half to even).
    #[inline]
    pub const fn round(self) -> Fix {
        let f = self.0 & FRAC_MASK;
        let fl = self.floor();
        if f < (1 << 31) {
            fl
        } else if f > (1 << 31) || fl.0 & ONE_RAW != 0 {
            fl.slow_add(Fix::ONE)
        } else {
            fl
        }
    }

    /// `Sqrt`; negative input faults and returns 0.
    #[inline]
    pub fn sqrt(self) -> Fix {
        if self.0 < 0 {
            raise(Fault::Argument);
            return Fix(0);
        }
        let n = (self.0 as u128) << 32;
        let r = n.isqrt();
        let r = if n - r * r > r { r + 1 } else { r };
        Fix(r as i64)
    }

    // -- trigonometry -------------------------------------------------------

    /// Shared angle reduction of Sin and FastSin (§6.1): `(q, flipH, flipV)`.
    #[inline]
    fn reduce(x: i64) -> (i64, bool, bool) {
        const C: i64 = 7_244_019_458_077_122_842;
        let mut r = x;
        for k in 0..=28 {
            r %= C >> k;
        }
        if x < 0 {
            r = r.wrapping_add(Fix::PI_TIMES_2.0);
        }
        let flip_v = r >= Fix::PI.0;
        let mut q = r;
        while q >= Fix::PI.0 {
            q -= Fix::PI.0;
        }
        let flip_h = q >= Fix::PI_OVER_2.0;
        if flip_h {
            q -= Fix::PI_OVER_2.0;
        }
        (q, flip_h, flip_v)
    }

    #[inline]
    fn lut_at(t: &[i64], i: i64) -> i64 {
        if i < 0 || i as usize >= LUT_SIZE {
            raise(Fault::Index);
            return 0;
        }
        t[i as usize]
    }

    /// Interpolated sine (§6.2).
    pub fn sin(self) -> Fix {
        let t = &luts().sin;
        let (q, flip_h, flip_v) = Fix::reduce(self.0);
        let f = Fix(q).mul(Fix::LUT_INTERVAL);
        let n = f.round();
        let d = f - n;
        let k = n.0 >> 32;
        let sd = d.sign() as i64;
        let (s0, s1) = if !flip_h {
            (Fix::lut_at(t, k), Fix::lut_at(t, k + sd))
        } else {
            (
                Fix::lut_at(t, 205_886 - k),
                Fix::lut_at(t, 205_886 - k - sd),
            )
        };
        let delta = d.mul(Fix(s0.wrapping_sub(s1)).abs()).0;
        let v = if flip_h {
            s0.wrapping_sub(delta)
        } else {
            s0.wrapping_add(delta)
        };
        Fix(if flip_v { v.wrapping_neg() } else { v })
    }

    /// Nearest-below table sine (§6.3).
    pub fn fast_sin(self) -> Fix {
        let t = &luts().sin;
        let (q, flip_h, flip_v) = Fix::reduce(self.0);
        let mut k = ((q as u64) >> 15) as usize;
        if k >= LUT_SIZE {
            k = LUT_SIZE - 1;
        }
        let v = t[if flip_h { 205_886 - k } else { k }];
        Fix(if flip_v { v.wrapping_neg() } else { v })
    }

    #[inline]
    fn cos_arg(self) -> Fix {
        if self.0 > 0 {
            Fix(self.0.wrapping_sub(Fix::THREE_HALF_PI.0))
        } else {
            Fix(self.0.wrapping_add(Fix::PI_OVER_2.0))
        }
    }

    /// Interpolated cosine.
    pub fn cos(self) -> Fix {
        self.cos_arg().sin()
    }

    /// Table cosine.
    pub fn fast_cos(self) -> Fix {
        self.cos_arg().fast_sin()
    }

    /// Tangent (§6.5).
    pub fn tan(self) -> Fix {
        let t = &luts().tan;
        let mut m = self.0 % Fix::PI.0;
        let mut neg = false;
        if m < 0 {
            m = -m;
            neg = true;
        }
        if m > Fix::PI_OVER_2.0 {
            neg = !neg;
            m = Fix::PI.0 - m;
        }
        let f = Fix(m).mul(Fix::LUT_INTERVAL);
        let n = f.round();
        let d = f - n;
        let k = n.0 >> 32;
        let t0 = Fix::lut_at(t, k);
        let t1 = Fix::lut_at(t, k + d.sign() as i64);
        let delta = d.mul(Fix(t0.wrapping_sub(t1)).abs()).0;
        let v = t0.wrapping_add(delta);
        Fix(if neg { v.wrapping_neg() } else { v })
    }

    /// Arc tangent (Euler series, §6.6).
    pub fn atan(self) -> Fix {
        let mut z = self;
        if z.0 == 0 {
            return Fix::ZERO;
        }
        let neg = z.0 < 0;
        if neg {
            z = -z;
        }
        let inv = z > Fix::ONE;
        if inv {
            z = Fix::ONE / z;
        }
        let zz = z.slow_mul(z);
        let zz2 = zz.slow_mul(Fix::TWO);
        let zz1 = zz.slow_add(Fix::ONE);
        let c = zz1.slow_mul(Fix::TWO);
        let mut num = zz2;
        let mut den = zz1.slow_mul(Fix::THREE);
        let mut sum = Fix::ONE;
        let mut term = Fix::ONE;
        for _ in 2..30 {
            term = term.slow_mul(num / den);
            sum = sum.slow_add(term);
            num = num.slow_add(zz2);
            den = den.slow_add(c);
            if term.0 == 0 {
                break;
            }
        }
        let mut res = sum.slow_mul(z) / zz1;
        if inv {
            res = Fix::PI_OVER_2.slow_sub(res);
        }
        if neg {
            res = -res;
        }
        res
    }

    /// `Atan2(y, x)` with the original's fast approximation and its steep
    /// branch transcription error (§6.7).
    pub fn atan2(y: Fix, x: Fix) -> Fix {
        if x.0 == 0 {
            return if y.0 > 0 {
                Fix::PI_OVER_2
            } else if y.0 == 0 {
                Fix::ZERO
            } else {
                -Fix::PI_OVER_2
            };
        }
        let z = y / x;
        let d = Fix::ONE.slow_add(Fix::ATAN2_K.slow_mul(z.slow_mul(z)));
        if d == Fix::MAX {
            return if y.0 < 0 {
                -Fix::PI_OVER_2
            } else {
                Fix::PI_OVER_2
            };
        }
        if z.slow_abs() < Fix::ONE {
            let res = z / d;
            if x.0 < 0 {
                if y.0 < 0 {
                    return res.slow_sub(Fix::PI);
                }
                return res.slow_add(Fix::PI);
            }
            res
        } else {
            let res = Fix::PI_OVER_2.slow_sub(z) / z.slow_mul(z).slow_add(Fix::ATAN2_K);
            if y.0 < 0 { res.slow_sub(Fix::PI) } else { res }
        }
    }

    /// Arc cosine (§6.8); out-of-range input faults and returns 0.
    pub fn acos(self) -> Fix {
        if self < -Fix::ONE || self > Fix::ONE {
            raise(Fault::Argument);
            return Fix::ZERO;
        }
        if self.0 == 0 {
            return Fix::PI_OVER_2;
        }
        let r = (Fix::ONE.slow_sub(self.slow_mul(self)).sqrt() / self).atan();
        if self.0 >= 0 { r } else { r.slow_add(Fix::PI) }
    }

    // -- exponentials (unused by the simulation, kept for completeness) ------

    /// `Log2`; non-positive input faults and returns 0.
    pub fn log2(self) -> Fix {
        if self.0 <= 0 {
            raise(Fault::Argument);
            return Fix::ZERO;
        }
        let mut b: i64 = 1 << 31;
        let mut y: i64 = 0;
        let mut r = self.0;
        while r < ONE_RAW {
            r <<= 1;
            y -= ONE_RAW;
        }
        while r >= 2 * ONE_RAW {
            r >>= 1;
            y += ONE_RAW;
        }
        let mut z = Fix(r);
        for _ in 0..32 {
            z = z.mul(z);
            if z.0 >= 2 * ONE_RAW {
                z = Fix(z.0 >> 1);
                y += b;
            }
            b >>= 1;
        }
        Fix(y)
    }

    /// `Ln = Log2 · Ln2`.
    pub fn ln(self) -> Fix {
        self.log2().mul(Fix::LN2)
    }

    /// `Pow2`.
    pub fn pow2(self) -> Fix {
        let mut x = self;
        if x.0 == 0 {
            return Fix::ONE;
        }
        let neg = x.0 < 0;
        if neg {
            x = -x;
        }
        if x == Fix::ONE {
            return if neg { Fix::ONE / Fix::TWO } else { Fix::TWO };
        }
        if x >= Fix::LOG2_MAX {
            return if neg { Fix::ONE / Fix::MAX } else { Fix::MAX };
        }
        if x <= Fix::LOG2_MIN {
            return if neg { Fix::MAX } else { Fix::ZERO };
        }
        let e = (x.floor().0 >> 32) as i32;
        let x = Fix(x.0 & FRAC_MASK);
        let mut res = Fix::ONE;
        let mut term = Fix::ONE;
        let mut k = 1;
        while term.0 != 0 {
            term = x.mul(term).mul(Fix::LN2) / Fix::from_int(k);
            res = res.slow_add(term);
            k += 1;
        }
        res = Fix(res.0.wrapping_shl(e as u32));
        if neg {
            res = Fix::ONE / res;
        }
        res
    }

    /// `Pow(b, e)`.
    pub fn pow(self, e: Fix) -> Fix {
        if self == Fix::ONE || e.0 == 0 {
            return Fix::ONE;
        }
        if self.0 == 0 {
            if e.0 < 0 {
                raise(Fault::DivideByZero);
            }
            return Fix::ZERO;
        }
        e.slow_mul(self.log2()).pow2()
    }
}

impl fmt::Debug for Fix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.to_f64(), self.0)
    }
}

impl fmt::Display for Fix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_f64())
    }
}

impl Add for Fix {
    type Output = Fix;
    #[inline]
    fn add(self, o: Fix) -> Fix {
        Fix(self.0.wrapping_add(o.0))
    }
}
impl Sub for Fix {
    type Output = Fix;
    #[inline]
    fn sub(self, o: Fix) -> Fix {
        Fix(self.0.wrapping_sub(o.0))
    }
}
impl Mul for Fix {
    type Output = Fix;
    #[inline]
    fn mul(self, o: Fix) -> Fix {
        Fix::mul(self, o)
    }
}
impl Div for Fix {
    type Output = Fix;
    #[inline]
    fn div(self, o: Fix) -> Fix {
        Fix::div(self, o)
    }
}
impl Rem for Fix {
    type Output = Fix;
    #[inline]
    fn rem(self, o: Fix) -> Fix {
        Fix::rem(self, o)
    }
}
impl Neg for Fix {
    type Output = Fix;
    #[inline]
    fn neg(self) -> Fix {
        if self.0 == i64::MIN {
            Fix::MAX
        } else {
            Fix(-self.0)
        }
    }
}
impl AddAssign for Fix {
    #[inline]
    fn add_assign(&mut self, o: Fix) {
        *self = *self + o;
    }
}
impl SubAssign for Fix {
    #[inline]
    fn sub_assign(&mut self, o: Fix) {
        *self = *self - o;
    }
}
impl MulAssign for Fix {
    #[inline]
    fn mul_assign(&mut self, o: Fix) {
        *self = *self * o;
    }
}
impl DivAssign for Fix {
    #[inline]
    fn div_assign(&mut self, o: Fix) {
        *self = *self / o;
    }
}

/// `(Fix)n` for an integer literal.
#[inline]
pub const fn fx(n: i32) -> Fix {
    Fix::from_int(n)
}

/// A Fix from its raw bits.
#[inline]
pub const fn raw(r: i64) -> Fix {
    Fix(r)
}

//! The sine and tangent lookup tables (fixmath.md §5).
//!
//! The tables are generated once per process from the documented formula,
//! with sin/tan evaluated in double-double arithmetic (≈ 106 bits) and rounded
//! once to `f64`, which reproduces a correctly rounded libm. Only IEEE-754
//! basic operations (`+ − × ÷` on `f64`, correctly rounded on every platform
//! Rust supports) are used, so the result is platform independent. The
//! tables are never stored or shipped; `tests` checks their SHA-256 against
//! the hashes in the spec.

use std::sync::OnceLock;

/// Number of entries of each table.
pub const LUT_SIZE: usize = 205_887;

/// The two tables (raw Q32.32 values).
pub struct Luts {
    /// `SinLut[i] = trunc(sin(θ_i) · 2^32)`.
    pub sin: Box<[i64]>,
    /// `TanLut[i]`, with `MaxValue` at π/2.
    pub tan: Box<[i64]>,
}

static LUTS: OnceLock<Luts> = OnceLock::new();

/// The process-wide tables (generated on first use, ≈ 0.1 s in release).
#[inline]
pub fn luts() -> &'static Luts {
    LUTS.get_or_init(generate)
}

/// Forces generation (e.g. at start-up, outside the first tick).
pub fn init() {
    let _ = luts();
}

// ---------------------------------------------------------------------------
// Double-double arithmetic (Dekker / Knuth error-free transformations).

#[derive(Clone, Copy, Debug)]
struct Dd {
    hi: f64,
    lo: f64,
}

#[inline]
fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let bb = s - a;
    let e = (a - (s - bb)) + (b - bb);
    Dd { hi: s, lo: e }
}

#[inline]
fn quick_two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let e = b - (s - a);
    Dd { hi: s, lo: e }
}

#[inline]
fn split(a: f64) -> (f64, f64) {
    const SPLITTER: f64 = 134_217_729.0; // 2^27 + 1
    let t = SPLITTER * a;
    let hi = t - (t - a);
    (hi, a - hi)
}

#[inline]
fn two_prod(a: f64, b: f64) -> Dd {
    let p = a * b;
    let (ah, al) = split(a);
    let (bh, bl) = split(b);
    let e = ((ah * bh - p) + ah * bl + al * bh) + al * bl;
    Dd { hi: p, lo: e }
}

impl Dd {
    const fn new(hi: f64, lo: f64) -> Dd {
        Dd { hi, lo }
    }

    #[inline]
    fn add(self, o: Dd) -> Dd {
        let s = two_sum(self.hi, o.hi);
        let t = two_sum(self.lo, o.lo);
        let s = quick_two_sum(s.hi, s.lo + t.hi);
        quick_two_sum(s.hi, s.lo + t.lo)
    }

    #[inline]
    fn neg(self) -> Dd {
        Dd::new(-self.hi, -self.lo)
    }

    #[inline]
    fn sub(self, o: Dd) -> Dd {
        self.add(o.neg())
    }

    #[inline]
    fn mul(self, o: Dd) -> Dd {
        let p = two_prod(self.hi, o.hi);
        let lo = p.lo + (self.hi * o.lo + self.lo * o.hi);
        quick_two_sum(p.hi, lo)
    }

    #[inline]
    fn div_f(self, b: f64) -> Dd {
        let q1 = self.hi / b;
        let p = two_prod(q1, b);
        let r = self.sub(p);
        let q2 = r.hi / b;
        let p2 = two_prod(q2, b);
        let r2 = r.sub(p2);
        let q3 = r2.hi / b;
        let q = quick_two_sum(q1, q2);
        q.add(Dd::new(q3, 0.0))
    }

    #[inline]
    fn div(self, b: Dd) -> Dd {
        let q1 = self.hi / b.hi;
        let r = self.sub(b.mul(Dd::new(q1, 0.0)));
        let q2 = r.hi / b.hi;
        let r = r.sub(b.mul(Dd::new(q2, 0.0)));
        let q3 = r.hi / b.hi;
        let q = quick_two_sum(q1, q2);
        q.add(Dd::new(q3, 0.0))
    }

    /// The double nearest to `hi + lo`.
    #[inline]
    fn to_f64(self) -> f64 {
        self.hi + self.lo
    }
}

/// π/2 as a double-double (≈ 107 bits).
const PI_2: Dd = Dd::new(std::f64::consts::FRAC_PI_2, 6.123_233_995_736_766e-17);

/// Taylor series of sin and cos for `|x| ≤ π/4`.
fn sin_cos_small(x: Dd) -> (Dd, Dd) {
    let x2 = x.mul(x);
    // sin
    let mut term = x;
    let mut sum = x;
    let mut k = 1.0;
    loop {
        term = term.mul(x2).div_f((2.0 * k) * (2.0 * k + 1.0)).neg();
        sum = sum.add(term);
        if term.hi.abs() < 1e-40 {
            break;
        }
        k += 1.0;
    }
    let s = sum;
    // cos
    let mut term = Dd::new(1.0, 0.0);
    let mut sum = term;
    let mut k = 1.0;
    loop {
        term = term.mul(x2).div_f((2.0 * k - 1.0) * (2.0 * k)).neg();
        sum = sum.add(term);
        if term.hi.abs() < 1e-40 {
            break;
        }
        k += 1.0;
    }
    (s, sum)
}

/// sin and cos of `a ∈ [0, π/2]` in double-double.
fn sin_cos(a: f64) -> (Dd, Dd) {
    if a <= std::f64::consts::FRAC_PI_4 {
        sin_cos_small(Dd::new(a, 0.0))
    } else {
        let y = PI_2.sub(Dd::new(a, 0.0));
        let (s, c) = sin_cos_small(y);
        (c, s)
    }
}

fn generate() -> Luts {
    let mut sin = vec![0i64; LUT_SIZE].into_boxed_slice();
    let mut tan = vec![0i64; LUT_SIZE].into_boxed_slice();
    const TWO32: f64 = 4_294_967_296.0;
    const M: f64 = 2_147_483_648.0;
    for i in 0..LUT_SIZE {
        let a = ((i as f64 * std::f64::consts::PI) * 0.5) / 205_886.0;
        let (s, c) = sin_cos(a);
        let sd = s.to_f64();
        sin[i] = (sd * TWO32) as i64;
        let mut t = if c.hi == 0.0 {
            f64::INFINITY
        } else {
            s.div(c).to_f64()
        };
        if t > M || t < 0.0 {
            t = M;
        }
        tan[i] = if t >= M { i64::MAX } else { (t * TWO32) as i64 };
    }
    Luts { sin, tan }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal SHA-256 (FIPS 180-4) for the table check.
    pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut h: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];
        let mut msg = data.to_vec();
        let bits = (data.len() as u64) * 8;
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bits.to_be_bytes());
        for chunk in msg.chunks(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes(chunk[4 * i..4 * i + 4].try_into().unwrap());
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let mut v = h;
            for i in 0..64 {
                let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
                let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
                let t1 = v[7]
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
                let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
                let t2 = s0.wrapping_add(maj);
                v[7] = v[6];
                v[6] = v[5];
                v[5] = v[4];
                v[4] = v[3].wrapping_add(t1);
                v[3] = v[2];
                v[2] = v[1];
                v[1] = v[0];
                v[0] = t1.wrapping_add(t2);
            }
            for i in 0..8 {
                h[i] = h[i].wrapping_add(v[i]);
            }
        }
        let mut out = [0u8; 32];
        for i in 0..8 {
            out[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
        }
        out
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn table_hash(t: &[i64]) -> String {
        let bytes: Vec<u8> = t.iter().flat_map(|v| v.to_le_bytes()).collect();
        hex(&sha256(&bytes))
    }

    #[test]
    fn sha256_known_answer() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn lut_endpoints() {
        let l = luts();
        assert_eq!(l.sin[0], 0);
        assert_eq!(l.sin[205_886], 1 << 32);
        assert_eq!(l.tan[0], 0);
        assert_eq!(l.tan[205_885], 562_946_081_303_480);
        assert_eq!(l.tan[205_886], i64::MAX);
    }

    #[test]
    fn lut_hashes_match_spec() {
        let l = luts();
        assert_eq!(
            table_hash(&l.sin),
            "c961c966d351773f613616cbb593e8b4c55de47d1d39b6c9d8cc3f6d4a0b58a3"
        );
        assert_eq!(
            table_hash(&l.tan),
            "335db3ae988eb72adf47fd97ca02052af3cb407cfff3d3954f6d10ab94850eda"
        );
    }
}

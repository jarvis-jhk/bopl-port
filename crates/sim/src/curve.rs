//! `AnimationCurveFixed`: Hermite segments on serialized Fix keys with
//! Unity's wrap modes (fixmath.md §10, engine.md §10c).

use crate::fix::Fix;

/// One key (raw Fix values from the data).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Key {
    pub time: Fix,
    pub value: Fix,
    pub in_tangent: Fix,
    pub out_tangent: Fix,
}

/// Unity's wrap mode codes: 0 ping-pong, 1 loop, 2 clamp (default).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Wrap {
    PingPong,
    Loop,
    #[default]
    Clamp,
}

impl Wrap {
    /// From the serialized `m_PreInfinity` / `m_PostInfinity` code.
    pub fn from_code(c: i64) -> Wrap {
        match c {
            0 => Wrap::PingPong,
            1 => Wrap::Loop,
            _ => Wrap::Clamp,
        }
    }
}

/// A fixed-point animation curve.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Curve {
    pub keys: Vec<Key>,
    pub pre: Wrap,
    pub post: Wrap,
}

impl Curve {
    /// Time of the last key (the curve's duration), 0 without keys.
    pub fn last_time(&self) -> Fix {
        self.keys.last().map_or(Fix::ZERO, |k| k.time)
    }

    /// `Evaluate(t)`.
    pub fn eval(&self, t: Fix) -> Fix {
        let n = self.keys.len();
        if n == 0 {
            return Fix::ZERO;
        }
        let t0 = self.keys[0].time;
        let t1 = self.keys[n - 1].time;
        let l = t1 - t0;
        let mut t = t;
        if n >= 2 && t <= t0 {
            match self.pre {
                Wrap::Loop => {
                    t = t - t0;
                    t = l - t.slow_abs().slow_mod(l);
                    t = t + t0;
                }
                Wrap::PingPong => {
                    t = t - t0;
                    let c = (t / l).slow_abs().to_long() as i32;
                    t = l - t.slow_abs().slow_mod(l);
                    t = t.slow_mod(l);
                    if c % 2 == 0 {
                        t = l - t;
                    }
                    t = t + t0;
                }
                Wrap::Clamp => return self.keys[0].value,
            }
        } else if n >= 2 && t >= t1 {
            match self.post {
                Wrap::Loop => {
                    t = (t - t0).slow_mod(l) + t0;
                }
                Wrap::PingPong => {
                    t = t - t0;
                    let c = (t / l).slow_abs().to_long() as i32;
                    t = t.slow_mod(l);
                    if c % 2 != 0 {
                        t = l - t;
                    }
                    t = t + t0;
                }
                Wrap::Clamp => return self.keys[n - 1].value,
            }
        }
        for i in 1..n {
            if t <= self.keys[i].time {
                return hermite(&self.keys[i - 1], &self.keys[i], t);
            }
        }
        Fix::ZERO
    }
}

fn hermite(a: &Key, b: &Key, t: Fix) -> Fix {
    let d = b.time - a.time;
    let u = (t - a.time) / d;
    let m0 = a.out_tangent * d;
    let m1 = b.in_tangent * d;
    let u2 = u * u;
    let u3 = u2 * u;
    let h00 = Fix::TWO * u3 - Fix::THREE * u2 + Fix::ONE;
    let h10 = u3 - Fix::TWO * u2 + u;
    let h01 = u3 - u2;
    let h11 = (-Fix::TWO) * u3 + Fix::THREE * u2;
    h00 * a.value + h10 * m0 + h01 * m1 + h11 * b.value
}

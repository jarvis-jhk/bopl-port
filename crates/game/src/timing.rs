//! Fixed-timestep accumulator with render interpolation.

/// The original's simulation step: `Updater` runs the deterministic sim at
/// Unity's fixed timestep, 1/60 s (`index.json` `settings.fixed_timestep`).
pub const FIXED_DT: f64 = 1.0 / 60.0;

/// Turns variable frame times into a whole number of fixed simulation steps
/// plus an interpolation factor for rendering between the last two states.
///
/// Exact: time is kept in integer nanoseconds times the step rate, so 60
/// steps happen per second of frame time with no floating-point drift,
/// whatever the display's refresh rate.
#[derive(Clone, Debug)]
pub struct FixedStep {
    dt: f64,
    /// Steps per second.
    rate: u64,
    /// Accumulated nanoseconds × `rate` (one step = 10⁹).
    acc: u64,
    /// Longest frame time accepted (ns); longer hitches are clamped so the
    /// sim does not spiral trying to catch up.
    max_frame_ns: u64,
    ticks: u64,
}

const NS: u64 = 1_000_000_000;

#[allow(dead_code)]
impl FixedStep {
    pub fn new(dt: f64) -> Self {
        Self {
            dt,
            rate: (1.0 / dt).round().max(1.0) as u64,
            acc: 0,
            max_frame_ns: NS / 4,
            ticks: 0,
        }
    }

    /// Adds a frame's elapsed time; returns how many steps to run now.
    pub fn advance(&mut self, frame_dt: f64) -> u32 {
        let ns = ((frame_dt.max(0.0) * 1e9).round() as u64).min(self.max_frame_ns);
        self.advance_ns(ns)
    }

    /// [`Self::advance`] in nanoseconds.
    pub fn advance_ns(&mut self, ns: u64) -> u32 {
        self.acc += ns.min(self.max_frame_ns) * self.rate;
        let n = self.acc / NS;
        self.acc %= NS;
        self.ticks += n;
        n as u32
    }

    /// How far (0..1) the current time is between the last step and the
    /// next: draw `lerp(previous, current, alpha)`.
    pub fn alpha(&self) -> f32 {
        (self.acc as f64 / NS as f64) as f32
    }

    /// Total steps run so far.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn dt(&self) -> f64 {
        self.dt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixty_hz_frames() {
        let mut f = FixedStep::new(FIXED_DT);
        let mut total = 0;
        for _ in 0..600 {
            total += f.advance(1.0 / 60.0);
            assert!((0.0..=1.0).contains(&f.alpha()));
        }
        // 10 s at 60 steps per second (±1 for float accumulation).
        assert!((599..=601).contains(&total), "{total}");
        assert_eq!(f.ticks(), total as u64);
    }

    #[test]
    fn exact_at_any_refresh_rate() {
        // 144 Hz, 75 Hz, 30 Hz, 59.94 Hz: one hour of frames gives one
        // hour of ticks to the step (no drift).
        for hz in [144u64, 75, 30] {
            let mut f = FixedStep::new(FIXED_DT);
            let frame = NS / hz;
            let rem = NS % hz;
            for s in 0..3600u64 {
                for k in 0..hz {
                    // Spread the remainder so each second is exact.
                    f.advance_ns(frame + u64::from(k < rem));
                }
                assert_eq!(f.ticks(), (s + 1) * 60, "{hz} Hz after {s} s");
            }
        }
        let mut f = FixedStep::new(FIXED_DT);
        for _ in 0..(59.94 * 600.0) as u32 {
            f.advance(1.0 / 59.94);
        }
        assert_eq!(f.ticks(), 600 * 60 - 1);
    }

    #[test]
    fn hitch_is_clamped() {
        let mut f = FixedStep::new(FIXED_DT);
        assert_eq!(f.advance(5.0), (0.25 / FIXED_DT) as u32);
    }
}

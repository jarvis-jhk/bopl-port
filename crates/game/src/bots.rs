//! Deterministic input generators for headless tests and bot matches.
//!
//! A [`Bot`] turns the sim's [`View`] into one player's [`PlayerInput`]
//! each tick with its own seeded RNG, so a bot match is a pure function of
//! (seed, players, level). [`script`] expands the `sim-trace` script syntax
//! (`"220:;40:R;1:RJ;200:L"`) into per-tick inputs.

use anyhow::{Context, Result};
use bopl_sim::fix::Fix;
use bopl_sim::flow::Circle;
use bopl_sim::input::{
    BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2, BTN_JUMP, BTN_SELECT, BTN_START, PlayerInput,
    angle_byte_from_direction,
};
use bopl_sim::view::View;

/// The angle byte of a direction of any length (normalised like a device
/// stick; 0 inside the dead zone).
fn byte(x: f32, y: f32) -> u8 {
    angle_byte_from_direction(bopl_sim::Vec2::new(Fix::from_f32(x), Fix::from_f32(y)))
}

fn parse_keys(k: &str) -> PlayerInput {
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let mut b = 0;
    for c in k.chars() {
        match c {
            'L' => x -= 1.0,
            'R' => x += 1.0,
            'U' => y += 1.0,
            'D' => y -= 1.0,
            'J' => b |= BTN_JUMP,
            '1' => b |= BTN_ABILITY0,
            '2' => b |= BTN_ABILITY1,
            '3' => b |= BTN_ABILITY2,
            'S' => b |= BTN_START,
            'E' => b |= BTN_SELECT,
            _ => {}
        }
    }
    PlayerInput::new(byte(x, y), b)
}

/// Expands `"N:KEYS;N:KEYS;..."` (keys: L R U D J 1 2 3 S=start E=select)
/// into per-tick inputs.
pub fn script(s: &str) -> Result<Vec<PlayerInput>> {
    let mut out = Vec::new();
    for step in s.split(';').filter(|x| !x.trim().is_empty()) {
        let (n, k) = step.split_once(':').unwrap_or((step, ""));
        let n: usize = n
            .trim()
            .parse()
            .with_context(|| format!("bad script step {step:?}"))?;
        out.extend(std::iter::repeat_n(parse_keys(k.trim()), n));
    }
    Ok(out)
}

/// What a bot does on the winner screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlowPolicy {
    /// Nothing (the host or a script decides).
    #[default]
    Idle,
    /// Press jump ("next level") a while after the winner screen appeared,
    /// and use the ability wheel when losing.
    NextLevel,
}

/// A simple deterministic bot.
#[derive(Clone, Debug)]
pub struct Bot {
    /// Player list index.
    pub index: usize,
    rng: u32,
    wander: f32,
    wander_ticks: u32,
    hold_jump: u32,
    ability_hold: u32,
    ability_slot: usize,
    wheel_plan: u32,
    pub policy: FlowPolicy,
}

impl Bot {
    pub fn new(index: usize, seed: u32) -> Self {
        let mut rng = seed ^ (index as u32 + 1).wrapping_mul(0x9e37_79b9);
        if rng == 0 {
            rng = 1;
        }
        Self {
            index,
            rng,
            wander: 0.0,
            wander_ticks: 0,
            hold_jump: 0,
            ability_hold: 0,
            ability_slot: 0,
            wheel_plan: 0,
            policy: FlowPolicy::default(),
        }
    }

    fn next(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    fn chance(&mut self, one_in: u32) -> bool {
        self.next().is_multiple_of(one_in.max(1))
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// This tick's input.
    pub fn input(&mut self, v: &View) -> PlayerInput {
        let Some(me) = v.players.get(self.index) else {
            return PlayerInput::default();
        };
        let flow = v.flow.players.get(self.index);
        // Winner screen and wheel.
        if self.policy == FlowPolicy::NextLevel {
            if let Some(fp) = flow {
                if fp.wheel.open {
                    // Change one ability, then close.
                    self.wheel_plan += 1;
                    return match self.wheel_plan {
                        10 => PlayerInput::new(byte(-1.0, 0.0), 0),
                        30 => PlayerInput::new(0, BTN_JUMP),
                        _ => PlayerInput::default(),
                    };
                }
                if let Circle::Loser { since, .. } = fp.circle
                    && v.tick - since > 200
                    && v.tick - since < 205
                    && (self.index % 2 == 1)
                {
                    self.wheel_plan = 0;
                    return PlayerInput::new(byte(0.0, 1.0), 0);
                }
            }
            if let Some(at) = v.flow.winner_screen_at {
                let blocked = v.flow.players.iter().any(|p| p.wheel.blocking());
                let wait = 150 + 20 * self.index as i64;
                if !blocked && v.tick - at > wait && (v.tick - at) % 30 == 0 {
                    return PlayerInput::new(0, BTN_JUMP);
                }
                return PlayerInput::default();
            }
        }
        if !me.spawned || !me.alive || v.round.winner_screen {
            return PlayerInput::default();
        }
        let pos = (me.position.x.to_f32(), me.position.y.to_f32());
        // Nearest enemy.
        let target = v
            .players
            .iter()
            .filter(|p| p.team != me.team && p.alive && p.spawned)
            .map(|p| (p.position.x.to_f32(), p.position.y.to_f32()))
            .min_by(|a, b| {
                let da = (a.0 - pos.0).powi(2) + (a.1 - pos.1).powi(2);
                let db = (b.0 - pos.0).powi(2) + (b.1 - pos.1).powi(2);
                da.total_cmp(&db)
            });
        let mut buttons = 0u8;
        let mut dy = 0.0f32;
        let mut dx;
        if self.wander_ticks == 0 {
            self.wander = self.unit() * 2.0 - 1.0;
            self.wander_ticks = 30 + self.next() % 60;
        }
        self.wander_ticks -= 1;
        match target {
            Some((tx, ty)) => {
                let (ex, ey) = (tx - pos.0, ty - pos.1);
                dx = if ex.abs() > 3.0 {
                    ex.signum()
                } else {
                    self.wander
                };
                if ey > 4.0 && self.chance(20) {
                    self.hold_jump = 8;
                }
                let d2 = ex * ex + ey * ey;
                if d2 < 30.0 * 30.0 && self.ability_hold == 0 && self.chance(40) {
                    self.ability_slot = (self.next() % 3) as usize;
                    self.ability_hold = 4 + self.next() % 20;
                }
                if self.ability_hold > 0 {
                    // Aim at the target while holding.
                    let l = d2.sqrt().max(1e-3);
                    dx = ex / l;
                    dy = ey / l;
                }
            }
            None => dx = self.wander,
        }
        if self.chance(150) {
            self.hold_jump = 4 + self.next() % 10;
        }
        // Falling: steer towards the nearest platform.
        if pos.1 < -4.0 && !me.grounded {
            if let Some(p) = v
                .platforms
                .iter()
                .filter(|p| p.visible)
                .map(|p| (p.position.x.to_f32(), p.position.y.to_f32()))
                .min_by(|a, b| (a.0 - pos.0).abs().total_cmp(&(b.0 - pos.0).abs()))
            {
                dx = (p.0 - pos.0).signum();
            }
            self.hold_jump = 0;
        }
        if self.hold_jump > 0 {
            self.hold_jump -= 1;
            buttons |= BTN_JUMP;
            dy = dy.max(0.5);
        }
        if self.ability_hold > 0 {
            self.ability_hold -= 1;
            buttons |= BTN_ABILITY0 << self.ability_slot;
        }
        PlayerInput::new(byte(dx, dy), buttons)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_expands() {
        let s = script("2:;1:RJ;3:L").unwrap();
        assert_eq!(s.len(), 6);
        assert_eq!(s[0], PlayerInput::default());
        assert_eq!(s[2].buttons, BTN_JUMP);
        assert_eq!(s[2].angle, 1);
        assert_ne!(s[3].angle, 0);
        assert!(script("x:R").is_err());
    }

    #[test]
    fn bots_are_deterministic() {
        let v = View::default();
        let mut a = Bot::new(0, 5);
        let mut b = Bot::new(0, 5);
        for _ in 0..100 {
            assert_eq!(a.input(&v), b.input(&v));
        }
    }
}

//! A tiny deterministic game for netcode tests, shaped like Bopl Battle: a
//! 320×240 arena with up to 12 "bopls" (dots) that walk, jump and use three
//! abilities (pellet shot, dash, high jump, blink), in teams or free-for-all,
//! with horizontal screen wrap and rounds every `round_frames` frames. Each
//! round after the first opens with an **ability-select phase** in which the
//! ability buttons cycle the player's abilities: mid-game ability selection
//! done through sim inputs, the way bopl-sim does it. Per-player choices come
//! from [`PlayerSlot::extra`] (three ability bytes), the round length from the
//! settings blob [`MatchSettings::game`] ([`ToySettings`]).
//!
//! Integer math only, so every machine computes bit-identical states. It
//! follows the leave rule of docs/net.md: a disconnected slot is inert for
//! the rest of the round and removed at the round end.
//!
//! Also an input bot that behaves like a human (holds an input for a while,
//! then changes it) and [`ToyMachine`], one machine playing the toy over a
//! [`crate::NetSession`].
//!
//! [`PlayerSlot::extra`]: crate::PlayerSlot::extra
//! [`MatchSettings::game`]: crate::MatchSettings::game

use crate::game::{NetGame, Request};
use crate::session::{InputStatus, NetEvent, NetSession};
use crate::wire::{DesyncReport, DoneReport, MatchStart};
use bytemuck::{Pod, Zeroable};
use std::collections::BTreeMap;

const SUB: i32 = 256;
const W: i32 = 320 * SUB;
const H: i32 = 240 * SUB;
const R: i32 = 5 * SUB;
const MAX_PELLETS: usize = 48;
/// Default frames per toy round (10 s at 60 Hz).
pub const ROUND_FRAMES: i32 = 600;
/// Ability-select phase at the start of every round after the first.
pub const SELECT_FRAMES: i32 = 90;
/// Abilities the toy knows (0 pellet, 1 dash, 2 high jump, 3 blink).
pub const ABILITIES: u8 = 4;

/// One player's input for one frame, laid out like the original's input
/// sample (docs/spec/net-original.md 4.1): an angle byte (0 = no direction,
/// `1..=255` = direction `2π (b − 1) / 255`, y down), held buttons, and the
/// WASD bits of keyboard players. 4 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct ToyInput {
    pub angle: u8,
    pub buttons: u8,
    pub keys: u8,
    pub reserved: u8,
}

impl ToyInput {
    pub const BTN_JUMP: u8 = 1 << 0;
    pub const BTN_AB1: u8 = 1 << 1;
    pub const BTN_AB2: u8 = 1 << 2;
    pub const BTN_AB3: u8 = 1 << 3;
    pub const BTN_START: u8 = 1 << 4;
    pub const BTN_SELECT: u8 = 1 << 5;
    pub const ABILITY_BUTTONS: [u8; 3] = [Self::BTN_AB1, Self::BTN_AB2, Self::BTN_AB3];

    /// Direction as an integer vector of length ~256 (16 directions).
    pub fn dir(&self) -> (i32, i32) {
        // cos/sin × 256 for k × 22.5°.
        const C: [i32; 16] = [
            256, 237, 181, 98, 0, -98, -181, -237, -256, -237, -181, -98, 0, 98, 181, 237,
        ];
        if self.angle == 0 {
            return (0, 0);
        }
        let k = ((self.angle as usize - 1) * 16 + 127) / 255 % 16;
        (C[k], C[(k + 12) % 16])
    }
}

/// The toy's settings blob ([`crate::MatchSettings::game`]): `round_frames`
/// as `u16` little endian; empty = defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToySettings {
    pub round_frames: u16,
}

impl Default for ToySettings {
    fn default() -> Self {
        ToySettings {
            round_frames: ROUND_FRAMES as u16,
        }
    }
}

impl ToySettings {
    pub fn to_bytes(self) -> Vec<u8> {
        self.round_frames.to_le_bytes().to_vec()
    }

    pub fn from_bytes(b: &[u8]) -> ToySettings {
        match b {
            [a, c, ..] => ToySettings {
                round_frames: u16::from_le_bytes([*a, *c]).max(SELECT_FRAMES as u16 + 60),
            },
            _ => ToySettings::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dot {
    pub x: i32,
    pub y: i32,
    pub vx: i32,
    pub vy: i32,
    pub team: u8,
    /// The three equipped abilities (from `PlayerSlot::extra`, changed in
    /// the ability-select phase).
    pub abilities: [u8; 3],
    pub cooldown: [u8; 3],
    pub score: u32,
    pub deaths: u32,
    /// Plays (false once removed after a disconnect).
    pub alive: bool,
    /// Its machine left: inert now, removed at the round end.
    pub gone: bool,
    /// Dash invulnerability frames.
    pub dash: u8,
    pub prev_buttons: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pellet {
    pub x: i32,
    pub y: i32,
    pub vx: i32,
    pub vy: i32,
    pub owner: u8,
    pub life: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toy {
    /// Frames simulated (= sim frame of this state).
    pub frame: i32,
    pub round: u32,
    /// Frame at which the current round started.
    pub round_start: i32,
    pub round_frames: i32,
    pub rng: u64,
    pub dots: Vec<Dot>,
    pub pellets: Vec<Pellet>,
    /// Desync injection (tests): this machine's state is corrupted when it
    /// reaches this frame (on every timeline, so re-simulation keeps it).
    /// Local configuration, not part of the checksum.
    pub corrupt_at: Option<i32>,
}

impl Toy {
    /// Free-for-all, default abilities and settings.
    pub fn new(players: usize, seed: u64) -> Toy {
        let teams: Vec<u8> = (0..players as u8).collect();
        let abilities = vec![[0, 1, 2]; players];
        Toy::with(&teams, &abilities, ToySettings::default(), seed)
    }

    /// The match as every machine builds it from [`MatchStart`]: teams from
    /// the slot table, abilities from each slot's `extra` (missing bytes:
    /// 0, 1, 2), round length from the settings blob.
    pub fn from_match(ms: &MatchStart) -> Toy {
        let teams: Vec<u8> = ms.players.iter().map(|p| p.team).collect();
        let abilities: Vec<[u8; 3]> = ms
            .players
            .iter()
            .map(|p| {
                let mut a = [0, 1, 2];
                for (k, b) in p.extra.iter().take(3).enumerate() {
                    a[k] = b % ABILITIES;
                }
                a
            })
            .collect();
        Toy::with(
            &teams,
            &abilities,
            ToySettings::from_bytes(&ms.settings.game),
            ms.seed,
        )
    }

    fn with(teams: &[u8], abilities: &[[u8; 3]], s: ToySettings, seed: u64) -> Toy {
        let mut t = Toy {
            frame: 0,
            round: 0,
            round_start: 0,
            round_frames: s.round_frames as i32,
            rng: seed | 1,
            dots: vec![Dot::default(); teams.len()],
            pellets: Vec::new(),
            corrupt_at: None,
        };
        for i in 0..teams.len() {
            t.dots[i].team = teams[i];
            t.dots[i].abilities = abilities[i];
            t.dots[i].alive = true;
            t.spawn(i);
        }
        t
    }

    fn next(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn spawn(&mut self, i: usize) {
        let x = R + (self.next() % (W - 2 * R) as u64) as i32;
        let y = R + (self.next() % (H / 2) as u64) as i32;
        let d = &mut self.dots[i];
        (d.x, d.y, d.vx, d.vy, d.dash, d.cooldown) = (x, y, 0, 0, 0, [0; 3]);
    }

    /// In the ability-select phase (rounds after the first).
    pub fn selecting(&self) -> bool {
        self.round > 0 && self.frame - self.round_start < SELECT_FRAMES
    }

    fn step(&mut self, inputs: &[(ToyInput, InputStatus)]) {
        let selecting = self.selecting();
        for i in 0..self.dots.len() {
            let (inp, status) = inputs
                .get(i)
                .copied()
                .unwrap_or((ToyInput::default(), InputStatus::Disconnected));
            let d = &mut self.dots[i];
            if status == InputStatus::Disconnected {
                d.gone = true; // input is zero from now on: inert
            }
            if !d.alive {
                continue;
            }
            let pressed = inp.buttons & !d.prev_buttons;
            d.prev_buttons = inp.buttons;
            if selecting {
                // Mid-game ability select: each ability button cycles its
                // slot. A sim input, so every machine agrees on the choice.
                for (k, b) in ToyInput::ABILITY_BUTTONS.iter().enumerate() {
                    if pressed & b != 0 {
                        d.abilities[k] = (d.abilities[k] + 1) % ABILITIES;
                    }
                }
                continue;
            }
            let (dx, dy) = inp.dir();
            let on_ground = d.y >= H - R - SUB;
            d.vx += dx * 3 / 2;
            d.vy += 48;
            if pressed & ToyInput::BTN_JUMP != 0 && on_ground {
                d.vy = -1400;
            }
            let mut shots = Vec::new();
            for (k, b) in ToyInput::ABILITY_BUTTONS.iter().enumerate() {
                if pressed & b == 0 || d.cooldown[k] > 0 {
                    continue;
                }
                let (ax, ay) = if dx == 0 && dy == 0 {
                    (256, 0)
                } else {
                    (dx, dy)
                };
                match d.abilities[k] {
                    0 => {
                        shots.push((ax, ay));
                        d.cooldown[k] = 20;
                    }
                    1 => {
                        d.vx += ax * 14;
                        d.vy += ay * 14;
                        d.dash = 12;
                        d.cooldown[k] = 60;
                    }
                    2 => {
                        d.vy = -2600;
                        d.cooldown[k] = 90;
                    }
                    _ => {
                        d.x = (d.x + ax * 160).rem_euclid(W);
                        d.y = (d.y + ay * 160).clamp(R, H - R);
                        d.cooldown[k] = 120;
                    }
                }
            }
            for c in d.cooldown.iter_mut() {
                *c = c.saturating_sub(1);
            }
            d.dash = d.dash.saturating_sub(1);
            d.vx -= d.vx / 16;
            d.vy -= d.vy / 64;
            d.x = (d.x + d.vx).rem_euclid(W);
            d.y += d.vy;
            if d.y > H - R {
                d.y = H - R;
                d.vy = 0;
            }
            if d.y < R {
                d.y = R;
                d.vy = -d.vy / 2;
            }
            let (x, y) = (d.x, d.y);
            for (ax, ay) in shots {
                if self.pellets.len() < MAX_PELLETS {
                    self.pellets.push(Pellet {
                        x,
                        y,
                        vx: ax * 6,
                        vy: ay * 6,
                        owner: i as u8,
                        life: 120,
                    });
                }
            }
        }
        let mut hits = Vec::new();
        for a in self.pellets.iter_mut() {
            a.vy += 20;
            a.x = (a.x + a.vx).rem_euclid(W);
            a.y += a.vy;
            if a.y > H || a.y < 0 {
                a.vy = -a.vy * 3 / 4;
                a.y = a.y.clamp(0, H);
            }
            a.life = a.life.saturating_sub(1);
            let team = self.dots[a.owner as usize].team;
            for (j, d) in self.dots.iter().enumerate() {
                if d.team == team || !d.alive || d.dash > 0 {
                    continue;
                }
                let (dx, dy) = ((a.x - d.x) as i64, (a.y - d.y) as i64);
                if dx * dx + dy * dy < (R as i64 * 2).pow(2) {
                    hits.push((a.owner as usize, j));
                    a.life = 0;
                    break;
                }
            }
        }
        self.pellets.retain(|a| a.life > 0);
        for (owner, victim) in hits {
            self.dots[owner].score += 1;
            self.dots[victim].deaths += 1;
            self.spawn(victim);
        }
        self.frame += 1;
        if self.frame - self.round_start >= self.round_frames {
            self.round += 1;
            self.round_start = self.frame;
            self.pellets.clear();
            for i in 0..self.dots.len() {
                if self.dots[i].gone {
                    self.dots[i].alive = false;
                } else {
                    self.spawn(i);
                }
            }
        }
        if self.corrupt_at == Some(self.frame) {
            // Persistent: positions heal on respawn, the PRNG and counters
            // do not.
            self.dots[0].x ^= 0x100;
            self.dots[0].deaths += 1000;
            self.rng = self.rng.rotate_left(7) ^ 1;
        }
    }

    /// FNV-1a over the whole state, two lanes.
    pub fn checksum(&self) -> u128 {
        let mut a: u64 = 0xcbf29ce484222325;
        let mut b: u64 = 0x84222325cbf29ce4;
        let mut eat = |v: u64| {
            for byte in v.to_le_bytes() {
                a = (a ^ byte as u64).wrapping_mul(0x100000001b3);
                b = (b ^ (byte as u64).wrapping_add(0x5f)).wrapping_mul(0x100000001b3);
            }
        };
        eat(self.frame as u64);
        eat(self.round as u64 | (self.round_start as u32 as u64) << 32);
        eat(self.round_frames as u64);
        eat(self.rng);
        for d in &self.dots {
            for v in [d.x, d.y, d.vx, d.vy] {
                eat(v as u32 as u64);
            }
            eat(((d.score as u64) << 32) | d.deaths as u64);
            eat(d.alive as u64 | (d.gone as u64) << 1 | (d.dash as u64) << 8);
            eat(d.prev_buttons as u64 | (d.team as u64) << 8);
            for k in 0..3 {
                eat(d.abilities[k] as u64 | (d.cooldown[k] as u64) << 8);
            }
        }
        eat(self.pellets.len() as u64);
        for p in &self.pellets {
            for v in [p.x, p.y, p.vx, p.vy] {
                eat(v as u32 as u64);
            }
            eat(p.owner as u64 | (p.life as u64) << 8);
        }
        ((a as u128) << 64) | b as u128
    }

    /// Slots still in the game.
    pub fn alive(&self) -> usize {
        self.dots.iter().filter(|d| d.alive).count()
    }
}

impl NetGame for Toy {
    type Input = ToyInput;

    fn advance(&mut self, inputs: &[(ToyInput, InputStatus)]) {
        self.step(inputs);
    }

    fn checksum(&self) -> u128 {
        Toy::checksum(self)
    }
}

/// Plays a [`Toy`] from requests and remembers each frame's checksum.
pub struct ToyRunner {
    pub state: Toy,
    /// state.frame -> checksum (re-simulation overwrites).
    pub history: BTreeMap<i32, u128>,
}

impl ToyRunner {
    pub fn new(players: usize, seed: u64) -> ToyRunner {
        ToyRunner {
            state: Toy::new(players, seed),
            history: BTreeMap::new(),
        }
    }

    pub fn from_match(ms: &MatchStart) -> ToyRunner {
        ToyRunner {
            state: Toy::from_match(ms),
            history: BTreeMap::new(),
        }
    }

    pub fn handle(&mut self, reqs: Vec<Request<Toy>>) {
        for r in reqs {
            match r {
                Request::Save { cell, frame } => {
                    debug_assert_eq!(frame, self.state.frame);
                    cell.save(frame, Some(self.state.clone()), Some(self.state.checksum()));
                }
                Request::Load { cell, frame } => {
                    let corrupt = self.state.corrupt_at;
                    self.state = cell.load().expect("saved state");
                    self.state.corrupt_at = corrupt;
                    debug_assert_eq!(frame, self.state.frame);
                }
                Request::Advance { frame, inputs, .. } => {
                    debug_assert_eq!(frame, self.state.frame);
                    self.state.step(&inputs);
                    self.history.insert(self.state.frame, self.state.checksum());
                }
            }
        }
        while self.history.len() > 8192 {
            self.history.pop_first();
        }
    }
}

/// A pseudo-human: holds an input for 4–40 frames, then picks another.
pub struct Bot {
    rng: u64,
    hold: u32,
    cur: ToyInput,
}

impl Bot {
    pub fn new(seed: u64) -> Bot {
        Bot {
            rng: seed.wrapping_mul(0x9E3779B97F4A7C15) | 1,
            hold: 0,
            cur: ToyInput::default(),
        }
    }

    fn next(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    pub fn input(&mut self) -> ToyInput {
        if self.hold == 0 {
            let r = self.next();
            self.hold = 4 + (r % 37) as u32;
            let b = (r >> 32) as u8;
            let mut buttons = 0;
            if b & 0b11 == 0 {
                buttons |= ToyInput::BTN_JUMP;
            }
            if (b >> 2) & 0b11 == 0 {
                buttons |= ToyInput::BTN_AB1;
            }
            if (b >> 4) & 0b111 == 0 {
                buttons |= ToyInput::BTN_AB2;
            }
            if (b >> 7) & 1 == 0 && (r >> 48).is_multiple_of(4) {
                buttons |= ToyInput::BTN_AB3;
            }
            // A quarter of the time no direction, else one of 16.
            let angle = if r.is_multiple_of(4) {
                0
            } else {
                1 + ((r >> 8) % 16) as u8 * 16
            };
            self.cur = ToyInput {
                angle,
                buttons,
                keys: 0,
                reserved: 0,
            };
        }
        self.hold -= 1;
        self.cur
    }
}

/// How a match ended for a [`ToyMachine`].
#[derive(Clone, Debug)]
pub struct ToyResult {
    pub reports: Vec<DoneReport>,
    pub agreed: bool,
    pub desyncs: Vec<DesyncReport>,
}

/// One machine playing the toy game over a [`NetSession`]: starts the sim on
/// `MatchStarted`, feeds bot inputs for the local slots, fulfils requests,
/// and reports `finish_match` once frame `target` is confirmed.
pub struct ToyMachine {
    pub net: NetSession<Toy>,
    pub runner: Option<ToyRunner>,
    bots: BTreeMap<usize, Bot>,
    /// Frame at which the match ends (its checksum is compared).
    pub target: i32,
    pub result: Option<ToyResult>,
    pub aborted: Option<String>,
    pub salt: u64,
    /// Desync injection: corrupt this machine's state from this frame on.
    pub corrupt_at: Option<i32>,
    /// `Desync` events seen here.
    pub desyncs_seen: u64,
}

impl ToyMachine {
    pub fn new(net: NetSession<Toy>, target: i32, salt: u64) -> ToyMachine {
        ToyMachine {
            net,
            runner: None,
            bots: BTreeMap::new(),
            target,
            result: None,
            aborted: None,
            salt,
            corrupt_at: None,
            desyncs_seen: 0,
        }
    }

    /// Poll and handle events; returns them for logging. Call often.
    pub fn poll(&mut self) -> Vec<NetEvent> {
        self.net.poll();
        let mut out = Vec::new();
        while let Some(e) = self.net.next_event() {
            match &e {
                NetEvent::MatchStarted(ms) => {
                    let mut r = ToyRunner::from_match(ms);
                    r.state.corrupt_at = self.corrupt_at;
                    self.runner = Some(r);
                    self.bots = self
                        .net
                        .local_slots()
                        .iter()
                        .map(|&s| {
                            (
                                s,
                                Bot::new(self.salt.wrapping_mul(31).wrapping_add(s as u64 + 1)),
                            )
                        })
                        .collect();
                    self.result = None;
                    self.aborted = None;
                }
                NetEvent::MatchEnded {
                    reports,
                    agreed,
                    desyncs,
                    ..
                } => {
                    self.result = Some(ToyResult {
                        reports: reports.clone(),
                        agreed: *agreed,
                        desyncs: desyncs.clone(),
                    })
                }
                NetEvent::MatchAborted { reason } => self.aborted = Some(reason.clone()),
                NetEvent::Desync { .. } => self.desyncs_seen += 1,
                _ => {}
            }
            out.push(e);
        }
        out
    }

    /// One sim step (call at 60 Hz).
    pub fn step(&mut self) {
        let Some(r) = self.runner.as_mut() else {
            return;
        };
        if self.net.current_match().is_none() {
            return;
        }
        for (&slot, bot) in self.bots.iter_mut() {
            let _ = self.net.add_local_input(slot, bot.input());
        }
        let reqs = self.net.advance();
        r.handle(reqs);
        if !self.net.is_spectator()
            && self.net.confirmed_frame() >= self.target
            && let Some(&c) = r.history.get(&self.target)
        {
            let _ = self.net.finish_match(self.target, c);
        }
    }

    /// Our checksum for the target frame, if simulated.
    pub fn target_checksum(&self) -> Option<u128> {
        self.runner.as_ref()?.history.get(&self.target).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{GgrsParams, MachineId, MatchSettings, PlayerSlot, TeamMode};

    fn play_on(mut t: Toy, frames: i32, gone_from: Option<(usize, i32)>) -> Toy {
        let mut bots: Vec<Bot> = (0..t.dots.len() as u64).map(|i| Bot::new(i + 1)).collect();
        for f in 0..frames {
            let inputs: Vec<_> = bots
                .iter_mut()
                .enumerate()
                .map(|(i, b)| match gone_from {
                    Some((s, g)) if s == i && f >= g => {
                        (ToyInput::default(), InputStatus::Disconnected)
                    }
                    _ => (b.input(), InputStatus::Confirmed),
                })
                .collect();
            t.advance(&inputs);
        }
        t
    }

    fn play(frames: i32, gone_from: Option<(usize, i32)>) -> Toy {
        play_on(Toy::new(12, 42), frames, gone_from)
    }

    #[test]
    fn deterministic() {
        assert_eq!(play(2000, None).checksum(), play(2000, None).checksum());
        assert_ne!(Toy::new(12, 1).checksum(), Toy::new(12, 2).checksum());
        let t = play(2000, None);
        assert!(
            t.dots.iter().map(|d| d.score).sum::<u32>() > 0,
            "pellets hit something"
        );
    }

    #[test]
    fn directions() {
        let d = |a: u8| {
            ToyInput {
                angle: a,
                ..Default::default()
            }
            .dir()
        };
        assert_eq!(d(0), (0, 0));
        assert_eq!(d(1), (256, 0), "angle 0 = right");
        assert_eq!(d(65), (0, 256), "quarter turn = down (y down)");
        assert_eq!(d(129), (-256, 0));
        assert_eq!(d(255), (256, 0), "wraps");
    }

    #[test]
    fn disconnected_slot_inert_then_removed() {
        let t = play(ROUND_FRAMES - 10, Some((3, 100)));
        assert!(
            t.dots[3].gone && t.dots[3].alive,
            "inert until the round ends"
        );
        let t = play(ROUND_FRAMES + 1, Some((3, 100)));
        assert!(!t.dots[3].alive, "removed at the round end");
        assert_eq!(t.alive(), 11);
    }

    #[test]
    fn match_start_feeds_teams_abilities_and_settings() {
        let players: Vec<PlayerSlot> = (0..4u8)
            .map(|i| PlayerSlot {
                slot: i,
                machine: MachineId(i / 2),
                local_index: i % 2,
                name: format!("p{i}"),
                colour: i,
                team: i % 2,
                extra: vec![3, 3, i],
            })
            .collect();
        let ms = MatchStart {
            match_id: 1,
            seed: 9,
            settings: MatchSettings {
                team_mode: TeamMode::Teams(2),
                game: ToySettings { round_frames: 300 }.to_bytes(),
                ..Default::default()
            },
            players,
            ggrs: GgrsParams::default(),
            spectators: Vec::new(),
        };
        let t = Toy::from_match(&ms);
        assert_eq!(t.round_frames, 300);
        assert_eq!(t.dots[2].abilities, [3, 3, 2]);
        assert_eq!(t.dots[3].team, 1);
        let t = play_on(t, 300 + SELECT_FRAMES + 5, None);
        assert_eq!(t.round, 1);
        assert!(!t.selecting());
        // Some ability got cycled by the bots during the select phase.
        assert!(
            t.dots
                .iter()
                .any(|d| d.abilities[0] != 3 || d.abilities[1] != 3)
        );
        assert_eq!(ToySettings::from_bytes(&[]), ToySettings::default());
    }

    #[test]
    fn corruption_changes_checksum() {
        let mut a = Toy::new(2, 5);
        let mut b = a.clone();
        b.corrupt_at = Some(3);
        let z = [(ToyInput::default(), InputStatus::Confirmed); 2];
        for _ in 0..2 {
            a.advance(&z);
            b.advance(&z);
        }
        assert_eq!(a.checksum(), b.checksum());
        a.advance(&z);
        b.advance(&z);
        assert_ne!(a.checksum(), b.checksum());
    }
}

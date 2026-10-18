//! The original's `.rep` replay format (net-original.md §7): read and write.

use anyhow::{Result, bail};
use bopl_sim::input::{
    BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2, BTN_JUMP, KEY_A, KEY_D, KEY_S, KEY_W, PlayerInput,
};
use bopl_sim::player::PlayerSetup;
use bopl_sim::world::{MatchSettings, World};

/// One player of the header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayPlayer {
    /// 1-based id.
    pub id: i32,
    pub uses_keyboard_and_mouse: bool,
    pub color: u8,
    pub team: u8,
    /// Ability-list indices (3 bytes; 0 = none).
    pub abilities: [u8; 3],
}

/// A parsed replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    pub sequence: u16,
    pub seed: u32,
    pub number_of_players: u8,
    pub number_of_abilities: u8,
    /// Level bag index (build index − 6).
    pub level: u8,
    pub delay: u8,
    pub demo_mask: u8,
    /// Present players (ability-1 byte non-zero), in id order.
    pub players: Vec<ReplayPlayer>,
    /// Per tick, the inputs of players 1..4 (absent players are neutral).
    pub frames: Vec<[PlayerInput; 4]>,
}

const BITS: [(u8, bool); 8] = [
    (BTN_JUMP, false),
    (BTN_ABILITY0, false),
    (BTN_ABILITY1, false),
    (BTN_ABILITY2, false),
    (KEY_W, true),
    (KEY_A, true),
    (KEY_S, true),
    (KEY_D, true),
];

fn get(p: &PlayerInput, b: usize) -> bool {
    let (m, key) = BITS[b];
    if key {
        p.keys & m != 0
    } else {
        p.buttons & m != 0
    }
}

fn set(p: &mut PlayerInput, b: usize, v: bool) {
    let (m, key) = BITS[b];
    let f = if key { &mut p.keys } else { &mut p.buttons };
    if v {
        *f |= m;
    } else {
        *f &= !m;
    }
}

impl Replay {
    /// Build index of the level (bag index + 6).
    pub fn build_index(&self) -> usize {
        self.level as usize + 6
    }

    /// Match settings as the replay viewer uses them (seed, ability count).
    pub fn settings(&self) -> MatchSettings {
        MatchSettings {
            number_of_abilities: self.number_of_abilities.max(1) as usize,
            seed: self.seed,
        }
    }

    /// Player setups in list order (present players, id order).
    pub fn setups(&self) -> Vec<PlayerSetup> {
        let n = self.settings().number_of_abilities;
        self.players
            .iter()
            .map(|p| PlayerSetup {
                id: p.id,
                team: p.team as i32,
                color: p.color as i32,
                abilities: p.abilities.iter().take(n).map(|a| *a as usize).collect(),
                uses_keyboard_and_mouse: p.uses_keyboard_and_mouse,
            })
            .collect()
    }

    /// Inputs of tick `t` in list order (neutral after the end of the file).
    pub fn inputs(&self, t: usize) -> Vec<PlayerInput> {
        match self.frames.get(t) {
            Some(f) => self
                .players
                .iter()
                .map(|p| f[(p.id - 1) as usize])
                .collect(),
            None => vec![PlayerInput::default(); self.players.len()],
        }
    }

    /// The inputs the original's replay viewer applies on the tick that reads
    /// frame `t` (net-original.md §8): while time is stopped (checked before
    /// the tick) only players protected from the time stop get the recorded
    /// frame; the others keep their previous input. `last` carries the
    /// applied inputs from tick to tick (start with an empty vector).
    pub fn viewer_inputs(
        &self,
        t: usize,
        w: &World,
        last: &mut Vec<PlayerInput>,
    ) -> Vec<PlayerInput> {
        let mut inp = self.inputs(t);
        if last.len() != inp.len() {
            *last = vec![PlayerInput::default(); inp.len()];
        }
        if w.is_time_stopped() {
            for (i, p) in w.players.iter().enumerate().take(inp.len()) {
                if !p.is_protected_from_time_stop {
                    inp[i] = last[i];
                }
            }
        }
        last.clone_from(&inp);
        inp
    }

    /// Parses a `.rep` file.
    pub fn parse(d: &[u8]) -> Result<Replay> {
        if d.len() < 105 {
            bail!("replay shorter than its header");
        }
        let be16 = |o: usize| u16::from_be_bytes([d[o], d[o + 1]]);
        let be32 = |o: usize| u32::from_be_bytes(d[o..o + 4].try_into().unwrap());
        let be64 = |o: usize| u64::from_be_bytes(d[o..o + 8].try_into().unwrap());
        let mut players = Vec::new();
        for i in 0..4 {
            let ab = [d[51 + 3 * i], d[52 + 3 * i], d[53 + 3 * i]];
            if ab[0] != 0 {
                players.push(ReplayPlayer {
                    id: i as i32 + 1,
                    uses_keyboard_and_mouse: be64(11 + 8 * i) == 1,
                    color: d[43 + i],
                    team: d[47 + i],
                    abilities: ab,
                });
            }
        }
        let mut r = Replay {
            sequence: be16(0),
            seed: be32(2),
            number_of_players: d[6],
            number_of_abilities: d[7],
            level: d[8],
            delay: d[9],
            demo_mask: d[10],
            players,
            frames: Vec::new(),
        };
        let mut pos = 105;
        let mut prev = [PlayerInput::default(); 4];
        let u32le = |pos: &mut usize| -> Result<u32> {
            if *pos + 4 > d.len() {
                bail!("truncated block");
            }
            let v = u32::from_le_bytes(d[*pos..*pos + 4].try_into().unwrap());
            *pos += 4;
            Ok(v)
        };
        while pos < d.len() {
            let mut block = [[PlayerInput::default(); 4]; 32];
            for pl in 0..4 {
                let changed = d[pos];
                pos += 1;
                let mut masks = [None; 8];
                for (b, m) in masks.iter_mut().enumerate() {
                    if changed >> b & 1 != 0 {
                        *m = Some(u32le(&mut pos)?);
                    }
                }
                let angle_changes = u32le(&mut pos)?;
                let mut angle = prev[pl].angle;
                for (t, fr) in block.iter_mut().enumerate() {
                    let mut inp = PlayerInput::default();
                    for (b, m) in masks.iter().enumerate() {
                        let v = match m {
                            Some(m) => m >> t & 1 != 0,
                            None => get(&prev[pl], b),
                        };
                        set(&mut inp, b, v);
                    }
                    if angle_changes >> t & 1 != 0 {
                        if pos >= d.len() {
                            bail!("truncated angles");
                        }
                        angle = d[pos];
                        pos += 1;
                    }
                    inp.angle = angle;
                    fr[pl] = inp;
                }
                prev[pl] = block[31][pl];
            }
            r.frames.extend_from_slice(&block);
        }
        Ok(r)
    }

    /// Serialises (header + blocks, padded with neutral frames like the
    /// original writer: always 1..32 extra frames).
    pub fn write(&self) -> Vec<u8> {
        let mut d = vec![0u8; 105];
        d[0..2].copy_from_slice(&self.sequence.to_be_bytes());
        d[2..6].copy_from_slice(&self.seed.to_be_bytes());
        d[6] = self.number_of_players;
        d[7] = self.number_of_abilities;
        d[8] = self.level;
        d[9] = self.delay;
        d[10] = self.demo_mask;
        for p in &self.players {
            let i = (p.id - 1) as usize;
            d[11 + 8 * i..19 + 8 * i]
                .copy_from_slice(&(p.uses_keyboard_and_mouse as u64).to_be_bytes());
            d[43 + i] = p.color;
            d[47 + i] = p.team;
            d[51 + 3 * i..54 + 3 * i].copy_from_slice(&p.abilities);
        }
        let mut frames = self.frames.clone();
        let pad = 32 - frames.len() % 32;
        frames.extend(std::iter::repeat_n([PlayerInput::default(); 4], pad));
        let mut prev = [PlayerInput::default(); 4];
        for block in frames.chunks(32) {
            for pl in 0..4 {
                let mut changed = 0u8;
                let mut masks = [0u32; 8];
                for (b, m) in masks.iter_mut().enumerate() {
                    for (t, fr) in block.iter().enumerate() {
                        if get(&fr[pl], b) {
                            *m |= 1 << t;
                        }
                        if get(&fr[pl], b) != get(&prev[pl], b) {
                            changed |= 1 << b;
                        }
                    }
                }
                d.push(changed);
                for (b, m) in masks.iter().enumerate() {
                    if changed >> b & 1 != 0 {
                        d.extend_from_slice(&m.to_le_bytes());
                    }
                }
                let mut ac = 0u32;
                let mut angles = Vec::new();
                let mut a = prev[pl].angle;
                for (t, fr) in block.iter().enumerate() {
                    if fr[pl].angle != a {
                        ac |= 1 << t;
                        a = fr[pl].angle;
                        angles.push(a);
                    }
                }
                d.extend_from_slice(&ac.to_le_bytes());
                d.extend_from_slice(&angles);
                prev[pl] = block[31][pl];
            }
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut frames = Vec::new();
        for t in 0..70u32 {
            let mut f = [PlayerInput::default(); 4];
            f[0].angle = (t % 7) as u8;
            if t % 5 == 0 {
                f[0].buttons |= BTN_JUMP;
            }
            f[1].keys = (t % 16) as u8;
            frames.push(f);
        }
        let r = Replay {
            sequence: 0,
            seed: 1234,
            number_of_players: 2,
            number_of_abilities: 3,
            level: 0,
            delay: 0,
            demo_mask: 255,
            players: vec![
                ReplayPlayer {
                    id: 1,
                    uses_keyboard_and_mouse: false,
                    color: 1,
                    team: 0,
                    abilities: [4, 7, 2],
                },
                ReplayPlayer {
                    id: 2,
                    uses_keyboard_and_mouse: true,
                    color: 2,
                    team: 1,
                    abilities: [3, 2, 5],
                },
            ],
            frames: frames.clone(),
        };
        let back = Replay::parse(&r.write()).unwrap();
        assert_eq!(back.players, r.players);
        assert_eq!(back.seed, 1234);
        assert_eq!(back.frames.len(), 96);
        assert_eq!(&back.frames[..70], &frames[..]);
    }

    #[test]
    fn real_replay_parses() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scratch/ref/replays/1.rep");
        let Ok(d) = std::fs::read(&p) else {
            return;
        };
        let r = Replay::parse(&d).unwrap();
        assert!(!r.players.is_empty());
        assert_eq!(r.frames.len() % 32, 0);
    }
}

//! The simulation as bopl-net sees it, and the formats bopl-port puts into
//! bopl-net's opaque fields (docs/net.md, "The game side"):
//!
//! * [`NetWorld`]: `NetGame` for a session of the sim ([`SimMatch`]: the
//!   `World` plus the level loads its flow requests). One input per global
//!   slot: [`PlayerInput`] (3 bytes: angle, buttons, WASD), exactly what
//!   the sim reads, so the wheel and the winner-screen prompts (sim flow)
//!   are inputs too. A slot whose machine left gets `Disconnected` zero
//!   inputs: the sim's leave rule (`flow::set_gone`) runs from that frame.
//! * [`OnlineSettings`]: the settings blob (`MatchSettings::game`).
//! * [`SlotExtra`]: a slot's `extra` (abilities, keyboard + mouse).

use std::rc::Rc;

use anyhow::Result;
use bopl_net::{InputStatus, MatchStart, NetGame};
use bopl_sim::PlayerInput;
use bopl_sim::flow;
use bopl_sim::player::PlayerSetup;
use bopl_sim::world::MatchSettings;

use crate::sim_match::{LevelCache, SimMatch};

/// Version byte of the settings blob.
pub const SETTINGS_VERSION: u8 = 1;
/// No first level given: Level1, like every session.
const NO_LEVEL: u16 = u16::MAX;

/// The host's match settings (`MatchSettings::game`):
/// `[version, number_of_abilities, first_level lo, first_level hi]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnlineSettings {
    /// Ability slots per player (1..3, the host's setting).
    pub number_of_abilities: u8,
    /// First level as a build index (tests); `None` = Level1.
    pub first_level: Option<u16>,
}

impl Default for OnlineSettings {
    fn default() -> Self {
        Self {
            number_of_abilities: 3,
            first_level: None,
        }
    }
}

impl OnlineSettings {
    pub fn encode(&self) -> Vec<u8> {
        let l = self.first_level.unwrap_or(NO_LEVEL);
        vec![
            SETTINGS_VERSION,
            self.number_of_abilities.clamp(1, 3),
            l as u8,
            (l >> 8) as u8,
        ]
    }

    /// `None` for a blob of another version.
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.is_empty() {
            return Some(Self::default());
        }
        if b[0] != SETTINGS_VERSION || b.len() < 4 {
            return None;
        }
        let l = u16::from_le_bytes([b[2], b[3]]);
        Some(Self {
            number_of_abilities: b[1].clamp(1, 3),
            first_level: (l != NO_LEVEL).then_some(l),
        })
    }
}

/// A slot's per-player choices (`SlotRequest::extra`):
/// `[ability slot 0, slot 1, slot 2, flags]` (ability-list indices; flags
/// bit 0 = keyboard + mouse player, whose WASD bits the sim reads).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotExtra {
    pub abilities: [usize; 3],
    pub mouse: bool,
}

impl Default for SlotExtra {
    fn default() -> Self {
        // Bow, Gust, Dash (the original's defaults).
        Self {
            abilities: [4, 7, 2],
            mouse: false,
        }
    }
}

impl SlotExtra {
    pub fn encode(&self) -> Vec<u8> {
        let a = |k: usize| self.abilities[k].min(255) as u8;
        vec![a(0), a(1), a(2), self.mouse as u8]
    }

    /// Unknown or out-of-range abilities become Random (1).
    pub fn decode(b: &[u8], ability_count: usize) -> Self {
        let mut x = Self::default();
        for k in 0..3 {
            if let Some(&v) = b.get(k) {
                let v = v as usize;
                x.abilities[k] = if (1..ability_count.max(2)).contains(&v) {
                    v
                } else {
                    1
                };
            }
        }
        x.mouse = b.get(3).is_some_and(|f| f & 1 != 0);
        x
    }
}

/// The sim session played online.
#[derive(Clone)]
pub struct NetWorld {
    pub sim: SimMatch,
}

impl NetWorld {
    /// The sim's input list from bopl-net's: disconnected slots get neutral
    /// input and are marked gone (before the step, on every machine at the
    /// same frame).
    fn inputs(&mut self, inputs: &[(PlayerInput, InputStatus)]) -> Vec<PlayerInput> {
        let mut out = Vec::with_capacity(inputs.len());
        for (i, (inp, st)) in inputs.iter().enumerate() {
            if *st == InputStatus::Disconnected {
                if !flow::is_gone(&self.sim.world, i) && i < self.sim.world.players.len() {
                    log::info!(
                        "tick {}: player {} left (inert this round, removed after)",
                        self.sim.world.time.simulation_ticks,
                        i + 1
                    );
                    flow::set_gone(&mut self.sim.world, i);
                }
                out.push(PlayerInput::default());
            } else {
                out.push(*inp);
            }
        }
        out
    }

    /// The session ended (back to character select).
    pub fn finished(&self) -> bool {
        self.sim.ended
    }
}

impl NetGame for NetWorld {
    type Input = PlayerInput;

    fn advance(&mut self, inputs: &[(PlayerInput, InputStatus)]) {
        let ins = self.inputs(inputs);
        if let Err(e) = self.sim.advance(&ins) {
            // Level data missing: the same on every machine with the same
            // import, so it stays deterministic; the session ends.
            log::error!("online sim step: {e:#}");
            self.sim.ended = true;
        }
    }

    fn checksum(&self) -> u128 {
        let w = &self.sim.world;
        ((w.state_hash() as u128) << 64)
            | ((w.checksum().0 as u64 as u128) ^ ((self.sim.level as u128) << 48))
    }
}

/// The sim's player setups for a match: slot order = player ids 1..N,
/// colour and team from the host's slot table, abilities and input kind
/// from each slot's `extra`.
pub fn setups(ms: &MatchStart, n_abilities: usize, ability_count: usize) -> Vec<PlayerSetup> {
    ms.players
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let x = SlotExtra::decode(&p.extra, ability_count);
            PlayerSetup {
                id: i as i32 + 1,
                team: p.team as i32,
                color: (p.colour % 12) as i32,
                abilities: x.abilities.iter().copied().take(n_abilities).collect(),
                uses_keyboard_and_mouse: x.mouse,
            }
        })
        .collect()
}

/// The 32-bit session seed from the host's 64-bit match seed.
pub fn session_seed(seed: u64) -> u32 {
    let s = (seed ^ (seed >> 32)) as u32;
    if s == 0 { 1 } else { s }
}

/// The starting state of a match on every machine.
pub fn new_world(ms: &MatchStart, levels: Rc<LevelCache>) -> Result<NetWorld> {
    let os = OnlineSettings::decode(&ms.settings.game)
        .ok_or_else(|| anyhow::anyhow!("the host's settings blob has another version"))?;
    let n_ab = os.number_of_abilities as usize;
    let setups = setups(ms, n_ab, levels.content.abilities.len());
    let settings = MatchSettings {
        number_of_abilities: n_ab,
        seed: session_seed(ms.seed),
    };
    let first = os.first_level.map(|l| l as usize);
    let sim = SimMatch::new(levels, &setups, settings, first)?;
    Ok(NetWorld { sim })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blobs_round_trip() {
        let s = OnlineSettings {
            number_of_abilities: 2,
            first_level: Some(27),
        };
        assert_eq!(OnlineSettings::decode(&s.encode()), Some(s));
        assert_eq!(
            OnlineSettings::decode(&OnlineSettings::default().encode()),
            Some(OnlineSettings::default())
        );
        assert_eq!(OnlineSettings::decode(&[9, 1, 0, 0]), None);
        let x = SlotExtra {
            abilities: [3, 12, 1],
            mouse: true,
        };
        assert_eq!(SlotExtra::decode(&x.encode(), 31), x);
        // Out of range / null ability: Random.
        assert_eq!(SlotExtra::decode(&[0, 40, 5, 0], 31).abilities, [1, 1, 5]);
        assert_eq!(session_seed(0), 1);
    }
}

//! One online match on this machine, without graphics or audio: carries
//! out bopl-net's requests on [`NetWorld`], feeds the [`EventGate`], and
//! reports the end of the session (back to character select) once it is on
//! the confirmed timeline. Used by the online screen and the headless test
//! harness.

use std::collections::BTreeMap;

use bopl_net::{NetGame, NetSession, Request};
use bopl_sim::PlayerInput;
use bopl_sim::view::Event;

use super::adapter::NetWorld;
use super::gate::EventGate;

pub struct OnlineMatch {
    pub game: NetWorld,
    pub gate: EventGate,
    /// Sim frame of `game` (the state after `frame` steps).
    pub frame: i32,
    /// State frames whose state is "session over", with their checksum
    /// (re-simulation corrects them).
    finished: BTreeMap<i32, u128>,
    /// The confirmed end: (state frame, checksum), reported to bopl-net.
    pub done: Option<(i32, u128)>,
    /// Frames simulated for the first time / again after rollbacks.
    pub advanced: u64,
    pub resimulated: u64,
    /// Newest frame whose inputs are final and simulated.
    pub confirmed: i32,
    /// Checksums of the states at multiples of [`CHECKPOINT`] frames
    /// (re-simulation overwrites; final once confirmed).
    pub checkpoints: BTreeMap<i32, u128>,
    /// Tests: the state of this frame counts as the end if the session has
    /// not ended before (the same frame on every machine).
    pub stop_at: Option<i32>,
}

/// Checkpoint interval (5 s).
pub const CHECKPOINT: i32 = 300;

impl OnlineMatch {
    pub fn new(game: NetWorld) -> Self {
        Self {
            game,
            gate: EventGate::new(),
            frame: 0,
            finished: BTreeMap::new(),
            done: None,
            advanced: 0,
            resimulated: 0,
            confirmed: -1,
            checkpoints: BTreeMap::new(),
            stop_at: None,
        }
    }

    fn is_over(&self) -> bool {
        self.game.finished() || self.stop_at.is_some_and(|s| self.frame >= s)
    }

    /// The end was the frame cap, not the session.
    pub fn capped(&self) -> bool {
        self.done.is_some() && !self.game.finished()
    }

    /// One 60 Hz step: local inputs in, requests carried out. Returns the
    /// cues to present (first seen across rollbacks).
    pub fn step(
        &mut self,
        net: &mut NetSession<NetWorld>,
        local: &[(usize, PlayerInput)],
    ) -> Vec<Event> {
        for &(slot, i) in local {
            if let Err(e) = net.add_local_input(slot, i) {
                log::debug!("local input for slot {slot}: {e}");
            }
        }
        let spectating = net.is_spectator();
        let reqs = net.advance();
        self.apply(reqs);
        // Inputs received while stalled at the prediction limit are
        // confirmed but not simulated yet: the final timeline ends at the
        // last simulated frame.
        let confirmed = if spectating {
            self.frame - 1
        } else {
            net.confirmed_frame().min(self.frame - 1)
        };
        self.confirmed = confirmed;
        if self.done.is_none()
            && let Some((&f, &c)) = self.finished.iter().next()
            && f - 1 <= confirmed
        {
            // State `f` is the first finished one and its inputs are final:
            // every machine reports the same frame and checksum.
            self.done = Some((f, c));
            if let Err(e) = net.finish_match(f, c) {
                log::warn!("finish_match: {e}");
            }
        }
        self.gate.take(confirmed as i64)
    }

    fn apply(&mut self, reqs: Vec<Request<NetWorld>>) {
        for r in reqs {
            match r {
                Request::Save { cell, frame } => {
                    cell.save(frame, Some(self.game.clone()), Some(self.game.checksum()));
                }
                Request::Load { cell, frame } => {
                    self.game = cell.load().expect("bopl-net hands out saved cells");
                    self.frame = frame;
                    self.finished.retain(|f, _| *f <= frame);
                }
                Request::Advance {
                    frame,
                    inputs,
                    resim,
                } => {
                    self.game.advance(&inputs);
                    self.frame = frame + 1;
                    if resim {
                        self.resimulated += 1;
                    } else {
                        self.advanced += 1;
                    }
                    self.gate.on_advance(&self.game.sim.world.events);
                    if (frame + 1) % CHECKPOINT == 0 {
                        self.checkpoints.insert(frame + 1, self.game.checksum());
                    }
                    if self.is_over() {
                        self.finished
                            .entry(frame + 1)
                            .or_insert_with(|| self.game.checksum());
                    }
                }
            }
        }
    }

    /// Scores, level and players for logs.
    pub fn summary(&self) -> String {
        let w = &self.game.sim.world;
        let scores: Vec<String> = w
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| {
                format!(
                    "{}{}",
                    p.games_won,
                    if bopl_sim::flow::is_gone(w, i) {
                        "x"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        format!(
            "level {} (#{}) scores [{}]{}",
            self.game.sim.level,
            w.flow.levels_loaded,
            scores.join(","),
            if self.game.finished() {
                " session over"
            } else {
                ""
            }
        )
    }
}

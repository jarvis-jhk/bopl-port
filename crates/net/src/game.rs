//! The simulation as bopl-net sees it: [`NetGame`].
//!
//! bopl-net does not know bopl-sim. Anything that is `Clone` (that is how
//! rollback saves a frame), can be stepped with one input per player slot and
//! can checksum itself can be played online. The game either handles the
//! [`Request`]s of [`crate::NetSession::advance`] itself (to interleave its own
//! bookkeeping) or lets [`fulfil`] / [`crate::NetSession::step`] do it.

use crate::input::NetInput;
use ggrs::{GameStateCell, InputStatus};

/// A deterministic, rollback-able simulation.
///
/// Requirements (the same on every machine, bit for bit): `advance` is a pure
/// function of the state and the inputs (no wall clock, no I/O, no thread
/// timing, no `HashMap` iteration order, no floats (bopl-sim: `Fix`),
/// randomness only from a PRNG inside the state); `clone` captures
/// everything `advance` reads; `checksum` covers everything that matters
/// for the outcome (it feeds desync detection, so leave out presentation-only
/// fields like particle timers if they are not deterministic).
pub trait NetGame: Clone + 'static {
    /// One player's input for one frame (bopl-sim's per-player input;
    /// any `bytemuck::Pod`).
    type Input: NetInput;

    /// Simulate one frame. `inputs[slot]` is the input of global player slot
    /// `slot` (see [`crate::MatchStart::players`]). Statuses: `Confirmed`,
    /// `Predicted` (may be corrected by a rollback later) or `Disconnected`
    /// (the slot's machine left the match: the input is always zero from the
    /// agreed frame on; see docs/net.md for what the sim should do).
    fn advance(&mut self, inputs: &[(Self::Input, InputStatus)]);

    /// A hash of the state (compared between machines).
    fn checksum(&self) -> u128;
}

/// A saved-state slot handed to the game. Frames are sim frames (they keep
/// counting across the internal GGRS sessions of one match).
pub struct StateCell<G> {
    pub(crate) cell: GameStateCell<G>,
    pub(crate) base: i32,
}

impl<G> Clone for StateCell<G> {
    fn clone(&self) -> Self {
        StateCell {
            cell: self.cell.clone(),
            base: self.base,
        }
    }
}

impl<G> StateCell<G> {
    /// Store the state of sim frame `frame` (the frame of the request).
    pub fn save(&self, frame: i32, state: Option<G>, checksum: Option<u128>) {
        self.cell.save(frame - self.base, state, checksum);
    }
}

impl<G: Clone> StateCell<G> {
    pub fn load(&self) -> Option<G> {
        self.cell.load()
    }
}

/// What the game must do, in order, after [`crate::NetSession::advance`].
pub enum Request<G: NetGame> {
    /// Save the current state (= the state of sim frame `frame`):
    /// `cell.save(frame, Some(state.clone()), Some(state.checksum()))`.
    Save { cell: StateCell<G>, frame: i32 },
    /// Restore the state saved for `frame`: `state = cell.load().unwrap()`.
    Load { cell: StateCell<G>, frame: i32 },
    /// Step the sim once, from the state of `frame` to `frame + 1`, with one
    /// input per slot (index = global slot). `resim`: this frame was
    /// simulated before and is simulated again after a rollback (the
    /// presentation layer should not replay its sounds/particles).
    Advance {
        frame: i32,
        inputs: Vec<(G::Input, InputStatus)>,
        resim: bool,
    },
}

/// What [`fulfil`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stepped {
    /// Frames simulated for the first time.
    pub advanced: u32,
    /// Frames simulated again after a rollback.
    pub resimulated: u32,
    /// The state was rolled back to this frame (first `Load`).
    pub rollback_to: Option<i32>,
}

/// Carry out `reqs` on `game` in order.
pub fn fulfil<G: NetGame>(game: &mut G, reqs: Vec<Request<G>>) -> Stepped {
    let mut out = Stepped::default();
    for r in reqs {
        match r {
            Request::Save { cell, frame } => {
                cell.save(frame, Some(game.clone()), Some(game.checksum()));
            }
            Request::Load { cell, frame } => {
                *game = cell.load().expect("GGRS hands out only saved cells");
                out.rollback_to.get_or_insert(frame);
            }
            Request::Advance { inputs, resim, .. } => {
                game.advance(&inputs);
                if resim {
                    out.resimulated += 1;
                } else {
                    out.advanced += 1;
                }
            }
        }
    }
    out
}

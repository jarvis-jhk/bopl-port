//! Session flow around the rounds (match-flow.md §5, §11): the level bag,
//! the ability-select circles, the mid-game ability wheel and the
//! winner-screen inputs ("next level" / "back to character select").
//!
//! In the original these are UI scripts on real time; bopl-port runs them
//! in the simulation from the players' raw inputs (match-flow.md §15), so
//! they are identical on every peer and survive rollback. Delays are counted
//! in ticks. The state lives in `World::flow` and survives level loads.
//!
//! Loading levels needs data the simulation does not own, so the flow only
//! *requests* it ([`FlowRequest`]); the host takes the request right after
//! the step and calls [`load_level`] (or leaves the session).

use std::sync::Arc;

use crate::data::LevelData;
use crate::fix::Fix;
use crate::input::{
    BTN_ABILITY1, BTN_JUMP, BTN_SELECT, BTN_START, KEY_A, KEY_D, KEY_S, KEY_W, PlayerInput,
    input_vector_table,
};
use crate::rng::Rng;
use crate::view::Cue;
use crate::world::World;

/// A loser can open its wheel this long after its circle appeared (2.5 s).
pub const WHEEL_DELAY_TICKS: i64 = 150;
/// Winner-screen inputs are taken this long after it appeared (2.0 s).
pub const WINNER_INPUT_DELAY_TICKS: i64 = 120;
/// After closing, the wheel still counts as open until its close animation
/// ended (0.301 s) plus the one-frame delay of the original's toggle.
pub const WHEEL_CLOSE_TICKS: u8 = 19;
/// Holding left/right on a gamepad repeats every `100 / 375` s.
pub const WHEEL_REPEAT_TICKS: u8 = 16;

/// `y · 2 > 0.4`: the wheel opens when the stick points up beyond 0.2.
const OPEN_Y: Fix = Fix(858_993_459);
/// `0.4 + 0.4`: wheel navigation threshold on a stick.
const NAV: Fix = Fix(3_435_973_837);
/// The stick left the open zone (`y < 0.4`, original `DEADZONE`).
const RELEASE_Y: Fix = Fix(1_717_986_918);

/// What the host must do after this step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FlowRequest {
    /// Load the level with this bag index (build index − 6) via [`load_level`].
    NextLevel(usize),
    /// Leave the session for character select (players keep their choices,
    /// including mid-game ability changes: `Player::abilities`).
    BackToSelect,
}

/// A player's ability-select circle (match-flow.md §11.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Circle {
    /// Slid down (not shown).
    #[default]
    Hidden,
    /// The player's team was eliminated: portrait by cause of death, wheel.
    Loser {
        /// Tick the circle appeared.
        since: i64,
        /// Position in the row (0 = leftmost).
        slot: usize,
    },
    /// The player won the round.
    Winner { since: i64, slot: usize },
}

impl Circle {
    pub fn slot(&self) -> Option<usize> {
        match *self {
            Circle::Hidden => None,
            Circle::Loser { slot, .. } | Circle::Winner { slot, .. } => Some(slot),
        }
    }
}

/// The mid-game ability wheel of a loser (match-flow.md §11.3–11.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Wheel {
    pub open: bool,
    /// Ticks left until a closed wheel stops blocking "next level".
    pub closing: u8,
    /// Selected row (0 = top; row → slot: 0 → 0, with 3 slots 1 → 2 and 2 → 1).
    pub row: u8,
    /// Tick of the last open/close (presentation animation).
    pub toggled_at: i64,
    /// Tick of the last ability change and its direction (+1 left, −1 right).
    pub changed_at: i64,
    pub change_dir: i8,
    /// "Clicked up" latch: after opening, the stick must drop below 0.4.
    up_latch: bool,
    was_up: bool,
    was_down: bool,
    was_left: bool,
    was_right: bool,
    repeat: u8,
}

impl Wheel {
    /// Counts as "ability select opened" (blocks the next level).
    pub fn blocking(&self) -> bool {
        self.open || self.closing > 0
    }
}

/// Per-player flow state (list order).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FlowPlayer {
    pub circle: Circle,
    pub wheel: Wheel,
    /// Tick this player's circle was last wiggled (refused next level).
    pub wiggle_at: Option<i64>,
}

/// Session flow state (`World::flow`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Flow {
    /// The session seed (every level's seed is derived from it).
    pub session_seed: u32,
    /// Levels loaded this session (1 = the first level).
    pub levels_loaded: u32,
    /// Played flags per bag index; empty = no bag (single-level runs).
    pub played: Vec<bool>,
    /// Bag index of the current level.
    pub current: usize,
    /// Next level, drawn when the round was decided.
    pub next: Option<usize>,
    /// Tick the winner screen appeared.
    pub winner_screen_at: Option<i64>,
    pub players: Vec<FlowPlayer>,
    /// Set by the step; taken by the host.
    pub request: Option<FlowRequest>,
    /// The bag's draws (separate from the level RNG, which is re-seeded at
    /// every level load).
    pub rng: Rng,
    prev: Vec<PlayerInput>,
    /// Players whose machine left an online session (list order; empty =
    /// nobody). See [`set_gone`].
    pub gone: Vec<bool>,
}

/// Player `index` left (online: its machine dropped out; docs/net.md, "the
/// rule"). From now on its input is neutral (the host feeds zeroes), and:
///
/// 1. for the rest of the round its bopl is inert: it stays where it is and
///    can be killed, but it does not count for the round end (a round whose
///    remaining connected players are all on one team ends as usual) and it
///    cannot win;
/// 2. from the next level on it does not spawn (its score stays);
/// 3. if fewer than two teams with connected players remain, the session
///    goes back to character select ([`FlowRequest::BackToSelect`] on the
///    next step).
///
/// Its ability wheel closes. Idempotent; call it before the step in which
/// the player's input is first missing, on every machine alike.
pub fn set_gone(w: &mut World, index: usize) {
    let n = w.players.len();
    if index >= n {
        return;
    }
    if w.flow.gone.len() != n {
        w.flow.gone.resize(n, false);
    }
    if w.flow.gone[index] {
        return;
    }
    w.flow.gone[index] = true;
    if let Some(fp) = w.flow.players.get_mut(index) {
        fp.wheel.open = false;
        fp.wheel.closing = 0;
    }
}

/// Player `index` left the session ([`set_gone`]).
pub fn is_gone(w: &World, index: usize) -> bool {
    w.flow.gone.get(index).copied().unwrap_or(false)
}

/// Distinct teams that still have a connected player.
pub fn connected_teams(w: &World) -> usize {
    let mut teams: Vec<i32> = Vec::new();
    for (i, p) in w.players.iter().enumerate() {
        if !is_gone(w, i) && !teams.contains(&p.team) {
            teams.push(p.team);
        }
    }
    teams.len()
}

/// Starts a session: `level_count` levels in the bag (124 in the original),
/// the first level is bag index 0 (Level1). Call before the first
/// [`load_level`].
pub fn begin_session(w: &mut World, level_count: usize) {
    let seed = w.settings.seed;
    w.flow = Flow {
        session_seed: seed,
        played: vec![false; level_count],
        rng: Rng::new(seed ^ 0x5bd1_e995),
        ..Flow::default()
    };
}

/// Per-level seed: the session seed for the first level, then a mix of it
/// with the level count (the original used the wall clock locally).
pub fn level_seed(session: u32, n: u32) -> u32 {
    if n <= 1 {
        return session;
    }
    let mut x = (session as u64) << 32 | n as u64;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^= x >> 33;
    (x as u32) | 1
}

/// Loads level `bag` (its data `lv`) as the next level of the session:
/// derives its seed, then `session::load_level` (which also resets the
/// circles and wheels).
pub fn load_level(w: &mut World, bag: usize, lv: Arc<LevelData>) {
    let f = &mut w.flow;
    f.levels_loaded += 1;
    f.current = bag;
    if f.session_seed == 0 && f.levels_loaded == 1 {
        f.session_seed = w.settings.seed;
    }
    w.settings.seed = level_seed(f.session_seed, f.levels_loaded);
    crate::session::load_level(w, lv);
}

/// The level bag draw (match-flow.md §5), from the flow RNG.
fn draw_next(f: &mut Flow) -> usize {
    if f.played.is_empty() {
        return f.current;
    }
    if let Some(p) = f.played.get_mut(f.current) {
        *p = true;
    }
    let u = f.played.iter().filter(|p| !**p).count();
    if u == 0 {
        // QUIRK: `u` stays 0, so the draw below picks the first level.
        f.played.fill(false);
    }
    let r = if u == 0 {
        0
    } else {
        f.rng.random_int(0, u as i32) as usize
    };
    f.played
        .iter()
        .enumerate()
        .filter(|(_, p)| !**p)
        .nth(r)
        .map_or(0, |(i, _)| i)
}

/// Buttons and directions of one player this tick (presses are edges).
#[derive(Clone, Copy, Debug, Default)]
struct Signals {
    /// Wheel open (keyboard: W/Up pressed; gamepad: stick up beyond 0.2, held).
    open: bool,
    /// Gamepad stick below 0.4 (releases the open latch).
    released: bool,
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    close: bool,
    next: bool,
    back: bool,
}

fn signals(inp: PlayerInput, prev: PlayerInput, mouse: bool) -> Signals {
    let pressed = |b: u8| inp.buttons & b != 0 && prev.buttons & b == 0;
    let key = |k: u8| inp.keys & k != 0 && prev.keys & k == 0;
    let mut s = Signals {
        next: pressed(BTN_START) || pressed(BTN_JUMP),
        back: pressed(BTN_SELECT) || (!mouse && pressed(BTN_ABILITY1)),
        ..Signals::default()
    };
    if mouse {
        s.open = key(KEY_W);
        s.up = key(KEY_W);
        s.down = key(KEY_S);
        s.left = key(KEY_A);
        s.right = key(KEY_D);
        s.close = pressed(BTN_START) || pressed(BTN_JUMP) || pressed(BTN_SELECT);
        s.released = true;
    } else {
        let v = if inp.angle == 0 {
            crate::vec2::Vec2::ZERO
        } else {
            input_vector_table()[inp.angle as usize]
        };
        s.open = v.y > OPEN_Y;
        s.released = v.y < RELEASE_Y;
        s.up = v.y > NAV;
        s.down = v.y < -NAV;
        s.right = v.x > NAV;
        s.left = v.x < -NAV;
        s.close =
            pressed(BTN_START) || pressed(BTN_JUMP) || pressed(BTN_ABILITY1) || pressed(BTN_SELECT);
    }
    s
}

/// Wheel rows shown (`NumberOfAbilities`, at most the chosen slots).
pub fn wheel_rows(w: &World, player: usize) -> usize {
    w.settings
        .number_of_abilities
        .min(w.players[player].abilities.len())
}

/// Ability slot shown in wheel `row` (the original's row/slot swap).
pub fn row_slot(rows: usize, row: usize) -> usize {
    if rows == 3 {
        [0, 2, 1][row.min(2)]
    } else {
        row
    }
}

/// Runs after every tick (`World::step`): circles, wheels, winner screen.
#[allow(clippy::needless_range_loop)] // parallel per-player vectors
pub fn update(w: &mut World, inputs: &[PlayerInput]) {
    let n = w.players.len();
    if w.flow.players.len() != n {
        w.flow.players.resize(n, FlowPlayer::default());
    }
    if w.flow.prev.len() != n {
        w.flow.prev.resize(n, PlayerInput::default());
    }
    let tick = w.time.simulation_ticks;
    // Circles from this tick's events (deaths first, then the round result).
    let events: Vec<Cue> = w.events.iter().map(|e| e.cue.clone()).collect();
    for cue in &events {
        match cue {
            Cue::PlayerDied { id, .. } => {
                let Some(team) = w.player(*id).map(|p| p.team) else {
                    continue;
                };
                if (0..n)
                    .any(|k| w.players[k].team == team && w.players[k].is_alive() && !is_gone(w, k))
                {
                    continue;
                }
                for i in 0..n {
                    if w.players[i].team == team && w.flow.players[i].circle == Circle::Hidden {
                        let slot = take_slot(&w.flow.players, n, true);
                        w.flow.players[i].circle = Circle::Loser { since: tick, slot };
                    }
                }
            }
            Cue::RoundWon { .. } | Cue::RoundDraw => {
                for i in 0..n {
                    if !w.players[i].won_this_round {
                        continue;
                    }
                    let slot = match w.flow.players[i].circle.slot() {
                        Some(s) => s,
                        None => take_slot(&w.flow.players, n, false),
                    };
                    let fp = &mut w.flow.players[i];
                    fp.wheel = Wheel::default();
                    fp.circle = Circle::Winner { since: tick, slot };
                }
            }
            _ => {}
        }
    }
    if w.round.decided && w.flow.next.is_none() {
        let next = draw_next(&mut w.flow);
        w.flow.next = Some(next);
    }
    if w.round.winner_screen && w.flow.winner_screen_at.is_none() {
        w.flow.winner_screen_at = Some(tick);
    }

    // Wheels.
    let count = w.content.abilities.len();
    let mut consumed = vec![false; n];
    for i in 0..n {
        let inp = inputs.get(i).copied().unwrap_or_default();
        let prev = w.flow.prev[i];
        let s = signals(inp, prev, w.players[i].uses_mouse);
        let rows = wheel_rows(w, i);
        let fp = &mut w.flow.players[i];
        if fp.wheel.closing > 0 {
            fp.wheel.closing -= 1;
        }
        let Circle::Loser { since, .. } = fp.circle else {
            continue;
        };
        if tick - since <= WHEEL_DELAY_TICKS || rows == 0 || count < 2 {
            continue;
        }
        let wh = &mut fp.wheel;
        if !wh.open {
            if s.open && !wh.up_latch {
                *wh = Wheel {
                    open: true,
                    toggled_at: tick,
                    up_latch: !w.players[i].uses_mouse,
                    changed_at: wh.changed_at,
                    // A held stick does not navigate on the opening tick.
                    was_up: true,
                    was_down: s.down,
                    was_left: s.left,
                    was_right: s.right,
                    ..Wheel::default()
                };
                consumed[i] = true;
            } else if wh.up_latch && s.released {
                wh.up_latch = false;
            }
            continue;
        }
        if s.close {
            wh.open = false;
            wh.closing = WHEEL_CLOSE_TICKS;
            wh.toggled_at = tick;
            consumed[i] = true;
            w.emit_late(Cue::Sound("selectAbilities"));
            continue;
        }
        if wh.up_latch && s.released {
            wh.up_latch = false;
        }
        let mouse = w.players[i].uses_mouse;
        // Row navigation (edge-triggered; returns like the original).
        let mut nav = None;
        let mut stop = false;
        if mouse {
            if s.up {
                nav = Some(-1);
            } else if s.down {
                nav = Some(1);
            }
        } else if s.up && !wh.was_up {
            wh.was_up = true;
            nav = Some(-1);
        } else if wh.was_up && !s.up {
            wh.was_up = false;
            stop = true;
        } else if s.down && !wh.was_down {
            wh.was_down = true;
            nav = Some(1);
        } else if wh.was_down && !s.down {
            wh.was_down = false;
            stop = true;
        }
        if let Some(d) = nav {
            wh.row = ((wh.row as i32 + d).rem_euclid(rows as i32)) as u8;
            w.emit_late(Cue::Sound("navigateMenu"));
            continue;
        }
        if stop {
            continue;
        }
        // Ability change: left = next index, right = previous (QUIRK).
        let mut change = 0i32;
        if mouse {
            if s.right {
                change = -1;
            } else if s.left {
                change = 1;
            }
        } else {
            if wh.repeat > 0 {
                wh.repeat -= 1;
            }
            if s.right && !wh.was_right {
                wh.was_right = true;
                change = -1;
            } else if !s.right || (wh.was_right && wh.repeat == 0) {
                wh.was_right = false;
            }
            if s.left && !wh.was_left {
                wh.was_left = true;
                change = 1;
            } else if !s.left || (wh.was_left && wh.repeat == 0) {
                wh.was_left = false;
            }
            if change != 0 {
                wh.repeat = WHEEL_REPEAT_TICKS;
            }
        }
        if change != 0 {
            wh.changed_at = tick;
            wh.change_dir = change as i8;
            let slot = row_slot(rows, wh.row as usize);
            let cur = w.players[i].abilities[slot];
            let c = count as i32;
            let mut next = (cur as i32 + change).rem_euclid(c);
            if next == 0 {
                next = if change > 0 { 1 } else { c - 1 };
            }
            w.players[i].abilities[slot] = next as usize;
            w.emit_late(Cue::Sound("navigateMenu"));
        }
    }

    // Players left (online) and fewer than two teams remain: back to
    // character select.
    if w.flow.request.is_none() && w.flow.gone.iter().any(|g| *g) && connected_teams(w) < 2 {
        w.flow.request = Some(FlowRequest::BackToSelect);
        w.emit_late(Cue::Sound("return"));
    }
    // Winner screen: next level / back to character select.
    if let Some(at) = w.flow.winner_screen_at
        && w.flow.request.is_none()
        && tick - at > WINNER_INPUT_DELAY_TICKS
    {
        let (mut next, mut back) = (None, None);
        for i in 0..n {
            if consumed[i] {
                continue;
            }
            let inp = inputs.get(i).copied().unwrap_or_default();
            let s = signals(inp, w.flow.prev[i], w.players[i].uses_mouse);
            if w.flow.players[i].wheel.blocking() {
                continue;
            }
            if s.back && back.is_none() {
                back = Some(i);
            }
            if s.next && next.is_none() {
                next = Some(i);
            }
        }
        if let Some(by) = back.or(next) {
            let blockers: Vec<usize> = (0..n)
                .filter(|&k| k != by && w.flow.players[k].wheel.blocking() && !is_gone(w, k))
                .collect();
            if !blockers.is_empty() {
                for k in blockers {
                    w.flow.players[k].wiggle_at = Some(tick);
                }
                w.emit_late(Cue::Sound("smallError"));
            } else if back.is_some() {
                w.flow.request = Some(FlowRequest::BackToSelect);
                w.emit_late(Cue::Sound("return"));
            } else {
                let bag = w.flow.next.unwrap_or(w.flow.current);
                w.flow.request = Some(FlowRequest::NextLevel(bag));
                w.emit_late(Cue::Sound("startGame"));
            }
        }
    }
    w.flow.prev.clear();
    w.flow
        .prev
        .extend((0..n).map(|i| inputs.get(i).copied().unwrap_or_default()));
}

/// The circle position a new circle takes: losers the rightmost free one,
/// winners the leftmost.
fn take_slot(players: &[FlowPlayer], n: usize, rightmost: bool) -> usize {
    let used: Vec<usize> = players.iter().filter_map(|p| p.circle.slot()).collect();
    let free = |s: &usize| !used.contains(s);
    if rightmost {
        (0..n).rev().find(free).unwrap_or(0)
    } else {
        (0..n).find(free).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bag_cycles_and_returns_to_level1() {
        let mut f = Flow {
            played: vec![false; 5],
            rng: Rng::new(7),
            ..Flow::default()
        };
        let mut seen = vec![0usize];
        for _ in 0..4 {
            let n = draw_next(&mut f);
            assert!(!seen.contains(&n), "{n} drawn twice in a cycle");
            seen.push(n);
            f.current = n;
        }
        // All 5 played: the quirk resets the bag and always yields index 0.
        assert_eq!(draw_next(&mut f), 0);
    }

    #[test]
    fn row_slot_swaps_with_three() {
        assert_eq!(
            (0..3).map(|r| row_slot(3, r)).collect::<Vec<_>>(),
            [0, 2, 1]
        );
        assert_eq!((0..2).map(|r| row_slot(2, r)).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn level_seeds_differ() {
        assert_eq!(level_seed(42, 1), 42);
        assert_ne!(level_seed(42, 2), level_seed(42, 3));
        assert_ne!(level_seed(42, 2), 0);
    }

    #[test]
    fn slots_fill_from_both_ends() {
        let mut ps = vec![FlowPlayer::default(); 4];
        ps[0].circle = Circle::Loser {
            since: 0,
            slot: take_slot(&ps, 4, true),
        };
        assert_eq!(ps[0].circle.slot(), Some(3));
        ps[1].circle = Circle::Winner {
            since: 0,
            slot: take_slot(&ps, 4, false),
        };
        assert_eq!(ps[1].circle.slot(), Some(0));
        assert_eq!(take_slot(&ps, 4, true), 2);
    }
}

//! The GGRS side of one match: sessions ("epochs"), the soft prediction
//! limit, frame offsets, freeze/resume when machines drop, and the spectator
//! input stream.
//!
//! GGRS 0.13 cannot drop a peer safely in sessions with more than two
//! machines: each machine disconnects the peer at the last frame *it*
//! received, and when those differ GGRS has to roll back past inputs it has
//! already discarded (`input_queue.rs` assertion), or past its window. So we
//! never let GGRS disconnect anybody. Instead the host freezes the match,
//! every machine reports the newest frame it has *fully confirmed and
//! simulated*, the host picks the minimum `D`, and every machine restores its
//! saved state of frame `D + 1` and continues in a fresh GGRS session in
//! which the dropped machine's slots are dummy local players of the host
//! that always send the zero input (reported as `InputStatus::Disconnected`).
//! Every machine thus sees the dropped players' real inputs up to frame `D`
//! and zero inputs from `D + 1` on: deterministic.

use crate::game::{NetGame, Request, StateCell};
use crate::input::Wire;
use crate::session::{GgrsConfig, NetError, NetEvent, NetStats};
use crate::transport::{Transport, WzSocket};
use crate::wire::{HOST_MACHINE, MachineId, MatchStart, SpecFrame};
use bytemuck::Zeroable;
use ggrs::{
    DesyncDetection, GameStateCell, GgrsError, GgrsEvent, GgrsRequest, InputStatus, P2PSession,
    PlayerType, SessionBuilder, SessionState,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use std::time::Duration;

pub(crate) struct MatchRun<G: NetGame> {
    pub info: MatchStart,
    pub me: MachineId,
    /// Slots played on this machine.
    pub local: Vec<usize>,
    pub gone: BTreeSet<MachineId>,
    pub epoch: u8,
    /// Sim frame of GGRS frame 0 in the current session.
    base: i32,
    session: P2PSession<GgrsConfig<G>>,
    inputs: Vec<Option<G::Input>>,
    /// Sim frame -> the cell its state was saved in (newest window only).
    cells: BTreeMap<i32, GameStateCell<G>>,
    /// Newest sim frame whose inputs were all confirmed when the requests of
    /// the last advance were produced (so its state is final once they ran).
    safe_confirmed: i32,
    pub frozen: Option<u8>,
    /// The state this session started from (after a resume) ...
    restart: Option<(StateCell<G>, i32)>,
    /// ... still to be handed to the game as a Load request.
    load_pending: bool,
    pub running: bool,
    pub finished: bool,
    /// Frame of the game's state as our requests leave it.
    sim_frame: i32,
    /// Frames below this were simulated before (re-simulation after rollback).
    high_water: i32,
    /// Host with spectators: inputs per sim frame not yet streamed.
    record: Option<BTreeMap<i32, SpecFrame>>,
    /// Next frame to stream to spectators.
    pub streamed_to: i32,
}

fn build<G: NetGame>(
    info: &MatchStart,
    me: MachineId,
    epoch: u8,
    gone: &BTreeSet<MachineId>,
    t: &Rc<RefCell<Transport>>,
) -> Result<P2PSession<GgrsConfig<G>>, NetError> {
    let p = info.ggrs;
    let g = |e: GgrsError| NetError::Ggrs(e.to_string());
    let mut b = SessionBuilder::<GgrsConfig<G>>::new()
        .with_num_players(info.players.len())
        .map_err(g)?
        .with_input_delay(p.input_delay as usize)
        .with_max_prediction_window(p.rollback_window.max(p.max_prediction) as usize)
        .with_fps(p.fps.max(1) as usize)
        .map_err(g)?
        .with_desync_detection_mode(if p.desync_interval > 0 {
            DesyncDetection::On {
                interval: p.desync_interval as u32,
            }
        } else {
            DesyncDetection::Off
        })
        // Drops are decided by the host (freeze/resume), never by GGRS.
        .with_disconnect_timeout(Duration::from_secs(600))
        .with_disconnect_notify_delay(Duration::from_millis(p.disconnect_notify_ms as u64));
    for pl in &info.players {
        let owner = if gone.contains(&pl.machine) {
            HOST_MACHINE
        } else {
            pl.machine
        };
        let ty = if owner == me {
            PlayerType::Local
        } else {
            PlayerType::Remote(owner)
        };
        b = b.add_player(ty, pl.slot as usize).map_err(g)?;
    }
    t.borrow_mut().set_match_tag(tag(info.match_id, epoch));
    b.start_p2p_session(WzSocket(t.clone())).map_err(g)
}

fn tag(match_id: u32, epoch: u8) -> u16 {
    ((match_id as u16) << 4) | (epoch as u16 & 0xf)
}

/// Frames the spectator stream stays behind the host's confirmed frame, so
/// that a freeze (which may restart a little before the host's confirmed
/// frame) never rewrites a frame spectators already have.
/// Spectator frames per message, at least.
const SPEC_BATCH: i32 = 4;

pub(crate) fn spectator_lag(info: &MatchStart) -> i32 {
    info.ggrs.max_prediction as i32 + info.ggrs.input_delay as i32 + 4
}

impl<G: NetGame> MatchRun<G> {
    pub fn new(
        info: MatchStart,
        me: MachineId,
        t: &Rc<RefCell<Transport>>,
    ) -> Result<Self, NetError> {
        let gone = BTreeSet::new();
        let session = build(&info, me, 0, &gone, t)?;
        let local = info.slots_of(me);
        let n = info.players.len();
        let record = (me == HOST_MACHINE && !info.spectators.is_empty()).then(BTreeMap::new);
        Ok(MatchRun {
            info,
            me,
            local,
            gone,
            epoch: 0,
            base: 0,
            session,
            inputs: vec![None; n],
            cells: BTreeMap::new(),
            safe_confirmed: -1,
            frozen: None,
            restart: None,
            load_pending: false,
            running: false,
            finished: false,
            sim_frame: 0,
            high_water: 0,
            record,
            streamed_to: 0,
        })
    }

    pub fn add_input(&mut self, slot: usize, input: G::Input) -> Result<(), NetError> {
        if !self.local.contains(&slot) {
            return Err(NetError::NotLocal(slot));
        }
        self.inputs[slot] = Some(input);
        Ok(())
    }

    pub fn current_frame(&self) -> i32 {
        self.session.current_frame() + self.base
    }

    pub fn confirmed_frame(&self) -> i32 {
        let c = self.session.confirmed_frame();
        if c < 0 { self.base - 1 } else { c + self.base }
    }

    pub fn frames_ahead(&self) -> i32 {
        self.session.frames_ahead()
    }

    pub fn network_stats(&self, m: MachineId) -> Option<ggrs::NetworkStats> {
        if self.gone.contains(&m) {
            return None;
        }
        let h = *self.info.slots_of(m).first()?;
        self.session.network_stats(h).ok()
    }

    pub fn poll(&mut self) {
        self.session.poll_remote_clients();
    }

    fn gone_slot(&self, slot: usize) -> bool {
        self.info
            .players
            .get(slot)
            .is_some_and(|p| self.gone.contains(&p.machine))
    }

    pub fn advance(&mut self, stats: &mut NetStats) -> Vec<Request<G>> {
        let mut out = Vec::new();
        if self.frozen.is_some() {
            self.session.poll_remote_clients();
            return out;
        }
        if self.load_pending
            && let Some((cell, frame)) = self.restart.clone()
        {
            self.load_pending = false;
            self.sim_frame = frame;
            out.push(Request::Load { cell, frame });
        }
        for pl in &self.info.players {
            let slot = pl.slot as usize;
            let mine = self.local.contains(&slot);
            let dummy = self.me == HOST_MACHINE && self.gone.contains(&pl.machine);
            if mine || dummy {
                let i = if mine { self.inputs[slot].take() } else { None }
                    .unwrap_or_else(G::Input::zeroed);
                let _ = self.session.add_local_input(slot, Wire(i));
            }
        }
        let ggrs_frame = self.session.current_frame();
        let ahead = ggrs_frame - self.session.confirmed_frame();
        if self.session.current_state() != SessionState::Running {
            self.session.poll_remote_clients();
            return out;
        }
        if ahead > self.info.ggrs.max_prediction as i32 {
            self.session.poll_remote_clients();
            stats.prediction_stalls += 1;
            return out;
        }
        match self.session.advance_frame() {
            Ok(reqs) => {
                let base = self.base;
                for r in reqs {
                    out.push(match r {
                        GgrsRequest::SaveGameState { cell, frame } => {
                            self.cells.insert(frame + base, cell.clone());
                            self.sim_frame = frame + base;
                            Request::Save {
                                cell: StateCell { cell, base },
                                frame: frame + base,
                            }
                        }
                        GgrsRequest::LoadGameState { cell, frame } => {
                            stats.rollbacks += 1;
                            stats.max_rollback_depth = stats
                                .max_rollback_depth
                                .max((ggrs_frame - frame).max(0) as u32);
                            self.sim_frame = frame + base;
                            Request::Load {
                                cell: StateCell { cell, base },
                                frame: frame + base,
                            }
                        }
                        GgrsRequest::AdvanceFrame { inputs } => {
                            let frame = self.sim_frame;
                            let resim = frame < self.high_water;
                            if resim {
                                stats.resimulated_frames += 1;
                            }
                            self.sim_frame += 1;
                            self.high_water = self.high_water.max(self.sim_frame);
                            let inputs: Vec<(G::Input, InputStatus)> = inputs
                                .into_iter()
                                .enumerate()
                                .map(|(slot, (w, s))| {
                                    if self.gone_slot(slot) {
                                        (G::Input::zeroed(), InputStatus::Disconnected)
                                    } else {
                                        (w.0, s)
                                    }
                                })
                                .collect();
                            if let Some(rec) = self.record.as_mut() {
                                rec.insert(frame, spec_frame(&inputs));
                            }
                            Request::Advance {
                                frame,
                                inputs,
                                resim,
                            }
                        }
                    });
                }
                let conf = self.session.confirmed_frame();
                if conf >= 0 {
                    self.safe_confirmed = self.safe_confirmed.max(conf + base);
                }
                let keep_from = self.current_frame()
                    - self
                        .info
                        .ggrs
                        .rollback_window
                        .max(self.info.ggrs.max_prediction) as i32;
                self.cells.retain(|f, _| *f >= keep_from);
            }
            Err(GgrsError::PredictionThreshold) => stats.prediction_stalls += 1,
            Err(e) => log::warn!("advance_frame: {e}"),
        }
        out
    }

    /// Host: final frames for spectators, from `streamed_to` on. `all`: the
    /// match is over, everything confirmed is final.
    pub fn take_spec(&mut self, all: bool) -> Option<(i32, Vec<SpecFrame>)> {
        let lag = if all { 0 } else { spectator_lag(&self.info) };
        let upto = self.confirmed_frame() - lag;
        // Batch: a message every few frames, not every frame.
        if !all && upto - self.streamed_to + 1 < SPEC_BATCH {
            return None;
        }
        let rec = self.record.as_mut()?;
        let first = self.streamed_to;
        let mut frames = Vec::new();
        while self.streamed_to <= upto {
            let Some(f) = rec.remove(&self.streamed_to) else {
                break;
            };
            frames.push(f);
            self.streamed_to += 1;
        }
        let next = self.streamed_to;
        rec.retain(|f, _| *f >= next);
        (!frames.is_empty()).then_some((first, frames))
    }

    /// Stop advancing; returns the frame `r` we can restart from (`r + 1`
    /// is saved and final here).
    pub fn freeze(&mut self, epoch: u8) -> i32 {
        self.frozen = Some(epoch);
        let newest_saved = self.cells.keys().next_back().copied().unwrap_or(self.base);
        self.safe_confirmed.min(newest_saved - 1).max(self.base - 1)
    }

    /// Continue from sim frame `frame` in a new session without `gone`.
    pub fn resume(
        &mut self,
        epoch: u8,
        frame: i32,
        gone: BTreeSet<MachineId>,
        t: &Rc<RefCell<Transport>>,
    ) -> Result<(), NetError> {
        // The state to restart from: a saved cell, the state this session
        // started from, or (nothing simulated yet) the initial state.
        let restart = match self.cells.get(&frame) {
            Some(c) => Some((
                StateCell {
                    cell: c.clone(),
                    base: self.base,
                },
                frame,
            )),
            None => match &self.restart {
                Some((c, f)) if *f == frame => Some((c.clone(), *f)),
                None if frame == self.base && self.current_frame() == self.base => None,
                _ => return Err(NetError::Ggrs(format!("no saved state for frame {frame}"))),
            },
        };
        if let Some(rec) = self.record.as_mut() {
            if self.streamed_to > frame {
                log::warn!(
                    "resume at {frame} behind the spectator stream ({}): spectators lose sync",
                    self.streamed_to
                );
            }
            rec.retain(|f, _| *f < frame);
        }
        self.gone = gone;
        self.epoch = epoch;
        self.session = build(&self.info, self.me, epoch, &self.gone, t)?;
        self.base = frame;
        self.cells.clear();
        self.safe_confirmed = frame - 1;
        self.load_pending = restart.is_some();
        self.restart = restart;
        self.frozen = None;
        self.running = false;
        Ok(())
    }

    /// GGRS events as [`NetEvent`]s (and the Running transition).
    pub fn events(&mut self, stats: &mut NetStats) -> Vec<NetEvent> {
        let mut out = Vec::new();
        for e in self.session.events() {
            out.push(match e {
                GgrsEvent::Synchronizing { addr, total, count } => NetEvent::Synchronizing {
                    machine: addr,
                    count,
                    total,
                },
                GgrsEvent::Synchronized { addr } => NetEvent::Synchronized { machine: addr },
                GgrsEvent::Disconnected { addr } => NetEvent::Disconnected { machine: addr },
                GgrsEvent::NetworkInterrupted {
                    addr,
                    disconnect_timeout,
                } => NetEvent::NetworkInterrupted {
                    machine: addr,
                    disconnect_timeout_ms: disconnect_timeout as u64,
                },
                GgrsEvent::NetworkResumed { addr } => NetEvent::NetworkResumed { machine: addr },
                GgrsEvent::WaitRecommendation { skip_frames } => NetEvent::WaitRecommendation {
                    frames: skip_frames,
                },
                GgrsEvent::DesyncDetected {
                    frame,
                    local_checksum,
                    remote_checksum,
                    addr,
                } => {
                    stats.desyncs += 1;
                    NetEvent::Desync {
                        frame: frame + self.base,
                        local: local_checksum,
                        remote: remote_checksum,
                        machine: addr,
                    }
                }
            });
        }
        let was = self.running;
        self.running = self.session.current_state() == SessionState::Running;
        if self.running && !was {
            out.push(NetEvent::Running);
        }
        out
    }
}

fn spec_frame<I: bytemuck::Pod>(inputs: &[(I, InputStatus)]) -> SpecFrame {
    let mut gone = 0u16;
    let mut bytes = Vec::with_capacity(inputs.len() * std::mem::size_of::<I>());
    for (slot, (i, s)) in inputs.iter().enumerate() {
        if *s == InputStatus::Disconnected {
            gone |= 1 << slot;
        }
        bytes.extend_from_slice(bytemuck::bytes_of(i));
    }
    SpecFrame {
        gone,
        inputs: bytes,
    }
}

/// A spectator's side of a match: replays the host's final inputs.
pub(crate) struct SpectatorRun<G: NetGame> {
    pub info: MatchStart,
    queue: VecDeque<SpecFrame>,
    /// Frame the next queued input belongs to / the game's state frame.
    pub next: i32,
    /// Frames received so far (= next frame expected from the host).
    received: i32,
    pub running: bool,
    announced: bool,
    _g: std::marker::PhantomData<G>,
}

impl<G: NetGame> SpectatorRun<G> {
    pub fn new(info: MatchStart) -> Self {
        SpectatorRun {
            info,
            queue: VecDeque::new(),
            next: 0,
            received: 0,
            running: false,
            announced: false,
            _g: std::marker::PhantomData,
        }
    }

    pub fn push(&mut self, first: i32, frames: Vec<SpecFrame>) {
        for (k, f) in frames.into_iter().enumerate() {
            let frame = first + k as i32;
            if frame == self.received {
                self.queue.push_back(f);
                self.received += 1;
            }
        }
        self.running = true;
    }

    /// Newest frame whose inputs we have.
    pub fn confirmed_frame(&self) -> i32 {
        self.received - 1
    }

    /// Frames buffered beyond what we simulated.
    pub fn buffered(&self) -> usize {
        self.queue.len()
    }

    pub fn advance(&mut self, stats: &mut NetStats) -> Vec<Request<G>> {
        // Catch up when far behind (late joins of the stream, hiccups).
        let lag = spectator_lag(&self.info) as usize;
        let n = match self.queue.len() {
            0 => 0,
            q if q > 4 * lag => 4,
            q if q > 2 * lag => 2,
            _ => 1,
        };
        let size = std::mem::size_of::<G::Input>();
        let mut out = Vec::new();
        for _ in 0..n {
            let Some(f) = self.queue.pop_front() else {
                break;
            };
            let inputs = (0..self.info.players.len())
                .map(|slot| {
                    let i = f
                        .inputs
                        .get(slot * size..(slot + 1) * size)
                        .map(bytemuck::pod_read_unaligned::<G::Input>)
                        .unwrap_or_else(G::Input::zeroed);
                    let s = if f.gone & (1 << slot) != 0 {
                        InputStatus::Disconnected
                    } else {
                        InputStatus::Confirmed
                    };
                    (i, s)
                })
                .collect();
            out.push(Request::Advance {
                frame: self.next,
                inputs,
                resim: false,
            });
            self.next += 1;
        }
        if n == 0 && self.running {
            stats.prediction_stalls += 1;
        }
        out
    }

    pub fn events(&mut self) -> Vec<NetEvent> {
        if self.running && !self.announced {
            self.announced = true;
            return vec![NetEvent::Running];
        }
        Vec::new()
    }
}

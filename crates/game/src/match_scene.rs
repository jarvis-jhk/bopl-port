//! A running match: input sources → sim → presentation, plus the in-match
//! overlays (HUD indicators, ability-select circles and wheels, winner
//! triangle, pause prompt).

use anyhow::Result;
use bopl_render::{Frame, PassTarget};
use bopl_sim::fix::Fix;
use bopl_sim::input::{
    BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2, BTN_JUMP, BTN_SELECT, BTN_START, KEY_A, KEY_D, KEY_S,
    KEY_W, PlayerInput, angle_byte_from_vector,
};
use bopl_sim::player::PlayerSetup;
use bopl_sim::view::View;
use bopl_sim::world::MatchSettings;
use glam::Vec2;

use crate::app::DrawContext;
use crate::audio::Audio;
use crate::bots::Bot;
use crate::input::{Buttons, DeviceId, InputState};
use crate::match_ui::MatchUi;
use crate::match_view::MatchPresenter;
use crate::present::WorldView;
use crate::shared::Shared;
use crate::sim_match::SimMatch;

/// Where a player's input comes from.
#[derive(Clone, Debug)]
pub enum Source {
    Device(DeviceId),
    Bot(Bot),
    /// Scripted inputs, then a bot (or nothing).
    Script {
        inputs: Vec<PlayerInput>,
        then: Option<Bot>,
    },
    /// A bot, overridden by the device whenever it is used (headless
    /// flow tests drive menus and the wheel while bots fight).
    Assisted {
        device: DeviceId,
        bot: Bot,
    },
    /// A player of another machine (online): its input comes over the net.
    Remote,
}

impl Source {
    /// The local device of a human player.
    pub fn device(&self) -> Option<DeviceId> {
        match self {
            Source::Device(d) | Source::Assisted { device: d, .. } => Some(*d),
            _ => None,
        }
    }
}

/// What the match wants from the app after a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchOutcome {
    Continue,
    /// "Ability select" on the winner screen: back to character select.
    BackToSelect,
    /// Pause prompt "leave": back to the main menu.
    LeaveToMenu,
}

/// The pause prompt (ui.md 7): one player controls it; its inputs are
/// neutral meanwhile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pause {
    pub player: usize,
    /// `true` = the leave (✓) button is selected.
    pub leave: bool,
    pub opened: f32,
}

/// Turns a device state into the sim input (player.md 2.2).
pub fn device_input(st: &InputState, mouse: bool, aim: Option<Vec2>) -> PlayerInput {
    let h = |b: Buttons| st.held.contains(b);
    let mut buttons = 0u8;
    for (b, bit) in [
        (Buttons::JUMP, BTN_JUMP),
        (Buttons::ABILITY0, BTN_ABILITY0),
        (Buttons::ABILITY1, BTN_ABILITY1),
        (Buttons::ABILITY2, BTN_ABILITY2),
        (Buttons::START, BTN_START),
        (Buttons::SELECT, BTN_SELECT),
    ] {
        if h(b) {
            buttons |= bit;
        }
    }
    let to_byte = |v: Vec2| {
        angle_byte_from_vector(bopl_sim::Vec2::new(Fix::from_f32(v.x), Fix::from_f32(v.y)))
    };
    if mouse {
        let mut keys = 0u8;
        for (b, bit) in [
            (Buttons::UP, KEY_W),
            (Buttons::LEFT, KEY_A),
            (Buttons::DOWN, KEY_S),
            (Buttons::RIGHT, KEY_D),
        ] {
            if h(b) {
                keys |= bit;
            }
        }
        // Aim: mouse world position − player; zero inside 0.2 (squared).
        let angle = match aim {
            Some(d) if d.length_squared() >= 0.2 => to_byte(d.normalize()),
            _ => 0,
        };
        PlayerInput {
            angle,
            buttons,
            keys,
        }
    } else {
        let v = st.move_axis;
        let angle = if v.length_squared() < 0.4 * 0.4 {
            0
        } else {
            to_byte(v.normalize())
        };
        PlayerInput::new(angle, buttons)
    }
}

/// `Rumble` states of a player (the original's `Player.RumbleState`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RumbleKind {
    #[default]
    None,
    /// `Rumble.BuildTension`: the extra-small motor speeds until it ends
    /// (Roll charging, Meteor vibrating, a spell's unstable last 2 s).
    Tension,
    Small,
    Big,
}

/// The original's per-device rumble (`InputUpdater`): a small or big
/// rumble runs for `rumbleSeconds` (0.5) at the prefab's motor speeds
/// (x low, y high frequency), "high" multiplies them by 3.
#[derive(Clone, Copy, Debug, Default)]
struct RumbleState {
    state: RumbleKind,
    prev: RumbleKind,
    time: f32,
}

/// Motor speeds (low, high) of a small and a big rumble (the player
/// prefab's `InputUpdater`).
const RUMBLE_SMALL: (f32, f32) = (0.0, 0.4);
/// `ExtraSmallRumbleMotorSpeeds` (build tension).
const RUMBLE_TENSION: (f32, f32) = (0.05, 0.05);
const RUMBLE_BIG: (f32, f32) = (0.4, 0.5);
const RUMBLE_SECONDS: f32 = 0.5;

pub struct MatchScene {
    pub sim: SimMatch,
    pub pres: MatchPresenter,
    pub sources: Vec<Source>,
    pub view: View,
    last_world: WorldView,
    size: (u32, u32),
    /// Ticks since the match started.
    pub ticks: u64,
    pub pause: Option<Pause>,
    pub ui: MatchUi,
    /// Inputs of the last tick (debug, tests).
    pub last_inputs: Vec<PlayerInput>,
    /// Online: the match under rollback; `sim` mirrors its current
    /// (predicted) state after every step.
    pub online: Option<Box<crate::online::runner::OnlineMatch>>,
    /// Rumble per player.
    rumble: Vec<RumbleState>,
    /// Mouse player's wheel: the hovered row, the row to reach, and a key
    /// sent last tick (edges need a release in between).
    wheel_hover: Option<usize>,
    wheel_target: Option<usize>,
    wheel_key_down: bool,
}

impl MatchScene {
    pub fn new(
        shared: &mut Shared,
        setups: &[PlayerSetup],
        sources: Vec<Source>,
        settings: MatchSettings,
        first_level: Option<usize>,
        audio: &mut Audio,
    ) -> Result<Self> {
        let sim = SimMatch::new(shared.levels.clone(), setups, settings, first_level)?;
        let mut pres =
            MatchPresenter::new(shared.visuals.clone(), sim.level, sim.level_generation)?;
        pres.screen_shake = shared.settings.screen_shake;
        pres.camera_zoom = shared.settings.camera_zoom;
        let view = sim.world.view();
        shared.sounds.match_started(audio);
        let mut ui = MatchUi::new(shared);
        ui.remote = sources
            .iter()
            .map(|s| matches!(s, Source::Remote))
            .collect();
        Ok(Self {
            sim,
            pres,
            sources,
            view,
            last_world: WorldView::default(),
            size: (1920, 1080),
            ticks: 0,
            pause: None,
            ui,
            last_inputs: Vec::new(),
            online: None,
            rumble: Vec::new(),
            wheel_hover: None,
            wheel_target: None,
            wheel_key_down: false,
        })
    }

    /// An online match: the presentation of `om`'s state.
    pub fn new_online(
        shared: &mut Shared,
        om: crate::online::runner::OnlineMatch,
        sources: Vec<Source>,
        audio: &mut Audio,
    ) -> Result<Self> {
        let sim = om.game.sim.clone();
        let mut pres =
            MatchPresenter::new(shared.visuals.clone(), sim.level, sim.level_generation)?;
        pres.screen_shake = shared.settings.screen_shake;
        pres.camera_zoom = shared.settings.camera_zoom;
        let view = sim.world.view();
        shared.sounds.match_started(audio);
        let mut ui = MatchUi::new(shared);
        ui.remote = sources
            .iter()
            .map(|s| matches!(s, Source::Remote))
            .collect();
        pres.online_local = view
            .players
            .iter()
            .zip(&sources)
            .filter(|(_, s)| !matches!(s, Source::Remote))
            .map(|(p, _)| p.id)
            .collect();
        Ok(Self {
            sim,
            pres,
            sources,
            view,
            last_world: WorldView::default(),
            size: (1920, 1080),
            ticks: 0,
            pause: None,
            ui,
            last_inputs: Vec::new(),
            online: Some(Box::new(om)),
            rumble: Vec::new(),
            wheel_hover: None,
            wheel_target: None,
            wheel_key_down: false,
        })
    }

    /// Controller hot-plug during a match: a player whose controller went
    /// away gets the next new controller that presses a button (SDL gives
    /// a re-plugged pad a new id). Until then the player's input is
    /// neutral.
    fn repair_devices(&mut self, devices: &[(DeviceId, InputState)]) {
        let present = |d: DeviceId| devices.iter().any(|(id, _)| *id == d);
        let orphan = self
            .sources
            .iter()
            .position(|s| matches!(s.device(), Some(d @ DeviceId::Controller(_)) if !present(d)));
        let Some(orphan) = orphan else { return };
        let used: Vec<DeviceId> = self.sources.iter().filter_map(Source::device).collect();
        let fresh = devices.iter().find(|(d, st)| {
            matches!(d, DeviceId::Controller(_)) && !used.contains(d) && st.pressed.0 != 0
        });
        if let Some((d, _)) = fresh {
            log::info!("player {} continues on {d:?}", orphan + 1);
            match &mut self.sources[orphan] {
                Source::Device(old) | Source::Assisted { device: old, .. } => *old = *d,
                _ => {}
            }
        }
    }

    fn inputs(&mut self, devices: &[(DeviceId, InputState)]) -> Vec<PlayerInput> {
        self.repair_devices(devices);
        let n = self.sim.world.players.len();
        let mut out = vec![PlayerInput::default(); n];
        let ticks = self.ticks as usize;
        for (i, src) in self.sources.iter_mut().enumerate().take(n) {
            out[i] = match src {
                Source::Device(d) => {
                    let Some((_, st)) = devices.iter().find(|(id, _)| id == d) else {
                        continue;
                    };
                    let mouse = *d == DeviceId::KeyboardMouse;
                    let aim = if mouse {
                        st.pointer.map(|px| {
                            let w = self.pres.screen_to_world(&self.last_world, px, self.size);
                            let p = &self.view.players[i].position;
                            w - Vec2::new(p.x.to_f32(), p.y.to_f32())
                        })
                    } else {
                        None
                    };
                    device_input(st, mouse, aim)
                }
                Source::Bot(b) => b.input(&self.view),
                Source::Assisted { device, bot } => {
                    let bi = bot.input(&self.view);
                    match devices.iter().find(|(id, _)| id == device) {
                        Some((_, st)) if st.held.0 != 0 || st.move_axis != Vec2::ZERO => {
                            device_input(st, *device == DeviceId::KeyboardMouse, None)
                        }
                        _ => bi,
                    }
                }
                Source::Remote => PlayerInput::default(),
                Source::Script { inputs, then } => match inputs.get(ticks) {
                    Some(i) => *i,
                    None => then
                        .as_mut()
                        .map_or(PlayerInput::default(), |b| b.input(&self.view)),
                },
            };
        }
        out
    }

    /// Lets a bot play the player of `device` (see [`Source::Assisted`]).
    pub fn assist(&mut self, device: DeviceId, seed: u32) {
        for (i, s) in self.sources.iter_mut().enumerate() {
            if matches!(s, Source::Device(d) if *d == device) {
                *s = Source::Assisted {
                    device,
                    bot: Bot::new(i, seed),
                };
            }
        }
    }

    /// One tick.
    pub fn tick(
        &mut self,
        shared: &mut Shared,
        devices: &[(DeviceId, InputState)],
        audio: &mut Audio,
    ) -> Result<MatchOutcome> {
        self.tick_with(shared, devices, audio, None)
    }

    /// One tick online: the local players' inputs go to bopl-net, which
    /// steps (and rolls back) the match; cues come through the event gate.
    pub fn tick_online(
        &mut self,
        shared: &mut Shared,
        devices: &[(DeviceId, InputState)],
        audio: &mut Audio,
        net: &mut bopl_net::NetSession<crate::online::adapter::NetWorld>,
    ) -> Result<MatchOutcome> {
        self.tick_with(shared, devices, audio, Some(net))
    }

    fn tick_with(
        &mut self,
        shared: &mut Shared,
        devices: &[(DeviceId, InputState)],
        audio: &mut Audio,
        net: Option<&mut bopl_net::NetSession<crate::online::adapter::NetWorld>>,
    ) -> Result<MatchOutcome> {
        let mut inputs = self.inputs(devices);
        // Pause prompt: opened by a device player's Start (gamepad) or Esc
        // (keyboard) while the winner screen is not up.
        let winner = self.view.round.winner_screen || self.view.round.decided;
        if let Some(p) = &mut self.pause {
            p.opened += 1.0 / 60.0;
            if winner {
                self.pause = None;
            }
        }
        if let Some(p) = self.pause {
            let dev = self.sources[p.player].device();
            inputs[p.player] = PlayerInput::default();
            if let Some(st) = dev.and_then(|d| devices.iter().find(|(id, _)| *id == d)) {
                let pr = st.1.pressed;
                if pr.contains(Buttons::LEFT) || pr.contains(Buttons::RIGHT) {
                    if let Some(p) = &mut self.pause {
                        p.leave = !p.leave;
                    }
                    shared.sounds.play(audio, "navigateMenu");
                }
                if pr.contains(Buttons::ACCEPT) || pr.contains(Buttons::JUMP) {
                    shared.sounds.play(audio, "return");
                    if p.leave {
                        self.pres.stop_loops(&mut shared.sounds, audio);
                        return Ok(MatchOutcome::LeaveToMenu);
                    }
                    self.pause = None;
                } else if pr.contains(Buttons::BACK) || pr.contains(Buttons::SELECT) {
                    shared.sounds.play(audio, "return");
                    self.pause = None;
                }
            }
        } else if !winner {
            for (i, src) in self.sources.iter().enumerate() {
                let Some(d) = &src.device() else { continue };
                let Some((_, st)) = devices.iter().find(|(id, _)| id == d) else {
                    continue;
                };
                let open = if *d == DeviceId::KeyboardMouse {
                    st.pressed.contains(Buttons::SELECT)
                } else {
                    st.pressed.contains(Buttons::START)
                };
                if open {
                    self.pause = Some(Pause {
                        player: i,
                        leave: false,
                        opened: 0.0,
                    });
                    inputs[i] = PlayerInput::default();
                    shared.sounds.play(audio, "return");
                    break;
                }
            }
        }
        // Hovering a wheel row selects it (`HoverCircle`, mouse players):
        // the row the pointer entered becomes the target, reached with the
        // wheel's W / S edges (one key every other tick).
        for (i, src) in self.sources.iter().enumerate() {
            if src.device() != Some(DeviceId::KeyboardMouse) {
                continue;
            }
            let Some((_, st)) = devices.iter().find(|(d, _)| *d == DeviceId::KeyboardMouse) else {
                continue;
            };
            let wheel = self.view.flow.players.get(i).map(|p| p.wheel);
            let hovered = match (st.pointer, wheel) {
                (Some(ptr), Some(w)) if w.open => self.ui.wheel_row_hit(i, ptr, self.size),
                _ => None,
            };
            if hovered != self.wheel_hover {
                self.wheel_hover = hovered;
                if hovered.is_some() {
                    self.wheel_target = hovered;
                }
            }
            match (self.wheel_target, wheel) {
                (Some(t), Some(w)) if w.open && t != w.row as usize => {
                    if !std::mem::take(&mut self.wheel_key_down) {
                        inputs[i].keys |= if t < w.row as usize { KEY_W } else { KEY_S };
                        self.wheel_key_down = true;
                    }
                }
                _ => {
                    self.wheel_target = None;
                    self.wheel_key_down = false;
                }
            }
        }
        // Mouse clicks on the winner-screen prompts become the keyboard
        // player's Start ("next level") or Select ("ability select"); on
        // the ability wheel's arrows and handle the keys a mouse player's
        // wheel reads (A / D / W) for this tick.
        for (i, src) in self.sources.iter().enumerate() {
            if src.device() != Some(DeviceId::KeyboardMouse) {
                continue;
            }
            let Some((_, st)) = devices.iter().find(|(d, _)| *d == DeviceId::KeyboardMouse) else {
                continue;
            };
            let Some(ptr) = st
                .pointer
                .filter(|_| st.pressed.contains(Buttons::ABILITY0))
            else {
                continue;
            };
            let prompt = self
                .view
                .flow
                .winner_screen_at
                .and_then(|_| self.ui.prompt_hit(ptr, self.size));
            if let Some(next) = prompt {
                inputs[i].buttons |= if next { BTN_START } else { BTN_SELECT };
            } else if let Some(k) = self.ui.wheel_hit(i, ptr, self.size) {
                let open = self.view.flow.players.get(i).is_some_and(|p| p.wheel.open);
                inputs[i].keys |= match k {
                    -1 => KEY_A,
                    1 => KEY_D,
                    _ if !open => KEY_W,
                    _ => 0,
                };
            }
        }
        let generation = self.sim.level_generation;
        let gated = match (net, self.online.as_mut()) {
            (Some(net), Some(om)) if net.frames_ahead() > 0 && self.ticks % 10 == 9 => {
                // Ahead of the others: run ~10 % slower (one tick in ten
                // without a frame) so that GGRS's time sync settles.
                let _ = om;
                Some(Vec::new())
            }
            (Some(net), Some(om)) => {
                let local: Vec<(usize, PlayerInput)> = self
                    .sources
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| !matches!(s, Source::Remote))
                    .map(|(i, _)| (i, inputs.get(i).copied().unwrap_or_default()))
                    .collect();
                let events = om.step(net, &local);
                self.sim.clone_from(&om.game.sim);
                Some(events)
            }
            _ => {
                self.sim.advance(&inputs)?;
                None
            }
        };
        self.last_inputs = inputs;
        self.ticks += 1;
        if self.sim.level_generation != generation {
            log::info!(
                "tick {}: level {} loaded",
                self.sim.world.time.simulation_ticks,
                self.sim.level
            );
            self.pres
                .set_level(self.sim.level, self.sim.level_generation)?;
            self.ui.level_started();
            shared.sounds.match_started(audio);
        }
        self.view = self.sim.world.view();
        let events = gated.unwrap_or_else(|| self.view.events.clone());
        for e in &events {
            match &e.cue {
                bopl_sim::view::Cue::RoundWon { team } => {
                    log::info!("tick {}: round won by team {team}", e.id.tick)
                }
                bopl_sim::view::Cue::RoundDraw => log::info!("tick {}: draw", e.id.tick),
                bopl_sim::view::Cue::WinnerScreen => {
                    log::info!("tick {}: winner screen", e.id.tick)
                }
                bopl_sim::view::Cue::PlayerDied { id, cause, .. } => {
                    log::info!("tick {}: player {id} died ({cause:?})", e.id.tick)
                }
                _ => {}
            }
        }
        self.pres.observe_events(
            &self.sim.world,
            &self.view,
            &events,
            self.online.is_none(),
            &mut shared.sounds,
            audio,
        );
        self.ui.observe(&self.view, &mut shared.sounds, audio);
        self.rumble(shared, &events);
        if self.sim.ended {
            self.pres.stop_loops(&mut shared.sounds, audio);
            return Ok(MatchOutcome::BackToSelect);
        }
        Ok(MatchOutcome::Continue)
    }

    /// Controller rumble (ui.md 3.1, match-flow.md 9): big for a player
    /// who was hit (each death of the bopl or a clone), small for the
    /// killer, big while a beam is being entered; requests go to the
    /// window host through `Shared::rumble`.
    fn rumble(&mut self, shared: &mut Shared, events: &[bopl_sim::view::Event]) {
        use bopl_sim::view::{Cue, EffectKind};
        let mode = shared.settings.rumble;
        if mode == crate::shared::Rumble::Off {
            return;
        }
        let n = self.view.players.len();
        self.rumble.resize(n, RumbleState::default());
        let index = |id: i32| self.view.players.iter().position(|p| p.id == id);
        let set = |i: Option<usize>, k: RumbleKind, r: &mut Vec<RumbleState>| {
            if let Some(i) = i
                && (k == RumbleKind::Big || r[i].state != RumbleKind::Big)
            {
                r[i].state = k;
            }
        };
        for e in events {
            match &e.cue {
                Cue::Effect {
                    kind: EffectKind::Death | EffectKind::BlastZone | EffectKind::WaterSplash,
                    player_id,
                    ..
                } => set(index(*player_id), RumbleKind::Big, &mut self.rumble),
                Cue::PlayerDied { id, killer, .. } if killer != id => {
                    set(index(*killer), RumbleKind::Small, &mut self.rumble)
                }
                _ => {}
            }
        }
        // Beam: big while entering (not frozen by a time stop).
        if !self.view.round.time_stopped {
            use bopl_sim::abilities::AbilityState;
            use bopl_sim::abilities::beam::BeamState;
            use bopl_sim::world::Kind;
            let w = &self.sim.world;
            for id in w.objects.ids() {
                if let Some(o) = w.objects.get(id)
                    && let Kind::Ability(a) = &o.kind
                    && let AbilityState::Beam(b) = &a.state
                    && a.fw.in_use
                    && b.state == BeamState::Enter
                {
                    set(
                        index(w.objects.owner_id(id)),
                        RumbleKind::Big,
                        &mut self.rumble,
                    );
                }
            }
        }
        // Build tension (from the abilities' state; no sim change).
        let mut tension = vec![false; n];
        {
            use bopl_sim::abilities::AbilityState;
            use bopl_sim::abilities::meteor::State as MeteorState;
            use bopl_sim::world::Kind;
            let w = &self.sim.world;
            for id in w.objects.ids() {
                let Some(o) = w.objects.get(id) else { continue };
                let Kind::Ability(a) = &o.kind else { continue };
                if !a.fw.in_use {
                    continue;
                }
                let on = match &a.state {
                    AbilityState::Roll(r) => r.is_charging && !r.has_charged,
                    AbilityState::Meteor(m) => {
                        matches!(m.state, MeteorState::Vibrate | MeteorState::Dash)
                    }
                    AbilityState::TimeStop(t) => {
                        t.has_started_unstable && !self.view.round.time_stopped
                    }
                    _ => false,
                };
                if on && let Some(i) = index(w.objects.owner_id(id)) {
                    tension[i] = true;
                }
            }
        }
        for (r, &t) in self.rumble.iter_mut().zip(&tension) {
            if matches!(r.state, RumbleKind::None | RumbleKind::Tension) {
                r.state = if t {
                    RumbleKind::Tension
                } else {
                    RumbleKind::None
                };
            }
        }
        let k = if mode == crate::shared::Rumble::High {
            3.0
        } else {
            1.0
        };
        for (i, r) in self.rumble.iter_mut().enumerate() {
            let dev = match self.sources.get(i).and_then(Source::device) {
                Some(d @ DeviceId::Controller(_)) => Some(d),
                _ => None,
            };
            if r.state == RumbleKind::Tension {
                // Continuous while it lasts (re-sent before SDL's duration ends).
                if r.prev != RumbleKind::Tension || r.time >= 4.0 {
                    r.time = 0.0;
                    if let Some(d) = dev {
                        let (lo, hi) = RUMBLE_TENSION;
                        shared.rumble.push((d, lo * k, hi * k, 5000));
                    }
                }
                r.time += 1.0 / 60.0;
                r.prev = r.state;
                continue;
            }
            if r.prev == RumbleKind::Tension && r.state == RumbleKind::None {
                r.time = 0.0;
                if let Some(d) = dev {
                    shared.rumble.push((d, 0.0, 0.0, 0));
                }
            }
            let active = r.state != RumbleKind::None;
            if active && matches!(r.prev, RumbleKind::None | RumbleKind::Tension) {
                r.time = 0.0;
                let (lo, hi) = if r.state == RumbleKind::Small {
                    RUMBLE_SMALL
                } else {
                    RUMBLE_BIG
                };
                if let Some(d) = dev {
                    let ms = (RUMBLE_SECONDS * 1000.0) as u32 + 100;
                    shared
                        .rumble
                        .push((d, (lo * k).min(1.0), (hi * k).min(1.0), ms));
                }
            }
            r.time = if active { r.time + 1.0 / 60.0 } else { 0.0 };
            if r.time > RUMBLE_SECONDS {
                if let Some(d) = dev {
                    shared.rumble.push((d, 0.0, 0.0, 0));
                }
                r.state = RumbleKind::None;
            }
            r.prev = r.state;
        }
    }

    pub fn draw(&mut self, shared: &mut Shared, ctx: &mut DrawContext<'_>) {
        self.size = ctx.size;
        self.pres.aspect = ctx.size.0 as f32 / ctx.size.1.max(1) as f32;
        let world = self.pres.world_view(
            &self.view,
            &mut crate::fx::Gfx {
                gpu: ctx.gpu,
                renderer: ctx.renderer,
                assets: &mut shared.assets,
            },
        );
        let (frame, _targets) =
            self.pres
                .level_view
                .frame(&world, ctx.size, ctx.gpu, ctx.renderer, &mut shared.assets);
        let mut frame: Frame = frame;
        let quads =
            self.pres
                .indicator_quads(&self.view, ctx.gpu, ctx.renderer, &mut shared.assets);
        if let Some(main) = frame
            .passes
            .iter_mut()
            .rev()
            .find(|p| p.target == PassTarget::Output)
        {
            for q in quads {
                main.push(q);
            }
        }
        self.last_world = world;
        self.ui.draw(
            shared,
            &self.view,
            self.pause,
            &self.sim.world,
            ctx.gpu,
            ctx.renderer,
            ctx.size,
            &mut frame,
        );
        *ctx.frame = frame;
    }
}

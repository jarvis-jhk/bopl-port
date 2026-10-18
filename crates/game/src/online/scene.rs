//! The online screen: connecting to Warpzone, hosting or joining a room,
//! the lobby (character select with the local boxes, remote players'
//! boxes from the host's lobby state, the room code as title, a status
//! line) and the match (the usual match scene driven by bopl-net).
//!
//! Character select is the lobby (docs/net.md, "Hook points"): each local
//! box is one `SlotRequest` (colour, team if picked, `extra` = abilities and
//! input kind), re-sent whenever it changes; the machine is ready when all
//! its boxes are. The host sees the start button once every machine is
//! ready and at least two teams play (or one player). Mid-game ability
//! selection and the winner screen are sim inputs; "back to character
//! select" ends the bopl-net match and everybody returns here with their
//! choices (abilities as edited mid-game).

use anyhow::Result;
use bopl_net::warpzone_core::client::{Event, HostOptions};
use bopl_net::warpzone_core::protocol::api::{Entry, Visibility};
use bopl_net::{
    MatchSettings, MatchStart, NetConfig, NetEvent, NetSession, PlayerSlot, SlotRequest, TeamMode,
};

use super::adapter::{NetWorld, OnlineSettings, SlotExtra, new_world};
use super::runner::OnlineMatch;
use crate::app::DrawContext;
use crate::audio::Audio;
use crate::charsel::{CharSelect, CharSelectOutcome, Lobby, OnlineView, RemoteBox};
use crate::input::{DeviceId, InputState};
use crate::match_scene::{MatchOutcome, MatchScene, Source};
use crate::shared::Shared;

/// What to do online (main menu, command line, launcher).
#[derive(Clone, Debug, PartialEq)]
pub enum OnlineIntent {
    /// Host a room (`private`: unlisted, joined by code or invite).
    Host { private: bool },
    /// Join by code (`K7QM3X`) or room id / invite link.
    Join(String),
}

impl OnlineIntent {
    /// `--warpzone-join ROOM|CODE|LINK`, `--warpzone-host`, invite links,
    /// `WARPZONE_JOIN`, `WARPZONE_HOST=1` (the descriptor's launch args).
    pub fn from_args(args: &[String]) -> Option<Self> {
        let i = bopl_net::warpzone_core::launch::from_args(args);
        if let Some(j) = i.join {
            // Keep the secret of an invite link: join through the original
            // argument.
            let raw = args
                .iter()
                .position(|a| a == "--warpzone-join")
                .and_then(|k| args.get(k + 1))
                .cloned()
                .or_else(|| {
                    args.iter()
                        .find(|a| bopl_net::warpzone_core::launch::parse_invite(a).is_some())
                        .cloned()
                })
                .unwrap_or(j);
            return Some(Self::Join(raw));
        }
        i.host.then_some(Self::Host { private: false })
    }
}

pub enum OnlineOutcome {
    Stay,
    /// Back to the main menu (left the room), with a message for the
    /// menu's error line (e.g. why the host refused us).
    Leave(Option<String>),
}

/// The player's Warpzone name if the identity has none yet.
pub fn default_name() -> String {
    let u = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let u: String = u
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(10)
        .collect();
    let base = if u.len() >= 3 { u } else { "bopl".into() };
    let n = std::process::id() % 1000;
    format!("{base}{n}")
}

pub struct OnlineScreen {
    pub net: NetSession<NetWorld>,
    select: Box<CharSelect>,
    pub game: Option<Box<MatchScene>>,
    intent: OnlineIntent,
    requested: bool,
    logged_in: bool,
    name: String,
    code: Option<String>,
    invite: Option<String>,
    status: String,
    sent_slots: Option<Vec<SlotRequest>>,
    sent_ready: bool,
    settings_sent: bool,
    leaving: bool,
    /// Slot table of the running match (to map local slots to boxes).
    current: Option<MatchStart>,
    /// The chosen player name (settings), else the Warpzone account's.
    chosen_name: Option<String>,
    /// Why we leave (shown on the main menu).
    error: Option<String>,
    content_id: String,
    ticks: u64,
    t0: std::time::Instant,
    spectating: bool,
}

impl OnlineScreen {
    pub fn new(shared: &mut Shared, intent: OnlineIntent) -> Result<Self> {
        let mut cfg = NetConfig::new(&super::net_version());
        let chosen = shared.settings.player_name.trim().to_string();
        let name = if chosen.is_empty() {
            default_name()
        } else {
            chosen.clone()
        };
        cfg.auto_register = Some(name.chars().filter(|c| c.is_ascii_alphanumeric()).collect());
        let content_id = super::local_content_id(&shared.data);
        log::info!("game data id {content_id}");
        cfg.content_id = content_id.clone();
        let net = NetSession::new(cfg).map_err(|e| anyhow::anyhow!("warpzone: {e}"))?;
        let select = Box::new(CharSelect::new(shared, &Lobby::default()));
        Ok(Self {
            net,
            select,
            game: None,
            intent,
            requested: false,
            logged_in: false,
            name,
            code: None,
            invite: None,
            status: String::new(),
            ticks: 0,
            t0: std::time::Instant::now(),
            spectating: false,
            sent_slots: None,
            sent_ready: false,
            settings_sent: false,
            leaving: false,
            current: None,
            chosen_name: (!chosen.is_empty()).then_some(chosen),
            error: None,
            content_id,
        })
    }

    /// Machines in the room (driver condition).
    pub fn machines(&self) -> usize {
        self.net.lobby().machines.len()
    }

    fn host_settings(&mut self, shared: &Shared) {
        if !self.net.is_host() {
            return;
        }
        let os = OnlineSettings {
            number_of_abilities: shared.settings.number_of_abilities.clamp(1, 3) as u8,
            first_level: None,
        };
        let players = self.net.lobby().preview.len();
        let teams = distinct_teams(&self.net.lobby().preview);
        let s = MatchSettings {
            team_mode: TeamMode::Teams(12),
            mode: if teams == players {
                "free-for-all".into()
            } else {
                format!("{teams} teams")
            },
            map: "Level1".into(),
            game: os.encode(),
        };
        if self.net.lobby().settings != s && self.net.set_settings(s).is_ok() {
            self.settings_sent = true;
        }
    }

    fn handle_events(&mut self, shared: &mut Shared, audio: &mut Audio) {
        self.net.poll();
        while let Some(e) = self.net.next_event() {
            match e {
                NetEvent::Warpzone(b) => match *b {
                    Event::LoggedIn {
                        account: Some(a), ..
                    } => {
                        log::info!("warpzone: logged in as {}", a.username);
                        self.name = self.chosen_name.clone().unwrap_or(a.username);
                        self.logged_in = true;
                    }
                    Event::Registered(a) => {
                        log::info!("warpzone: registered {}", a.username);
                        self.name = self.chosen_name.clone().unwrap_or(a.username);
                        self.logged_in = true;
                    }
                    Event::Error { op, code, message } => {
                        log::warn!("warpzone error {op}/{code}: {message}");
                        if op == "register" {
                            let base: String = self
                                .name
                                .chars()
                                .filter(|c| c.is_ascii_alphanumeric())
                                .take(10)
                                .collect();
                            let n = format!("{base}{}", std::process::id() % 97);
                            self.net.register(&n);
                        } else if op == "join" {
                            self.error = Some(format!("cannot join: {message}"));
                            self.leaving = true;
                        } else {
                            self.status = format!("error: {message}");
                        }
                    }
                    Event::RoomHosted { room, code, invite } => {
                        println!("ROOM {} {}", room.id, code.clone().unwrap_or("-".into()));
                        log::info!("hosting room {} code {code:?} invite {invite}", room.id);
                        self.code = code.or(Some(room.id.clone()));
                        self.invite = Some(invite);
                    }
                    Event::RoomJoined { room, host } => {
                        log::info!("in room {} (host: {host})", room.id);
                        if self.code.is_none() {
                            self.code = Some(match &self.intent {
                                OnlineIntent::Join(v) if super::is_code(v) => {
                                    v.to_ascii_uppercase()
                                }
                                _ => room.id.clone(),
                            });
                        }
                    }
                    _ => {}
                },
                NetEvent::Welcome { machine } => log::info!("welcome as {machine}"),
                NetEvent::Rejected { reason } => {
                    log::warn!("rejected by the host: {reason}");
                    let m = super::reject_message(&reason, &self.content_id);
                    self.status = m.clone();
                    self.error = Some(m);
                    self.leaving = true;
                }
                NetEvent::MatchStarted(ms) => {
                    if let Err(e) = self.start_game(shared, ms, audio) {
                        log::error!("starting the online match: {e:#}");
                        self.status = format!("error: {e}");
                    }
                }
                NetEvent::WaitingForNextMatch { .. } => {
                    self.status = "a match is running: you play from the next one".into()
                }
                NetEvent::MatchFrozen { .. } => self.status = "a player left, resyncing...".into(),
                NetEvent::PlayersDropped { slots, .. } => {
                    log::info!("players dropped: slots {slots:?}");
                }
                NetEvent::Desync { frame, machine, .. } => {
                    log::warn!("desync with {machine} at frame {frame}")
                }
                NetEvent::MatchEnded { agreed, .. } => {
                    log::info!("online match ended (checksums agree: {agreed})");
                    self.back_to_lobby(shared, audio);
                    if !agreed {
                        self.status = "warning: the machines disagreed (desync)".into();
                    }
                }
                NetEvent::MatchAborted { reason } => {
                    log::warn!("online match aborted: {reason}");
                    self.back_to_lobby(shared, audio);
                    self.status = format!("match aborted: {reason}");
                }
                NetEvent::RoomLeft { reason } => {
                    log::info!("left the room: {reason}");
                    if self.error.is_none() && !self.net.is_host() && self.game.is_none() {
                        self.error = Some(format!("left the room: {reason}"));
                    }
                    self.leaving = true;
                }
                _ => {}
            }
        }
    }

    /// Builds the match from the host's `MatchStart`.
    fn start_game(&mut self, shared: &mut Shared, ms: MatchStart, audio: &mut Audio) -> Result<()> {
        let world = new_world(&ms, shared.levels.clone())?;
        let me = self.net.me();
        let locals = self.select.local_boxes();
        let sources = ms
            .players
            .iter()
            .map(|p| {
                if Some(p.machine) == me
                    && let Some(b) = locals.get(p.local_index as usize)
                {
                    Source::Device(b.device)
                } else {
                    Source::Remote
                }
            })
            .collect();
        let om = OnlineMatch::new(world);
        let scene = MatchScene::new_online(shared, om, sources, audio)?;
        self.current = Some(ms);
        self.game = Some(Box::new(scene));
        self.status.clear();
        Ok(())
    }

    /// The match ended: back to the lobby with the mid-game abilities.
    fn back_to_lobby(&mut self, shared: &mut Shared, audio: &mut Audio) {
        let Some(game) = self.game.take() else {
            return;
        };
        let me = self.net.me();
        let mut abilities = Vec::new();
        if let Some(ms) = &self.current {
            let w = &game.sim.world;
            let mut mine: Vec<&PlayerSlot> = ms
                .players
                .iter()
                .filter(|p| Some(p.machine) == me)
                .collect();
            mine.sort_by_key(|p| p.local_index);
            for p in mine {
                let mut a = [4, 7, 2];
                if let Some(wp) = w.players.get(p.slot as usize) {
                    for (k, x) in wp.abilities.iter().take(3).enumerate() {
                        a[k] = *x;
                    }
                }
                abilities.push(a);
            }
        }
        self.select.back_from_match(&abilities);
        self.current = None;
        shared.sounds.play_menu_music(audio);
    }

    /// The local boxes as slot requests, and whether they are all ready.
    fn requests(&self) -> (Vec<SlotRequest>, bool) {
        let boxes = self.select.local_boxes();
        let ready = !boxes.is_empty() && boxes.iter().all(|b| b.ready);
        let reqs = boxes
            .iter()
            .enumerate()
            .map(|(k, b)| SlotRequest {
                name: if boxes.len() > 1 {
                    format!("{} {}", self.name, k + 1)
                } else {
                    self.name.clone()
                },
                colour: b.color.clamp(0, 11) as u8,
                team: b.team.map(|t| t.clamp(0, 11) as u8),
                extra: SlotExtra {
                    abilities: b.abilities,
                    mouse: b.device == DeviceId::KeyboardMouse,
                }
                .encode(),
            })
            .collect();
        (reqs, ready)
    }

    fn lobby_view(&self, shared: &Shared) -> OnlineView {
        let lobby = self.net.lobby();
        let me = self.net.me();
        let ability_count = shared.levels.content.abilities.len();
        let remote = lobby
            .preview
            .iter()
            .filter(|p| Some(p.machine) != me)
            .map(|p| {
                let x = SlotExtra::decode(&p.extra, ability_count);
                RemoteBox {
                    name: p.name.clone(),
                    color: p.colour as i32,
                    team: p.team as i32,
                    abilities: x.abilities,
                    ready: lobby
                        .machines
                        .iter()
                        .any(|m| m.machine == p.machine && m.ready),
                }
            })
            .collect();
        let mut resolved: Vec<Option<(i32, i32)>> = Vec::new();
        for p in lobby.preview.iter().filter(|p| Some(p.machine) == me) {
            let k = p.local_index as usize;
            if resolved.len() <= k {
                resolved.resize(k + 1, None);
            }
            resolved[k] = Some((p.colour as i32, p.team as i32));
        }
        let all_ready =
            !lobby.machines.is_empty() && lobby.machines.iter().all(|m| m.ready || m.spectate);
        let players = lobby.preview.len();
        let teams = distinct_teams(&lobby.preview);
        let can_start = self.net.is_host() && all_ready && (teams >= 2 || players == 1);
        let title = match &self.code {
            Some(c) => format!("online  {c}"),
            None => "online".into(),
        };
        let status = if !self.status.is_empty() {
            self.status.clone()
        } else if !self.net.in_room() {
            if self.logged_in {
                "opening the room...".into()
            } else {
                "connecting to warpzone...".into()
            }
        } else {
            let n = lobby.machines.len();
            let waiting = lobby.machines.iter().filter(|m| !m.ready).count();
            let who = if self.net.is_host() {
                if can_start {
                    "everybody is ready: start!".to_string()
                } else if waiting > 0 {
                    format!("waiting for {waiting} not ready")
                } else if players > 1 && teams < 2 {
                    "pick at least two teams".into()
                } else {
                    "join with a device, then ready".into()
                }
            } else if all_ready {
                "waiting for the host to start".into()
            } else {
                format!("waiting for {waiting} not ready")
            };
            format!(
                "{n} machine{} · {players} player{} · {who}",
                if n == 1 { "" } else { "s" },
                if players == 1 { "" } else { "s" }
            )
        };
        OnlineView {
            title,
            status,
            can_start,
            remote,
            resolved,
            n_abilities: OnlineSettings::decode(&lobby.settings.game)
                .map(|s| s.number_of_abilities as usize)
                .unwrap_or(shared.settings.number_of_abilities),
        }
    }

    pub fn tick(
        &mut self,
        shared: &mut Shared,
        devices: &[(DeviceId, InputState)],
        raw: &[crate::input::RawPress],
        audio: &mut Audio,
    ) -> OnlineOutcome {
        self.handle_events(shared, audio);
        if self.leaving {
            self.net.leave();
            return OnlineOutcome::Leave(self.error.take());
        }
        if self.logged_in && !self.requested {
            self.requested = true;
            match self.intent.clone() {
                OnlineIntent::Host { private } => self.net.host(HostOptions {
                    max_players: 12,
                    visibility: if private {
                        Visibility::Unlisted
                    } else {
                        Visibility::Public
                    },
                    entry: Entry::Open,
                    preset: Some("ffa".into()),
                    ..Default::default()
                }),
                OnlineIntent::Join(v) => super::join(&mut self.net, &v),
            }
        }
        self.ticks += 1;
        if self.ticks.is_multiple_of(600) && self.game.is_some() {
            let st = self.net.stats();
            log::info!(
                "online: {:.1}s, frame {} confirmed {} ahead {} rollbacks {} resim {} stalls {}",
                self.t0.elapsed().as_secs_f32(),
                st.current_frame,
                st.confirmed_frame,
                st.frames_ahead,
                st.rollbacks,
                st.resimulated_frames,
                st.prediction_stalls
            );
        }
        // The match.
        if let Some(game) = self.game.as_mut() {
            match game.tick_online(shared, devices, audio, &mut self.net) {
                Ok(MatchOutcome::LeaveToMenu) => {
                    self.net.leave();
                    return OnlineOutcome::Leave(None);
                }
                Ok(_) => {}
                Err(e) => log::error!("online match: {e:#}"),
            }
            return OnlineOutcome::Stay;
        }
        // The lobby.
        self.host_settings(shared);
        let (reqs, ready) = self.requests();
        if self.sent_slots.as_ref() != Some(&reqs) {
            log::info!(
                "lobby: my slots {:?}",
                reqs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>()
            );
            self.net.set_local_slots(reqs.clone());
            self.sent_slots = Some(reqs);
        }
        if self.sent_ready != ready {
            self.net.set_ready(ready);
            self.sent_ready = ready;
        }
        // A guest without players watches the matches (the host must play).
        let spectate =
            !self.net.is_host() && self.sent_slots.as_ref().is_some_and(|r| r.is_empty());
        if self.spectating != spectate && self.net.in_room() {
            self.net.set_spectate(spectate);
            self.spectating = spectate;
        }
        let mut view = self.lobby_view(shared);
        // Streamer mode (settings "hide"): remote names as "player N".
        if shared.settings.hide != crate::shared::Hide::Nothing {
            for (k, r) in view.remote.iter_mut().enumerate() {
                r.name = format!("player {}", k + 1);
            }
        }
        self.select.set_online(view);
        match self.select.tick(shared, devices, raw, audio) {
            CharSelectOutcome::Stay => {}
            CharSelectOutcome::Back => {
                self.net.leave();
                return OnlineOutcome::Leave(None);
            }
            CharSelectOutcome::Start(_) => {
                if let Err(e) = self.net.start_match(None) {
                    log::warn!("start: {e}");
                    self.status = format!("cannot start: {e}");
                }
            }
        }
        OnlineOutcome::Stay
    }

    pub fn draw(&mut self, shared: &mut Shared, ctx: &mut DrawContext<'_>) {
        match self.game.as_mut() {
            Some(g) => g.draw(shared, ctx),
            None => self.select.draw(shared, ctx),
        }
    }

    /// Leave the room (quitting the app).
    pub fn leave(&mut self) {
        self.net.leave();
    }
}

fn distinct_teams(slots: &[PlayerSlot]) -> usize {
    let mut t: Vec<u8> = slots.iter().map(|p| p.team).collect();
    t.sort();
    t.dedup();
    t.len()
}

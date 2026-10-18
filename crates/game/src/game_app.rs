//! The game: main menu → character select → match → winner screen → …,
//! as one [`Scene`] for the window or headless host. Also the sandbox and
//! bot-match modes for tests and screenshots.

use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{Context, Result};
use bopl_import::GameData;
use bopl_render::Gpu;
use bopl_sim::player::PlayerSetup;
use bopl_sim::world::MatchSettings;

use crate::app::{DrawContext, LoadContext, Scene, TickContext};
use crate::bots::{Bot, FlowPolicy};
use crate::charsel::{CharSelect, CharSelectOutcome, Lobby};
use crate::driver::Driver;
use crate::input::{DeviceId, InputState};
use crate::match_scene::{MatchOutcome, MatchScene, Source};
use crate::menu::{MainMenu, MenuOutcome};
use crate::online::scene::{OnlineIntent, OnlineOutcome, OnlineScreen};
use crate::shared::Shared;

/// How the app starts.
#[derive(Clone, Debug)]
pub enum Mode {
    /// The full game from the main menu.
    Game,
    /// One player on a level with scripted input (`--sandbox`); optional
    /// abilities (`--abilities`) and idle target players (`--players`, the
    /// second one scripted with `--script2`).
    Sandbox {
        level: usize,
        script: String,
        abilities: Option<Vec<usize>>,
        players: usize,
        script2: Option<String>,
    },
    /// One of the original's `.rep` replays (`--replay`): its level, seed,
    /// players and inputs (golden comparisons, docs/reference-capture.md).
    Replay { path: PathBuf },
    /// `n` bots on a level (`--bot-match`); next level when decided.
    BotMatch {
        players: usize,
        level: Option<usize>,
        seed: u32,
        teams: Option<usize>,
        /// Team sizes (`--split 1,11`), overrides `teams`.
        split: Option<Vec<usize>>,
    },
}

/// Options of the game app.
#[derive(Clone, Debug)]
pub struct GameOptions {
    pub mode: Mode,
    /// Screenshots after these ticks (`--shots 120,300`), named
    /// `<stem>-<tick>.png` next to `screenshot`.
    pub shots: Vec<u64>,
    pub screenshot: Option<PathBuf>,
    /// Quit after this many ticks (headless runs).
    pub quit_tick: Option<u64>,
    /// Headless scripted device input (`--drive FILE`).
    pub driver: Option<Driver>,
    pub music: bool,
    pub seed: u32,
    /// Screenshot every winner screen (`<stem>-winner-<n>.png`).
    pub winner_shots: bool,
    /// Quit after this many winner screens (with `winner_shots`).
    pub rounds: Option<u32>,
    /// Start online (`--warpzone-host`, `--warpzone-join`, launcher).
    pub online: Option<OnlineIntent>,
    /// `--camera-zoom`: the setting on for this run.
    pub camera_zoom: bool,
}

enum ScreenState {
    Menu(Box<MainMenu>),
    Select(Box<CharSelect>),
    Match(Box<MatchScene>),
    /// Online: lobby and match (`online::scene`).
    Online(Box<OnlineScreen>),
    /// The tutorial (`tutorial.rs`).
    Tutorial(Box<crate::tutorial::TutorialScene>),
    Empty,
}

pub struct GameApp {
    data: Rc<GameData>,
    opts: GameOptions,
    shared: Option<Shared>,
    screen: ScreenState,
    /// Players kept between character select and the matches.
    lobby: Lobby,
    ticks: u64,
    /// Screens shot so far (flow test names).
    pub shot_names: Vec<(u64, PathBuf)>,
    devices: Vec<(DeviceId, InputState)>,
    /// Winner screens seen (`winner_shots`) and the tick of the last one.
    winners_seen: u32,
    last_winner_at: Option<i64>,
    /// The cursor for the current screen was requested.
    cursor_sent: bool,
    quit_after_shot: bool,
}

/// `--winner-shots`: ticks after the winner screen appeared (the prompts
/// are in).
const WINNER_SHOT_DELAY: i64 = 150;

impl GameApp {
    pub fn new(data: Rc<GameData>, opts: GameOptions) -> Self {
        Self {
            data,
            opts,
            shared: None,
            screen: ScreenState::Empty,
            lobby: Lobby::default(),
            ticks: 0,
            shot_names: Vec::new(),
            devices: Vec::new(),
            winners_seen: 0,
            last_winner_at: None,
            cursor_sent: false,
            quit_after_shot: false,
        }
    }

    fn shot_path(&self, label: &str) -> Option<PathBuf> {
        let base = self.opts.screenshot.as_ref()?;
        let stem = base
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "shot".into());
        Some(base.with_file_name(format!("{stem}-{label}.png")))
    }

    fn start_match(
        &mut self,
        setups: Vec<PlayerSetup>,
        sources: Vec<Source>,
        level: Option<usize>,
        audio: &mut crate::audio::Audio,
    ) -> Result<()> {
        let shared = self.shared.as_mut().expect("loaded");
        let settings = MatchSettings {
            number_of_abilities: shared.settings.number_of_abilities,
            seed: self.opts.seed,
        };
        let m = MatchScene::new(shared, &setups, sources, settings, level, audio)?;
        self.screen = ScreenState::Match(Box::new(m));
        Ok(())
    }
}

/// Default setups for `n` players (bot matches): free-for-all, `teams`
/// teams (player `i` in team `i mod teams`) or explicit team sizes
/// (`split`, players in order), distinct colours, varied abilities.
pub fn bot_setups(
    n: usize,
    teams: Option<usize>,
    split: Option<&[usize]>,
    abilities: usize,
) -> Vec<PlayerSetup> {
    const COLORS: [i32; 12] = [0, 6, 2, 3, 4, 5, 1, 7, 8, 9, 10, 11];
    let split_team = |i: usize| -> Option<i32> {
        let mut acc = 0;
        for (t, &size) in split?.iter().enumerate() {
            acc += size;
            if i < acc {
                return Some(t as i32);
            }
        }
        None
    };
    (0..n)
        .map(|i| PlayerSetup {
            id: i as i32 + 1,
            team: match (split_team(i), teams) {
                (Some(t), _) => t,
                (None, Some(t)) => (i % t.max(1)) as i32,
                (None, None) => i as i32,
            },
            color: COLORS[i % 12],
            abilities: (0..3)
                .map(|k| 2 + (i * 3 + k * 7) % (abilities.max(3) - 2))
                .collect(),
            uses_keyboard_and_mouse: false,
        })
        .collect()
}

impl GameApp {
    /// What the headless driver may wait for.
    fn status(&self) -> crate::driver::Status {
        use bopl_sim::flow::Circle;
        let mut s = crate::driver::Status {
            screen: match &self.screen {
                ScreenState::Menu(_) => "menu",
                ScreenState::Select(_) => "charsel",
                ScreenState::Match(_) => "match",
                ScreenState::Online(o) if o.game.is_some() => "match",
                ScreenState::Online(_) => "charsel",
                ScreenState::Tutorial(_) => "tutorial",
                ScreenState::Empty => "",
            },
            ..Default::default()
        };
        if let ScreenState::Online(o) = &self.screen {
            s.machines = o.machines();
        }
        let m = match &self.screen {
            ScreenState::Match(m) => Some(&**m),
            ScreenState::Online(o) => o.game.as_deref(),
            _ => None,
        };
        if let Some(m) = m {
            let v = &m.view;
            s.round_live = v.round.in_progress;
            s.winner_ready = v
                .flow
                .winner_screen_at
                .is_some_and(|at| v.tick - at > bopl_sim::flow::WINNER_INPUT_DELAY_TICKS + 5);
            s.wheel_ready = v
                .flow
                .players
                .iter()
                .map(|p| match p.circle {
                    Circle::Loser { since, .. } => {
                        v.tick - since > bopl_sim::flow::WHEEL_DELAY_TICKS + 5 && !p.wheel.open
                    }
                    _ => false,
                })
                .collect();
            s.level_generation = m.sim.level_generation;
            s.remote_wheel_open = v
                .flow
                .players
                .iter()
                .enumerate()
                .any(|(i, p)| p.wheel.open && matches!(m.sources.get(i), Some(Source::Remote)));
            s.loser = v
                .flow
                .players
                .iter()
                .enumerate()
                .find(|(i, p)| s.wheel_ready.get(*i).copied().unwrap_or(false) || p.wheel.open)
                .and_then(|(i, _)| m.sources.get(i).and_then(|src| src.device()));
        }
        s
    }
}

impl Scene for GameApp {
    fn load(&mut self, ctx: &mut LoadContext<'_>) -> Result<()> {
        ctx.audio.set_master_volume(crate::sound::ENGINE_VOLUME);
        let mut shared = Shared::new(
            self.data.clone(),
            self.opts.driver.is_some()
                || matches!(
                    self.opts.mode,
                    Mode::Sandbox { .. } | Mode::Replay { .. } | Mode::BotMatch { .. }
                ),
        )?;
        if self.opts.camera_zoom {
            shared.settings.camera_zoom = true;
        }
        shared.sounds.master_volume = shared.settings.master_volume as f32;
        if !self.opts.music {
            shared.sounds.music_volume = 0.0;
        }
        shared.sounds.apply_volumes(ctx.audio);
        let ability_count = shared.levels.content.abilities.len();
        self.shared = Some(shared);
        match self.opts.mode.clone() {
            Mode::Game => {
                let shared = self.shared.as_mut().unwrap();
                shared.sounds.play_menu_music(ctx.audio);
                self.screen = match self.opts.online.clone() {
                    Some(intent) => {
                        ScreenState::Online(Box::new(OnlineScreen::new(shared, intent)?))
                    }
                    None => ScreenState::Menu(Box::new(MainMenu::new(shared))),
                };
            }
            Mode::Sandbox {
                level,
                script,
                abilities,
                players,
                script2,
            } => {
                let inputs = crate::bots::script(&script)?;
                let mut setups = vec![PlayerSetup {
                    id: 1,
                    team: 0,
                    color: 0,
                    abilities: abilities.unwrap_or_else(|| vec![4, 7, 2]),
                    uses_keyboard_and_mouse: false,
                }];
                let mut sources = vec![Source::Script { inputs, then: None }];
                for i in 1..players.clamp(1, 12) {
                    setups.push(PlayerSetup {
                        id: i as i32 + 1,
                        team: i as i32,
                        color: [6, 2, 3, 4, 5, 1, 7, 8, 9, 10, 11][(i - 1) % 11],
                        abilities: vec![4, 7, 2],
                        uses_keyboard_and_mouse: false,
                    });
                    let s = match (&script2, i) {
                        (Some(s), 1) => s.clone(),
                        _ => "1:".to_string(),
                    };
                    sources.push(Source::Script {
                        inputs: crate::bots::script(&s)?,
                        then: None,
                    });
                }
                self.start_match(setups, sources, Some(level), ctx.audio)?;
            }
            Mode::Replay { path } => {
                let r = bopl_content::replay::Replay::parse(
                    &std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
                )?;
                let sources = (0..r.players.len())
                    .map(|i| Source::Script {
                        inputs: (0..r.frames.len()).map(|t| r.inputs(t)[i]).collect(),
                        then: None,
                    })
                    .collect();
                let shared = self.shared.as_mut().expect("loaded");
                let m = MatchScene::new(
                    shared,
                    &r.setups(),
                    sources,
                    r.settings(),
                    Some(r.build_index()),
                    ctx.audio,
                )?;
                self.screen = ScreenState::Match(Box::new(m));
            }
            Mode::BotMatch {
                players,
                level,
                seed,
                teams,
                split,
            } => {
                let setups = bot_setups(players, teams, split.as_deref(), ability_count);
                let sources = (0..players)
                    .map(|i| {
                        let mut b = Bot::new(i, seed);
                        b.policy = FlowPolicy::NextLevel;
                        Source::Bot(b)
                    })
                    .collect();
                self.start_match(setups, sources, level, ctx.audio)?;
            }
        }
        Ok(())
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        self.ticks += 1;
        // Device input: the real devices, or the headless driver's.
        self.devices.clear();
        let status = self.status();
        match &mut self.opts.driver {
            Some(d) => {
                d.advance(self.ticks, &status);
                let bindings = self
                    .shared
                    .as_ref()
                    .map(|s| s.settings.bindings.clone())
                    .unwrap_or_default();
                d.devices(self.ticks, &bindings, &mut self.devices);
                let m = match &mut self.screen {
                    ScreenState::Match(m) => Some(&mut **m),
                    ScreenState::Online(o) => o.game.as_deref_mut(),
                    _ => None,
                };
                if let Some(m) = m {
                    for &dev in d.bots() {
                        m.assist(dev, self.opts.seed);
                    }
                }
            }
            None => self.devices.extend(ctx.inputs.iter().cloned()),
        }
        let raw: Vec<crate::input::RawPress> = match &self.opts.driver {
            Some(d) => d.raw().to_vec(),
            None => ctx.raw.to_vec(),
        };
        let devices = std::mem::take(&mut self.devices);
        for (d, st) in &devices {
            if st.pressed.0 != 0 {
                log::debug!("tick {}: {d:?} pressed {:#x}", self.ticks, st.pressed.0);
            }
        }
        let Some(shared) = self.shared.as_mut() else {
            return;
        };
        shared.sounds.advance(1.0 / 60.0);
        shared.platform = ctx.platform.clone();
        if shared.bindings_changed {
            shared.bindings_changed = false;
            ctx.requests.bindings = Some(shared.settings.bindings.clone());
        }
        if std::mem::take(&mut shared.display_changed) {
            ctx.requests.display = Some(shared.settings.display());
        }
        let mut next: Option<ScreenState> = None;
        match &mut self.screen {
            ScreenState::Menu(m) => match m.tick(shared, &devices, &raw, ctx.audio) {
                MenuOutcome::Stay => {}
                MenuOutcome::Online(intent) => match OnlineScreen::new(shared, intent) {
                    Ok(o) => next = Some(ScreenState::Online(Box::new(o))),
                    Err(e) => log::error!("online: {e:#}"),
                },
                MenuOutcome::Tutorial(device) => {
                    match crate::tutorial::TutorialScene::new(shared, device, ctx.audio) {
                        Ok(t) => next = Some(ScreenState::Tutorial(Box::new(t))),
                        Err(e) => log::error!("tutorial: {e:#}"),
                    }
                }
                MenuOutcome::Play => {
                    next = Some(ScreenState::Select(Box::new(CharSelect::new(
                        shared,
                        &self.lobby,
                    ))));
                }
                MenuOutcome::Quit => ctx.requests.quit = true,
            },
            ScreenState::Select(s) => match s.tick(shared, &devices, &raw, ctx.audio) {
                CharSelectOutcome::Stay => {}
                CharSelectOutcome::Back => {
                    self.lobby = Lobby::default();
                    next = Some(ScreenState::Menu(Box::new(MainMenu::new(shared))));
                }
                CharSelectOutcome::Start(lobby) => {
                    self.lobby = lobby;
                    let (setups, sources) = self.lobby.setups(shared.settings.number_of_abilities);
                    let settings = MatchSettings {
                        number_of_abilities: shared.settings.number_of_abilities,
                        seed: self.opts.seed,
                    };
                    match MatchScene::new(shared, &setups, sources, settings, None, ctx.audio) {
                        Ok(m) => next = Some(ScreenState::Match(Box::new(m))),
                        Err(e) => log::error!("starting the match: {e:#}"),
                    }
                }
            },
            ScreenState::Match(m) => match m.tick(shared, &devices, ctx.audio) {
                Ok(MatchOutcome::Continue) => {}
                Ok(MatchOutcome::BackToSelect) => {
                    // Keep the players with their (mid-game edited) abilities
                    // and their devices (a re-plugged controller).
                    self.lobby.absorb(&m.sim.world);
                    for (p, src) in self.lobby.players.iter_mut().zip(&m.sources) {
                        if let Source::Device(d) = src {
                            p.device = *d;
                        }
                    }
                    shared.sounds.play_menu_music(ctx.audio);
                    next = Some(ScreenState::Select(Box::new(CharSelect::new(
                        shared,
                        &self.lobby,
                    ))));
                }
                Ok(MatchOutcome::LeaveToMenu) => {
                    self.lobby.absorb(&m.sim.world);
                    shared.sounds.play_menu_music(ctx.audio);
                    next = Some(ScreenState::Menu(Box::new(MainMenu::new(shared))));
                }
                Err(e) => {
                    log::error!("match: {e:#}");
                    next = Some(ScreenState::Menu(Box::new(MainMenu::new(shared))));
                }
            },
            ScreenState::Online(o) => match o.tick(shared, &devices, &raw, ctx.audio) {
                OnlineOutcome::Stay => {}
                OnlineOutcome::Leave(error) => {
                    shared.sounds.play_menu_music(ctx.audio);
                    let mut m = MainMenu::new(shared);
                    m.error = error;
                    next = Some(ScreenState::Menu(Box::new(m)));
                }
            },
            ScreenState::Tutorial(t) => {
                if let crate::tutorial::TutorialOutcome::Leave =
                    t.tick(shared, &devices, &raw, ctx.audio)
                {
                    shared.sounds.play_menu_music(ctx.audio);
                    next = Some(ScreenState::Menu(Box::new(MainMenu::new(shared))));
                }
            }
            ScreenState::Empty => {}
        }
        if let Some(n) = next {
            self.screen = n;
            self.cursor_sent = false;
        }
        // CursorUpdater (main menu, character select, tutorial, online
        // select): the cursor texture recoloured once per scene start; it
        // stays during matches.
        if !self.cursor_sent {
            self.cursor_sent = true;
            let color = match &self.screen {
                ScreenState::Menu(_) | ScreenState::Tutorial(_) | ScreenState::Online(_) => {
                    Some(None)
                }
                ScreenState::Select(_) => Some(
                    self.lobby
                        .players
                        .iter()
                        .find(|p| p.device == crate::input::DeviceId::KeyboardMouse)
                        .map(|p| p.color),
                ),
                _ => None,
            };
            if let Some(player) = color {
                ctx.requests.cursor = cursor_image(shared, player);
            }
        }
        // Long-run health (`RUST_LOG=info`): every minute, the sizes of the
        // presentation's collections and the process's memory.
        if self.ticks.is_multiple_of(3600) {
            let m = match &self.screen {
                ScreenState::Match(m) => Some(&**m),
                ScreenState::Online(o) => o.game.as_deref(),
                _ => None,
            };
            log::info!(
                "health: tick {} rss {} KiB loops {}{}",
                self.ticks,
                crate::platform::rss_kib().unwrap_or(0),
                shared.sounds.loops_playing(),
                m.map(|m| format!(" {}", m.pres.stats()))
                    .unwrap_or_default()
            );
        }
        // Loops, music ducking and volumes (the original's LateUpdate).
        shared.sounds.update(ctx.audio, 1.0 / 60.0);
        ctx.requests.rumble.append(&mut shared.rumble);
        self.devices = devices;
        // Screenshots.
        if self.opts.winner_shots
            && let ScreenState::Match(m) = &self.screen
            && let Some(at) = m.view.flow.winner_screen_at
            && self.last_winner_at != Some(at)
            && m.view.tick - at >= WINNER_SHOT_DELAY
        {
            self.last_winner_at = Some(at);
            self.winners_seen += 1;
            if let Some(p) = self.shot_path(&format!("winner-{}", self.winners_seen)) {
                ctx.requests.screenshot = Some(p);
            }
            if self.opts.rounds.is_some_and(|r| self.winners_seen >= r) {
                self.quit_after_shot = true;
            }
        } else if self.quit_after_shot {
            ctx.requests.quit = true;
        }
        if self.opts.shots.contains(&self.ticks)
            && let Some(p) = self.shot_path(&format!("{}", self.ticks))
        {
            ctx.requests.screenshot = Some(p);
        }
        if let Some(label) = self
            .opts
            .driver
            .as_ref()
            .and_then(|d| d.shot().map(str::to_string))
            && let Some(p) = self.shot_path(&label)
        {
            println!("shot {label} at tick {}", self.ticks);
            self.shot_names.push((self.ticks, p.clone()));
            ctx.requests.screenshot = Some(p);
        }
        if self.opts.quit_tick.is_some_and(|q| self.ticks >= q)
            || self.opts.driver.as_ref().is_some_and(|d| d.done())
        {
            ctx.requests.quit = true;
        }
    }

    fn draw(&mut self, ctx: &mut DrawContext<'_>) {
        let Some(shared) = self.shared.as_mut() else {
            return;
        };
        match &mut self.screen {
            ScreenState::Menu(m) => m.draw(shared, ctx),
            ScreenState::Select(s) => s.draw(shared, ctx),
            ScreenState::Match(m) => m.draw(shared, ctx),
            ScreenState::Online(o) => o.draw(shared, ctx),
            ScreenState::Tutorial(t) => t.draw(shared, ctx),
            ScreenState::Empty => {}
        }
    }

    fn audio_reopened(&mut self, audio: &mut crate::audio::Audio) {
        if let Some(shared) = self.shared.as_mut() {
            shared.sounds.reopened(audio);
        }
    }

    fn finish(&mut self, _gpu: &Gpu, _renderer: &bopl_render::Renderer) -> Result<()> {
        if let ScreenState::Online(o) = &mut self.screen {
            o.leave();
        }
        if let ScreenState::Match(m) = &self.screen {
            let w = &m.sim.world;
            println!(
                "match: tick {}, level {} ({} loaded), scores {:?}",
                w.time.simulation_ticks,
                m.sim.level,
                w.flow.levels_loaded,
                w.players.iter().map(|p| p.games_won).collect::<Vec<_>>()
            );
        }
        Ok(())
    }
}

/// `CursorUpdater.SetCursor`: the main menu's cursor texture with each texel
/// `lerp(black, colour, green)` (alpha kept); the colour is the
/// keyboard + mouse player's `_ShadowColor`, or the script's default colour.
fn cursor_image(shared: &Shared, player: Option<i32>) -> Option<crate::app::CursorImage> {
    let data = &shared.data;
    let scene = data.scene(0).ok()?;
    let fields = scene.objects.iter().find_map(|o| {
        o.components
            .iter()
            .find(|c| c.script.as_deref() == Some("CursorUpdater"))
            .map(|c| c.fields.clone())
    })?;
    let comp = |v: &serde_json::Value, k: &str| v[k].as_f64().unwrap_or(1.0) as f32;
    let default = &fields["defaultCursorColor"];
    let mut color = [comp(default, "r"), comp(default, "g"), comp(default, "b")];
    if let Some(c) = player
        && let Some(id) = shared.visuals.player_material(c)
        && let Some(sc) = data
            .materials
            .get(&id)
            .and_then(|m| m.colors.get("_ShadowColor"))
    {
        color = [sc[0], sc[1], sc[2]];
    }
    let tex = data
        .textures
        .get(fields["originalTexture"]["$ref"].as_str()?)?;
    let (width, height, mut rgba) = bopl_import::load::read_png(&data.dir.join(&tex.png)).ok()?;
    for px in rgba.chunks_mut(4) {
        let g = px[1] as f32 / 255.0;
        for k in 0..3 {
            px[k] = (color[k] * g * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Some(crate::app::CursorImage {
        width,
        height,
        rgba,
    })
}

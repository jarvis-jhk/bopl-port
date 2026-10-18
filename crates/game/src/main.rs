//! bopl-port: clean-room reimplementation of Bopl Battle.
//!
//! The game (main menu, character select, matches on `bopl-sim`), plus the
//! headless sandbox, bot matches, driven flow tests and the M2 level viewer;
//! see `--help` and `docs/platform.md`.

// Release builds for Windows are GUI programs (no console window).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod assets;
mod audio;
mod bots;
mod charsel;
mod data;
mod driver;
mod fx;
mod game_app;
mod input;
mod level_scene;
mod level_sheet;
mod level_view;
mod match_scene;
mod match_ui;
mod match_view;
mod menu;
mod online;
mod particles;
mod platform;
mod present;
mod remap;
mod shared;
mod sim_match;
mod sound;
mod timing;
mod tutorial;
mod ui;
mod vpad;

use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{Context, Result, bail};

const USAGE: &str = "\
bopl-port

USAGE:
    bopl-port [--size WxH] [--windowed|--fullscreen]         (the game)
    bopl-port --viewer [--level N|NAME]                      (level viewer window)
    bopl-port --headless --sandbox [--level N] [--script S] --shots 200,260 --screenshot scratch/m3/s.png
    bopl-port --headless --bot-match --players N [--teams T | --split 1,11] [--level N] [--seed S]
              --shots 400,900 --screenshot scratch/m4/b.png [--ticks T] [--winner-shots] [--rounds R]
    bopl-port --headless --drive FILE --screenshot scratch/m4/flow.png
    bopl-port --headless --level N --screenshot scratch/x.png [--size 1920x1080]
              [--time S] [--frames F] [--wav out.wav] [--dump-targets DIR]
    bopl-port --headless --level-sheet scratch/levels.png [--size 480x270] [--cols 12]
    bopl-port --list-devices
    bopl-port --import [--game DIR] [--force]

OPTIONS:
    --viewer             The M2 level viewer instead of the game
    --sandbox            One bopl with scripted input (--script, default walk + jump)
    --replay FILE.rep    Play one of the original's replays (golden comparisons)
    --script S           Sandbox input: \"N:KEYS;...\" (keys L R U D J 1 2 3 S E)
    --abilities A,B,C    Sandbox: player 1's abilities (list index or name)
    --players N          Sandbox: N-1 extra (idle) players as targets
    --script2 S          Sandbox: input of player 2
    --bot-match          Bots play (--players N up to 12, --teams T, --seed S)
    --teams T            Bot match: players i -> team i mod T
    --split A,B,..       Bot match: team sizes (\"1,11\", \"2v2v2\"); sets --players
    --camera-zoom        Turn the camera-zoom setting on (more than 4 players)
    --winner-shots       Screenshot every winner screen (<stem>-winner-<n>.png)
    --rounds R           Quit after the R-th winner screen (with --winner-shots)
    --drive FILE         Headless: scripted device input through the whole flow
    --realtime           Headless: pace frames to the wall clock (online from the menu)
    --shots T1,T2,..     Screenshots after these ticks (<stem>-<tick>.png)
    --ticks T            Quit after T ticks (sandbox, bot match)
    --seed S             Session seed
    --level N|NAME       Level by build index (6..129), bag index (b0 = Level1),
                         or scene name / path suffix (Level1, \"Snow/Level14\", \"espace 3\")
    --headless           Render offscreen: no window, no audio device
    --screenshot PATH    Headless: save the last frame as PNG
    --size WxH           Output size (default 1920x1080 headless, 1280x720 window);
                         --level-sheet: the size of one tile (default 480x270)
    --time S             Seconds since level load to show (default 2: after the fade-in)
    --frames N           Headless: frames to run at --fps (default: enough for --time)
    --fps F              Headless: virtual frame rate (default 60)
    --camera-x X         Main camera x (default 0, the original's start)
    --overlay            Draw the level / device overlay (default on in the window)
    --no-overlay         Hide it
    --music              Play the match music (default on in the window)
    --no-music           Do not
    --play-sound NAME    Play a sound by its name in the original (e.g. arrowShoot)
    --wav PATH           Headless: write the mixed audio to a WAV file
    --dump-targets DIR   Headless: save the cameras' render textures as PNGs
    --level-sheet PATH   Headless: every level through the GPU path in one image
    --cols N             --level-sheet: tiles per row (default 12)
    --levels A,B,..      --level-sheet: only these build indices
    --list-devices       Print GPUs, controllers and audio devices, then exit
    --import             Find Bopl Battle, import its data (if needed) and exit
    --force              --import: re-import even when up to date
    --game DIR           Bopl Battle install (folder with BoplBattle_Data)
    --data DIR           Use an existing import dir as is (no install needed)
    --fullscreen         Start in borderless fullscreen (F11 / Alt+Enter toggles)
    --windowed           Start in a window (default: the settings' screen mode,
                         borderless fullscreen at first; --size implies a window)
    --no-vsync           Disable vsync
    --quit-after S       Window: quit after S seconds (tests)
    --warpzone-host      Online: host a room on Warpzone (the launcher's host action)
    --warpzone-join R    Online: join room R (room id, join code or invite link)
    --net-harness ...    Headless multi-process online test (see scripts/net/game-matrix.sh)
    --virtual-pads FILE  Window: simulated controllers driven by commands appended to
                         FILE (attach N, detach P, tap P a, axis P lx 1; see vpad.rs)
    -V, --version        Print the version and exit
    -h, --help           Show this help

ENVIRONMENT:
    BOPL_GAME_DIR=DIR             Bopl Battle install (after --game)
    WARPZONE_DATA_BOPL=DIR        Bopl Battle install (set by the Warpzone launcher)
    WARPZONE_SERVER=URL           Warpzone server (default: the public one)
    WARPZONE_JOIN=ROOM, WARPZONE_HOST=1   Online launch (like the flags)
    BOPL_NET_DELAY_MS, BOPL_NET_JITTER_MS, BOPL_NET_LOSS   Online link simulation (tests)
    BOPL_PORT_DATA_DIR=DIR        User data dir (import, settings)
    WGPU_BACKEND=vulkan|dx12|gl   Force a graphics backend
    WGPU_ADAPTER_NAME=llvmpipe    Pick a GPU by name
    RUST_LOG=info|debug           Log level
";

#[derive(Debug, Default)]
struct Args {
    headless: bool,
    level: Option<String>,
    screenshot: Option<PathBuf>,
    size: Option<(u32, u32)>,
    time: Option<f32>,
    frames: Option<u32>,
    fps: Option<f64>,
    camera_x: Option<f32>,
    overlay: Option<bool>,
    music: Option<bool>,
    sound: Option<String>,
    wav: Option<PathBuf>,
    dump_targets: Option<PathBuf>,
    level_sheet: Option<PathBuf>,
    cols: Option<u32>,
    levels: Vec<usize>,
    list_devices: bool,
    import: bool,
    force: bool,
    game: Option<PathBuf>,
    data: Option<PathBuf>,
    fullscreen: bool,
    windowed: bool,
    camera_zoom: bool,
    no_vsync: bool,
    quit_after: Option<f64>,
    virtual_pads: Option<PathBuf>,
    version: bool,
    viewer: bool,
    sandbox: bool,
    replay: Option<PathBuf>,
    script: Option<String>,
    script2: Option<String>,
    abilities: Option<String>,
    bot_match: bool,
    players: Option<usize>,
    teams: Option<usize>,
    split: Option<Vec<usize>>,
    winner_shots: bool,
    rounds: Option<u32>,
    seed: Option<u32>,
    drive: Option<PathBuf>,
    realtime: bool,
    shots: Vec<u64>,
    ticks: Option<u64>,
    /// `--warpzone-join`, `--warpzone-host`, invite links (online).
    warpzone: Vec<String>,
}

fn parse_size(s: &str) -> Result<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X']).context("size must be WxH")?;
    let (w, h): (u32, u32) = (w.parse()?, h.parse()?);
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        bail!("size out of range: {s}");
    }
    Ok((w, h))
}

fn parse_args(mut it: impl Iterator<Item = String>) -> Result<Option<Args>> {
    let mut a = Args::default();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().with_context(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--headless" => a.headless = true,
            "--level" => a.level = Some(value("--level")?),
            "--screenshot" => a.screenshot = Some(value("--screenshot")?.into()),
            "--size" => a.size = Some(parse_size(&value("--size")?)?),
            "--time" => a.time = Some(value("--time")?.parse()?),
            "--frames" => a.frames = Some(value("--frames")?.parse()?),
            "--fps" => a.fps = Some(value("--fps")?.parse()?),
            "--camera-x" => a.camera_x = Some(value("--camera-x")?.parse()?),
            "--overlay" => a.overlay = Some(true),
            "--no-overlay" => a.overlay = Some(false),
            "--music" => a.music = Some(true),
            "--no-music" => a.music = Some(false),
            "--play-sound" => a.sound = Some(value("--play-sound")?),
            "--wav" => a.wav = Some(value("--wav")?.into()),
            "--dump-targets" => a.dump_targets = Some(value("--dump-targets")?.into()),
            "--level-sheet" => a.level_sheet = Some(value("--level-sheet")?.into()),
            "--cols" => a.cols = Some(value("--cols")?.parse()?),
            "--levels" => {
                a.levels = value("--levels")?
                    .split(',')
                    .filter(|x| !x.trim().is_empty())
                    .map(|x| x.trim().parse())
                    .collect::<Result<_, _>>()?
            }
            "--list-devices" => a.list_devices = true,
            "--import" => a.import = true,
            "--force" => a.force = true,
            "--game" => a.game = Some(value("--game")?.into()),
            "--data" => a.data = Some(value("--data")?.into()),
            "--fullscreen" => a.fullscreen = true,
            "--windowed" => a.windowed = true,
            "--camera-zoom" => a.camera_zoom = true,
            "--no-vsync" => a.no_vsync = true,
            "--quit-after" => a.quit_after = Some(value("--quit-after")?.parse()?),
            "--virtual-pads" => a.virtual_pads = Some(value("--virtual-pads")?.into()),
            "-V" | "--version" => a.version = true,
            "--viewer" => a.viewer = true,
            "--sandbox" => a.sandbox = true,
            "--replay" => a.replay = Some(value("--replay")?.into()),
            "--script" => a.script = Some(value("--script")?),
            "--script2" => a.script2 = Some(value("--script2")?),
            "--abilities" => a.abilities = Some(value("--abilities")?),
            "--bot-match" => a.bot_match = true,
            "--players" => a.players = Some(value("--players")?.parse()?),
            "--teams" => a.teams = Some(value("--teams")?.parse()?),
            "--split" => {
                a.split = Some(
                    value("--split")?
                        .split([',', 'v'])
                        .filter(|x| !x.trim().is_empty())
                        .map(|x| x.trim().parse())
                        .collect::<Result<_, _>>()?,
                )
            }
            "--winner-shots" => a.winner_shots = true,
            "--rounds" => a.rounds = Some(value("--rounds")?.parse()?),
            "--seed" => a.seed = Some(value("--seed")?.parse()?),
            "--drive" => a.drive = Some(value("--drive")?.into()),
            "--realtime" => a.realtime = true,
            "--ticks" => a.ticks = Some(value("--ticks")?.parse()?),
            "--shots" => {
                a.shots = value("--shots")?
                    .split(',')
                    .filter(|x| !x.trim().is_empty())
                    .map(|x| x.trim().parse())
                    .collect::<Result<_, _>>()?
            }
            "--warpzone-host" => a.warpzone.push(arg),
            "--warpzone-join" => {
                let v = value("--warpzone-join")?;
                a.warpzone.extend(["--warpzone-join".to_string(), v]);
            }
            w if w.starts_with("--warpzone-join=")
                || bopl_net::warpzone_core::launch::parse_invite(w).is_some() =>
            {
                a.warpzone.push(arg)
            }
            "-h" | "--help" => return Ok(None),
            other => bail!("unknown argument {other:?} (see --help)"),
        }
    }
    Ok(Some(a))
}

fn init_logging() {
    struct Logger(log::LevelFilter);
    impl log::Log for Logger {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= self.0
        }
        fn log(&self, r: &log::Record) {
            // Keep wgpu/naga internals quiet unless asked for.
            let noisy = r.target().starts_with("wgpu") || r.target().starts_with("naga");
            if self.enabled(r.metadata())
                && (!noisy || r.level() <= log::Level::Warn || self.0 >= log::LevelFilter::Debug)
            {
                eprintln!("[{:5}] {}", r.level(), r.args());
            }
        }
        fn flush(&self) {}
    }
    let level = match std::env::var("RUST_LOG").as_deref() {
        Ok("trace") => log::LevelFilter::Trace,
        Ok("debug") => log::LevelFilter::Debug,
        Ok("info") => log::LevelFilter::Info,
        Ok("error") => log::LevelFilter::Error,
        _ => log::LevelFilter::Warn,
    };
    if log::set_logger(Box::leak(Box::new(Logger(level)))).is_ok() {
        log::set_max_level(level);
    }
}

/// `--list-devices`: GPUs (wgpu), controllers and audio (SDL, no window).
fn list_devices() -> Result<()> {
    let instance = bopl_render::new_instance();
    let adapters =
        pollster::block_on(instance.enumerate_adapters(bopl_render::wgpu::Backends::all()));
    println!("GPU adapters ({}):", adapters.len());
    for a in &adapters {
        let i = a.get_info();
        println!(
            "  {} [{:?}, {:?}] driver: {} {}",
            i.name, i.backend, i.device_type, i.driver, i.driver_info
        );
    }
    platform::set_hints();
    let sdl = sdl2::init().map_err(anyhow::Error::msg)?;
    let v = sdl2::version::version();
    println!("SDL {}.{}.{}", v.major, v.minor, v.patch);
    let gc = sdl.game_controller().map_err(anyhow::Error::msg)?;
    // Let SDL deliver the initial device-added events (the hot-plug path).
    let mut pump = sdl.event_pump().map_err(anyhow::Error::msg)?;
    let mut hub = input::InputHub::new(gc.clone());
    for _ in 0..10 {
        for e in pump.poll_iter() {
            hub.handle_event(&e);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let n = gc.num_joysticks().map_err(anyhow::Error::msg)?;
    let controllers = hub.controllers();
    println!(
        "Controllers ({} game controllers, {} joysticks):",
        controllers.len(),
        n
    );
    for c in &controllers {
        println!(
            "  {:?} {} (rumble: {}, bindings key: {})",
            c.id,
            c.name,
            if c.has_rumble { "yes" } else { "no" },
            c.key
        );
    }
    for i in 0..n {
        if !gc.is_game_controller(i) {
            let joy = sdl.joystick().map_err(anyhow::Error::msg)?;
            let name = joy.name_for_index(i).unwrap_or_else(|_| "?".into());
            println!(
                "  joystick #{i} {name} (no controller mapping; add one to gamecontrollerdb.txt)"
            );
        }
    }
    match sdl.audio() {
        Ok(audio) => {
            println!("Audio driver: {}", audio.current_audio_driver());
            let n = audio.num_audio_playback_devices().unwrap_or(0);
            println!("Audio playback devices ({n}):");
            for i in 0..n {
                let name = audio.audio_playback_device_name(i).unwrap_or_else(|e| e);
                println!("  #{i} {name}");
            }
        }
        Err(e) => println!("Audio: unavailable ({e})"),
    }
    let drivers: Vec<_> = sdl2::audio::drivers().collect();
    println!("Audio drivers compiled into SDL: {}", drivers.join(", "));
    Ok(())
}

fn main() -> Result<()> {
    init_logging();
    // Headless online tests (docs/net.md, scripts/net/game-matrix.sh).
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("--net-harness") {
        std::process::exit(online::harness::orchestrate(&raw)?);
    }
    if raw.first().map(String::as_str) == Some("--net-peer") {
        let data_opts = data::DataOptions {
            game: None,
            import_dir: raw
                .iter()
                .position(|a| a == "--data")
                .and_then(|i| raw.get(i + 1))
                .map(PathBuf::from),
            force: false,
        };
        let data = Rc::new(data::prepare_and_load(&data_opts)?);
        let levels = Rc::new(sim_match::LevelCache::new(data)?);
        std::process::exit(online::harness::peer(&raw, levels)?);
    }
    let Some(args) = parse_args(std::env::args().skip(1))? else {
        print!("{USAGE}");
        return Ok(());
    };
    if args.version {
        println!("bopl-port {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.list_devices {
        return list_devices();
    }
    let data_opts = data::DataOptions {
        game: args.game.clone(),
        import_dir: args.data.clone(),
        force: args.force,
    };
    if args.import {
        let plan = data::prepare(&data_opts)?;
        match &plan.install {
            Some(i) => println!("Bopl Battle install: {} ({})", i.dir.display(), i.source),
            None => println!("using {} as is", plan.import_dir.display()),
        }
        data::run_import(&plan)?;
        let d = data::load(&plan.import_dir)?;
        println!(
            "{}: {} levels, {} sprites, {} sounds, {} fonts",
            plan.import_dir.display(),
            d.level_indices().len(),
            d.sprites.len(),
            d.audio.len(),
            d.fonts.len()
        );
        return Ok(());
    }
    let data = Rc::new(data::prepare_and_load(&data_opts)?);
    if let Some(out) = &args.level_sheet {
        return level_sheet::level_sheet(
            data,
            args.size.unwrap_or((480, 270)),
            args.cols.unwrap_or(12),
            args.time.unwrap_or(2.0),
            &args.levels,
            out,
        );
    }
    let level = match &args.level {
        Some(k) => level_scene::find_level(&data, k)?,
        None => data
            .level_indices()
            .first()
            .copied()
            .context("the import has no levels")?,
    };
    // The window opens the game; headless runs without a game mode keep the
    // M2 level viewer (screenshots of levels).
    let game_mode = args.sandbox
        || args.replay.is_some()
        || args.bot_match
        || args.drive.is_some()
        || !args.warpzone.is_empty();
    if !args.viewer && (game_mode || !args.headless) {
        return run_game(data, &args, level);
    }
    let time = args.time.unwrap_or(2.0);
    let opts = level_scene::LevelSceneOptions {
        start_time: if args.headless { 0.0 } else { time },
        music: args.music.unwrap_or(!args.headless),
        sound: args.sound.clone(),
        overlay: args.overlay.unwrap_or(!args.headless),
        camera_x: args.camera_x,
        dump_targets: args.dump_targets.clone(),
    };
    let mut scene = level_scene::LevelScene::new(data, level, opts);
    if args.headless {
        let fps = args.fps.unwrap_or(60.0);
        let frames = args
            .frames
            .unwrap_or_else(|| ((time as f64) * fps).round().max(1.0) as u32);
        app::run_headless(
            &mut scene,
            &app::HeadlessOptions {
                size: args.size.unwrap_or((1920, 1080)),
                frames,
                fps,
                screenshot: args.screenshot.clone(),
                wav: args.wav.clone(),
                shots: Vec::new(),
                render_all: false,
                realtime: false,
            },
        )
    } else {
        app::run_windowed(
            &mut scene,
            &app::WindowOptions {
                size: args.size.unwrap_or((1280, 720)),
                display: shared::Display {
                    mode: if args.fullscreen {
                        shared::ScreenMode::Borderless
                    } else {
                        shared::ScreenMode::Windowed
                    },
                    resolution: None,
                    vsync: !args.no_vsync,
                },
                quit_after: args.quit_after,
                virtual_pads: args.virtual_pads.clone(),
                screenshot: args.screenshot.clone(),
            },
        )
    }
}

/// The game (menus and matches), the sandbox, bot matches, driven runs.
fn run_game(data: Rc<data_types::GameData>, args: &Args, level: usize) -> Result<()> {
    use game_app::{GameApp, GameOptions, Mode};
    let mode = if let Some(path) = &args.replay {
        Mode::Replay { path: path.clone() }
    } else if args.sandbox {
        // Abilities by list index or name (`Bow`, `Black hole`, `black_hole`).
        let abilities = match &args.abilities {
            Some(list) => {
                let names: Vec<String> =
                    data.abilities.full.iter().map(|a| a.name.clone()).collect();
                let norm = |s: &str| s.to_ascii_lowercase().replace([' ', '_', '-'], "");
                Some(
                    list.split(',')
                        .map(|x| {
                            let x = x.trim();
                            x.parse::<usize>()
                                .ok()
                                .or_else(|| names.iter().position(|n| norm(n) == norm(x)))
                                .with_context(|| format!("unknown ability {x:?}"))
                        })
                        .collect::<Result<Vec<_>>>()?,
                )
            }
            None => None,
        };
        Mode::Sandbox {
            level,
            script: args
                .script
                .clone()
                .unwrap_or_else(|| "230:;12:R;3:UJ;40:;10:L;3:ULJ;30:L;100:".into()),
            abilities,
            players: args.players.unwrap_or(1),
            script2: args.script2.clone(),
        }
    } else if args.bot_match {
        let players = match &args.split {
            Some(s) => s.iter().sum(),
            None => args.players.unwrap_or(4),
        };
        if !(1..=12).contains(&players) {
            bail!("--players (or the --split sum) must be 1..12");
        }
        Mode::BotMatch {
            players,
            level: args.level.as_ref().map(|_| level),
            seed: args.seed.unwrap_or(1),
            teams: args.teams,
            split: args.split.clone(),
        }
    } else {
        Mode::Game
    };
    let driver = match &args.drive {
        Some(p) => Some(driver::Driver::parse(
            &std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?,
        )?),
        None => None,
    };
    let headless = args.headless;
    let quit_tick = args.ticks.or_else(|| {
        if headless && driver.is_none() && args.rounds.is_some() {
            Some(216_000)
        } else if headless && driver.is_none() {
            Some(args.shots.iter().copied().max().unwrap_or(600) + 1)
        } else if headless {
            // Safety net for driven runs waiting on a condition that never
            // comes (one hour of game time).
            Some(216_000)
        } else {
            None
        }
    });
    let opts = GameOptions {
        mode,
        shots: args.shots.clone(),
        screenshot: args.screenshot.clone(),
        quit_tick,
        driver,
        music: args.music.unwrap_or(!headless),
        seed: args.seed.unwrap_or(1),
        winner_shots: args.winner_shots,
        rounds: args.rounds,
        online: online::scene::OnlineIntent::from_args(&args.warpzone),
        camera_zoom: args.camera_zoom,
    };
    let realtime = opts.online.is_some() || args.realtime;
    let mut app = GameApp::new(data, opts);
    if headless {
        let fps = args.fps.unwrap_or(60.0);
        app::run_headless(
            &mut app,
            &app::HeadlessOptions {
                size: args.size.unwrap_or((1920, 1080)),
                frames: args.frames.unwrap_or(u32::MAX),
                fps,
                screenshot: None,
                wav: args.wav.clone(),
                shots: Vec::new(),
                render_all: true,
                realtime,
            },
        )
    } else {
        // The settings' display options; `--size` (windowed at that size),
        // `--windowed`, `--fullscreen` and `--no-vsync` override them.
        let mut display = shared::Settings::load().display();
        if args.size.is_some() || args.windowed {
            display.mode = shared::ScreenMode::Windowed;
            display.resolution = args.size.or(display.resolution);
        }
        if args.fullscreen {
            display.mode = shared::ScreenMode::Borderless;
        }
        if args.no_vsync {
            display.vsync = false;
        }
        app::run_windowed(
            &mut app,
            &app::WindowOptions {
                size: args.size.unwrap_or((1280, 720)),
                display,
                quit_after: args.quit_after,
                virtual_pads: args.virtual_pads.clone(),
                screenshot: args.screenshot.clone(),
            },
        )
    }
}

mod data_types {
    pub use bopl_import::GameData;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Option<Args>> {
        parse_args(s.split_whitespace().map(String::from))
    }

    #[test]
    fn cli() {
        let a = parse("--headless --level 27 --screenshot scratch/x.png --size 640x360 --time 3")
            .unwrap()
            .unwrap();
        assert!(a.headless);
        assert_eq!(a.level.as_deref(), Some("27"));
        assert_eq!(a.size, Some((640, 360)));
        assert_eq!(a.time, Some(3.0));
        assert!(parse("--help").unwrap().is_none());
        assert!(parse("--bogus").is_err());
        assert!(parse("--size 0x10").is_err());
        assert!(parse("--level").is_err());
        let s = parse("--headless --level-sheet s.png --levels 6,27,39 --cols 3")
            .unwrap()
            .unwrap();
        assert_eq!(s.levels, vec![6, 27, 39]);
        assert_eq!(s.cols, Some(3));
        let i = parse("--import --game /x --force").unwrap().unwrap();
        assert!(i.import && i.force && i.game.is_some());
    }
}

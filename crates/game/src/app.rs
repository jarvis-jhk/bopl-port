//! The application loop: fixed-step ticks, rendering, and the two hosts
//! (SDL window, headless offscreen).
//!
//! A [`Scene`] is what the loop runs. The match will implement it around
//! `bopl-sim`: `tick` turns device states into the sim's per-player inputs
//! and steps the sim (1/60 s); `draw` builds a
//! [`WorldView`](crate::present::WorldView) from the interpolated state and
//! lets the [`LevelView`](crate::level_view::LevelView) turn it into a
//! render frame; sim events become sounds.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bopl_render::{Frame, Gpu, OFFSCREEN_FORMAT, OffscreenTarget, Renderer, SurfaceTarget};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};
use sdl2::video::FullscreenType;

use crate::audio::Audio;
use crate::input::{ControllerInfo, DeviceId, InputHub, InputState, RawPress};
use crate::shared::{Display, ScreenMode};
use crate::timing::{FIXED_DT, FixedStep};

/// What the host knows about the platform.
#[derive(Clone, Debug, Default)]
pub struct PlatformInfo {
    /// No window (offscreen run).
    pub headless: bool,
    /// Connected controllers.
    pub controllers: Vec<ControllerInfo>,
    /// The display's distinct resolutions (largest first) and the
    /// desktop's (window hosts only).
    pub resolutions: Vec<(u32, u32)>,
    pub desktop: Option<(u32, u32)>,
}

/// Requests from a scene to its host.
#[derive(Debug, Default)]
pub struct AppRequests {
    pub quit: bool,
    /// Render this frame and save it here (headless).
    pub screenshot: Option<PathBuf>,
    /// New key / button bindings for the input hub.
    pub bindings: Option<crate::input::Bindings>,
    /// New display options (window hosts apply them).
    pub display: Option<Display>,
    /// Controller rumble: device, low / high motor strength (0..1),
    /// duration in milliseconds (0 stops).
    pub rumble: Vec<(DeviceId, f32, f32, u32)>,
    /// A new mouse cursor image (window hosts apply it).
    pub cursor: Option<CursorImage>,
}

/// An RGBA8 cursor image, top row first, hot spot at the top-left corner.
#[derive(Clone, Debug)]
pub struct CursorImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Passed to [`Scene::tick`].
#[allow(dead_code)] // Part of the API the match scene uses (M3+).
pub struct TickContext<'a> {
    /// Input of every local device for this tick.
    pub inputs: &'a [(DeviceId, InputState)],
    /// Keys / buttons that went down since the previous tick.
    pub raw: &'a [RawPress],
    pub audio: &'a mut Audio,
    /// Index of this tick (0-based).
    pub tick: u64,
    pub dt: f32,
    pub platform: &'a PlatformInfo,
    pub requests: &'a mut AppRequests,
}

/// Passed to [`Scene::draw`].
#[allow(dead_code)] // Part of the API the match scene uses (M3+).
pub struct DrawContext<'a> {
    pub frame: &'a mut Frame,
    /// Interpolation factor between the previous and current tick.
    pub alpha: f32,
    /// Output size in pixels.
    pub size: (u32, u32),
    pub gpu: &'a Gpu,
    pub renderer: &'a mut Renderer,
}

/// Passed to [`Scene::load`] once at start.
#[allow(dead_code)] // Part of the API the match scene uses (M3+).
pub struct LoadContext<'a> {
    pub gpu: &'a Gpu,
    pub renderer: &'a mut Renderer,
    pub audio: &'a mut Audio,
}

pub trait Scene {
    fn load(&mut self, ctx: &mut LoadContext<'_>) -> Result<()>;
    fn tick(&mut self, ctx: &mut TickContext<'_>);
    fn draw(&mut self, ctx: &mut DrawContext<'_>);
    /// The audio device was reopened (the mixer is new: bus volumes).
    fn audio_reopened(&mut self, _audio: &mut Audio) {}
    /// Headless only: called after the final frame with the GPU still
    /// alive (dump render targets, print statistics).
    fn finish(&mut self, _gpu: &Gpu, _renderer: &Renderer) -> Result<()> {
        Ok(())
    }
}

/// Options for the headless host.
pub struct HeadlessOptions {
    pub size: (u32, u32),
    pub frames: u32,
    /// Simulated frame rate (virtual time; no wall clock involved).
    pub fps: f64,
    pub screenshot: Option<PathBuf>,
    pub wav: Option<PathBuf>,
    /// Also save screenshots after these frames (1-based), named
    /// `<screenshot stem>-<frame>.png`.
    pub shots: Vec<u32>,
    /// Render every frame, not only the captured ones (presentation state
    /// that accumulates on the GPU, like slime trails, needs every frame).
    pub render_all: bool,
    /// Pace the frames to the wall clock (online: the other machines run
    /// in real time).
    pub realtime: bool,
}

/// Path of the screenshot taken after `frame`.
pub fn shot_path(base: &Path, frame: u32) -> PathBuf {
    let stem = base
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "shot".into());
    base.with_file_name(format!("{stem}-{frame}.png"))
}

/// Runs `frames` frames offscreen at a virtual frame rate and saves the last
/// one. Deterministic: no wall clock, no window, no audio device.
pub fn run_headless(scene: &mut dyn Scene, opts: &HeadlessOptions) -> Result<()> {
    let gpu = Gpu::headless().context("creating a headless GPU device")?;
    println!("GPU: {}", gpu.describe());
    let mut renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT);
    let target = OffscreenTarget::new(&gpu, opts.size, OFFSCREEN_FORMAT);
    let mut audio = Audio::offline(opts.wav.as_deref())?;
    println!("audio: {}", audio.description());
    scene.load(&mut LoadContext {
        gpu: &gpu,
        renderer: &mut renderer,
        audio: &mut audio,
    })?;
    let mut step = FixedStep::new(FIXED_DT);
    let inputs: Vec<(DeviceId, InputState)> = [DeviceId::KeyboardMouse]
        .into_iter()
        .map(|d| (d, InputState::default()))
        .collect();
    let platform = PlatformInfo {
        headless: true,
        ..Default::default()
    };
    let mut requests = AppRequests::default();
    let frame_dt = 1.0 / opts.fps;
    let started = Instant::now();
    // Wall-clock pacing (realtime): frame `f` is due at `pace + f · dt`.
    let mut pace = (Instant::now(), 0u32);
    let mut rendered = 0u32;
    for f in 1..=opts.frames.max(1) {
        if opts.realtime {
            let due = pace.0 + Duration::from_secs_f64((f - pace.1) as f64 * frame_dt);
            let now = Instant::now();
            match due.checked_duration_since(now) {
                Some(wait) => std::thread::sleep(wait),
                // Far behind (a hitch): do not race to catch up.
                None if now - due > Duration::from_millis(250) => pace = (now, f),
                None => {}
            }
        }
        for _ in 0..step.advance(frame_dt) {
            scene.tick(&mut TickContext {
                inputs: &inputs,
                raw: &[],
                audio: &mut audio,
                tick: step.ticks(),
                dt: FIXED_DT as f32,
                platform: &platform,
                requests: &mut requests,
            });
        }
        let is_shot = opts.shots.contains(&f);
        let requested = requests.screenshot.take();
        let last = f == opts.frames.max(1) || requests.quit;
        if is_shot || requested.is_some() || last || opts.render_all {
            let mut frame = Frame::new();
            scene.draw(&mut DrawContext {
                frame: &mut frame,
                alpha: step.alpha(),
                size: opts.size,
                gpu: &gpu,
                renderer: &mut renderer,
            });
            renderer.render(&gpu, target.target(), &frame);
            rendered += 1;
            if !(is_shot || requested.is_some() || last) {
                audio.advance_offline(frame_dt)?;
                continue;
            }
            let mut paths = Vec::new();
            if is_shot {
                paths.push(shot_path(
                    opts.screenshot
                        .as_deref()
                        .unwrap_or(Path::new("scratch/shot.png")),
                    f,
                ));
            }
            paths.extend(requested);
            if last && let Some(p) = &opts.screenshot {
                paths.push(p.clone());
            }
            for path in paths {
                target.capture(&gpu)?.save_png(&path)?;
                println!(
                    "screenshot: {} (frame {f}, tick {})",
                    path.display(),
                    step.ticks()
                );
            }
        }
        audio.advance_offline(frame_dt)?;
        if requests.quit {
            break;
        }
    }
    let s = renderer.stats();
    println!(
        "{} ticks, rendered {rendered} frame(s) at {}x{} in {:.2?}; last frame: {} passes, {} quads, {} draws, {} pipelines",
        step.ticks(),
        opts.size.0,
        opts.size.1,
        started.elapsed(),
        s.passes,
        s.quads,
        s.draw_calls,
        s.pipelines
    );
    scene.finish(&gpu, &renderer)?;
    audio.finish()
}

/// Options for the windowed host.
pub struct WindowOptions {
    /// The window's size when windowed without a chosen resolution.
    pub size: (u32, u32),
    /// Screen mode, resolution and vsync to start with.
    pub display: Display,
    /// Quit after this many seconds (tests on the private Xvfb).
    pub quit_after: Option<f64>,
    /// `--virtual-pads FILE`: simulated controllers (tests).
    pub virtual_pads: Option<PathBuf>,
    /// Save a screenshot of the window's frame at quit time.
    pub screenshot: Option<PathBuf>,
}

/// The display's distinct resolutions, largest first.
/// An SDL colour cursor from an RGBA image (hot spot top-left, like the
/// original's `Cursor.SetCursor(texture, Vector2.zero)`).
fn make_cursor(c: CursorImage) -> Option<sdl2::mouse::Cursor> {
    let mut rgba = c.rgba;
    let surface = sdl2::surface::Surface::from_data(
        &mut rgba,
        c.width,
        c.height,
        c.width * 4,
        sdl2::pixels::PixelFormatEnum::RGBA32,
    )
    .map_err(|e| log::warn!("cursor surface: {e}"))
    .ok()?;
    let cursor = sdl2::mouse::Cursor::from_surface(surface, 0, 0)
        .map_err(|e| log::warn!("cursor: {e}"))
        .ok()?;
    cursor.set();
    Some(cursor)
}

fn display_resolutions(video: &sdl2::VideoSubsystem, display: i32) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for i in 0..video.num_display_modes(display).unwrap_or(0) {
        if let Ok(m) = video.display_mode(display, i) {
            let r = (m.w.max(1) as u32, m.h.max(1) as u32);
            if !out.contains(&r) {
                out.push(r);
            }
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse((r.0 * r.1, r.0)));
    out
}

/// Applies screen mode and resolution to the window (vsync is the
/// surface's).
fn apply_display(
    window: &mut sdl2::video::Window,
    video: &sdl2::VideoSubsystem,
    d: &Display,
    windowed_size: (u32, u32),
) {
    let display = window.display_index().unwrap_or(0);
    let res = |r: Option<(u32, u32)>| r.filter(|r| r.0 > 0 && r.1 > 0);
    let result = match d.mode {
        ScreenMode::Windowed => {
            let (w, h) = res(d.resolution).unwrap_or(windowed_size);
            window.set_fullscreen(FullscreenType::Off).and_then(|_| {
                window.set_size(w, h).map_err(|e| e.to_string())?;
                window.set_position(
                    sdl2::video::WindowPos::Centered,
                    sdl2::video::WindowPos::Centered,
                );
                Ok(())
            })
        }
        ScreenMode::Borderless => {
            // Without a window manager (and on some WMs) SDL keeps the
            // window where and as large as it was: cover the display first
            // (going back to a window sets the size again).
            if let Ok(b) = video.display_bounds(display) {
                window.set_position(
                    sdl2::video::WindowPos::Positioned(b.x()),
                    sdl2::video::WindowPos::Positioned(b.y()),
                );
                let _ = window.set_size(b.width(), b.height());
            }
            window.set_fullscreen(FullscreenType::Desktop)
        }
        ScreenMode::Fullscreen => {
            let desktop = video.desktop_display_mode(display).ok();
            let mode = match (res(d.resolution), desktop) {
                (Some((w, h)), Some(dm)) => {
                    // The closest real mode of that size (refresh rate,
                    // pixel format).
                    let want = sdl2::video::DisplayMode::new(dm.format, w as i32, h as i32, 0);
                    video
                        .closest_display_mode(display, &want)
                        .ok()
                        .or(Some(want))
                }
                (_, dm) => dm,
            };
            if let Ok(b) = video.display_bounds(display) {
                window.set_position(
                    sdl2::video::WindowPos::Positioned(b.x()),
                    sdl2::video::WindowPos::Positioned(b.y()),
                );
            }
            window
                .set_display_mode(mode)
                .and_then(|_| window.set_fullscreen(FullscreenType::True))
        }
    };
    match result {
        Ok(()) => log::info!(
            "display: {:?} {:?} ({}x{} px)",
            d.mode,
            d.resolution,
            crate::platform::window_pixels(window).0,
            crate::platform::window_pixels(window).1
        ),
        Err(e) => log::warn!("display {:?} {:?}: {e}", d.mode, d.resolution),
    }
}

/// Opens the SDL window and runs until it is closed. F11 or Alt+Enter
/// toggles borderless fullscreen.
pub fn run_windowed(scene: &mut dyn Scene, opts: &WindowOptions) -> Result<()> {
    crate::platform::set_hints();
    let sdl = sdl2::init().map_err(anyhow::Error::msg)?;
    let video = sdl.video().map_err(anyhow::Error::msg)?;
    let controllers = sdl.game_controller().map_err(anyhow::Error::msg)?;
    // No audio subsystem (no driver, no sound server): play silently.
    let sdl_audio = match sdl.audio() {
        Ok(a) => Some(a),
        Err(e) => {
            log::warn!("audio unavailable ({e}); continuing without sound");
            None
        }
    };
    let mut events = sdl.event_pump().map_err(anyhow::Error::msg)?;

    let start = match opts.display.mode {
        ScreenMode::Windowed => opts.display.resolution.unwrap_or(opts.size),
        _ => opts.size,
    };
    let mut win = video.window("Bopl Battle (bopl-port)", start.0, start.1);
    win.position_centered().resizable().allow_highdpi();
    let mut window = win.build().context("creating window")?;
    let mut display = opts.display;
    if display.mode != ScreenMode::Windowed {
        apply_display(&mut window, &video, &display, opts.size);
    }
    crate::platform::check_native_handles(&window)?;
    let instance = bopl_render::new_instance();
    // SAFETY: the surface is dropped before the window (explicitly at the
    // end of this function), so the handles stay valid.
    let surface = unsafe {
        instance.create_surface_unsafe(
            bopl_render::wgpu::SurfaceTargetUnsafe::from_display_and_window(&window, &window)?,
        )
    }
    .context("creating wgpu surface")?;
    let gpu = Gpu::new(instance, Some(&surface))?;
    log::info!("GPU: {}", gpu.describe());
    let mut surface = SurfaceTarget::new(
        &gpu,
        surface,
        crate::platform::window_pixels(&window),
        display.vsync,
    )?;
    let mut renderer = Renderer::new(&gpu, surface.format());
    log::info!("surface format {:?}", surface.format());

    let open_audio = || match sdl_audio.as_ref().map(Audio::open_device) {
        Some(Ok(a)) => a,
        Some(Err(e)) => {
            log::warn!("{e:#}; continuing without sound");
            Audio::null()
        }
        None => Audio::null(),
    };
    let mut audio = open_audio();
    let mut input = InputHub::new(controllers);
    let mut vpads = opts.virtual_pads.clone().map(crate::vpad::VirtualPads::new);
    scene.load(&mut LoadContext {
        gpu: &gpu,
        renderer: &mut renderer,
        audio: &mut audio,
    })?;

    let mut step = FixedStep::new(FIXED_DT);
    let mut inputs = Vec::new();
    let mut last = Instant::now();
    let started = Instant::now();
    let mut platform = PlatformInfo {
        resolutions: display_resolutions(&video, window.display_index().unwrap_or(0)),
        desktop: video
            .desktop_display_mode(window.display_index().unwrap_or(0))
            .ok()
            .map(|m| (m.w.max(1) as u32, m.h.max(1) as u32)),
        ..Default::default()
    };
    log::info!(
        "display modes: {:?} (desktop {:?})",
        platform.resolutions,
        platform.desktop
    );
    let mut requests = AppRequests::default();
    // The current colour cursor (kept alive while it is set).
    let mut cursor: Option<sdl2::mouse::Cursor> = None;
    let mut reopened_audio = false;
    let mut stats_t = Instant::now();
    let (mut stats_frames, mut stats_ticks0) = (0u32, 0u64);
    'main: loop {
        stats_frames += 1;
        if stats_t.elapsed() >= Duration::from_secs(10) {
            let secs = stats_t.elapsed().as_secs_f64();
            log::info!(
                "{:.1} fps, {:.1} ticks/s",
                stats_frames as f64 / secs,
                (step.ticks() - stats_ticks0) as f64 / secs
            );
            stats_t = Instant::now();
            stats_frames = 0;
            stats_ticks0 = step.ticks();
        }
        if let Some(v) = &mut vpads {
            v.poll();
        }
        for event in events.poll_iter() {
            match &event {
                Event::Quit { .. } => break 'main,
                Event::Window {
                    win_event: WindowEvent::SizeChanged(..) | WindowEvent::Resized(..),
                    ..
                } => surface.resize(&gpu, crate::platform::window_pixels(&window)),
                Event::AudioDeviceRemoved {
                    iscapture: false, ..
                } if audio.is_device() => {
                    // The playback device went away (USB headset unplugged,
                    // sound server restart): reopen the default one, else
                    // go on silently.
                    log::warn!("audio device removed; reopening the default device");
                    audio = open_audio();
                    reopened_audio = true;
                }
                Event::KeyDown {
                    keycode: Some(k),
                    keymod,
                    repeat: false,
                    ..
                } if *k == Keycode::F11
                    || (*k == Keycode::Return
                        && keymod.intersects(Mod::LALTMOD | Mod::RALTMOD)) =>
                {
                    display.mode = match window.fullscreen_state() {
                        FullscreenType::Off => ScreenMode::Borderless,
                        _ => ScreenMode::Windowed,
                    };
                    apply_display(&mut window, &video, &display, opts.size);
                    surface.resize(&gpu, crate::platform::window_pixels(&window));
                    continue;
                }
                _ => {}
            }
            input.handle_event(&event);
        }
        let now = Instant::now();
        let dt = (now - last).as_secs_f64();
        last = now;
        // Mouse positions arrive in window points; scenes work in drawable
        // pixels (differs with high-DPI scaling).
        let (pw, ph) = crate::platform::window_pixels(&window);
        let (ww, wh) = window.size();
        let dpi = glam::Vec2::new(pw as f32 / ww.max(1) as f32, ph as f32 / wh.max(1) as f32);
        platform.controllers = input.controllers();
        let mut raw = input.take_raw();
        for _ in 0..step.advance(dt) {
            input.snapshot(&mut inputs);
            for (_, s) in inputs.iter_mut() {
                if let Some(p) = &mut s.pointer {
                    *p *= dpi;
                }
            }
            scene.tick(&mut TickContext {
                inputs: &inputs,
                raw: &raw,
                audio: &mut audio,
                tick: step.ticks(),
                dt: FIXED_DT as f32,
                platform: &platform,
                requests: &mut requests,
            });
            raw.clear();
            if let Some(b) = requests.bindings.take() {
                input.set_bindings(b);
            }
            for (dev, low, high, ms) in requests.rumble.drain(..) {
                input.rumble(dev, low, high, ms);
            }
            if let Some(c) = requests.cursor.take() {
                cursor = make_cursor(c).or(cursor);
            }
            if let Some(d) = requests.display.take() {
                if d.vsync != display.vsync {
                    surface.set_vsync(&gpu, d.vsync);
                }
                if (d.mode, d.resolution) != (display.mode, display.resolution) {
                    display = d;
                    apply_display(&mut window, &video, &display, opts.size);
                }
                display = d;
                surface.resize(&gpu, crate::platform::window_pixels(&window));
            }
        }
        if std::mem::take(&mut reopened_audio) {
            // The scenes' volumes go to the new device's mixer.
            scene.audio_reopened(&mut audio);
        }
        // The drawable size can change without an event (fullscreen
        // switches on some window managers): follow it every frame.
        let px = crate::platform::window_pixels(&window);
        if px != surface.size() {
            surface.resize(&gpu, px);
        }
        let quit_now = opts
            .quit_after
            .is_some_and(|s| started.elapsed().as_secs_f64() >= s);
        if requests.quit {
            break 'main;
        }
        let Some(out) = surface.acquire(&gpu) else {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        let mut frame = Frame::new();
        scene.draw(&mut DrawContext {
            frame: &mut frame,
            alpha: step.alpha(),
            size: out.size,
            gpu: &gpu,
            renderer: &mut renderer,
        });
        if quit_now && let Some(path) = &opts.screenshot {
            // The same frame, offscreen in the surface's format, for the record.
            let shot = OffscreenTarget::new(&gpu, out.size, surface.format());
            renderer.render(&gpu, shot.target(), &frame);
            shot.capture(&gpu)?.save_png(path)?;
            println!("screenshot: {}", path.display());
        }
        renderer.render(&gpu, out.target(), &frame);
        out.present(&gpu);
        if quit_now {
            println!(
                "window: {} ticks in {:.1}s, {} controller(s), audio: {}",
                step.ticks(),
                started.elapsed().as_secs_f64(),
                platform.controllers.len(),
                audio.description()
            );
            break 'main;
        }
    }
    drop(surface);
    drop(window);
    audio.finish()
}

//! The level viewer scene (M2): shows a level with the original's look,
//! cycles levels, plays the match music, lists input devices.
//!
//! Keys / buttons: Left/Right (any device: arrows, A/D, d-pad, stick) =
//! previous/next level, Jump = a jump sound, Select (Esc, Back) = quit.

use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{Context, Result, bail};
use bopl_import::GameData;
use bopl_render::{Color, FontId, Gpu, Pass, PassTarget, Quad, Renderer, Text, TextureId, View};
use glam::Vec2;

use crate::app::{DrawContext, LoadContext, Scene, TickContext};
use crate::assets::Assets;
use crate::input::Buttons;
use crate::level_view::LevelView;
use crate::present::WorldView;
use crate::sound::Sounds;

/// The font asset used for overlays (the English UI font of the original).
pub const UI_FONT: &str = "en_se_de_fr_it_es";
/// Its fallback for characters the static atlas lacks: the original's
/// dynamic font (glyphs rasterised from SF Arch Rival at run time).
pub const UI_FALLBACK_FONT: &str = "Dynamic_en_se_fallbacks";

/// The overlay font with its dynamic fallback.
pub fn ui_font(assets: &mut Assets, gpu: &Gpu, renderer: &mut Renderer) -> Option<FontId> {
    let font = assets.font(gpu, renderer, UI_FONT)?;
    if let Some(fb) = assets.font(gpu, renderer, UI_FALLBACK_FONT) {
        renderer.set_sdf_fallbacks(font, &[fb]);
    }
    Some(font)
}

/// Options of the level scene.
#[derive(Clone, Debug, Default)]
pub struct LevelSceneOptions {
    /// Seconds since level load at start (the fade-in takes 1 s).
    pub start_time: f32,
    /// Play the match music (TheBopl).
    pub music: bool,
    /// A sound to play at start (by name).
    pub sound: Option<String>,
    /// Draw the device / level overlay.
    pub overlay: bool,
    /// Main camera x (default: the level's 0).
    pub camera_x: Option<f32>,
    /// Headless: save every render target here at the end.
    pub dump_targets: Option<PathBuf>,
}

pub struct LevelScene {
    data: Rc<GameData>,
    assets: Assets,
    sounds: Sounds,
    levels: Vec<usize>,
    current: usize,
    view: Option<LevelView>,
    time: f32,
    opts: LevelSceneOptions,
    font: Option<FontId>,
    devices: Vec<String>,
    last_press: String,
    targets: Vec<(String, TextureId)>,
}

/// Resolves `--level`: a build index (6..129), a bag index prefixed with
/// `b` (`b0` = Level1), or a scene name / path suffix.
pub fn find_level(data: &GameData, key: &str) -> Result<usize> {
    let levels = data.level_indices();
    if let Ok(n) = key.parse::<usize>() {
        if levels.contains(&n) {
            return Ok(n);
        }
        bail!(
            "no level with build index {n} (levels are {}..{})",
            levels[0],
            levels[levels.len() - 1]
        );
    }
    if let Some(b) = key.strip_prefix('b').and_then(|b| b.parse::<usize>().ok()) {
        return levels
            .get(b)
            .copied()
            .with_context(|| format!("no level with bag index {b}"));
    }
    let k = key.to_lowercase();
    data.index
        .scenes
        .iter()
        .filter(|s| s.level)
        .find(|s| {
            s.name.to_lowercase() == k
                || s.path
                    .to_lowercase()
                    .trim_end_matches(".unity")
                    .ends_with(&k)
        })
        .map(|s| s.build_index)
        .with_context(|| format!("no level named {key:?}"))
}

impl LevelScene {
    pub fn new(data: Rc<GameData>, level: usize, opts: LevelSceneOptions) -> Self {
        let levels = data.level_indices();
        let current = levels.iter().position(|&l| l == level).unwrap_or(0);
        Self {
            assets: Assets::new(data.clone()),
            sounds: Sounds::new(data.clone()),
            data,
            levels,
            current,
            view: None,
            time: opts.start_time,
            opts,
            font: None,
            devices: Vec::new(),
            last_press: String::new(),
            targets: Vec::new(),
        }
    }

    fn load_level(&mut self) -> Result<()> {
        let bi = self.levels[self.current];
        let level = self.data.level(bi)?;
        log::info!("level {bi} {} ({})", level.name, level.path);
        let view = LevelView::new(&self.data, level);
        for c in view.describe_cameras() {
            log::debug!("  camera {c}");
        }
        self.view = Some(view);
        self.time = self.opts.start_time;
        Ok(())
    }

    fn switch(&mut self, delta: i32) {
        let n = self.levels.len() as i32;
        self.current = (self.current as i32 + delta).rem_euclid(n) as usize;
        if let Err(e) = self.load_level() {
            log::error!("{e:#}");
        }
    }

    fn overlay(&mut self, gpu: &Gpu, renderer: &mut Renderer, size: (u32, u32)) -> Option<Pass> {
        let font = self.font.or_else(|| {
            self.font = ui_font(&mut self.assets, gpu, renderer);
            self.font
        })?;
        let view = self.view.as_ref()?;
        let px = size.1 as f32 / 24.0;
        let mut pass = Pass::new("overlay", PassTarget::Output, View::Screen);
        let mut lines = vec![format!(
            "{} - {} ({})",
            view.level.build_index,
            view.level.name,
            view.level
                .path
                .trim_start_matches("Assets/Scenes/")
                .trim_end_matches(".unity")
        )];
        lines.extend(self.devices.iter().cloned());
        if !self.last_press.is_empty() {
            lines.push(self.last_press.clone());
        }
        let text = lines.join("\n");
        let block = renderer.measure_text(font, &text, px);
        pass.push(Quad {
            pivot: Vec2::ZERO,
            ..Quad::rect(
                Vec2::splat(px * 0.4),
                block + Vec2::splat(px * 0.6),
                Color::rgba(0.0, 0.0, 0.0, 0.35),
            )
        });
        pass.push(Text {
            text,
            font,
            size: px,
            position: Vec2::splat(px * 0.7),
            color: Color::WHITE,
            outline: 0.2,
            outline_color: Color::rgb(0.05, 0.2, 0.45),
            ..Default::default()
        });
        Some(pass)
    }
}

impl Scene for LevelScene {
    fn load(&mut self, ctx: &mut LoadContext<'_>) -> Result<()> {
        self.load_level()?;
        ctx.audio.set_master_volume(crate::sound::ENGINE_VOLUME);
        if self.opts.music
            && let Some(song) = self.sounds.match_song(0)
        {
            println!("music: {}", song.name);
            self.sounds.play_song(ctx.audio, &song);
        }
        if let Some(name) = self.opts.sound.clone() {
            match self.sounds.play(ctx.audio, &name) {
                Some(_) => println!("sound: {name}"),
                None => bail!(
                    "no sound named {name:?}; known: {}",
                    self.sounds.names().join(", ")
                ),
            }
        }
        Ok(())
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        self.time += ctx.dt;
        let mut delta = 0i32;
        for (dev, st) in ctx.inputs {
            let p = st.pressed;
            if p.contains(Buttons::RIGHT) {
                delta += 1;
            }
            if p.contains(Buttons::LEFT) {
                delta -= 1;
            }
            if p.contains(Buttons::JUMP) {
                self.sounds.play(ctx.audio, "slimeJump1");
            }
            if p.contains(Buttons::SELECT) && !ctx.platform.headless {
                ctx.requests.quit = true;
            }
            if p.0 != 0 {
                self.last_press = format!("{}: buttons {:#06x}", dev.label(), p.0);
            }
        }
        for r in ctx.raw {
            self.last_press = format!("{}: {}", r.device.label(), r.name);
        }
        if delta != 0 {
            self.sounds.play(ctx.audio, "navigateMenu");
            self.switch(delta.signum());
        }
        self.devices = std::iter::once("keyboard + mouse".to_string())
            .chain(
                ctx.platform
                    .controllers
                    .iter()
                    .map(|c| format!("controller {:?}: {}", c.id, c.name)),
            )
            .collect();
    }

    fn draw(&mut self, ctx: &mut DrawContext<'_>) {
        let Some(view) = self.view.as_mut() else {
            return;
        };
        let world = WorldView {
            time: self.time,
            camera_x: self.opts.camera_x,
            ..Default::default()
        };
        let (frame, targets) =
            view.frame(&world, ctx.size, ctx.gpu, ctx.renderer, &mut self.assets);
        *ctx.frame = frame;
        self.targets = targets;
        if self.opts.overlay
            && let Some(p) = self.overlay(ctx.gpu, ctx.renderer, ctx.size)
        {
            ctx.frame.passes.push(p);
        }
    }

    fn finish(&mut self, gpu: &Gpu, renderer: &Renderer) -> Result<()> {
        if let Some(dir) = &self.opts.dump_targets {
            std::fs::create_dir_all(dir)?;
            for (name, id) in &self.targets {
                let path = dir.join(format!("{}.png", name.trim_start_matches('_')));
                renderer.capture(gpu, *id)?.save_png(&path)?;
                println!("render target {name}: {}", path.display());
            }
        }
        Ok(())
    }
}

//! `--level-sheet`: every level rendered through the real GPU path (all
//! cameras, shaders, masks) into one contact sheet, labelled in the game's
//! own font.

use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use anyhow::{Context, Result};
use bopl_import::GameData;
use bopl_render::{
    Color, Frame, Gpu, Image, OFFSCREEN_FORMAT, OffscreenTarget, Pass, PassTarget, Renderer, Text,
    View,
};
use glam::Vec2;

use crate::assets::Assets;
use crate::level_scene::ui_font;
use crate::level_view::LevelView;
use crate::present::WorldView;

/// Renders the levels `only` (all when empty) as `tile`-sized tiles,
/// `cols` per row, at `time` seconds after load.
pub fn level_sheet(
    data: Rc<GameData>,
    tile: (u32, u32),
    cols: u32,
    time: f32,
    only: &[usize],
    out: &Path,
) -> Result<()> {
    let started = Instant::now();
    let gpu = Gpu::headless().context("creating a headless GPU device")?;
    println!("GPU: {}", gpu.describe());
    let mut renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT);
    let target = OffscreenTarget::new(&gpu, tile, OFFSCREEN_FORMAT);
    let mut assets = Assets::new(data.clone());
    let font = ui_font(&mut assets, &gpu, &mut renderer);
    let levels: Vec<usize> = data
        .level_indices()
        .into_iter()
        .filter(|l| only.is_empty() || only.contains(l))
        .collect();
    let cols = cols.clamp(1, levels.len().max(1) as u32);
    let rows = (levels.len() as u32).div_ceil(cols);
    let gap = 4;
    let mut sheet = Image::solid(
        cols * (tile.0 + gap) + gap,
        rows * (tile.1 + gap) + gap,
        [60, 60, 64, 255],
    );
    for (k, &bi) in levels.iter().enumerate() {
        let level = data.level(bi)?;
        let name = level.name.clone();
        let mut view = LevelView::new(&data, level);
        let world = WorldView {
            time,
            ..Default::default()
        };
        let (frame, _) = view.frame(&world, tile, &gpu, &mut renderer, &mut assets);
        let mut frame: Frame = frame;
        if let Some(font) = font {
            let mut label = Pass::new("label", PassTarget::Output, View::Screen);
            label.push(Text {
                text: format!("{bi} {name}"),
                font,
                size: (tile.1 as f32 / 10.0).max(10.0),
                position: Vec2::new(4.0, 2.0),
                color: Color::WHITE,
                outline: 0.25,
                outline_color: Color::rgb(0.05, 0.1, 0.25),
                ..Default::default()
            });
            frame.passes.push(label);
        }
        renderer.render(&gpu, target.target(), &frame);
        let img = target.capture(&gpu)?;
        let (c, r) = (k as u32 % cols, k as u32 / cols);
        let (x0, y0) = (gap + c * (tile.0 + gap), gap + r * (tile.1 + gap));
        for y in 0..tile.1 {
            let src = &img.pixels[(y * tile.0 * 4) as usize..((y + 1) * tile.0 * 4) as usize];
            let d = (((y0 + y) * sheet.width + x0) * 4) as usize;
            sheet.pixels[d..d + src.len()].copy_from_slice(src);
        }
    }
    // Rendering may leave alpha below 1 where nothing opaque was drawn.
    for px in sheet.pixels.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
    sheet.save_png(out)?;
    println!(
        "{} levels -> {} ({}x{}) in {:.1?}",
        levels.len(),
        out.display(),
        sheet.width,
        sheet.height,
        started.elapsed()
    );
    Ok(())
}

//! Dev tool: renders every imported level into one contact sheet PNG.
//!
//! Usage: `level-sheet [--data DIR] [--out FILE] [--cols N] [--tile WxH]
//! [--shapes] [--level N|NAME]` (defaults: `cache/import`, else the user
//! data dir's import; `scratch/levels-sheet.png`, 12 columns, 320x180).
//!
//! Sprites are drawn as textured, tinted quads with their world transforms,
//! sorted like Unity's 2D renderer (sorting layer, order in layer, then
//! depth), limited to the layers the main camera renders (its culling
//! mask); sprite masks are not applied. The game's physics shapes
//! (`DPhysicsBox`, `DPhysicsCircle`, `DPhysicsRoundedRect`, from the raw
//! Q32.32 values and the `FixTransform`) are outlined where the object has
//! no visible sprite, or everywhere with `--shapes`; team spawns are red
//! dots. Each tile shows the main camera's view (its orthographic size,
//! 16:9) and the level's build index.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use bopl_import::GameData;
use bopl_import::format::{DetShapeKind, Level, Object};
use bopl_import::load::read_png;
use rayon::prelude::*;
use tiny_skia::{
    Color, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform,
};

/// Background colour of a tile.
const BACKGROUND: [u8; 3] = [14, 14, 17];

struct SpriteImage {
    pixmap: Pixmap,
    /// Pixel → sprite-local units.
    local: Transform,
    /// Size in units (for sliced/tiled draw modes).
    size_units: [f32; 2],
}

fn premultiplied(w: u32, h: u32, rgba: &[u8]) -> Option<Pixmap> {
    let mut p = Pixmap::new(w, h)?;
    for (d, s) in p
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(rgba.as_chunks::<4>().0)
    {
        let a = s[3] as u16;
        d[0] = ((s[0] as u16 * a + 127) / 255) as u8;
        d[1] = ((s[1] as u16 * a + 127) / 255) as u8;
        d[2] = ((s[2] as u16 * a + 127) / 255) as u8;
        d[3] = s[3];
    }
    Some(p)
}

fn load_sprites(data: &GameData, maps: &[Level]) -> HashMap<String, SpriteImage> {
    let mut ids: Vec<&str> = maps
        .iter()
        .flat_map(|m| m.objects.iter())
        .filter_map(|o| o.sprite.as_ref()?.sprite.as_deref())
        .collect();
    ids.sort();
    ids.dedup();
    let mut tex_ids: Vec<&str> = ids
        .iter()
        .filter_map(|s| data.sprites.get(*s)?.texture.as_deref())
        .collect();
    tex_ids.sort();
    tex_ids.dedup();
    let textures: HashMap<&str, (u32, u32, Vec<u8>)> = tex_ids
        .par_iter()
        .filter_map(|t| Some((*t, read_png(&data.texture_path(t)?).ok()?)))
        .collect();
    ids.par_iter()
        .filter_map(|id| {
            let s = data.sprites.get(*id)?;
            let (tw, th, rgba) = textures.get(s.texture.as_deref()?)?;
            let [x, y, w, h] = s.texture_rect;
            let (x0, y0) = (x.round().max(0.0) as u32, y.round().max(0.0) as u32);
            let (w, h) = (
                (w.round() as u32).min(tw - x0).max(1),
                (h.round() as u32).min(th - y0).max(1),
            );
            let mut crop = vec![0u8; (w * h * 4) as usize];
            for row in 0..h {
                let src = (((y0 + row) * tw + x0) * 4) as usize;
                crop[(row * w * 4) as usize..][..(w * 4) as usize]
                    .copy_from_slice(&rgba[src..src + (w * 4) as usize]);
            }
            let ppu = s.pixels_per_unit;
            let left = (s.texture_rect_offset[0] - s.pivot[0] * s.rect[2]) / ppu;
            let bottom = (s.texture_rect_offset[1] - s.pivot[1] * s.rect[3]) / ppu;
            Some((
                id.to_string(),
                SpriteImage {
                    pixmap: premultiplied(w, h, &crop)?,
                    local: Transform::from_row(
                        1.0 / ppu,
                        0.0,
                        0.0,
                        -1.0 / ppu,
                        left,
                        bottom + h as f32 / ppu,
                    ),
                    size_units: [s.rect[2] / ppu, s.rect[3] / ppu],
                },
            ))
        })
        .collect()
}

fn tinted(src: &Pixmap, c: [f32; 4]) -> Pixmap {
    let mut p = src.clone();
    let m = [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]].map(|v| v.clamp(0.0, 1.0));
    for px in p.data_mut().as_chunks_mut::<4>().0 {
        for k in 0..4 {
            px[k] = (px[k] as f32 * m[k]).round() as u8;
        }
    }
    p
}

fn world_tf(o: &Object) -> Transform {
    let [a, b, c, d, tx, ty] = o.world.matrix;
    Transform::from_row(a, b, c, d, tx, ty)
}

/// Q32.32 raw bits to `f32`.
fn fx(raw: i64) -> f32 {
    (raw as f64 / 4_294_967_296.0) as f32
}

fn outline(px: &mut Pixmap, o: &Object, view: Transform, color: Color) {
    let Some(shape) = &o.shape else { return };
    // Position and rotation from the FixTransform (the simulation's), else
    // from Unity's transform.
    let (x, y, rot) = match &o.fix {
        Some(f) => (fx(f.position[0]), fx(f.position[1]), fx(f.rotation)),
        None => (
            o.world.trs.position[0],
            o.world.trs.position[1],
            o.world.rotation_z.to_radians(),
        ),
    };
    let tf = Transform::from_rotate(rot.to_degrees())
        .post_translate(x, y)
        .post_concat(view);
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    let stroke = Stroke {
        width: 1.0,
        ..Default::default()
    };
    let mut pb = PathBuilder::new();
    match &shape.kind {
        DetShapeKind::Box { extents, .. } => {
            let (ex, ey) = (fx(extents[0]), fx(extents[1]));
            if let Some(r) = Rect::from_xywh(-ex, -ey, 2.0 * ex, 2.0 * ey) {
                pb.push_rect(r);
            }
        }
        DetShapeKind::Circle { radius } => pb.push_circle(0.0, 0.0, fx(*radius)),
        DetShapeKind::RoundedRect { extents, radius } => {
            rounded_rect(&mut pb, fx(extents[0]), fx(extents[1]), fx(*radius))
        }
    }
    if let Some(path) = pb.finish().and_then(|p| p.transform(tf)) {
        px.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// A box of half size `ex`x`ey` grown by `r` (straight edges and quarter
/// circles), centred on the origin.
fn rounded_rect(pb: &mut PathBuilder, ex: f32, ey: f32, r: f32) {
    if r <= 0.0 {
        if let Some(rect) = Rect::from_xywh(-ex, -ey, 2.0 * ex, 2.0 * ey) {
            pb.push_rect(rect);
        }
        return;
    }
    let corners = [
        (ex, ey, 0.0f32),
        (-ex, ey, 90.0),
        (-ex, -ey, 180.0),
        (ex, -ey, 270.0),
    ];
    for (i, (cx, cy, start)) in corners.into_iter().enumerate() {
        for k in 0..=8 {
            let a = (start + k as f32 * 90.0 / 8.0).to_radians();
            let (px, py) = (cx + r * a.cos(), cy + r * a.sin());
            if i == 0 && k == 0 {
                pb.move_to(px, py);
            } else {
                pb.line_to(px, py);
            }
        }
    }
    pb.close();
}

/// Culling mask, position and orthographic size of the level's main camera.
fn main_camera(level: &Level) -> (u32, [f32; 2], f32) {
    let cam = level.main_camera.and_then(|i| level.objects.get(i));
    let fields = cam.and_then(|o| o.component("Camera")).map(|c| &c.fields);
    let mask = fields
        .and_then(|f| f["m_CullingMask"]["m_Bits"].as_u64())
        .unwrap_or(u32::MAX as u64) as u32;
    let size = fields
        .and_then(|f| f["orthographic size"].as_f64())
        .filter(|s| *s > 0.0)
        .unwrap_or(30.0) as f32;
    let pos = cam.map_or([0.0, 0.0], |o| {
        [o.world.trs.position[0], o.world.trs.position[1]]
    });
    (mask, pos, size)
}

/// 3x5 pixel digits for the build index label.
const DIGITS: [u16; 10] = [
    0b111_101_101_101_111,
    0b010_110_010_010_111,
    0b111_001_111_100_111,
    0b111_001_111_001_111,
    0b101_101_111_001_001,
    0b111_100_111_001_111,
    0b111_100_111_101_111,
    0b111_001_010_010_010,
    0b111_101_111_101_111,
    0b111_101_111_001_111,
];

fn label(px: &mut Pixmap, n: usize, scale: f32) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 220);
    for (i, ch) in n.to_string().bytes().enumerate() {
        let bits = DIGITS[(ch - b'0') as usize];
        for row in 0..5 {
            for col in 0..3 {
                if bits >> (14 - (row * 3 + col)) & 1 == 1 {
                    let x = 4.0 + (i as f32 * 4.0 + col as f32) * scale;
                    let y = 4.0 + row as f32 * scale;
                    px.fill_rect(
                        Rect::from_xywh(x, y, scale, scale).unwrap(),
                        &paint,
                        Transform::identity(),
                        None,
                    );
                }
            }
        }
    }
}

/// How a sprite's material is approximated on the sheet.
enum Look {
    /// Plain sprite (tinted by the renderer colour).
    Sprite,
    /// Sprite tinted by a material colour and alpha factor.
    Tint([f32; 4]),
    /// A vertical gradient over the sprite quad (bottom, top colours).
    Gradient([f32; 4], [f32; 4]),
    /// Not drawn (shader effects such as distortion and aurora).
    Skip,
}

fn look(data: &GameData, r: &bopl_import::format::SpriteRenderer) -> Look {
    let Some(m) = r.material_ids.first().and_then(|id| data.materials.get(id)) else {
        return Look::Sprite;
    };
    let col = |k: &str| m.colors.get(k).copied();
    match m.shader.as_str() {
        "Custom/SingleColorAlpha" | "Custom/AdditiveColoredA8" => {
            let mut c = col("_Color").unwrap_or([1.0; 4]);
            c[3] *= m.floats.get("_Alpha").copied().unwrap_or(1.0);
            Look::Tint(c)
        }
        // The reflective water quad doubles as the sky of levels without a
        // sky sprite: a gradient, dark at the top, light at the horizon.
        "Custom/ReflectiveWaterWithDistortion" => {
            let top = col("_Gradient").unwrap_or([0.0, 0.1, 0.2, 1.0]);
            let bot = col("_Gradient2").unwrap_or([0.0, 0.6, 1.0, 1.0]);
            Look::Gradient(bot, top)
        }
        "Unlit/aurora" | "Custom/WaterDistortion" => Look::Skip,
        _ => Look::Sprite,
    }
}

fn gradient_paint(bot: [f32; 4], top: [f32; 4]) -> Option<Paint<'static>> {
    let c = |v: [f32; 4]| Color::from_rgba(v[0], v[1], v[2], v[3].clamp(0.0, 1.0));
    let shader = tiny_skia::LinearGradient::new(
        tiny_skia::Point::from_xy(0.0, 0.0),
        tiny_skia::Point::from_xy(0.0, 1.0),
        vec![
            tiny_skia::GradientStop::new(0.0, c(bot)?),
            tiny_skia::GradientStop::new(1.0, c(top)?),
        ],
        tiny_skia::SpreadMode::Pad,
        Transform::identity(),
    )?;
    Some(Paint {
        shader,
        anti_alias: true,
        ..Default::default()
    })
}

fn render_level(
    data: &GameData,
    level: &Level,
    sprites: &HashMap<String, SpriteImage>,
    w: u32,
    h: u32,
    all_shapes: bool,
) -> Pixmap {
    let mut px = Pixmap::new(w, h).unwrap();
    px.fill(Color::from_rgba8(
        BACKGROUND[0],
        BACKGROUND[1],
        BACKGROUND[2],
        255,
    ));
    let (mask, [cx, cy], half_h) = main_camera(level);
    let s = h as f32 / (2.0 * half_h);
    let view = Transform::from_row(
        s,
        0.0,
        0.0,
        -s,
        w as f32 / 2.0 - cx * s,
        h as f32 / 2.0 + cy * s,
    );
    let drawn = |o: &Object| {
        o.active_in_hierarchy
            && mask >> (o.layer & 31) & 1 == 1
            && o.component("SpriteMask").is_none()
            // Full-screen fades (FadeInOnLoad) are transparent in play.
            && o.component("AlphaFade").is_none()
            && o.sprite
                .as_ref()
                .is_some_and(|r| r.enabled && r.color[3] > 0.0 && r.sprite.is_some())
    };
    let mut draws: Vec<(usize, &Object)> = level
        .objects
        .iter()
        .enumerate()
        .filter(|(_, o)| drawn(o))
        .collect();
    draws.sort_by(|(ia, a), (ib, b)| {
        let (ra, rb) = (a.sprite.as_ref().unwrap(), b.sprite.as_ref().unwrap());
        (ra.sorting_layer_index, ra.sorting_order)
            .cmp(&(rb.sorting_layer_index, rb.sorting_order))
            // Farther (larger z) first.
            .then(b.world.trs.position[2].total_cmp(&a.world.trs.position[2]))
            .then(ia.cmp(ib))
    });
    let mut tints: HashMap<(String, [u32; 4]), Pixmap> = HashMap::new();
    for (_, o) in &draws {
        let r = o.sprite.as_ref().unwrap();
        let Some(img) = r.sprite.as_ref().and_then(|id| sprites.get(id)) else {
            continue;
        };
        let color = match look(data, r) {
            Look::Skip => continue,
            Look::Sprite => r.color,
            Look::Tint(c) => [0, 1, 2, 3].map(|k| r.color[k] * c[k]),
            Look::Gradient(bot, top) => {
                // The sprite's rect in local units, filled with the gradient.
                let (sw, sh) = (img.size_units[0], img.size_units[1]);
                let s = data.sprites.get(r.sprite.as_deref().unwrap_or(""));
                let (px0, py0) = s.map_or((0.5, 0.5), |s| (s.pivot[0], s.pivot[1]));
                let unit = Transform::from_row(sw, 0.0, 0.0, sh, -px0 * sw, -py0 * sh);
                let tf = unit.post_concat(world_tf(o)).post_concat(view);
                let tint = |c: [f32; 4]| [0, 1, 2, 3].map(|k| c[k] * r.color[k]);
                if let Some(paint) = gradient_paint(tint(bot), tint(top))
                    && let Some(rect) = Rect::from_xywh(0.0, 0.0, 1.0, 1.0)
                {
                    px.fill_rect(rect, &paint, tf, None);
                }
                continue;
            }
        };
        let key = (r.sprite.clone().unwrap(), color.map(f32::to_bits));
        let tex = tints
            .entry(key)
            .or_insert_with(|| tinted(&img.pixmap, color));
        let mut local = img.local;
        if r.draw_mode != 0 && img.size_units[0] > 0.0 && img.size_units[1] > 0.0 {
            local = local.post_scale(r.size[0] / img.size_units[0], r.size[1] / img.size_units[1]);
        }
        local = local.post_scale(
            if r.flip_x { -1.0 } else { 1.0 },
            if r.flip_y { -1.0 } else { 1.0 },
        );
        let tf = local.post_concat(world_tf(o)).post_concat(view);
        let paint = PixmapPaint {
            quality: tiny_skia::FilterQuality::Bilinear,
            ..Default::default()
        };
        px.draw_pixmap(0, 0, tex.as_ref(), &paint, tf, None);
    }
    for o in &level.objects {
        if o.active_in_hierarchy
            && o.shape.as_ref().is_some_and(|s| s.enabled)
            && (all_shapes || !drawn(o))
        {
            outline(&mut px, o, view, Color::from_rgba8(80, 220, 120, 220));
        }
    }
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 60, 60, 255);
    for sp in &level.team_spawns {
        let p = view.map_point_xy(fx(sp[0]), fx(sp[1]));
        if let Some(c) = PathBuilder::from_circle(p.0, p.1, 2.5) {
            px.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
    label(&mut px, level.build_index, 2.0);
    px
}

/// Maps a point through a transform.
trait MapPoint {
    fn map_point_xy(&self, x: f32, y: f32) -> (f32, f32);
}

impl MapPoint for Transform {
    fn map_point_xy(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.sx * x + self.kx * y + self.tx,
            self.ky * x + self.sy * y + self.ty,
        )
    }
}

fn main() -> Result<()> {
    let root = std::env::var_os("BOPL_ROOT")
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut data_dir = root.join("cache/import");
    if !data_dir.join("index.json").is_file()
        && let Some(d) = bopl_import::paths::import_dir()
    {
        data_dir = d;
    }
    let mut out = root.join("scratch/levels-sheet.png");
    let (mut cols, mut tw, mut th, mut shapes) = (12u32, 320u32, 180u32, false);
    let mut only: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data" => data_dir = args.next().context("--data DIR")?.into(),
            "--out" => out = args.next().context("--out FILE")?.into(),
            "--cols" => cols = args.next().context("--cols N")?.parse()?,
            "--tile" => {
                let t = args.next().context("--tile WxH")?;
                let (a, b) = t.split_once('x').context("--tile WxH")?;
                (tw, th) = (a.parse()?, b.parse()?);
            }
            "--shapes" => shapes = true,
            "--level" => only = Some(args.next().context("--level N|NAME")?),
            _ => anyhow::bail!("unknown argument {a}"),
        }
    }
    let data = GameData::load(&data_dir)?;
    let wanted: Vec<usize> = data
        .index
        .scenes
        .iter()
        .filter(|e| e.level && e.file.is_some())
        .filter(|e| {
            only.as_ref()
                .is_none_or(|n| &e.name == n || e.build_index.to_string() == *n)
        })
        .map(|e| e.build_index)
        .collect();
    let levels: Vec<Level> = wanted
        .par_iter()
        .map(|&i| data.level(i))
        .collect::<Result<_>>()?;
    let sprites = load_sprites(&data, &levels);
    let tiles: Vec<Pixmap> = levels
        .par_iter()
        .map(|l| render_level(&data, l, &sprites, tw, th, shapes))
        .collect();
    let cols = cols.min(tiles.len().max(1) as u32);
    let rows = (tiles.len() as u32).div_ceil(cols);
    let gap = 4;
    let mut sheet =
        Pixmap::new(cols * (tw + gap) + gap, rows * (th + gap) + gap).context("sheet size")?;
    sheet.fill(Color::from_rgba8(60, 60, 64, 255));
    for (i, t) in tiles.iter().enumerate() {
        let (c, r) = (i as u32 % cols, i as u32 / cols);
        sheet.draw_pixmap(
            (gap + c * (tw + gap)) as i32,
            (gap + r * (th + gap)) as i32,
            t.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    sheet.save_png(&out)?;
    println!(
        "{} levels -> {} ({}x{})",
        tiles.len(),
        out.display(),
        sheet.width(),
        sheet.height()
    );
    Ok(())
}

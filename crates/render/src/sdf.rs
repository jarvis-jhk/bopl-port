//! Signed-distance-field fonts: TextMeshPro font assets.
//!
//! Static assets come with an SDF atlas texture and a glyph table
//! ([`SdfFontDesc`]). Dynamic assets (TMP "dynamic" population mode: the
//! `dyn_*` fonts of the original) have an empty atlas; their glyphs are
//! rasterised from the TrueType file on first use ([`DynamicSdfDesc`]),
//! converted to a distance field with the same encoding TMP uses (0.5 at the
//! outline, `1 / (2 (padding + 1))` per atlas texel) and packed into one
//! shared atlas ([`crate::TextureId`] `DYN_SDF`).

use std::collections::{HashMap, HashSet};

use glam::Vec2;

use crate::RenderError;
use crate::text::{FontId, SDF_BASE};
use crate::texture::TextureId;

/// One glyph of an SDF atlas, in atlas pixels at the font's point size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SdfGlyph {
    pub unicode: u32,
    /// Rect in the atlas `(x, y, w, h)`, top-left origin (PNG space),
    /// padding not included.
    pub rect: [f32; 4],
    /// Pen position to the glyph's left edge.
    pub bearing_x: f32,
    /// Baseline to the glyph's top edge (up).
    pub bearing_y: f32,
    pub advance: f32,
}

/// A static SDF font: an atlas texture (distance in alpha, 0.5 at the edge)
/// plus metrics, e.g. a TextMeshPro font asset from the import.
#[derive(Clone, Debug)]
pub struct SdfFontDesc {
    pub atlas: TextureId,
    pub atlas_size: (u32, u32),
    /// Size the metrics are given at (TMP `m_PointSize`).
    pub point_size: f32,
    pub ascent: f32,
    /// Negative.
    pub descent: f32,
    pub line_height: f32,
    /// Atlas padding around each glyph rect (pixels).
    pub padding: f32,
    pub glyphs: Vec<SdfGlyph>,
    /// Kerning pairs `(first char, second char, x advance adjustment)` in
    /// atlas pixels at `point_size`.
    pub kerning: Vec<(u32, u32, f32)>,
}

/// A dynamic SDF font: glyphs are generated from `ttf` at `point_size`
/// with `padding` texels of distance field around them.
#[derive(Clone, Debug)]
pub struct DynamicSdfDesc {
    pub ttf: Vec<u8>,
    pub point_size: f32,
    pub padding: f32,
    /// Line metrics at `point_size`; `None` takes them from the font file.
    pub line_metrics: Option<(f32, f32, f32)>,
}

/// A glyph ready to draw: where it is and how to place it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlacedSdfGlyph {
    pub atlas: TextureId,
    pub atlas_size: (u32, u32),
    pub rect: [f32; 4],
    pub bearing_x: f32,
    pub bearing_y: f32,
    pub advance: f32,
    pub padding: f32,
    pub point_size: f32,
}

pub(crate) struct SdfFont {
    point_size: f32,
    ascent: f32,
    descent: f32,
    line_height: f32,
    padding: f32,
    glyphs: HashMap<u32, PlacedSdfGlyph>,
    kerning: HashMap<(u32, u32), f32>,
    source: Option<fontdue::Font>,
    missing: HashSet<u32>,
    /// Fonts tried, in order, for characters this font lacks (TMP fallback
    /// font asset table).
    pub fallbacks: Vec<FontId>,
}

/// A quad of laid-out SDF text: top-left (y down) and size in output units
/// relative to the block's top-left.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SdfQuad {
    pub top_left: Vec2,
    pub size: Vec2,
    pub glyph: PlacedSdfGlyph,
}

/// Side length of the dynamic SDF atlas.
pub const DYN_ATLAS_SIZE: u32 = 2048;

/// All SDF fonts plus the shared atlas of generated glyphs.
pub(crate) struct SdfFonts {
    fonts: Vec<SdfFont>,
    pub atlas: Vec<u8>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    /// Rows `[min, max)` changed since the last upload.
    pub dirty_rows: Option<(u32, u32)>,
}

impl SdfFonts {
    pub fn new() -> Self {
        Self {
            fonts: Vec::new(),
            atlas: vec![0; (DYN_ATLAS_SIZE * DYN_ATLAS_SIZE * 4) as usize],
            shelf_x: 0,
            shelf_y: 0,
            shelf_h: 0,
            dirty_rows: None,
        }
    }

    pub fn add_static(&mut self, d: SdfFontDesc) -> FontId {
        let glyphs = d
            .glyphs
            .iter()
            .map(|g| {
                (
                    g.unicode,
                    PlacedSdfGlyph {
                        atlas: d.atlas,
                        atlas_size: d.atlas_size,
                        rect: g.rect,
                        bearing_x: g.bearing_x,
                        bearing_y: g.bearing_y,
                        advance: g.advance,
                        padding: d.padding,
                        point_size: d.point_size,
                    },
                )
            })
            .collect();
        self.push(SdfFont {
            point_size: d.point_size,
            ascent: d.ascent,
            descent: d.descent,
            line_height: d.line_height,
            padding: d.padding,
            glyphs,
            kerning: d.kerning.iter().map(|&(a, b, k)| ((a, b), k)).collect(),
            source: None,
            missing: HashSet::new(),
            fallbacks: Vec::new(),
        })
    }

    pub fn add_dynamic(&mut self, d: DynamicSdfDesc) -> Result<FontId, RenderError> {
        let font = fontdue::Font::from_bytes(d.ttf.as_slice(), fontdue::FontSettings::default())
            .map_err(|e| RenderError::Font(e.to_string()))?;
        let (ascent, descent, line_height) = d.line_metrics.unwrap_or_else(|| {
            font.horizontal_line_metrics(d.point_size).map_or(
                (d.point_size * 0.8, -d.point_size * 0.2, d.point_size * 1.2),
                |m| (m.ascent, m.descent, m.new_line_size),
            )
        });
        Ok(self.push(SdfFont {
            point_size: d.point_size,
            ascent,
            descent,
            line_height,
            padding: d.padding,
            glyphs: HashMap::new(),
            kerning: HashMap::new(),
            source: Some(font),
            missing: HashSet::new(),
            fallbacks: Vec::new(),
        }))
    }

    fn push(&mut self, f: SdfFont) -> FontId {
        self.fonts.push(f);
        FontId(SDF_BASE + self.fonts.len() as u32 - 1)
    }

    fn index(&self, id: FontId) -> Option<usize> {
        let i = id.0.checked_sub(SDF_BASE)? as usize;
        (i < self.fonts.len()).then_some(i)
    }

    pub fn contains(&self, id: FontId) -> bool {
        self.index(id).is_some()
    }

    pub fn set_fallbacks(&mut self, id: FontId, fallbacks: &[FontId]) {
        if let Some(i) = self.index(id) {
            self.fonts[i].fallbacks = fallbacks.to_vec();
        }
    }

    /// The glyph for `ch` in font `i` only (generating it for dynamic fonts).
    fn own_glyph(&mut self, i: usize, ch: u32) -> Option<PlacedSdfGlyph> {
        if let Some(g) = self.fonts[i].glyphs.get(&ch) {
            return Some(*g);
        }
        if self.fonts[i].missing.contains(&ch) || self.fonts[i].source.is_none() {
            return None;
        }
        let g = self.generate(i, ch);
        match g {
            Some(g) => {
                self.fonts[i].glyphs.insert(ch, g);
            }
            None => {
                self.fonts[i].missing.insert(ch);
            }
        }
        g
    }

    /// Resolves `ch`: the font itself, its fallbacks (depth first, like TMP),
    /// then the other letter case.
    fn resolve(&mut self, i: usize, ch: char) -> Option<PlacedSdfGlyph> {
        let mut seen = Vec::new();
        if let Some(g) = self.resolve_in(i, ch as u32, &mut seen) {
            return Some(g);
        }
        let alt = if ch.is_lowercase() {
            ch.to_uppercase().next()
        } else {
            ch.to_lowercase().next()
        }?;
        if alt == ch {
            return None;
        }
        seen.clear();
        self.resolve_in(i, alt as u32, &mut seen)
    }

    fn resolve_in(&mut self, i: usize, ch: u32, seen: &mut Vec<usize>) -> Option<PlacedSdfGlyph> {
        if seen.contains(&i) {
            return None;
        }
        seen.push(i);
        if let Some(g) = self.own_glyph(i, ch) {
            return Some(g);
        }
        let fallbacks = self.fonts[i].fallbacks.clone();
        for fb in fallbacks {
            if let Some(j) = self.index(fb)
                && let Some(g) = self.resolve_in(j, ch, seen)
            {
                return Some(g);
            }
        }
        None
    }

    /// Generates a glyph of dynamic font `i` into the shared atlas.
    fn generate(&mut self, i: usize, ch: u32) -> Option<PlacedSdfGlyph> {
        let c = char::from_u32(ch)?;
        let f = &self.fonts[i];
        let font = f.source.as_ref()?;
        let gi = font.lookup_glyph_index(c);
        if gi == 0 && !c.is_whitespace() {
            return None;
        }
        let (pt, pad) = (f.point_size, f.padding);
        let metrics = font.metrics_indexed(gi, pt);
        let mut g = PlacedSdfGlyph {
            atlas: TextureId::DYN_SDF,
            atlas_size: (DYN_ATLAS_SIZE, DYN_ATLAS_SIZE),
            rect: [0.0; 4],
            bearing_x: metrics.xmin as f32,
            bearing_y: (metrics.ymin + metrics.height as i32) as f32,
            advance: metrics.advance_width,
            padding: pad,
            point_size: pt,
        };
        if metrics.width == 0 || metrics.height == 0 {
            return Some(g);
        }
        // Rasterise at twice the size, distance-transform, downsample.
        const SS: usize = 2;
        let (m2, cov) = font.rasterize_indexed(gi, pt * SS as f32);
        let pad_px = pad.ceil() as usize;
        let field = signed_distance(&cov, m2.width, m2.height, (pad_px + 1) * SS);
        let (fw, fh) = (
            m2.width + 2 * (pad_px + 1) * SS,
            m2.height + 2 * (pad_px + 1) * SS,
        );
        let (w, h) = (fw / SS, fh / SS);
        let spread = 2.0 * (pad + 1.0);
        let mut alpha = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut d = 0.0;
                for sy in 0..SS {
                    for sx in 0..SS {
                        d += field[(y * SS + sy) * fw + x * SS + sx];
                    }
                }
                // Distance in output texels, positive outside.
                let d = d / (SS * SS) as f32 / SS as f32;
                alpha[y * w + x] = ((0.5 - d / spread).clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
        // The glyph rect excludes TMP's padding; our bitmap has pad + 1
        // texels around the (2x-derived) glyph box.
        let (ax, ay) = self.alloc(w as u32, h as u32)?;
        for row in 0..h {
            for col in 0..w {
                let p = (((ay as usize + row) * DYN_ATLAS_SIZE as usize) + ax as usize + col) * 4;
                self.atlas[p..p + 4].copy_from_slice(&[255, 255, 255, alpha[row * w + col]]);
            }
        }
        let (lo, hi) = self.dirty_rows.unwrap_or((ay, ay + h as u32));
        self.dirty_rows = Some((lo.min(ay), hi.max(ay + h as u32)));
        let inset = (pad_px + 1) as f32;
        g.rect = [
            ax as f32 + inset,
            ay as f32 + inset,
            w as f32 - 2.0 * inset,
            h as f32 - 2.0 * inset,
        ];
        // The 2x raster's box, mapped back, may differ by a texel from the
        // 1x metrics; keep the 2x geometry so the bitmap lines up.
        g.bearing_x = m2.xmin as f32 / SS as f32;
        g.bearing_y = (m2.ymin + m2.height as i32) as f32 / SS as f32;
        Some(g)
    }

    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if self.shelf_x + w + 1 > DYN_ATLAS_SIZE {
            self.shelf_x = 0;
            self.shelf_y += self.shelf_h + 1;
            self.shelf_h = 0;
        }
        if self.shelf_y + h + 1 > DYN_ATLAS_SIZE || w + 1 > DYN_ATLAS_SIZE {
            log::warn!("dynamic SDF atlas full");
            return None;
        }
        let at = (self.shelf_x, self.shelf_y);
        self.shelf_x += w + 1;
        self.shelf_h = self.shelf_h.max(h);
        Some(at)
    }

    /// Lays out `text` (lines split at `\n`) at font size `size` (output
    /// units); `align` 0 left, 0.5 centre, 1 right. Returns the quads and the
    /// block size.
    #[cfg(test)]
    pub fn layout(
        &mut self,
        font: FontId,
        text: &str,
        size: f32,
        align: f32,
    ) -> (Vec<SdfQuad>, Vec2) {
        self.layout_spaced(font, text, size, align, 0.0)
    }

    /// [`Self::layout`] with `spacing` em added between characters (TMP's
    /// character spacing, bold spacing and normal spacing offset, / 100).
    pub fn layout_spaced(
        &mut self,
        font: FontId,
        text: &str,
        size: f32,
        align: f32,
        spacing: f32,
    ) -> (Vec<SdfQuad>, Vec2) {
        let Some(fi) = self.index(font) else {
            return (Vec::new(), Vec2::ZERO);
        };
        let (pt, ascent, descent, line_h) = {
            let f = &self.fonts[fi];
            (f.point_size.max(1e-3), f.ascent, f.descent, f.line_height)
        };
        let k = size / pt;
        let mut out = Vec::new();
        let mut widths = Vec::new();
        let mut starts = Vec::new();
        for (li, line) in text.split('\n').enumerate() {
            starts.push(out.len());
            let baseline = (ascent + li as f32 * line_h) * k;
            let mut pen = 0.0f32;
            let mut prev: Option<char> = None;
            for (ci, ch) in line.chars().enumerate() {
                if ci > 0 {
                    pen += spacing * size;
                }
                let Some(g) = self.resolve(fi, ch) else {
                    pen += size * 0.3;
                    prev = None;
                    continue;
                };
                // Glyph units -> output units for the font that owns it.
                let gk = size / g.point_size.max(1e-3);
                if let Some(p) = prev {
                    pen += self.fonts[fi]
                        .kerning
                        .get(&(p as u32, ch as u32))
                        .copied()
                        .unwrap_or(0.0)
                        * k;
                }
                prev = Some(ch);
                let [_, _, w, h] = g.rect;
                if w > 0.0 && h > 0.0 {
                    let pad = g.padding;
                    let tl = Vec2::new(
                        pen + (g.bearing_x - pad) * gk,
                        baseline - (g.bearing_y + pad) * gk,
                    );
                    out.push(SdfQuad {
                        top_left: tl,
                        size: Vec2::new(w + 2.0 * pad, h + 2.0 * pad) * gk,
                        glyph: g,
                    });
                }
                pen += g.advance * gk;
            }
            widths.push(pen);
        }
        let width = widths.iter().copied().fold(0.0, f32::max);
        starts.push(out.len());
        for (li, w) in widths.iter().enumerate() {
            let shift = (width - w) * align;
            for q in &mut out[starts[li]..starts[li + 1]] {
                q.top_left.x += shift;
            }
        }
        let lines = widths.len().max(1) as f32;
        let height = (ascent + (lines - 1.0) * line_h - descent) * k;
        (out, Vec2::new(width, height))
    }
}

/// Signed distance (pixels, positive outside) of a coverage bitmap, on a
/// grid enlarged by `margin` pixels on every side. Coverage >= 0.5 is
/// inside; distances are measured to the boundary between pixel centres.
pub(crate) fn signed_distance(cov: &[u8], w: usize, h: usize, margin: usize) -> Vec<f32> {
    let (fw, fh) = (w + 2 * margin, h + 2 * margin);
    let inside = |x: usize, y: usize| -> bool {
        x >= margin
            && y >= margin
            && x < margin + w
            && y < margin + h
            && cov[(y - margin) * w + (x - margin)] >= 128
    };
    const INF: f32 = 1e20;
    let mut to_in = vec![INF; fw * fh];
    let mut to_out = vec![INF; fw * fh];
    for y in 0..fh {
        for x in 0..fw {
            if inside(x, y) {
                to_in[y * fw + x] = 0.0;
            } else {
                to_out[y * fw + x] = 0.0;
            }
        }
    }
    edt_2d(&mut to_in, fw, fh);
    edt_2d(&mut to_out, fw, fh);
    to_in
        .iter()
        .zip(&to_out)
        .map(|(&a, &b)| {
            if a > 0.0 {
                a.sqrt() - 0.5
            } else {
                -(b.sqrt() - 0.5)
            }
        })
        .collect()
}

/// Squared Euclidean distance transform in place (Felzenszwalb &
/// Huttenlocher): `g` holds 0 at feature pixels and a large value elsewhere.
fn edt_2d(g: &mut [f32], w: usize, h: usize) {
    let n = w.max(h);
    let mut f = vec![0.0f32; n];
    let mut d = vec![0.0f32; n];
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f32; n + 1];
    for x in 0..w {
        for y in 0..h {
            f[y] = g[y * w + x];
        }
        edt_1d(&f[..h], &mut d[..h], &mut v, &mut z);
        for y in 0..h {
            g[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&g[y * w..y * w + w]);
        edt_1d(&f[..w], &mut d[..w], &mut v, &mut z);
        g[y * w..y * w + w].copy_from_slice(&d[..w]);
    }
}

fn edt_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let meet = |q: usize, p: usize| {
        ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * (q as f32 - p as f32))
    };
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        let mut s = meet(q, v[k]);
        // z[0] is -inf, so this stops at k = 0.
        while s <= z[k] {
            k -= 1;
            s = meet(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f32::INFINITY;
    }
    k = 0;
    for (q, dq) in d.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        let dx = q as f32 - p as f32;
        *dq = dx * dx + f[p];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_of_a_square() {
        // A 4x4 opaque square: centre texels are 1.5..2 px inside, a texel
        // 3 px away is ~2.5 px outside.
        let cov = vec![255u8; 16];
        let d = signed_distance(&cov, 4, 4, 3);
        let fw = 10;
        let at = |x: usize, y: usize| d[y * fw + x];
        assert!(at(4, 4) < 0.0 && at(4, 4) > -2.0, "{}", at(4, 4));
        assert!((at(2, 5) - 0.5).abs() < 1e-4, "{}", at(2, 5));
        assert!((at(3, 5) + 0.5).abs() < 1e-4, "{}", at(3, 5));
        assert!((at(0, 5) - 2.5).abs() < 1e-4, "{}", at(0, 5));
        // Diagonal outside the corner.
        assert!((at(1, 1) - (8.0f32.sqrt() - 0.5)).abs() < 1e-4);
    }

    #[test]
    fn dynamic_glyphs_are_generated_and_cached() {
        let mut fonts = SdfFonts::new();
        let id = fonts
            .add_dynamic(DynamicSdfDesc {
                ttf: crate::DEFAULT_FONT_BYTES.to_vec(),
                point_size: 40.0,
                padding: 4.0,
                line_metrics: None,
            })
            .unwrap();
        let (q, size) = fonts.layout(id, "Ab", 20.0, 0.0);
        assert_eq!(q.len(), 2);
        assert!(size.x > 15.0 && size.y > 15.0, "{size}");
        assert!(fonts.dirty_rows.is_some());
        let g = q[0].glyph;
        assert_eq!(g.atlas, TextureId::DYN_SDF);
        // The glyph's centre is inside (> 0.5), the padding outside (< 0.5).
        let [x, y, w, h] = g.rect;
        let px =
            |x: f32, y: f32| fonts.atlas[((y as u32 * DYN_ATLAS_SIZE + x as u32) * 4 + 3) as usize];
        assert!(px(x - 3.0, y + h / 2.0) < 100);
        let row = (y + h * 0.9) as u32;
        let any_inside = (x as u32..(x + w) as u32).any(|cx| px(cx as f32, row as f32) > 160);
        assert!(any_inside);
        // Cached: same rect the second time.
        let (q2, _) = fonts.layout(id, "A", 20.0, 0.0);
        assert_eq!(q2[0].glyph.rect, q[0].glyph.rect);
    }

    #[test]
    fn fallbacks_and_case() {
        let mut fonts = SdfFonts::new();
        let glyph = |u: char| SdfGlyph {
            unicode: u as u32,
            rect: [0.0, 0.0, 10.0, 10.0],
            bearing_x: 0.0,
            bearing_y: 10.0,
            advance: 12.0,
        };
        let desc = |glyphs| SdfFontDesc {
            atlas: TextureId::WHITE,
            atlas_size: (64, 64),
            point_size: 10.0,
            ascent: 10.0,
            descent: -2.0,
            line_height: 12.0,
            padding: 1.0,
            glyphs,
            kerning: vec![],
        };
        let a = fonts.add_static(desc(vec![glyph('A')]));
        let b = fonts.add_static(desc(vec![glyph('Z')]));
        fonts.set_fallbacks(a, &[b]);
        let (q, _) = fonts.layout(a, "aZ?", 10.0, 0.0);
        assert_eq!(q.len(), 2, "a -> A by case, Z from the fallback, ? missing");
    }
}

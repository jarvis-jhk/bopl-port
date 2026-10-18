//! Text: fonts, a dynamic glyph atlas and simple layout.
//!
//! Raster text: glyphs are rasterised with `fontdue` at the pixel size they
//! are displayed at and packed into one RGBA8 atlas (white, coverage in
//! alpha), so text is drawn by the ordinary sprite path. Used for developer
//! overlays with the embedded font; the game's own TextMeshPro fonts are
//! signed-distance-field fonts ([`crate::sdf`]).

use std::collections::HashMap;

use glam::Vec2;

use crate::RenderError;

/// The embedded default font: Noto Sans Bold (SIL OFL 1.1, see
/// `assets/fonts/OFL.txt`).
pub const DEFAULT_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/NotoSans-Bold.ttf");

/// Handle to a loaded font.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct FontId(pub(crate) u32);

impl FontId {
    /// The embedded Noto Sans Bold.
    pub const DEFAULT: FontId = FontId(0);

    /// Whether this is a signed-distance-field font
    /// ([`SdfFontDesc`](crate::SdfFontDesc)).
    pub fn is_sdf(self) -> bool {
        self.0 >= SDF_BASE
    }
}

/// Font ids from here on are SDF fonts.
pub(crate) const SDF_BASE: u32 = 1 << 16;

/// Atlas side length in pixels.
pub const ATLAS_SIZE: u32 = 2048;
/// Largest rasterised pixel size; bigger text is scaled up from this.
pub const MAX_GLYPH_PX: u32 = 256;
const PADDING: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: u32,
    glyph: u16,
    px: u32,
}

/// Where a rasterised glyph lives in the atlas and how to place it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AtlasGlyph {
    /// Pixel rect in the atlas (x, y, w, h), top-left origin.
    pub rect: [u32; 4],
    /// Offset of the bitmap's left edge from the pen position (pixels).
    pub xmin: f32,
    /// Offset of the bitmap's bottom edge above the baseline (pixels).
    pub ymin: f32,
}

/// A laid-out glyph, positioned in a pixel box whose origin is the top-left
/// of the text block, y down.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlacedGlyph {
    pub atlas: AtlasGlyph,
    /// Top-left of the bitmap in the block (pixels, y down).
    pub pos: Vec2,
}

/// Fonts plus the shared CPU-side atlas.
pub(crate) struct TextSystem {
    fonts: Vec<fontdue::Font>,
    /// Fonts tried, in order, for characters the requested font lacks
    /// (e.g. a system CJK font).
    pub fallbacks: Vec<FontId>,
    glyphs: HashMap<GlyphKey, AtlasGlyph>,
    pub atlas: Vec<u8>,
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    /// Rows `[min, max)` of the atlas that changed since the last upload.
    pub dirty_rows: Option<(u32, u32)>,
    /// Set when the atlas overflowed; it is cleared at the start of the next
    /// frame.
    full: bool,
}

impl TextSystem {
    pub fn new() -> Self {
        let mut s = Self {
            fonts: Vec::new(),
            fallbacks: Vec::new(),
            glyphs: HashMap::new(),
            atlas: vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
            shelf_x: 0,
            shelf_y: 0,
            shelf_h: 0,
            dirty_rows: None,
            full: false,
        };
        s.add_font(DEFAULT_FONT_BYTES)
            .expect("embedded font parses");
        // Make the top-left texel white so the atlas could double as a
        // white texture; also keeps glyph rects off the very edge.
        s.atlas[..4].copy_from_slice(&[255; 4]);
        s.shelf_x = 2;
        s
    }

    pub fn add_font(&mut self, bytes: &[u8]) -> Result<FontId, RenderError> {
        self.add_font_indexed(bytes, 0)
    }

    /// A font from a file that may be a collection (`.ttc`): face `index`.
    pub fn add_font_indexed(&mut self, bytes: &[u8], index: u32) -> Result<FontId, RenderError> {
        let settings = fontdue::FontSettings {
            collection_index: index,
            ..Default::default()
        };
        let font = fontdue::Font::from_bytes(bytes, settings)
            .map_err(|e| RenderError::Font(e.to_string()))?;
        self.fonts.push(font);
        Ok(FontId(self.fonts.len() as u32 - 1))
    }

    pub fn has_char(&self, font: FontId, ch: char) -> bool {
        self.font(font).lookup_glyph_index(ch) != 0
    }

    /// The font and glyph that draw `ch`: `font` if it has it, else the
    /// first fallback that does (else `font`'s missing glyph).
    fn resolve(&self, font: FontId, ch: char) -> (FontId, u16) {
        let gi = self.font(font).lookup_glyph_index(ch);
        if gi != 0 || ch.is_whitespace() {
            return (font, gi);
        }
        for &fb in &self.fallbacks {
            let g = self.font(fb).lookup_glyph_index(ch);
            if g != 0 {
                return (fb, g);
            }
        }
        (font, gi)
    }

    fn font(&self, id: FontId) -> &fontdue::Font {
        self.fonts.get(id.0 as usize).unwrap_or(&self.fonts[0])
    }

    /// Clears the atlas if it overflowed during the previous frame.
    pub fn begin_frame(&mut self) {
        if self.full {
            log::warn!("glyph atlas full; clearing");
            self.glyphs.clear();
            self.atlas.fill(0);
            self.atlas[..4].copy_from_slice(&[255; 4]);
            self.shelf_x = 2;
            self.shelf_y = 0;
            self.shelf_h = 0;
            self.dirty_rows = Some((0, ATLAS_SIZE));
            self.full = false;
        }
    }

    /// Line metrics at `px`: (ascent, descent (negative), line height).
    pub fn line_metrics(&self, font: FontId, px: f32) -> (f32, f32, f32) {
        match self.font(font).horizontal_line_metrics(px) {
            Some(m) => (m.ascent, m.descent, m.new_line_size),
            None => (px * 0.8, -px * 0.2, px * 1.2),
        }
    }

    fn glyph(&mut self, font: FontId, glyph: u16, px: u32) -> AtlasGlyph {
        let key = GlyphKey {
            font: font.0,
            glyph,
            px,
        };
        if let Some(g) = self.glyphs.get(&key) {
            return *g;
        }
        let (metrics, bitmap) = self.font(font).rasterize_indexed(glyph, px as f32);
        let (w, h) = (metrics.width as u32, metrics.height as u32);
        let mut out = AtlasGlyph {
            rect: [0, 0, w, h],
            xmin: metrics.xmin as f32,
            ymin: metrics.ymin as f32,
        };
        if w > 0 && h > 0 {
            if self.shelf_x + w + PADDING > ATLAS_SIZE {
                self.shelf_x = 0;
                self.shelf_y += self.shelf_h + PADDING;
                self.shelf_h = 0;
            }
            if self.shelf_y + h + PADDING > ATLAS_SIZE {
                // Out of space: draw nothing this frame, rebuild next frame.
                self.full = true;
                out.rect = [0, 0, 0, 0];
                return out;
            }
            let (x0, y0) = (self.shelf_x, self.shelf_y);
            for row in 0..h {
                for col in 0..w {
                    let a = bitmap[(row * w + col) as usize];
                    let i = (((y0 + row) * ATLAS_SIZE + x0 + col) * 4) as usize;
                    self.atlas[i..i + 4].copy_from_slice(&[255, 255, 255, a]);
                }
            }
            out.rect = [x0, y0, w, h];
            self.shelf_x += w + PADDING;
            self.shelf_h = self.shelf_h.max(h);
            let (lo, hi) = self.dirty_rows.unwrap_or((y0, y0 + h));
            self.dirty_rows = Some((lo.min(y0), hi.max(y0 + h)));
        }
        self.glyphs.insert(key, out);
        out
    }

    /// Lays out `text` (lines split at `\n`) at pixel size `px`. Returns the
    /// glyphs and the block size; `align` (0 left, 0.5 centre, 1 right)
    /// aligns each line within the block.
    pub fn layout(
        &mut self,
        font: FontId,
        text: &str,
        px: u32,
        align: f32,
        out: &mut Vec<PlacedGlyph>,
    ) -> Vec2 {
        let pxf = px as f32;
        let (ascent, _descent, line_h) = self.line_metrics(font, pxf);
        let mut widths = Vec::new();
        let mut line_starts = Vec::new();
        for (li, line) in text.split('\n').enumerate() {
            line_starts.push(out.len());
            let baseline = ascent + li as f32 * line_h;
            let mut pen = 0.0f32;
            let mut prev: Option<(FontId, u16)> = None;
            for ch in line.chars() {
                let (fid, gi) = self.resolve(font, ch);
                let f = self.font(fid);
                if let Some((pf, p)) = prev
                    && pf == fid
                {
                    pen += f.horizontal_kern_indexed(p, gi, pxf).unwrap_or(0.0);
                }
                let advance = f.metrics_indexed(gi, pxf).advance_width;
                let g = self.glyph(fid, gi, px);
                if g.rect[2] > 0 {
                    out.push(PlacedGlyph {
                        atlas: g,
                        pos: Vec2::new(
                            (pen + g.xmin).round(),
                            (baseline - g.ymin - g.rect[3] as f32).round(),
                        ),
                    });
                }
                pen += advance;
                prev = Some((fid, gi));
            }
            widths.push(pen);
        }
        let width = widths.iter().cloned().fold(0.0, f32::max);
        // Align lines.
        line_starts.push(out.len());
        for (li, w) in widths.iter().enumerate() {
            let shift = ((width - w) * align).round();
            for g in &mut out[line_starts[li]..line_starts[li + 1]] {
                g.pos.x += shift;
            }
        }
        let lines = widths.len() as f32;
        Vec2::new(
            width,
            ascent + (lines - 1.0) * line_h - self.line_metrics(font, pxf).1,
        )
    }

    /// Size of the text block in pixels at `px`, without rasterising.
    pub fn measure(&self, font: FontId, text: &str, px: f32) -> Vec2 {
        let (ascent, descent, line_h) = self.line_metrics(font, px);
        let mut width = 0.0f32;
        let mut lines = 0;
        for line in text.split('\n') {
            lines += 1;
            let mut pen = 0.0;
            let mut prev: Option<(FontId, u16)> = None;
            for ch in line.chars() {
                let (fid, gi) = self.resolve(font, ch);
                let f = self.font(fid);
                if let Some((pf, p)) = prev
                    && pf == fid
                {
                    pen += f.horizontal_kern_indexed(p, gi, px).unwrap_or(0.0);
                }
                pen += f.metrics_indexed(gi, px).advance_width;
                prev = Some((fid, gi));
            }
            width = width.max(pen);
        }
        Vec2::new(width, ascent + (lines - 1) as f32 * line_h - descent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_measure_agree() {
        let mut ts = TextSystem::new();
        let mut out = Vec::new();
        let size = ts.layout(FontId::DEFAULT, "Hello\nBOPL", 32, 0.5, &mut out);
        let m = ts.measure(FontId::DEFAULT, "Hello\nBOPL", 32.0);
        assert!((size - m).length() < 1e-3, "{size} vs {m}");
        assert_eq!(out.len(), 9);
        assert!(size.x > 50.0 && size.y > 60.0);
        // Glyphs stay inside the block (allowing for side bearings).
        for g in &out {
            assert!(g.pos.x >= -2.0 && g.pos.x + g.atlas.rect[2] as f32 <= size.x + 3.0);
            assert!(g.pos.y >= -2.0 && g.pos.y + g.atlas.rect[3] as f32 <= size.y + 2.0);
        }
        assert!(ts.dirty_rows.is_some());
    }

    #[test]
    fn glyphs_are_cached() {
        let mut ts = TextSystem::new();
        let mut out = Vec::new();
        ts.layout(FontId::DEFAULT, "aaaa", 20, 0.0, &mut out);
        let r = out[0].atlas.rect;
        assert!(out.iter().all(|g| g.atlas.rect == r));
    }
}

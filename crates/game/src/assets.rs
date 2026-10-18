//! GPU-side game assets: import textures uploaded on first use, sprite
//! geometry, TextMeshPro fonts registered with the renderer.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use bopl_import::GameData;
use bopl_import::format::Font;
use bopl_render::{
    DynamicSdfDesc, FontId, Gpu, Image, Renderer, SdfFontDesc, SdfGlyph, TextureId, TextureOptions,
    UvRect,
};
use glam::Vec2;

/// How to draw a sprite as a quad: texture, texture rect, size in world
/// units and pivot in 0..1 of the quad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteGeom {
    pub texture: TextureId,
    pub uv: UvRect,
    pub size: Vec2,
    pub pivot: Vec2,
    /// Texture size in pixels.
    pub texture_size: (u32, u32),
    /// The sprite's full rect in world units (before trimming).
    pub rect_size: Vec2,
}

/// Lazily uploaded textures and fonts of one import.
pub struct Assets {
    pub data: Rc<GameData>,
    textures: HashMap<String, (TextureId, (u32, u32))>,
    failed: HashSet<String>,
    fonts: HashMap<String, FontId>,
}

impl Assets {
    pub fn new(data: Rc<GameData>) -> Self {
        Self {
            data,
            textures: HashMap::new(),
            failed: HashSet::new(),
            fonts: HashMap::new(),
        }
    }

    /// The GPU texture of an import texture id (uploads it on first use).
    pub fn texture(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        id: &str,
    ) -> Option<(TextureId, (u32, u32))> {
        if let Some(t) = self.textures.get(id) {
            return Some(*t);
        }
        if self.failed.contains(id) {
            return None;
        }
        let Some(tex) = self.data.textures.get(id) else {
            self.failed.insert(id.to_string());
            return None;
        };
        let path = self.data.dir.join(&tex.png);
        match Image::load_png(&path) {
            Ok(img) => {
                let size = (img.width, img.height);
                let opts = TextureOptions::unity(tex.filter_mode, tex.wrap_mode);
                let t = renderer.create_texture(gpu, &img, opts);
                self.textures.insert(id.to_string(), (t, size));
                Some((t, size))
            }
            Err(e) => {
                log::warn!("texture {id} ({}): {e}", path.display());
                self.failed.insert(id.to_string());
                None
            }
        }
    }

    /// Quad geometry of a sprite (uploads its texture on first use).
    pub fn sprite(&mut self, gpu: &Gpu, renderer: &mut Renderer, id: &str) -> Option<SpriteGeom> {
        let s = self.data.sprites.get(id)?.clone();
        let (texture, (tw, th)) = self.texture(gpu, renderer, s.texture.as_deref()?)?;
        Some(sprite_geom(&s, texture, (tw, th)))
    }

    /// A TextMeshPro font asset by name (`en_se_de_fr_it_es`,
    /// `dyn_russian`, ...), registered with its fallbacks on first use.
    pub fn font(&mut self, gpu: &Gpu, renderer: &mut Renderer, name: &str) -> Option<FontId> {
        if let Some(f) = self.fonts.get(name) {
            return Some(*f);
        }
        let data = self.data.clone();
        let font = data.fonts.iter().find(|f| f.name == name)?;
        let id = self.register_font(gpu, renderer, font)?;
        self.fonts.insert(name.to_string(), id);
        // Fallback font assets (by reference), registered recursively.
        let fallbacks: Vec<String> = font.fields["m_FallbackFontAssetTable"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| r["$ref"].as_str())
                    .filter_map(|rid| data.fonts.iter().find(|f| f.id == rid))
                    .map(|f| f.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let ids: Vec<FontId> = fallbacks
            .iter()
            .filter_map(|n| self.font(gpu, renderer, n))
            .collect();
        if !ids.is_empty() {
            renderer.set_sdf_fallbacks(id, &ids);
        }
        Some(id)
    }

    fn register_font(&mut self, gpu: &Gpu, renderer: &mut Renderer, f: &Font) -> Option<FontId> {
        let info = f.info.as_ref()?;
        let dynamic = f.fields["m_AtlasPopulationMode"].as_i64() == Some(1);
        if dynamic {
            let file = self.data.font_files.get(f.source_font.as_deref()?)?;
            let ttf = std::fs::read(self.data.dir.join(&file.file))
                .map_err(|e| log::warn!("font {}: {e}", file.file))
                .ok()?;
            return renderer
                .add_dynamic_sdf_font(DynamicSdfDesc {
                    ttf,
                    point_size: info.point_size,
                    padding: info.padding,
                    line_metrics: Some((info.ascent, info.descent, info.line_height)),
                })
                .map_err(|e| log::warn!("font {}: {e}", f.name))
                .ok();
        }
        let atlas_id = f.atlases.first()?;
        let (atlas, size) = self.texture(gpu, renderer, atlas_id)?;
        let mut by_glyph: HashMap<u32, Vec<u32>> = HashMap::new();
        for (&u, &g) in &info.characters {
            by_glyph.entry(g).or_default().push(u);
        }
        let mut glyphs = Vec::new();
        for g in &info.glyphs {
            if g.atlas != 0 {
                continue; // only single-atlas fonts in the original
            }
            for &u in by_glyph.get(&g.index).map(Vec::as_slice).unwrap_or(&[]) {
                glyphs.push(SdfGlyph {
                    unicode: u,
                    rect: g.rect_png,
                    bearing_x: g.bearing_x,
                    bearing_y: g.bearing_y,
                    advance: g.advance,
                });
            }
        }
        let mut kerning = Vec::new();
        for k in &info.kerning {
            if let (Some(a), Some(b)) = (by_glyph.get(&k.first), by_glyph.get(&k.second)) {
                for &ua in a {
                    for &ub in b {
                        kerning.push((ua, ub, k.first_adjust[2]));
                    }
                }
            }
        }
        Some(renderer.add_sdf_font(SdfFontDesc {
            atlas,
            atlas_size: size,
            point_size: info.point_size,
            ascent: info.ascent,
            descent: info.descent,
            line_height: info.line_height,
            padding: info.padding,
            glyphs,
            kerning,
        }))
    }
}

/// Quad geometry of a sprite on a texture of `tex_size` pixels.
pub fn sprite_geom(
    s: &bopl_import::format::Sprite,
    texture: TextureId,
    tex_size: (u32, u32),
) -> SpriteGeom {
    let ppu = s.pixels_per_unit.max(1e-6);
    let [tx, ty, tw, th] = s.texture_rect;
    let size = Vec2::new(tw, th) / ppu;
    // The drawn rect is the (possibly trimmed) texture rect, offset inside
    // the sprite rect; the pivot is relative to the full rect.
    let pivot_px = Vec2::new(s.pivot[0] * s.rect[2], s.pivot[1] * s.rect[3]);
    let off = Vec2::new(s.texture_rect_offset[0], s.texture_rect_offset[1]);
    let pivot = Vec2::new(
        if tw > 0.0 {
            (pivot_px.x - off.x) / tw
        } else {
            0.5
        },
        if th > 0.0 {
            (pivot_px.y - off.y) / th
        } else {
            0.5
        },
    );
    SpriteGeom {
        texture,
        uv: UvRect::from_pixels(tx, ty, tw, th, tex_size.0, tex_size.1),
        size,
        pivot,
        texture_size: tex_size,
        rect_size: Vec2::new(s.rect[2], s.rect[3]) / ppu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite(
        rect: [f32; 4],
        tr: [f32; 4],
        off: [f32; 2],
        pivot: [f32; 2],
    ) -> bopl_import::format::Sprite {
        bopl_import::format::Sprite {
            name: "s".into(),
            texture: Some("t".into()),
            rect,
            texture_rect: tr,
            texture_rect_offset: off,
            pivot,
            pixels_per_unit: 100.0,
            border: [0.0; 4],
            packed: false,
            packing_rotation: 0,
            physics_shape: vec![],
        }
    }

    #[test]
    fn untrimmed_sprite() {
        let s = sprite(
            [0.0, 0.0, 200.0, 100.0],
            [0.0, 0.0, 200.0, 100.0],
            [0.0, 0.0],
            [0.5, 0.5],
        );
        let g = sprite_geom(&s, TextureId::WHITE, (200, 100));
        assert_eq!(g.size, Vec2::new(2.0, 1.0));
        assert_eq!(g.pivot, Vec2::splat(0.5));
        assert_eq!(g.uv, UvRect::FULL);
    }

    #[test]
    fn trimmed_sprite_keeps_its_pivot_in_place() {
        // 100x100 rect, pivot centre (50, 50); the visible 60x40 part sits at
        // (30, 10) inside the rect.
        let s = sprite(
            [0.0, 0.0, 100.0, 100.0],
            [0.0, 0.0, 60.0, 40.0],
            [30.0, 10.0],
            [0.5, 0.5],
        );
        let g = sprite_geom(&s, TextureId::WHITE, (64, 64));
        assert_eq!(g.size, Vec2::new(0.6, 0.4));
        // Local x of the quad's left edge = -pivot.x * size.x = (30 - 50)/100.
        assert!((-g.pivot.x * g.size.x - (-0.2)).abs() < 1e-6);
        assert!((-g.pivot.y * g.size.y - (-0.4)).abs() < 1e-6);
    }
}

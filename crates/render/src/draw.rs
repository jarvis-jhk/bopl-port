//! What the presentation hands the renderer each frame: a [`Frame`] of
//! [`Pass`]es, one per camera, each with its items already in draw order.
//!
//! The model follows the original's Unity setup: several cameras render
//! layer subsets, some into render textures that later materials sample
//! (reflection, background, wave particles, platform masks, slime trails),
//! the main camera into the window. The renderer keeps the item order of a
//! pass (the presentation sorts like Unity: sorting layer, order in layer,
//! camera distance) and batches consecutive items that share pipeline and
//! textures.

use glam::{Affine2, Mat4, Vec2, Vec3, Vec4};

use crate::camera::Camera;
use crate::color::Color;
use crate::text::FontId;
use crate::texture::TextureId;

/// How an item blends with the target (Unity `Blend src dst`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Blend {
    /// `Blend One OneMinusSrcAlpha`: premultiplied alpha (sprites and almost
    /// every custom shader of the original).
    #[default]
    Premultiplied,
    /// `Blend One One` (Custom/AdditiveColoredA8).
    Additive,
    /// `Blend OneMinusSrcAlpha SrcAlpha` (Unlit/GradientColor): the item's
    /// alpha says how much of the background *stays*.
    InverseAlpha,
    /// `Blend One OneMinusSrcColor` (Legacy Particles/Additive (Soft)).
    SoftAdditive,
}

/// Sprite-mask stencil behaviour (Unity `SpriteMask` +
/// `SpriteRenderer.maskInteraction`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Stencil {
    /// No interaction.
    #[default]
    None,
    /// Visible only inside the masks drawn so far in the pass.
    VisibleInside,
    /// Visible only outside the masks drawn so far in the pass.
    VisibleOutside,
    /// Adds a mask (stencil +1 where the texture alpha exceeds the cutoff);
    /// draws no colour.
    MaskAdd,
    /// Removes a mask again (stencil -1, same shape); draws no colour.
    MaskRemove,
}

/// A rectangle in normalised texture coordinates, `v = 0` at the top row of
/// the image (PNG order).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvRect {
    pub min: Vec2,
    pub max: Vec2,
}

impl UvRect {
    pub const FULL: UvRect = UvRect {
        min: Vec2::ZERO,
        max: Vec2::ONE,
    };

    /// From a pixel rect with a top-left origin.
    pub fn from_pixels(x: f32, y: f32, w: f32, h: f32, tex_w: u32, tex_h: u32) -> Self {
        let s = Vec2::new(tex_w as f32, tex_h as f32);
        Self {
            min: Vec2::new(x, y) / s,
            max: Vec2::new(x + w, y + h) / s,
        }
    }
}

impl Default for UvRect {
    fn default() -> Self {
        Self::FULL
    }
}

/// Parameters of Custom/ReflectiveWaterWithDistortion (material values plus
/// the three global render textures it reads).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterParams {
    /// `_Alpha`: weight of the reflection/background against the gradient.
    pub alpha: f32,
    pub distortion: f32,
    /// `_WorldHorizonHeight` (world units).
    pub horizon_height: f32,
    pub surface_gradient: f32,
    pub extra_noise: f32,
    pub gradient_height: f32,
    pub horizon_gradient_height: f32,
    /// `_ReflH`: the reflection lookup is taken this far above the pixel.
    pub reflection_height: f32,
    /// `_Gradient` (top) and `_Gradient2` (bottom).
    pub gradient_top: Color,
    pub gradient_bottom: Color,
    pub water_color: Color,
    /// `_WaveParticleRT`, `_ReflectionRT`, `_BackgroundRT`.
    pub wave: TextureId,
    pub reflection: TextureId,
    pub background: TextureId,
}

/// Parameters of Custom/WaterDistortion (the animated wave texture the wave
/// camera renders).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DistortionParams {
    pub noise: TextureId,
    pub strength: f32,
    pub subdivide: f32,
    pub subdivide_noise: f32,
    pub noise_scale: Vec3,
    pub strength3: Vec3,
    pub uv_center: Vec4,
}

/// Parameters of Unlit/aurora.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuroraParams {
    pub noise: TextureId,
    pub mask: TextureId,
    pub height: f32,
    pub blue: f32,
    pub wobbliness: f32,
}

/// Which of the beam shaders (all share the beam quad layout: u along the
/// beam, v across).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BeamKind {
    /// Custom/Beam: the noisy orange core line.
    #[default]
    Core,
    /// Custom/BeamGlow: the red glow band.
    Glow,
    /// Custom/BeamWhite: the white / yellow centre.
    White,
    /// Custom/AdditiveSoftBorders: the additive halo.
    SoftBorders,
}

/// Parameters of the beam shaders (material values; `_Scale` = beam
/// length in quad units, set per beam).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeamParams {
    pub kind: BeamKind,
    pub noise: TextureId,
    /// `_platformRT`: walls hide the beam.
    pub platform: TextureId,
    pub line_width: f32,
    pub noise_freq: f32,
    pub scale: f32,
    pub aura: f32,
    pub time_scale: f32,
    pub dist_str: f32,
    pub dist_str2: f32,
    pub uv_ratio: f32,
    pub uv_ratio2: f32,
}

/// Parameters of Custom/Distortion (a sprite wobbled by scrolling noise).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteDistortParams {
    pub noise: TextureId,
    pub speed: f32,
    pub uv_center: Vec2,
    pub affect_radius: f32,
    pub cc: f32,
    pub cw: f32,
    pub strength: f32,
}

/// Parameters of Custom/LightningA8 (a tiled, mirrored lightning strip with
/// a radial glow).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightningParams {
    /// `_Scale`: repeats along the strip.
    pub scale: f32,
    pub uv_min: f32,
    pub uv_max: f32,
    pub blur: f32,
    pub directions: f32,
    pub quality: f32,
}

/// How a quad is shaded: the original's shaders, reimplemented from their
/// observable behaviour (see `docs/platform.md`, "Shaders").
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Shading {
    /// Texture × colour (Sprites/Default, Custom/FlippableSprite).
    #[default]
    Sprite,
    /// Colour × texture alpha (Custom/SingleColorAlpha,
    /// Custom/AdditiveColoredA8).
    AlphaTint,
    /// Custom/CustomColourShader: red and green channels (relative to blue)
    /// are recoloured with `main` and `shadow`, then blended towards the
    /// renderer colour by its darkness.
    ChannelColour {
        main: Color,
        shadow: Color,
    },
    /// Unlit/GradientColor: vertical gradient in texture space; use with
    /// [`Blend::InverseAlpha`].
    Gradient {
        top: Color,
        bottom: Color,
        alpha_top: f32,
        alpha_bottom: f32,
        height: f32,
    },
    /// Unlit/SlimeTrailedMovable: the platform sprite with its slime-trail
    /// render texture painted on top (`trail_texel` = 1 / its size).
    SlimeTrailed {
        trail: TextureId,
        trail_texel: Vec2,
        flip_x: bool,
        flip_y: bool,
    },
    ReflectiveWater(WaterParams),
    WaterDistortion(DistortionParams),
    Aurora(AuroraParams),
    /// Anti-aliased ellipse filling the quad.
    Ellipse,
    /// Ring of `thickness` (outer radius = half the quad), arc from `start`
    /// over `sweep` radians.
    Ring {
        thickness: f32,
        start: f32,
        sweep: f32,
    },
    /// Custom/AbilityIndicatorWithCircle: the texture (ability icon) over a
    /// centred circle of `radius` (quad units) in `circle` with a rim of
    /// `rim_width` in `rim`; the part not yet charged (clockwise from 12
    /// o'clock over `1 − progress` of the turn) is multiplied by `cd`
    /// (colour and alpha). Texels outside `uv_clamp` (the icon's own rect in
    /// the atlas) are transparent. The vertex colour multiplies the result.
    AbilityIndicator {
        circle: Color,
        rim: Color,
        cd: Color,
        radius: f32,
        rim_width: f32,
        progress: f32,
        uv_clamp: UvRect,
    },
    /// Rounded rectangle filling the quad.
    RoundedRect {
        radius: f32,
    },
    /// Writes the sprite-mask stencil where the texture alpha exceeds
    /// `cutoff` (used with [`Stencil::MaskAdd`] / [`Stencil::MaskRemove`]).
    Mask {
        cutoff: f32,
    },
    /// Texture × colour on a free-form quad: the local corner (1, 1) sits
    /// at `corner` (world) instead of completing the parallelogram (rope
    /// mesh strips).
    SpriteCorner {
        corner: Vec3,
    },
    /// The texture as stored (copies of premultiplied render textures).
    Copy,
    /// Custom/BlackHole2: the screen behind (`grab`) pulled towards the
    /// centre, a glow in the vertex colour and a black hole of radius 0.05
    /// (quad units), anti-aliased over a few pixels (`scale` = the
    /// renderer's world scale).
    BlackHole {
        grab: TextureId,
        distortion: f32,
        proximity: f32,
        scale: f32,
        hole_brightness: f32,
    },
    /// Custom/GrabInvert: the screen behind inverted by the texture's
    /// alpha, plus the vertex colour.
    GrabInvert {
        grab: TextureId,
    },
    /// Custom/Distortion.
    Distort(SpriteDistortParams),
    /// Custom/SmearSprite: three vertically offset samples of the sprite
    /// (its rect `offset` in texture uv, repeated by `scale`), coloured like
    /// the player channel shader, faded towards the rect's ends.
    Smear {
        main: Color,
        shadow: Color,
        offset: Vec4,
        scale: Vec2,
    },
    /// Custom/DrawOnlyWhite: the vertex colour where the red channel is
    /// bright (alpha = (r·a)⁸).
    OnlyWhite,
    /// Custom/ExpandingCircle: a noisy ring (water wave rings).
    ExpandingCircle {
        noise: TextureId,
        /// `_Mask` (red channel scales alpha; WHITE for none).
        mask: TextureId,
        progress: f32,
        dist: Vec3,
        radius: Vec3,
    },
    /// Custom/WhiteToColA8: the vertex colour, blended to white where the
    /// texture alpha is high; vertex alpha unused.
    WhiteToColour,
    /// Custom/WallMaskedA8: colour × texture alpha, hidden where
    /// `_platformRT` has walls.
    WallMasked {
        platform: TextureId,
    },
    /// Custom/ColorSingleChannel: the vertex colour with the alpha of one
    /// texture channel (`mask` weights r, g, b).
    SingleChannel {
        mask: Vec3,
    },
    /// Unlit/ColorChannelsSeparetely: each texture channel paints its own
    /// colour (rgb × a weight).
    Channels3 {
        red: Color,
        green: Color,
        blue: Color,
    },
    /// Unlit/ResizablePlatform: a rounded rectangle (inner `half` extents,
    /// corner `radius`, quad units) in the vertex colour with a black
    /// outline of `border`.
    ResizablePlatform {
        half: Vec2,
        radius: f32,
        border: f32,
    },
    /// Custom/LightningA8.
    Lightning(LightningParams),
    /// The beam shaders.
    Beam(BeamParams),
    /// Custom/CloneEffect: four offset copies of the sprite spreading along
    /// `direction` with `progress` (platform duplication).
    Clone {
        noise: TextureId,
        progress: f32,
        noise_str: f32,
        noise_freq: f32,
        direction: Vec2,
        uv_offset: f32,
    },
    /// Legacy Particles/Alpha Blended Premultiply: colour × texture × colour
    /// alpha (the texture is premultiplied).
    ParticlePremultiply,
    /// Legacy Particles/Alpha Blended: 2 × colour × tint × texture.
    ParticleBlended {
        tint: Color,
    },
}

/// A textured, shaded quad: the one primitive of the renderer.
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    pub texture: TextureId,
    pub uv: UvRect,
    /// Local quad space (units) to world.
    pub transform: Mat4,
    /// Quad size in local units (Unity sprite: rect / pixels per unit).
    pub size: Vec2,
    /// Pivot in 0..1 of the quad; world passes: (0,0) = bottom-left, screen
    /// passes: (0,0) = top-left.
    pub pivot: Vec2,
    /// Vertex colour (the SpriteRenderer colour), straight alpha.
    pub color: Color,
    pub shading: Shading,
    pub blend: Blend,
    pub stencil: Stencil,
}

impl Default for Quad {
    fn default() -> Self {
        Self {
            texture: TextureId::WHITE,
            uv: UvRect::FULL,
            transform: Mat4::IDENTITY,
            size: Vec2::ONE,
            pivot: Vec2::splat(0.5),
            color: Color::WHITE,
            shading: Shading::Sprite,
            blend: Blend::Premultiplied,
            stencil: Stencil::None,
        }
    }
}

impl Quad {
    /// A solid rectangle centred at `center`.
    pub fn rect(center: Vec2, size: Vec2, color: Color) -> Self {
        Self {
            transform: Mat4::from_translation(center.extend(0.0)),
            size,
            color,
            ..Default::default()
        }
    }

    /// A solid circle.
    pub fn circle(center: Vec2, radius: f32, color: Color) -> Self {
        Self {
            shading: Shading::Ellipse,
            ..Self::rect(center, Vec2::splat(radius * 2.0), color)
        }
    }

    /// A line segment of the given width.
    pub fn line(a: Vec2, b: Vec2, width: f32, color: Color) -> Self {
        let d = b - a;
        Self {
            transform: mat4_from_affine2(Affine2::from_angle_translation(
                d.y.atan2(d.x),
                (a + b) * 0.5,
            )),
            size: Vec2::new(d.length(), width),
            color,
            ..Default::default()
        }
    }
}

/// Embeds a 2D affine transform (z unchanged).
pub fn mat4_from_affine2(a: Affine2) -> Mat4 {
    let m = a.matrix2;
    Mat4::from_cols(
        m.x_axis.extend(0.0).extend(0.0),
        m.y_axis.extend(0.0).extend(0.0),
        Vec4::Z,
        a.translation.extend(0.0).extend(1.0),
    )
}

/// Text in an SDF (TextMeshPro) or raster font.
#[derive(Clone, Debug)]
pub struct Text {
    pub text: String,
    pub font: FontId,
    /// Font size (em) in world units, or pixels in screen passes.
    pub size: f32,
    pub position: Vec2,
    /// Rotation in radians around `position`.
    pub rotation: f32,
    /// Which point of the text block sits at `position`, in 0..1:
    /// (0,0) top-left, (0.5,0.5) centre (in every pass). `anchor.x` also
    /// aligns the lines of multi-line text.
    pub anchor: Vec2,
    pub color: Color,
    /// SDF fonts: outline width as a fraction of the distance range
    /// (TMP `_OutlineWidth`, 0..1) and its colour.
    pub outline: f32,
    pub outline_color: Color,
    /// SDF fonts: dilation of the face (TMP `_FaceDilate`, -1..1).
    pub dilate: f32,
    /// SDF fonts: extra advance between characters in em (TMP character
    /// spacing + bold spacing, / 100).
    pub spacing: f32,
    pub stencil: Stencil,
}

impl Default for Text {
    fn default() -> Self {
        Self {
            text: String::new(),
            font: FontId::DEFAULT,
            size: 1.0,
            position: Vec2::ZERO,
            rotation: 0.0,
            anchor: Vec2::ZERO,
            color: Color::WHITE,
            outline: 0.0,
            outline_color: Color::BLACK,
            dilate: 0.0,
            spacing: 0.0,
            stencil: Stencil::None,
        }
    }
}

/// One draw item of a pass.
#[derive(Clone, Debug)]
pub enum Item {
    Quad(Quad),
    Text(Text),
}

impl From<Quad> for Item {
    fn from(q: Quad) -> Self {
        Item::Quad(q)
    }
}

impl From<Text> for Item {
    fn from(t: Text) -> Self {
        Item::Text(t)
    }
}

/// Where a pass draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassTarget {
    /// The frame's output (window or offscreen target).
    Output,
    /// A render texture created with
    /// [`Renderer::render_target`](crate::Renderer::render_target).
    Texture(TextureId),
}

/// The view of a pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    /// A world camera.
    Camera(Camera),
    /// Target pixels, origin top-left, y down (HUD, overlays).
    Screen,
}

/// One camera's rendering.
#[derive(Clone, Debug)]
pub struct Pass {
    pub label: String,
    pub target: PassTarget,
    /// Clear colour (Unity clear flag "solid colour"); `None` keeps the
    /// target's contents ("don't clear", used by the slime-trail cameras).
    pub clear: Option<Color>,
    pub view: View,
    pub items: Vec<Item>,
}

impl Pass {
    pub fn new(label: impl Into<String>, target: PassTarget, view: View) -> Self {
        Self {
            label: label.into(),
            target,
            clear: None,
            view,
            items: Vec::new(),
        }
    }

    pub fn push(&mut self, item: impl Into<Item>) {
        self.items.push(item.into());
    }
}

/// Everything rendered in one frame, in pass order.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub passes: Vec<Pass>,
    /// Shader time in seconds (Unity `_Time.y`).
    pub time: f32,
    /// Global `_Timestop01`: 1 while time runs, 0 during a time stop.
    pub timestop: f32,
}

impl Frame {
    pub fn new() -> Self {
        Self {
            passes: Vec::new(),
            time: 0.0,
            timestop: 1.0,
        }
    }

    /// Number of draw items over all passes.
    pub fn len(&self) -> usize {
        self.passes.iter().map(|p| p.items.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_embedding() {
        let a =
            Affine2::from_scale_angle_translation(Vec2::new(2.0, 3.0), 0.5, Vec2::new(4.0, -1.0));
        let m = mat4_from_affine2(a);
        let p = Vec2::new(0.7, -0.3);
        let want = a.transform_point2(p);
        let got = m.transform_point3(p.extend(0.0));
        assert!((got.truncate() - want).length() < 1e-5);
        assert_eq!(got.z, 0.0);
    }

    #[test]
    fn line_quad() {
        let q = Quad::line(Vec2::ZERO, Vec2::new(0.0, 2.0), 0.1, Color::WHITE);
        assert!((q.size - Vec2::new(2.0, 0.1)).length() < 1e-6);
        let mid = q.transform.transform_point3(Vec3::ZERO);
        assert!((mid - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-6);
    }
}

//! A level's presentation: the original's camera setup rebuilt as render
//! passes.
//!
//! Every level has the same camera rig (levels.md 7): a background camera
//! (Sky layer -> `_BackgroundRT`), a reflection camera (mirrored, everything
//! but sky and water -> `_ReflectionRT`), a perspective wave camera (wave
//! particles -> `_WaveParticleRT`), a platform/water mask camera
//! (`_platformRT`), one slime-trail camera per platform (its own render
//! texture, never cleared) and the main camera (most layers -> the screen).
//! Space levels have only the main and mask cameras. Each camera renders
//! the sprites of its culling mask, sorted like Unity (sorting layer, order
//! in layer, then farther first), with sprite masks applied through the
//! stencil buffer; materials sample the render textures by their global
//! names.
//!
//! Per-frame script behaviour that only affects presentation is reproduced
//! here: `FollowMainCamera` (including its quirk of adding the y offset a
//! second time when z is followed), `FullScreenQuad`, `ParallaxLayer`,
//! `KeepInCameraCorner`, `ScaleBasedOnCameraWidth` and `AlphaFade`.
//! Platform poses and dynamic sprites come from the [`WorldView`].

use std::collections::HashMap;

use bopl_import::GameData;
use bopl_import::format::{Level, Material, Object};
use bopl_render::{
    AuroraParams, BeamKind, BeamParams, Blend, Camera, Color, DistortionParams, Frame, Gpu,
    LightningParams, Pass, PassTarget, Projection, Quad, Renderer, Shading, SpriteDistortParams,
    Stencil, TextureId, View, WaterParams, Wrap,
};
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use serde_json::Value as J;

use crate::assets::Assets;
use crate::present::WorldView;

/// Sorting key of a renderer: (sorting layer index, order in layer).
type SortKey = (usize, i32);

/// A sprite mask's custom sorting range (back, front), inclusive.
type MaskRange = Option<(SortKey, SortKey)>;

/// The original's per-material shading, resolved once per material.
#[derive(Clone, Debug, PartialEq)]
enum MaterialKind {
    Sprite {
        tint: Color,
    },
    AlphaTint {
        additive: bool,
    },
    Channel {
        main: Color,
        shadow: Color,
    },
    Gradient {
        top: Color,
        bottom: Color,
        alpha_top: f32,
        alpha_bottom: f32,
        height: f32,
    },
    SlimeTrailed,
    Water(WaterParams),
    Distortion {
        noise: Option<String>,
        params: DistortionParams,
    },
    Aurora {
        noise: Option<String>,
        mask: Option<String>,
        params: AuroraParams,
    },
    /// Custom/BlackHole2 (grab pass).
    BlackHole {
        distortion: f32,
        proximity: f32,
        scale: f32,
        hole_brightness: f32,
    },
    /// Custom/GrabInvert (grab pass).
    GrabInvert,
    /// Custom/Distortion.
    Distort {
        noise: Option<String>,
        params: SpriteDistortParams,
    },
    Smear {
        main: Color,
        shadow: Color,
        offset: Vec4,
        scale: Vec2,
    },
    OnlyWhite,
    Expanding {
        noise: Option<String>,
        mask: Option<String>,
        progress: f32,
        particle_speed: f32,
        dist: Vec3,
        radius: Vec3,
    },
    WhiteToColour,
    WallMasked,
    SingleChannel {
        mask: Vec3,
    },
    Channels3 {
        red: Color,
        green: Color,
        blue: Color,
    },
    ResizablePlatform,
    Lightning(LightningParams),
    Beam {
        noise: Option<String>,
        params: BeamParams,
    },
    Clone {
        noise: Option<String>,
        progress: f32,
        noise_str: f32,
        noise_freq: f32,
        direction: Vec2,
        uv_offset: f32,
    },
    /// Legacy Particles/Alpha Blended Premultiply.
    ParticlePremultiply,
    /// Legacy Particles/Alpha Blended (`_TintColor`).
    ParticleBlended {
        tint: Color,
    },
    /// Legacy Particles/Additive (Soft).
    SoftAdditive,
}

impl MaterialKind {
    /// Reads the screen behind it (Unity `GrabPass`).
    fn grabs(&self) -> bool {
        matches!(
            self,
            MaterialKind::BlackHole { .. } | MaterialKind::GrabInvert
        )
    }
}

fn col(m: &Material, k: &str, default: Color) -> Color {
    m.colors.get(k).map_or(default, |c| Color::from_array(*c))
}

fn flt(m: &Material, k: &str, default: f32) -> f32 {
    m.floats
        .get(k)
        .copied()
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}

fn tex(m: &Material, k: &str) -> Option<String> {
    m.textures.get(k).and_then(|t| t.texture.clone())
}

fn material_kind(m: Option<&Material>) -> MaterialKind {
    let Some(m) = m else {
        return MaterialKind::Sprite { tint: Color::WHITE };
    };
    let none = TextureId::TRANSPARENT;
    match m.shader.as_str() {
        "Custom/SingleColorAlpha" => MaterialKind::AlphaTint { additive: false },
        "Custom/AdditiveColoredA8" => MaterialKind::AlphaTint { additive: true },
        "Custom/CustomColourShader" => MaterialKind::Channel {
            main: col(m, "_MainColor", Color::WHITE),
            shadow: col(m, "_ShadowColor", Color::WHITE),
        },
        "Unlit/GradientColor" => MaterialKind::Gradient {
            top: col(m, "_Gradient", Color::WHITE),
            bottom: col(m, "_Gradient2", Color::WHITE),
            alpha_top: flt(m, "_AlphaTop", 1.0),
            alpha_bottom: flt(m, "_AlphaBot", 1.0),
            height: flt(m, "_GradientHeight", 0.0),
        },
        "Unlit/SlimeTrailedMovable" => MaterialKind::SlimeTrailed,
        "Custom/ReflectiveWaterWithDistortion" => MaterialKind::Water(WaterParams {
            alpha: flt(m, "_Alpha", 0.2),
            distortion: flt(m, "_DistortionStrength", 0.0),
            horizon_height: flt(m, "_WorldHorizonHeight", 1.0),
            surface_gradient: flt(m, "_SurfaceGradient", 1.0),
            extra_noise: flt(m, "_ExtraNoise", 1.0),
            gradient_height: flt(m, "_GradientHeight", 0.0),
            horizon_gradient_height: flt(m, "_HorizonGradientHeight", 0.0),
            reflection_height: flt(m, "_ReflH", 0.0),
            gradient_top: col(m, "_Gradient", Color::WHITE),
            gradient_bottom: col(m, "_Gradient2", Color::WHITE),
            water_color: col(m, "_WaterColor", Color::WHITE),
            wave: none,
            reflection: none,
            background: none,
        }),
        "Custom/WaterDistortion" => {
            let v3 = |k| {
                let c = col(m, k, Color::rgba(0.0, 0.0, 0.0, 0.0));
                Vec3::new(c.r, c.g, c.b)
            };
            let c = col(m, "_UVCenter", Color::rgba(0.5, 0.5, 0.0, 0.0));
            MaterialKind::Distortion {
                noise: tex(m, "_PerlinNoise"),
                params: DistortionParams {
                    noise: none,
                    strength: flt(m, "_DistStr", 0.0),
                    subdivide: flt(m, "_SubDivide", 1.0),
                    subdivide_noise: flt(m, "_SubDivideNoise", 1.0),
                    noise_scale: v3("_NoiseScale"),
                    strength3: v3("_DistStr3"),
                    uv_center: Vec4::new(c.r, c.g, c.b, c.a),
                },
            }
        }
        "Unlit/aurora" => MaterialKind::Aurora {
            noise: tex(m, "_Noise"),
            mask: tex(m, "_Mask"),
            params: AuroraParams {
                noise: none,
                mask: none,
                height: flt(m, "_Height", 1.0),
                blue: flt(m, "_Blue", 0.0),
                wobbliness: flt(m, "_Wobbliness", 0.0),
            },
        },
        "Custom/BlackHole2" => MaterialKind::BlackHole {
            distortion: flt(m, "_Distortion", -1.0),
            proximity: flt(m, "_ProximityMod", 3.3),
            scale: flt(m, "_Scale", 100.0),
            hole_brightness: flt(m, "_HoleBrightness", 0.0),
        },
        "Custom/GrabInvert" => MaterialKind::GrabInvert,
        "Custom/Distortion" => {
            let c = col(m, "_UVCenter", Color::rgba(0.5, 0.5, 1.0, 1.0));
            MaterialKind::Distort {
                noise: tex(m, "_PerlinNoise"),
                params: SpriteDistortParams {
                    noise: none,
                    speed: flt(m, "_SimulationSpeed", 1.0),
                    uv_center: Vec2::new(c.r, c.g),
                    affect_radius: flt(m, "_AffectRadius", 0.5),
                    cc: flt(m, "_CCStr", 1.0),
                    cw: flt(m, "_CWStr", 1.0),
                    strength: flt(m, "_DistStr", 0.1),
                },
            }
        }
        "Custom/SmearSprite" => {
            let o = col(m, "_Offset", Color::rgba(0.0, 0.0, 1.0, 1.0));
            let sc = col(m, "_Scale", Color::WHITE);
            MaterialKind::Smear {
                main: col(m, "_MainColor", Color::WHITE),
                shadow: col(m, "_ShadowColor", Color::WHITE),
                offset: Vec4::new(o.r, o.g, o.b, o.a),
                scale: Vec2::new(sc.r, sc.g),
            }
        }
        "Custom/DrawOnlyWhite" => MaterialKind::OnlyWhite,
        "Custom/ExpandingCircle" => {
            let v3 = |k, d: Color| {
                let c = col(m, k, d);
                Vec3::new(c.r, c.g, c.b)
            };
            MaterialKind::Expanding {
                noise: tex(m, "_PerlinNoise"),
                mask: tex(m, "_Mask"),
                progress: flt(m, "_Progress", 0.0),
                particle_speed: flt(m, "_ParticleSpeed", 1.0),
                dist: v3("_DistStr3", Color::rgba(0.08, 2.0, 1.0, 1.0)),
                radius: v3("_Radius", Color::rgba(8.0, 8.5, 5.0, 1.0)),
            }
        }
        "Custom/WhiteToColA8" => MaterialKind::WhiteToColour,
        "Custom/WallMaskedA8" => MaterialKind::WallMasked,
        "Custom/ColorSingleChannel" => {
            let c = col(m, "_ColorMask", Color::rgba(1.0, 0.0, 0.0, 1.0));
            MaterialKind::SingleChannel {
                mask: Vec3::new(c.r, c.g, c.b),
            }
        }
        "Unlit/ColorChannelsSeparetely" => MaterialKind::Channels3 {
            red: col(m, "_RedChannel", Color::WHITE),
            green: col(m, "_GreenChannel", Color::WHITE),
            blue: col(m, "_BlueChannel", Color::WHITE),
        },
        "Unlit/ResizablePlatform" => MaterialKind::ResizablePlatform,
        "Custom/LightningA8" => MaterialKind::Lightning(LightningParams {
            scale: flt(m, "_Scale", 1.0),
            uv_min: flt(m, "_UVYMin", 0.0),
            uv_max: flt(m, "_UVYMax", 1.0),
            blur: flt(m, "_BlurSize", 15.0),
            directions: flt(m, "_Directions", 16.0),
            quality: flt(m, "_Quality", 3.0),
        }),
        "Custom/Beam" | "Custom/BeamGlow" | "Custom/BeamWhite" | "Custom/AdditiveSoftBorders" => {
            MaterialKind::Beam {
                noise: tex(m, "_NoiseTex"),
                params: BeamParams {
                    kind: match m.shader.as_str() {
                        "Custom/Beam" => BeamKind::Core,
                        "Custom/BeamGlow" => BeamKind::Glow,
                        "Custom/BeamWhite" => BeamKind::White,
                        _ => BeamKind::SoftBorders,
                    },
                    noise: none,
                    platform: none,
                    line_width: flt(m, "_LineWidth", 0.2),
                    noise_freq: flt(m, "_NoiseFreq", 1.0),
                    scale: flt(m, "_Scale", 40.0),
                    aura: flt(m, "_Aura", 1.0),
                    time_scale: flt(m, "_TimeScale", 1.0),
                    dist_str: flt(m, "_DistStr", 0.2),
                    dist_str2: flt(m, "_DistStr2", 0.2),
                    uv_ratio: flt(m, "_UVScaleRatio", 1.0),
                    uv_ratio2: flt(m, "_UVScaleRatio2", 1.0),
                },
            }
        }
        "Custom/CloneEffect" => {
            let d = col(m, "_Direction", Color::rgba(0.0, 1.0, 0.0, 0.0));
            MaterialKind::Clone {
                noise: tex(m, "_Noise"),
                progress: flt(m, "_Progress01", 0.0),
                noise_str: flt(m, "_NoiseStr", 1.0),
                noise_freq: flt(m, "_NoiseFreq", 1.0),
                direction: Vec2::new(d.r, d.g),
                uv_offset: flt(m, "_UVOffset", 0.5),
            }
        }
        "Legacy Shaders/Particles/Alpha Blended Premultiply" => MaterialKind::ParticlePremultiply,
        "Legacy Shaders/Particles/Alpha Blended" => MaterialKind::ParticleBlended {
            tint: col(m, "_TintColor", Color::rgba(0.5, 0.5, 0.5, 0.5)),
        },
        "Legacy Shaders/Particles/Additive (Soft)" => MaterialKind::SoftAdditive,
        // Sprites/Default, Custom/FlippableSprite (with its default flip of
        // (1, 1) it is a plain sprite), Particles/Standard Unlit and
        // anything unknown.
        _ => MaterialKind::Sprite {
            tint: col(m, "_Color", Color::WHITE),
        },
    }
}

/// The shading of a material id (cached in `cache`).
fn material_of(
    cache: &mut HashMap<String, MaterialKind>,
    assets: &Assets,
    id: Option<&str>,
) -> MaterialKind {
    match id {
        Some(m) => cache
            .entry(m.to_string())
            .or_insert_with(|| material_kind(assets.data.materials.get(m)))
            .clone(),
        None => MaterialKind::Sprite { tint: Color::WHITE },
    }
}

/// Where a camera renders.
#[derive(Clone, Debug, PartialEq)]
enum CamTarget {
    Output,
    /// A global render texture (`ParticleCamera.globalTextureName`,
    /// `_ReflectionRT`).
    Global {
        name: String,
        wrap: Wrap,
    },
}

#[derive(Clone, Debug)]
struct Follow {
    x: bool,
    y: bool,
    z: bool,
    neg: [bool; 3],
    mimic_size: bool,
    x_mul: f32,
    offset: Vec3,
}

#[derive(Clone, Debug)]
struct CameraDef {
    object: usize,
    name: String,
    depth: f32,
    culling: u32,
    clear: Option<Color>,
    projection: Projection,
    near: f32,
    far: f32,
    position: Vec3,
    rotation: Quat,
    aspect: Option<f32>,
    follow: Option<Follow>,
    target: CamTarget,
}

/// Per-frame presentation script of an object.
#[derive(Clone, Debug)]
enum Behaviour {
    FullScreenQuad {
        w: f32,
        h: f32,
        follow: bool,
    },
    Parallax {
        speed: f32,
    },
    KeepInCorner,
    ScaleWidth {
        min: f32,
        max: f32,
        speed: f32,
        base: f32,
    },
    AlphaFade {
        curve: Vec<[f32; 4]>,
        destroy: bool,
    },
}

#[derive(Clone, Debug)]
struct MaskDef {
    object: usize,
    sprite: String,
    cutoff: f32,
    /// Custom range (back, front), inclusive; `None` affects everything.
    range: Option<(SortKey, SortKey)>,
}

/// One sprite to draw in the current frame.
#[derive(Clone, Debug)]
struct Draw {
    layer: u32,
    key: SortKey,
    /// World z (larger = farther from the cameras, which look along +z).
    z: f32,
    seq: usize,
    quad: Quad,
    mask_interaction: i32,
    /// Samples the screen behind it (`GrabPass`).
    grab: bool,
}

/// A platform's slime-trail camera (`PlatformSlimeTrailCamera`): renders
/// the SlimeTrail layer into a never-cleared texture the size of the
/// platform's sprite texture, which the platform's material paints over the
/// sprite (levels.md 7).
#[derive(Clone, Debug)]
struct TrailCam {
    /// The platform object (the camera's parent with the sprite renderer).
    platform: usize,
    /// Camera position relative to the platform.
    local: Vec3,
    /// Sprite bounds (world units at scale 1).
    bounds: Vec2,
    /// Render texture size (the sprite's texture).
    texture: (u32, u32),
    culling: u32,
}

/// The SlimeTrail layer.
pub const SLIME_TRAIL_LAYER: u32 = 15;

/// The presentation of one level.
pub struct LevelView {
    pub level: Level,
    local: Vec<Mat4>,
    cameras: Vec<CameraDef>,
    main: usize,
    behaviours: Vec<(usize, Behaviour)>,
    masks: Vec<MaskDef>,
    materials: HashMap<String, MaterialKind>,
    /// Main camera position (authored).
    main_position: Vec3,
    main_size: f32,
    trail_cams: Vec<TrailCam>,
}

fn b(f: &J, k: &str) -> bool {
    f[k].as_i64().is_some_and(|v| v != 0) || f[k].as_bool() == Some(true)
}

fn f32_of(f: &J, k: &str, default: f32) -> f32 {
    f[k].as_f64().map_or(default, |v| v as f32)
}

fn v3_of(f: &J) -> Vec3 {
    Vec3::new(
        f32_of(f, "x", 0.0),
        f32_of(f, "y", 0.0),
        f32_of(f, "z", 0.0),
    )
}

fn color_of(f: &J) -> Color {
    Color::rgba(
        f32_of(f, "r", 0.0),
        f32_of(f, "g", 0.0),
        f32_of(f, "b", 0.0),
        f32_of(f, "a", 0.0),
    )
}

/// Local TRS of an object (rotation normalised). Never decompose the
/// composed matrices instead: Unity sprites often have a z scale of 0.
fn local_trs(o: &Object) -> (Vec3, Quat, Vec3) {
    let t = &o.local;
    let [x, y, z, w] = t.rotation;
    let q = Quat::from_xyzw(x, y, z, w);
    let q = if q.length_squared() > 1e-12 {
        q.normalize()
    } else {
        Quat::IDENTITY
    };
    (Vec3::from(t.scale), q, Vec3::from(t.position))
}

fn local_matrix(o: &Object) -> Mat4 {
    let (s, r, t) = local_trs(o);
    Mat4::from_scale_rotation_translation(s, r, t)
}

/// Unity `AnimationCurve.Evaluate` (cubic Hermite, clamped ends) for keys
/// `[time, value, in_slope, out_slope]`.
pub fn eval_curve(keys: &[[f32; 4]], t: f32) -> f32 {
    let Some(first) = keys.first() else {
        return 0.0;
    };
    if t <= first[0] {
        return first[1];
    }
    let last = keys[keys.len() - 1];
    if t >= last[0] {
        return last[1];
    }
    let i = keys.partition_point(|k| k[0] <= t);
    let (k0, k1) = (keys[i - 1], keys[i]);
    let dt = k1[0] - k0[0];
    if dt <= 0.0 {
        return k1[1];
    }
    let s = (t - k0[0]) / dt;
    let (s2, s3) = (s * s, s * s * s);
    let (m0, m1) = (k0[3] * dt, k1[2] * dt);
    if !m0.is_finite() || !m1.is_finite() {
        return k0[1]; // stepped tangents
    }
    (2.0 * s3 - 3.0 * s2 + 1.0) * k0[1]
        + (s3 - 2.0 * s2 + s) * m0
        + (-2.0 * s3 + 3.0 * s2) * k1[1]
        + (s3 - s2) * m1
}

fn curve_keys(f: &J) -> Vec<[f32; 4]> {
    f["m_Curve"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|k| {
                    [
                        f32_of(k, "time", 0.0),
                        f32_of(k, "value", 0.0),
                        k["inSlope"].as_f64().map_or(f32::INFINITY, |v| v as f32),
                        k["outSlope"].as_f64().map_or(f32::INFINITY, |v| v as f32),
                    ]
                })
                .collect()
        })
        .unwrap_or_default()
}

impl LevelView {
    pub fn new(data: &GameData, level: Level) -> Self {
        let local: Vec<Mat4> = level.objects.iter().map(local_matrix).collect();
        let sorting_index = |id: i64| -> usize {
            let id = id as u32;
            data.index
                .sorting_layers
                .iter()
                .position(|l| l.id == id)
                .unwrap_or(0)
        };
        let mut cameras = Vec::new();
        let mut behaviours = Vec::new();
        let mut masks = Vec::new();
        let mut main = None;
        let mut trail_cams = Vec::new();
        for (i, o) in level.objects.iter().enumerate() {
            if !o.active_in_hierarchy {
                continue;
            }
            if o.component("PlatformSlimeTrailCamera").is_some()
                && let Some(parent) = o.parent
                && let Some(r) = &level.objects[parent].sprite
                && let Some(sp) = r.sprite.as_deref().and_then(|id| data.sprites.get(id))
                && let Some(tex) = sp.texture.as_deref().and_then(|t| data.textures.get(t))
            {
                let ppu = sp.pixels_per_unit.max(1e-6);
                let culling = o
                    .component("Camera")
                    .and_then(|c| c.fields["m_CullingMask"]["m_Bits"].as_u64())
                    .unwrap_or(1 << SLIME_TRAIL_LAYER) as u32;
                trail_cams.push(TrailCam {
                    platform: parent,
                    local: Vec3::from(o.local.position),
                    bounds: Vec2::new(sp.rect[2], sp.rect[3]) / ppu,
                    texture: (tex.width.max(1), tex.height.max(1)),
                    culling,
                });
            }
            if let Some(c) = o.component("Camera")
                && c.enabled != Some(false)
                && o.component("PlatformSlimeTrailCamera").is_none()
            {
                let f = &c.fields;
                let follow = o.component("FollowMainCamera").map(|fc| {
                    let g = &fc.fields;
                    Follow {
                        x: b(g, "followX"),
                        y: b(g, "followY"),
                        z: b(g, "followZ"),
                        neg: [b(g, "negateX"), b(g, "negateY"), b(g, "negateZ")],
                        mimic_size: b(g, "mimicCameraSize"),
                        x_mul: f32_of(g, "xMultiplier", 1.0),
                        offset: v3_of(&g["offset"]),
                    }
                });
                let particle = o.component("ParticleCamera");
                let target = if Some(i) == level.main_camera {
                    CamTarget::Output
                } else if o.component("reflectiveCamera").is_some() {
                    CamTarget::Global {
                        name: "_ReflectionRT".into(),
                        // A new RenderTexture repeats by default.
                        wrap: Wrap::Repeat,
                    }
                } else if let Some(p) = particle {
                    CamTarget::Global {
                        name: p.fields["globalTextureName"]
                            .as_str()
                            .unwrap_or("_FireParticleRT")
                            .to_string(),
                        wrap: Wrap::Mirror,
                    }
                } else {
                    // A camera without a render-texture script would draw
                    // onto the screen under the main camera; none exists in
                    // the shipped levels.
                    log::debug!("{}: camera {} not drawn", level.name, o.name);
                    continue;
                };
                let force_aspect = particle
                    .is_some_and(|p| b(&p.fields, "force18by9"))
                    .then_some(2.0);
                let flags = f["m_ClearFlags"].as_i64().unwrap_or(2);
                let projection = if b(f, "orthographic") {
                    Projection::Orthographic {
                        size: f32_of(f, "orthographic size", 5.0),
                    }
                } else {
                    Projection::Perspective {
                        fov_y: f32_of(f, "field of view", 60.0),
                    }
                };
                let [x, y, z, w] = o.world.trs.rotation;
                if Some(i) == level.main_camera {
                    main = Some(cameras.len());
                }
                cameras.push(CameraDef {
                    object: i,
                    name: o.name.clone(),
                    depth: f32_of(f, "m_Depth", 0.0),
                    culling: f["m_CullingMask"]["m_Bits"].as_u64().unwrap_or(u64::MAX) as u32,
                    // 1 skybox (none in the game) and 2 solid colour clear;
                    // 3 depth only and 4 nothing keep the colour.
                    clear: matches!(flags, 1 | 2).then(|| color_of(&f["m_BackGroundColor"])),
                    projection,
                    near: f32_of(f, "near clip plane", 0.3),
                    far: f32_of(f, "far clip plane", 1000.0),
                    position: Vec3::from(o.world.trs.position),
                    rotation: Quat::from_xyzw(x, y, z, w).normalize(),
                    aspect: force_aspect,
                    follow,
                    target,
                });
            }
            for c in &o.components {
                let f = &c.fields;
                let beh = match c.script.as_deref() {
                    Some("FullScreenQuad") => Some(Behaviour::FullScreenQuad {
                        w: f32_of(f, "spriteWidth", 2.0),
                        h: f32_of(f, "spriteHeight", 2.0),
                        follow: b(f, "followMainCamera"),
                    }),
                    Some("ParallaxLayer") => Some(Behaviour::Parallax {
                        speed: f32_of(f, "parallaxSpeed", 0.5),
                    }),
                    Some("KeepInCameraCorner") => Some(Behaviour::KeepInCorner),
                    Some("ScaleBasedOnCameraWidth") => Some(Behaviour::ScaleWidth {
                        min: f32_of(f, "min", 1.3),
                        max: f32_of(f, "max", 2.0),
                        speed: f32_of(f, "scaleSpeed", 0.7),
                        base: f32_of(f, "baseSize", 1.3),
                    }),
                    Some("AlphaFade") if c.enabled != Some(false) => Some(Behaviour::AlphaFade {
                        curve: curve_keys(&f["fadeAnimation"]),
                        destroy: b(f, "destroyWhenDone"),
                    }),
                    _ => None,
                };
                if let Some(beh) = beh {
                    behaviours.push((i, beh));
                }
                if c.kind == "SpriteMask"
                    && c.enabled != Some(false)
                    && let Some(sprite) = f["m_Sprite"]["$ref"].as_str()
                {
                    let range = b(f, "m_IsCustomRangeActive").then(|| {
                        (
                            (
                                sorting_index(f["m_BackSortingLayerID"].as_i64().unwrap_or(0)),
                                f["m_BackSortingOrder"].as_i64().unwrap_or(0) as i32,
                            ),
                            (
                                sorting_index(f["m_FrontSortingLayerID"].as_i64().unwrap_or(0)),
                                f["m_FrontSortingOrder"].as_i64().unwrap_or(0) as i32,
                            ),
                        )
                    });
                    masks.push(MaskDef {
                        object: i,
                        sprite: sprite.to_string(),
                        cutoff: f32_of(f, "m_MaskAlphaCutoff", 0.2),
                        range,
                    });
                }
            }
        }
        // Unity renders cameras by ascending depth (stable for equal depth).
        let main_object = main.map(|m| cameras[m].object);
        cameras.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        let main = main_object
            .and_then(|o| cameras.iter().position(|c| c.object == o))
            .unwrap_or_else(|| {
                // No main camera (should not happen): a default one.
                cameras.push(CameraDef {
                    object: usize::MAX,
                    name: "Main Camera".into(),
                    depth: 1.0,
                    culling: u32::MAX,
                    clear: Some(Color::BLACK),
                    projection: Projection::Orthographic { size: 30.0 },
                    near: 0.3,
                    far: 1000.0,
                    position: Vec3::new(0.0, 8.0, -100.0),
                    rotation: Quat::IDENTITY,
                    aspect: None,
                    follow: None,
                    target: CamTarget::Output,
                });
                cameras.len() - 1
            });
        let main_position = cameras[main].position;
        let main_size = match cameras[main].projection {
            Projection::Orthographic { size } => size,
            _ => 30.0,
        };
        let mut materials = HashMap::new();
        for o in &level.objects {
            if let Some(r) = &o.sprite {
                for id in r.material_ids.iter().take(1) {
                    materials
                        .entry(id.clone())
                        .or_insert_with(|| material_kind(data.materials.get(id)));
                }
            }
        }
        Self {
            level,
            local,
            cameras,
            main,
            behaviours,
            masks,
            materials,
            main_position,
            main_size,
            trail_cams,
        }
    }

    /// The main camera for a target of `size` with the view's camera x
    /// (mouse aim, HUD placement).
    pub fn main_camera(&self, view: &WorldView) -> Camera {
        let c = &self.cameras[self.main];
        let (position, size) = self.main_pose(view);
        Camera {
            position,
            rotation: c.rotation,
            projection: Projection::Orthographic { size },
            near: c.near,
            far: c.far,
            aspect: None,
        }
    }

    /// Main camera position and size for `view` (camera x, zoom, shake).
    fn main_pose(&self, view: &WorldView) -> (Vec3, f32) {
        let mut position = self.main_position;
        if let Some(x) = view.camera_x {
            position.x = x;
        }
        let mut size = self.main_size;
        if let Some(s) = view.camera_size {
            // Zoomed out: the bottom edge stays.
            position.y += s - size;
            size = s;
        }
        position += view.shake.extend(0.0);
        (position, size)
    }

    /// Main camera half height (orthographic size).
    #[allow(dead_code)]
    pub fn main_size(&self) -> f32 {
        self.main_size
    }

    /// Platform objects (by level platform index) that have a slime-trail
    /// camera.
    #[allow(dead_code)]
    pub fn has_trails(&self) -> bool {
        !self.trail_cams.is_empty()
    }

    /// Names of the cameras in render order, with their targets (debug).
    pub fn describe_cameras(&self) -> Vec<String> {
        self.cameras
            .iter()
            .map(|c| {
                format!(
                    "{} (depth {}, mask {:#x}) -> {}",
                    c.name,
                    c.depth,
                    c.culling,
                    match &c.target {
                        CamTarget::Output => "screen".to_string(),
                        CamTarget::Global { name, .. } => name.clone(),
                    }
                )
            })
            .collect()
    }

    /// Builds the render frame for `view` on a target of `size` pixels.
    /// Returns the frame and the render targets it uses (by global name).
    pub fn frame(
        &mut self,
        view: &WorldView,
        size: (u32, u32),
        gpu: &Gpu,
        renderer: &mut Renderer,
        assets: &mut Assets,
    ) -> (Frame, Vec<(String, TextureId)>) {
        let aspect = size.0.max(1) as f32 / size.1.max(1) as f32;
        let (main_pos, main_size) = self.main_pose(view);

        // World matrices with the presentation scripts applied.
        let objects = &self.level.objects;
        let n = objects.len();
        let mut local = self.local.clone();
        let mut world_pos: Vec<Option<Vec3>> = vec![None; n];
        let mut world_x: Vec<Option<f32>> = vec![None; n];
        let mut alpha: Vec<Option<f32>> = vec![None; n];
        let mut hidden = vec![false; n];
        for (i, beh) in &self.behaviours {
            let i = *i;
            match beh {
                Behaviour::FullScreenQuad { w, h, follow } => {
                    let full_h = main_size * 2.0;
                    let full_w = full_h * aspect;
                    let (_, r, _) = local_trs(&objects[i]);
                    let t = if *follow {
                        Vec3::new(main_pos.x, main_pos.y, 0.0)
                    } else {
                        Vec3::ZERO
                    };
                    local[i] = Mat4::from_scale_rotation_translation(
                        Vec3::new(full_w / w, full_h / h, 1.0),
                        r,
                        Vec3::ZERO,
                    );
                    world_pos[i] = Some(t);
                }
                Behaviour::Parallax { speed } => {
                    world_pos[i] = Some(main_pos * (1.0 - speed.max(0.05)));
                }
                Behaviour::KeepInCorner => {
                    world_x[i] = Some(main_size * aspect + main_pos.x);
                }
                Behaviour::ScaleWidth {
                    min,
                    max,
                    speed,
                    base,
                } => {
                    let t = (aspect * speed).clamp(0.0, 1.0);
                    let sx = base + min + (max - min) * t;
                    let (mut s, r, t) = local_trs(&objects[i]);
                    s.x = sx;
                    local[i] = Mat4::from_scale_rotation_translation(s, r, t);
                }
                Behaviour::AlphaFade { curve, destroy } => {
                    let end = curve.last().map_or(0.0, |k| k[0]);
                    if *destroy && view.time >= end {
                        hidden[i] = true;
                    } else {
                        alpha[i] = Some(eval_curve(curve, view.time));
                    }
                }
            }
        }
        // Platform poses from the simulation.
        let mut pose: Vec<Option<(Vec2, f32, f32)>> = vec![None; n];
        for p in &view.platforms {
            if let Some(&i) = self.level.platforms.get(p.platform) {
                if p.visible {
                    pose[i] = Some((p.position, p.rotation, p.scale));
                } else {
                    hidden[i] = true;
                }
            }
        }
        let mut world = vec![Mat4::IDENTITY; n];
        for i in 0..n {
            let o = &objects[i];
            let parent = o.parent.map_or(Mat4::IDENTITY, |p| world[p]);
            if let Some(p) = o.parent
                && hidden[p]
            {
                hidden[i] = true;
            }
            let mut m = parent * local[i];
            if let Some((pos, rot, k)) = pose[i] {
                let s = Vec3::from(o.world.trs.scale);
                m = Mat4::from_scale_rotation_translation(
                    s * Vec3::new(k, k, 1.0),
                    Quat::from_rotation_z(rot),
                    Vec3::new(pos.x, pos.y, m.w_axis.z),
                );
            }
            if let Some(p) = world_pos[i] {
                m.w_axis = p.extend(1.0);
            }
            if let Some(x) = world_x[i] {
                m.w_axis.x = x;
            }
            world[i] = m;
        }

        // Slime-trail render textures (one per platform with a trail camera).
        let mut trail_rt: HashMap<usize, TextureId> = HashMap::new();
        for (k, tc) in self.trail_cams.iter().enumerate() {
            let id = renderer.render_target(gpu, &format!("_SlimeRT{k}"), tc.texture, Wrap::Clamp);
            trail_rt.insert(tc.platform, id);
        }

        // Sprites.
        let mut draws: Vec<Draw> = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            if !o.active_in_hierarchy || hidden[i] {
                continue;
            }
            let Some(r) = &o.sprite else { continue };
            let Some(sprite) = r.sprite.as_deref() else {
                continue;
            };
            if !r.enabled || o.component("SpriteMask").is_some() {
                continue;
            }
            let mut color = Color::from_array(r.color);
            if let Some(a) = alpha[i] {
                color.a = a;
            }
            let kind = r
                .material_ids
                .first()
                .and_then(|m| self.materials.get(m))
                .cloned()
                .unwrap_or(MaterialKind::Sprite { tint: Color::WHITE });
            let draw_size = (r.draw_mode != 0).then(|| Vec2::new(r.size[0], r.size[1]));
            if let Some(mut quad) = sprite_quad(
                gpu,
                renderer,
                assets,
                sprite,
                world[i],
                (r.flip_x, r.flip_y),
                draw_size,
                color,
                &kind,
                &Extra::default(),
            ) {
                if let (Shading::SlimeTrailed { trail, .. }, Some(rt)) =
                    (&mut quad.shading, trail_rt.get(&i))
                {
                    *trail = *rt;
                }
                draws.push(Draw {
                    layer: o.layer,
                    key: (r.sorting_layer_index, r.sorting_order),
                    z: world[i].w_axis.z,
                    seq: draws.len(),
                    quad,
                    mask_interaction: r.mask_interaction,
                    grab: kind.grabs(),
                });
            }
        }
        for s in &view.sprites {
            let kind = material_of(&mut self.materials, assets, s.material.as_deref());
            let m = Mat4::from_scale_rotation_translation(
                s.scale.extend(1.0),
                Quat::from_rotation_z(s.rotation),
                s.position,
            );
            let extra = Extra {
                params: s.params,
                colors: s.colors,
            };
            if let Some(quad) = sprite_quad(
                gpu,
                renderer,
                assets,
                &s.sprite,
                m,
                (s.flip_x, s.flip_y),
                s.size,
                s.color,
                &kind,
                &extra,
            ) {
                draws.push(Draw {
                    layer: s.layer,
                    key: (s.sorting_layer, s.sorting_order),
                    z: s.position.z,
                    seq: draws.len(),
                    quad,
                    mask_interaction: s.mask_interaction,
                    grab: kind.grabs(),
                });
            }
        }
        for q in &view.quads {
            let mut quad = q.quad;
            let mut grab = false;
            if let Some(m) = q.material.as_deref() {
                let kind = material_of(&mut self.materials, assets, Some(m));
                let tex_size = renderer.texture_size(quad.texture).unwrap_or((1, 1));
                let extra = Extra {
                    params: q.params,
                    colors: None,
                };
                if !apply_material(
                    gpu,
                    renderer,
                    assets,
                    &mut quad,
                    &kind,
                    &extra,
                    (false, false),
                    tex_size,
                ) {
                    continue;
                }
                grab = kind.grabs();
            }
            draws.push(Draw {
                layer: q.layer,
                key: (q.sorting_layer, q.sorting_order),
                z: q.z,
                seq: draws.len(),
                quad,
                mask_interaction: q.mask_interaction,
                grab,
            });
        }

        // Render targets (created before any pass references them).
        let mut targets: Vec<(String, TextureId)> = Vec::new();
        for c in &self.cameras {
            if let CamTarget::Global { name, wrap } = &c.target
                && !targets.iter().any(|(n, _)| n == name)
            {
                targets.push((name.clone(), renderer.render_target(gpu, name, size, *wrap)));
            }
        }
        let rt = |name: &str| {
            targets
                .iter()
                .find(|(n, _)| n == name)
                .map_or(TextureId::TRANSPARENT, |(_, id)| *id)
        };
        let platform_rt = rt("_platformRT");
        for d in &mut draws {
            match &mut d.quad.shading {
                Shading::ReflectiveWater(w) => {
                    w.wave = rt("_WaveParticleRT");
                    w.reflection = rt("_ReflectionRT");
                    w.background = rt("_BackgroundRT");
                }
                Shading::WallMasked { platform } => *platform = platform_rt,
                Shading::Beam(b) => b.platform = platform_rt,
                _ => {}
            }
        }

        // Mask quads.
        let mask_quads: Vec<(usize, Quad, MaskRange)> = self
            .masks
            .iter()
            .filter(|m| objects[m.object].active_in_hierarchy && !hidden[m.object])
            .filter_map(|m| {
                let g = assets.sprite(gpu, renderer, &m.sprite)?;
                Some((
                    m.object,
                    Quad {
                        texture: g.texture,
                        uv: g.uv,
                        transform: world[m.object],
                        size: g.size,
                        pivot: g.pivot,
                        shading: Shading::Mask { cutoff: m.cutoff },
                        stencil: Stencil::MaskAdd,
                        ..Default::default()
                    },
                    m.range,
                ))
            })
            .collect();

        let mut frame = Frame::new();
        frame.time = view.time;
        frame.timestop = if view.time_stopped { 0.0 } else { 1.0 };

        // Slime-trail cameras: children of their platform, never cleared
        // (except on the first frame of a level).
        for tc in &self.trail_cams {
            if hidden[tc.platform] || !objects[tc.platform].active_in_hierarchy {
                continue;
            }
            let Some(&rt) = trail_rt.get(&tc.platform) else {
                continue;
            };
            let m = world[tc.platform];
            let scale = m.x_axis.truncate().length().max(1e-6);
            let angle = m.x_axis.y.atan2(m.x_axis.x);
            let pos = m.transform_point3(tc.local);
            let camera = Camera {
                position: Vec3::new(pos.x, pos.y, pos.z.min(-10.0)),
                rotation: Quat::from_rotation_z(angle),
                projection: Projection::Orthographic {
                    size: tc.bounds.y * 0.5 * scale,
                },
                near: 0.3,
                far: 1000.0,
                aspect: Some(tc.bounds.x / tc.bounds.y.max(1e-6)),
            };
            let mut pass = Pass::new(
                "SlimeTrailCam",
                PassTarget::Texture(rt),
                View::Camera(camera),
            );
            pass.clear = view.clear_trails.then_some(Color::rgba(0.0, 0.0, 0.0, 0.0));
            let mut list: Vec<&Draw> = draws
                .iter()
                .filter(|d| tc.culling >> (d.layer & 31) & 1 == 1)
                .collect();
            list.sort_by(|a, b| a.key.cmp(&b.key).then(a.seq.cmp(&b.seq)));
            for d in list {
                pass.push(d.quad);
            }
            frame.passes.push(pass);
        }

        // One pass per camera.
        let main_def = self.cameras[self.main].clone();
        let mut grabs = 0usize;
        for (ci, c) in self.cameras.iter().enumerate() {
            let mut cam_pos = if ci == self.main {
                main_pos
            } else {
                c.position
            };
            let mut proj = c.projection;
            if let Some(f) = &c.follow {
                let m = main_pos;
                if f.x {
                    cam_pos.x = if f.neg[0] { -m.x } else { m.x } * f.x_mul + f.offset.x;
                }
                if f.y {
                    cam_pos.y = if f.neg[1] { -m.y } else { m.y } + f.offset.y;
                }
                if f.z {
                    cam_pos.z = if f.neg[2] { -m.z } else { m.z };
                    // The original adds the y offset again here.
                    cam_pos.y += f.offset.y;
                }
                if f.mimic_size
                    && let Projection::Orthographic { .. } = proj
                {
                    proj = Projection::Orthographic { size: main_size };
                }
            }
            let camera = Camera {
                position: cam_pos,
                rotation: if ci == self.main {
                    main_def.rotation
                } else {
                    c.rotation
                },
                projection: proj,
                near: c.near,
                far: c.far,
                aspect: c.aspect,
            };
            let target = match &c.target {
                CamTarget::Output => PassTarget::Output,
                CamTarget::Global { name, .. } => PassTarget::Texture(rt(name)),
            };
            let mut pass = Pass::new(c.name.clone(), target, View::Camera(camera));
            pass.clear = c.clear;
            // Visible sprites, Unity order: sorting layer, order in layer,
            // then back to front along the view axis, then submission.
            let forward = camera.rotation * Vec3::Z;
            let mut list: Vec<&Draw> = draws
                .iter()
                .filter(|d| c.culling >> (d.layer & 31) & 1 == 1)
                .collect();
            list.sort_by(|a, b| {
                let da = (Vec3::new(0.0, 0.0, a.z) - cam_pos).dot(forward);
                let db = (Vec3::new(0.0, 0.0, b.z) - cam_pos).dot(forward);
                a.key
                    .cmp(&b.key)
                    .then(db.total_cmp(&da))
                    .then(a.seq.cmp(&b.seq))
            });
            // Sprite masks seen by this camera: active from the first
            // sprite in their range to the last.
            let mut add_at: Vec<(usize, Quad)> = Vec::new();
            let mut remove_at: Vec<(usize, Quad)> = Vec::new();
            for (obj, q, range) in &mask_quads {
                if c.culling >> (objects[*obj].layer & 31) & 1 == 0 {
                    continue;
                }
                let (start, end) = match range {
                    Some((back, front)) => {
                        let s = list.iter().position(|d| d.key >= *back);
                        let e = list.iter().rposition(|d| d.key <= *front);
                        match (s, e) {
                            (Some(s), Some(e)) if s <= e => (s, e + 1),
                            _ => continue,
                        }
                    }
                    None => (0, list.len()),
                };
                add_at.push((start, *q));
                remove_at.push((
                    end,
                    Quad {
                        stencil: Stencil::MaskRemove,
                        ..*q
                    },
                ));
            }
            // Grab-pass items (black hole, time-stop sphere) read what was
            // drawn before them: the pass so far renders into a grab
            // texture, which the rest of the pass starts from (main camera
            // only).
            let grab_ok = matches!(c.target, CamTarget::Output);
            let mut active: Vec<(usize, Quad)> = Vec::new();
            for k in 0..=list.len() {
                for (i, (_, q)) in remove_at.iter().enumerate().filter(|(_, (at, _))| *at == k) {
                    pass.push(*q);
                    active.retain(|(j, _)| *j != i);
                }
                for (i, (_, q)) in add_at.iter().enumerate().filter(|(_, (at, _))| *at == k) {
                    pass.push(*q);
                    active.push((i, *q));
                }
                if let Some(d) = list.get(k) {
                    let mut q = d.quad;
                    q.stencil = match d.mask_interaction {
                        1 => Stencil::VisibleInside,
                        2 => Stencil::VisibleOutside,
                        _ => Stencil::None,
                    };
                    if d.grab {
                        if !grab_ok {
                            continue;
                        }
                        let tex = renderer.render_target(
                            gpu,
                            &format!("_GrabRT{grabs}"),
                            size,
                            Wrap::Clamp,
                        );
                        grabs += 1;
                        let mut next = Pass::new(c.name.clone(), pass.target, pass.view);
                        next.clear = Some(Color::rgba(0.0, 0.0, 0.0, 0.0));
                        let mut done = std::mem::replace(&mut pass, next);
                        done.target = PassTarget::Texture(tex);
                        frame.passes.push(done);
                        let (half_h, half_w) = match camera.projection {
                            Projection::Orthographic { size: s } => (s, s * aspect),
                            _ => (main_size, main_size * aspect),
                        };
                        pass.push(Quad {
                            texture: tex,
                            transform: Mat4::from_translation(Vec3::new(cam_pos.x, cam_pos.y, 0.0)),
                            size: Vec2::new(half_w * 2.0, half_h * 2.0),
                            shading: Shading::Copy,
                            ..Default::default()
                        });
                        for (_, m) in &active {
                            pass.push(*m);
                        }
                        match &mut q.shading {
                            Shading::BlackHole { grab, .. } | Shading::GrabInvert { grab } => {
                                *grab = tex
                            }
                            _ => {}
                        }
                    }
                    pass.push(q);
                }
            }
            frame.passes.push(pass);
        }
        (frame, targets)
    }
}

/// Per-renderer inputs the original's scripts set on its material
/// ([`crate::present::SpriteDraw::params`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct Extra {
    pub params: [f32; 4],
    pub colors: Option<[Color; 2]>,
}

/// The quad of a sprite renderer, or `None` when it draws nothing.
#[allow(clippy::too_many_arguments)]
fn sprite_quad(
    gpu: &Gpu,
    renderer: &mut Renderer,
    assets: &mut Assets,
    sprite: &str,
    world: Mat4,
    flip: (bool, bool),
    draw_size: Option<Vec2>,
    color: Color,
    kind: &MaterialKind,
    extra: &Extra,
) -> Option<Quad> {
    let g = assets.sprite(gpu, renderer, sprite)?;
    let mut transform = world;
    if flip.0 || flip.1 {
        transform *= Mat4::from_scale(Vec3::new(
            if flip.0 { -1.0 } else { 1.0 },
            if flip.1 { -1.0 } else { 1.0 },
            1.0,
        ));
    }
    let mut size = g.size;
    if let Some(ds) = draw_size
        && g.rect_size.x > 0.0
        && g.rect_size.y > 0.0
    {
        size *= ds / g.rect_size;
    }
    let mut quad = Quad {
        texture: g.texture,
        uv: g.uv,
        transform,
        size,
        pivot: g.pivot,
        color,
        ..Default::default()
    };
    apply_material(
        gpu,
        renderer,
        assets,
        &mut quad,
        kind,
        extra,
        flip,
        g.texture_size,
    )
    .then_some(quad)
}

/// Applies a material's shading to a quad (texture, uv, geometry and vertex
/// colour already set); `false` when it draws nothing.
///
/// Per-renderer values (`extra.params`), by shader: BlackHole2 `_Scale`,
/// `_Distortion`, `_HoleBrightness` (0 = the material's); SmearSprite
/// `_Scale.y` in `[1]`; LightningA8 `_Scale` in `[0]` (uv rows from the
/// sprite); ResizablePlatform inner half extents, corner radius (world)
/// and the quad's world scale; beams `_Scale` in `[0]`; ExpandingCircle
/// `_Progress` in `[0]` when `[3]` is 1; CloneEffect `_Progress01` in `[0]`
/// when `[3]` is 1, plus `_Direction` (angle) in `[1]` and `_UVOffset` in
/// `[2]` when `[3]` is 2.
#[allow(clippy::too_many_arguments)]
fn apply_material(
    gpu: &Gpu,
    renderer: &mut Renderer,
    assets: &mut Assets,
    quad: &mut Quad,
    kind: &MaterialKind,
    extra: &Extra,
    flip: (bool, bool),
    texture_size: (u32, u32),
) -> bool {
    let color = quad.color;
    let invisible = color.a <= 0.0;
    let p = extra.params;
    let or = |v: f32, d: f32| if v != 0.0 { v } else { d };
    let mut tex = |n: &Option<String>| {
        n.as_deref()
            .and_then(|n| assets.texture(gpu, renderer, n))
            .map_or(TextureId::WHITE, |t| t.0)
    };
    match kind {
        MaterialKind::Sprite { tint } => {
            if invisible {
                return false;
            }
            quad.color = color * *tint;
        }
        MaterialKind::AlphaTint { additive } => {
            if invisible {
                return false;
            }
            quad.shading = Shading::AlphaTint;
            if *additive {
                quad.blend = Blend::Additive;
            }
        }
        MaterialKind::Channel { main, shadow } => {
            quad.shading = Shading::ChannelColour {
                main: *main,
                shadow: *shadow,
            };
        }
        MaterialKind::Gradient {
            top,
            bottom,
            alpha_top,
            alpha_bottom,
            height,
        } => {
            quad.shading = Shading::Gradient {
                top: *top,
                bottom: *bottom,
                alpha_top: *alpha_top,
                alpha_bottom: *alpha_bottom,
                height: *height,
            };
            quad.blend = Blend::InverseAlpha;
        }
        MaterialKind::SlimeTrailed => {
            // The per-platform slime-trail render texture is bound by the
            // caller; an empty trail leaves the sprite.
            let (tw, th) = texture_size;
            quad.shading = Shading::SlimeTrailed {
                trail: TextureId::TRANSPARENT,
                trail_texel: Vec2::new(1.0 / tw.max(1) as f32, 1.0 / th.max(1) as f32),
                flip_x: flip.0,
                flip_y: flip.1,
            };
        }
        MaterialKind::Water(w) => {
            quad.shading = Shading::ReflectiveWater(*w);
        }
        MaterialKind::Distortion { noise, params } => {
            let mut d = *params;
            d.noise = tex(noise);
            quad.shading = Shading::WaterDistortion(d);
        }
        MaterialKind::Aurora {
            noise,
            mask,
            params,
        } => {
            let mut a = *params;
            a.noise = tex(noise);
            a.mask = tex(mask);
            quad.shading = Shading::Aurora(a);
        }
        MaterialKind::BlackHole {
            distortion,
            proximity,
            scale,
            hole_brightness,
        } => {
            // `WriteScaleToShader`: `_Scale` = the renderer's scale.
            let world_scale = quad.transform.x_axis.truncate().length();
            quad.shading = Shading::BlackHole {
                grab: TextureId::TRANSPARENT,
                distortion: or(p[1], *distortion),
                proximity: *proximity,
                scale: or(
                    p[0],
                    if world_scale > 0.0 {
                        world_scale
                    } else {
                        *scale
                    },
                ),
                hole_brightness: or(p[2], *hole_brightness),
            };
        }
        MaterialKind::GrabInvert => {
            quad.shading = Shading::GrabInvert {
                grab: TextureId::TRANSPARENT,
            };
        }
        MaterialKind::Distort { noise, params } => {
            // `_AffectRadius` animated by scripts (`DarknessAura`) in
            // `[2]` when `[3]` is 1.
            let mut d = *params;
            d.noise = tex(noise);
            if p[3] == 1.0 {
                d.affect_radius = p[2];
            }
            quad.shading = Shading::Distort(d);
        }
        MaterialKind::Smear {
            main,
            shadow,
            offset,
            scale,
        } => {
            let (m, s) = extra.colors.map_or((*main, *shadow), |c| (c[0], c[1]));
            quad.shading = Shading::Smear {
                main: m,
                shadow: s,
                offset: *offset,
                scale: Vec2::new(scale.x, or(p[1], scale.y)),
            };
        }
        MaterialKind::OnlyWhite => quad.shading = Shading::OnlyWhite,
        MaterialKind::Expanding {
            noise,
            mask,
            progress,
            particle_speed,
            dist,
            radius,
        } => {
            quad.shading = Shading::ExpandingCircle {
                noise: tex(noise),
                mask: if mask.is_some() {
                    tex(mask)
                } else {
                    TextureId::WHITE
                },
                // `[3]` 2: a particle, `[0]` its age fraction (custom
                // stream × `_ParticleSpeed`, smootherstep, + `_Progress`).
                progress: if p[3] == 1.0 {
                    p[0]
                } else if p[3] == 2.0 {
                    let x = (p[0] * particle_speed).clamp(0.0, 1.0);
                    x * x * x * (x * (x * 6.0 - 15.0) + 10.0) + *progress
                } else {
                    *progress
                },
                dist: *dist,
                radius: *radius,
            };
        }
        MaterialKind::WhiteToColour => quad.shading = Shading::WhiteToColour,
        MaterialKind::WallMasked => {
            quad.shading = Shading::WallMasked {
                platform: TextureId::TRANSPARENT,
            }
        }
        MaterialKind::SingleChannel { mask } => {
            if invisible {
                return false;
            }
            quad.shading = Shading::SingleChannel { mask: *mask };
        }
        MaterialKind::Channels3 { red, green, blue } => {
            quad.shading = Shading::Channels3 {
                red: *red,
                green: *green,
                blue: *blue,
            };
        }
        MaterialKind::ResizablePlatform => {
            // The quad is the white 2x2 sprite (1 unit) scaled to cover the
            // shape; the shape is computed in quad units.
            let k = (quad.transform.x_axis.truncate().length() * quad.size.x).max(1e-4);
            quad.shading = Shading::ResizablePlatform {
                half: Vec2::new(p[0], p[1]) / k,
                radius: p[2] / k,
                border: 0.08 / k,
            };
        }
        MaterialKind::Lightning(l) => {
            // `StretchableLightning`: `_Scale` = the renderer's x scale,
            // `_UVYMin`/`_UVYMax` = the sprite's texture rect rows.
            let mut l = *l;
            if p[0] > 0.0 {
                l.scale = p[0];
            }
            l.uv_min = 1.0 - quad.uv.max.y;
            l.uv_max = 1.0 - quad.uv.min.y;
            quad.shading = Shading::Lightning(l);
        }
        MaterialKind::Beam { noise, params } => {
            let mut b = *params;
            b.noise = tex(noise);
            b.scale = or(p[0], b.scale);
            quad.shading = Shading::Beam(b);
            if b.kind == BeamKind::SoftBorders {
                quad.blend = Blend::Additive;
            }
        }
        MaterialKind::Clone {
            noise,
            progress,
            noise_str,
            noise_freq,
            direction,
            uv_offset,
        } => {
            // The original's vertex shader draws the mesh 4x larger.
            quad.size *= 4.0;
            // [3] = 2: progress, direction angle and uv offset per
            // renderer (the duplicator's platform effect).
            let (dir, uv) = if p[3] == 2.0 {
                (Vec2::from_angle(p[1]), p[2])
            } else {
                (*direction, *uv_offset)
            };
            quad.shading = Shading::Clone {
                noise: tex(noise),
                progress: if p[3] >= 1.0 { p[0] } else { *progress },
                noise_str: *noise_str,
                noise_freq: *noise_freq,
                direction: dir,
                uv_offset: uv,
            };
        }
        MaterialKind::ParticlePremultiply => quad.shading = Shading::ParticlePremultiply,
        MaterialKind::ParticleBlended { tint } => {
            quad.shading = Shading::ParticleBlended { tint: *tint };
        }
        MaterialKind::SoftAdditive => {
            if invisible {
                return false;
            }
            quad.blend = Blend::SoftAdditive;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_evaluation() {
        // Linear ramp 1 -> 0 over 1 s with matching tangents.
        let keys = [[0.0, 1.0, -1.0, -1.0], [1.0, 0.0, -1.0, -1.0]];
        assert_eq!(eval_curve(&keys, -1.0), 1.0);
        assert!((eval_curve(&keys, 0.5) - 0.5).abs() < 1e-6);
        assert_eq!(eval_curve(&keys, 2.0), 0.0);
        // Flat tangents: smoothstep.
        let keys = [[0.0, 0.0, 0.0, 0.0], [2.0, 1.0, 0.0, 0.0]];
        assert!((eval_curve(&keys, 1.0) - 0.5).abs() < 1e-6);
        assert!(eval_curve(&keys, 0.5) < 0.25);
    }

    #[test]
    fn materials_map_to_shading() {
        let mut m = Material {
            name: "x".into(),
            shader: "Unlit/GradientColor".into(),
            render_queue: -1,
            keywords: vec![],
            textures: Default::default(),
            floats: [("_AlphaTop".to_string(), 0.5)].into_iter().collect(),
            colors: [("_Gradient".to_string(), [1.0, 0.0, 0.0, 1.0])]
                .into_iter()
                .collect(),
        };
        match material_kind(Some(&m)) {
            MaterialKind::Gradient {
                top,
                alpha_top,
                alpha_bottom,
                ..
            } => {
                assert_eq!(top, Color::RED);
                assert_eq!(alpha_top, 0.5);
                assert_eq!(alpha_bottom, 1.0);
            }
            k => panic!("{k:?}"),
        }
        m.shader = "Custom/AdditiveColoredA8".into();
        assert_eq!(
            material_kind(Some(&m)),
            MaterialKind::AlphaTint { additive: true }
        );
        m.shader = "Sprites/Default".into();
        assert_eq!(
            material_kind(Some(&m)),
            MaterialKind::Sprite { tint: Color::WHITE }
        );
    }
}

#[cfg(test)]
mod data_tests {
    use super::*;

    /// The dev import (`cache/import`), if present.
    fn data() -> Option<GameData> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/import");
        GameData::load(&dir).ok()
    }

    #[test]
    fn composed_locals_match_imported_world_positions() {
        let Some(data) = data() else {
            eprintln!("skipping: no cache/import");
            return;
        };
        for bi in data.level_indices() {
            let level = data.level(bi).unwrap();
            let mut world = vec![Mat4::IDENTITY; level.objects.len()];
            for (i, o) in level.objects.iter().enumerate() {
                let parent = o.parent.map_or(Mat4::IDENTITY, |p| world[p]);
                world[i] = parent * local_matrix(o);
                let want = Vec3::from(o.world.trs.position);
                let got = world[i].w_axis.truncate();
                assert!(
                    (got - want).length() < 1e-2 * (1.0 + want.length()),
                    "level {bi} {}: {got} vs {want}",
                    o.name
                );
            }
        }
    }

    #[test]
    fn every_level_has_the_camera_rig() {
        let Some(data) = data() else {
            eprintln!("skipping: no cache/import");
            return;
        };
        for bi in data.level_indices() {
            let level = data.level(bi).unwrap();
            let space = level.level_type == 2;
            let v = LevelView::new(&data, level);
            let names: Vec<String> = v
                .cameras
                .iter()
                .filter_map(|c| match &c.target {
                    CamTarget::Global { name, .. } => Some(name.clone()),
                    CamTarget::Output => None,
                })
                .collect();
            assert!(matches!(v.cameras[v.main].target, CamTarget::Output));
            assert_eq!(v.main, v.cameras.len() - 1, "main camera renders last");
            assert!(names.contains(&"_platformRT".to_string()), "{bi}");
            if !space {
                for n in ["_ReflectionRT", "_BackgroundRT", "_WaveParticleRT"] {
                    assert!(names.iter().any(|x| x == n), "{bi} lacks {n}");
                }
                assert_eq!(v.masks.len(), 1, "{bi}: the water mask");
            }
        }
    }
}

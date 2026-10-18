//! Presentation of a running simulation (`bopl_sim::World::view()`): the
//! bopls with their animation frames, colours and facing, the slime trails
//! they paint on platforms, ability objects and pickups, platform poses,
//! death / water-splash effects, the HUD ability indicators, sounds from the
//! sim's cues (each event id played once), the camera follow and shake.
//!
//! Everything here is presentation: floats, a local RNG, no feedback into
//! the simulation.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use bopl_import::GameData;
use bopl_import::format::Object;
use bopl_render::{Color, Gpu, Quad, Renderer, Shading, TextureId, UvRect};
use bopl_sim::World;
use bopl_sim::data::{Content, SpriteId};
use bopl_sim::fix::Fix;
use bopl_sim::object::ObjId;
use bopl_sim::view::{
    Cue, EffectKind, EventId, FxCue, Look, PlayerView, SpriteKind, SpriteView, View, Visual,
};
use glam::{Mat4, Quat, Vec2, Vec3};

use crate::assets::Assets;
use crate::audio::Audio;
use crate::fx::{Fx, FxLib, FxStyle, Gfx, YELLOW_SLIME, root_at};
use crate::level_view::{LevelView, SLIME_TRAIL_LAYER};
use crate::present::{CameraFollow, PlatformPose, QuadDraw, SpriteDraw, WorldView};
use crate::sound::Sounds;

/// How an imported sprite renderer draws.
#[derive(Clone, Debug)]
pub struct RendererInfo {
    pub sprite: Option<String>,
    pub color: Color,
    pub sorting_layer: usize,
    pub sorting_order: i32,
    pub mask: i32,
    pub layer: u32,
    pub material: Option<String>,
    /// The material's name (`yellowSlime` = the default player material).
    pub material_name: Option<String>,
    pub flip: (bool, bool),
    /// The renderer component is enabled.
    pub enabled: bool,
    /// Draw size of sliced/tiled renderers.
    pub size: Option<Vec2>,
}

impl RendererInfo {
    pub fn of(o: &Object) -> Option<Self> {
        let r = o.sprite.as_ref()?;
        Some(Self {
            material_name: r.materials.first().cloned(),
            enabled: r.enabled,
            size: (r.draw_mode != 0).then(|| Vec2::new(r.size[0], r.size[1])),
            sprite: r.sprite.clone(),
            color: Color::from_array(r.color),
            sorting_layer: r.sorting_layer_index,
            sorting_order: r.sorting_order,
            mask: r.mask_interaction,
            layer: o.layer,
            material: r.material_ids.first().cloned(),
            flip: (r.flip_x, r.flip_y),
        })
    }

    /// A sprite draw with this renderer's settings at `m`.
    pub fn draw(&self, sprite: &str, m: Mat4) -> SpriteDraw {
        let (scale, rot, pos) = m.to_scale_rotation_translation();
        let angle = {
            let (axis, a) = rot.to_axis_angle();
            if axis.z < 0.0 { -a } else { a }
        };
        SpriteDraw {
            sprite: sprite.to_string(),
            position: pos,
            rotation: angle,
            scale: scale.truncate(),
            color: self.color,
            flip_x: self.flip.0,
            flip_y: self.flip.1,
            sorting_layer: self.sorting_layer,
            sorting_order: self.sorting_order,
            layer: self.layer,
            material: self.material.clone(),
            mask_interaction: self.mask,
            params: [0.0; 4],
            colors: None,
            size: self.size,
        }
    }
}

fn local_trs(o: &Object) -> Mat4 {
    let [x, y, z, w] = o.local.rotation;
    let q = Quat::from_xyzw(x, y, z, w);
    let q = if q.length_squared() > 1e-12 {
        q.normalize()
    } else {
        Quat::IDENTITY
    };
    Mat4::from_scale_rotation_translation(
        Vec3::from(o.local.scale),
        q,
        Vec3::from(o.local.position),
    )
}

/// One object of a prefab visual's subtree.
#[derive(Clone, Debug)]
pub struct PrefabNode {
    /// Index in the prefab's object list.
    pub object: u16,
    /// Parent node (`None` = the visual's root object).
    pub parent: Option<usize>,
    /// Local transform (the root: its authored scale only).
    pub local: Mat4,
    pub active: bool,
    pub renderer: Option<RendererInfo>,
    /// `PlayAnimation`: its animation, then the follow-up (looped); shown
    /// unless the module sets the frame.
    pub anim: Option<(AnimFrames, AnimFrames)>,
}

/// `DarknessAura` (Time stop's channel aura): node indices and curves.
#[derive(Clone, Debug, Default)]
pub struct AuraDef {
    /// The aura object's node.
    pub node: usize,
    pub red_glow: Option<usize>,
    pub black_glow: Option<usize>,
    pub black_streams: Option<usize>,
    pub black_fade: Option<usize>,
    pub orange_flare: Option<usize>,
    pub character: Option<usize>,
    pub red_curve: Vec<[f32; 4]>,
    pub black_curve: Vec<[f32; 4]>,
    pub streams_curve: Vec<[f32; 4]>,
    pub orange_curve: Vec<[f32; 4]>,
    pub unstable_curve: Vec<[f32; 4]>,
    pub target_color: Color,
    pub fade_from: f32,
    pub fade_to: f32,
    pub fade_time: f32,
}

/// `BeamEffect` (a beam box's layers): node indices.
#[derive(Clone, Debug, Default)]
pub struct BeamDef {
    pub white: Option<usize>,
    pub yellow: Option<usize>,
    pub orange: Option<usize>,
    pub red: Option<usize>,
    pub additive: Option<usize>,
    pub visual_offset: f32,
}

/// `TimeStop` (the inversion sphere): scale and exit alpha curves.
#[derive(Clone, Debug, Default)]
pub struct SphereDef {
    pub scale: Vec<[f32; 4]>,
    pub exit_alpha: Vec<[f32; 4]>,
}

/// The renderers of a prefab object's subtree.
#[derive(Clone, Debug, Default)]
pub struct PrefabVisual {
    /// Presentation scripts driven from the module's `Look::params`.
    pub aura: Option<AuraDef>,
    pub sphere: Option<SphereDef>,
    pub beam: Option<BeamDef>,
    /// Import prefab id.
    pub prefab_id: String,
    /// The visual's root object (prefab object index).
    pub root_object: usize,
    /// Root first, then descendants depth first.
    pub nodes: Vec<PrefabNode>,
    /// The subtree has particle systems.
    pub particles: bool,
}

/// A frame animation for presentation: (sprite id, ticks, mirrored).
pub type AnimFrames = Vec<(String, i32, bool)>;

/// An effect prefab played as a sequence of frame animations.
#[derive(Clone, Debug)]
pub struct EffectDef {
    pub info: RendererInfo,
    pub scale: Vec2,
    pub anims: Vec<AnimFrames>,
}

impl EffectDef {
    fn lengths(&self) -> Vec<i64> {
        self.anims
            .iter()
            .map(|a| a.iter().map(|f| f.1.abs() as i64).sum::<i64>() + 1)
            .collect()
    }

    /// Sprite at `age` ticks, `None` when finished.
    fn frame_at(&self, age: i64) -> Option<(&str, bool)> {
        let mut t = age;
        for (a, len) in self.anims.iter().zip(self.lengths()) {
            if t < len {
                let mut acc = 0i64;
                for f in a {
                    acc += f.1.abs() as i64;
                    if t < acc {
                        return Some((&f.0, f.2));
                    }
                }
                return a.last().map(|f| (f.0.as_str(), f.2));
            }
            t -= len;
        }
        None
    }
}

/// Team colours (match-flow.md 4.6).
#[derive(Clone, Copy, Debug)]
pub struct TeamColor {
    pub fill: Color,
    pub border: Color,
    pub saturated: Color,
}

/// Static lookups for drawing the simulation.
pub struct MatchVisuals {
    pub data: Rc<GameData>,
    pub content: Arc<Content>,
    /// Player colour index → in-game material id (`PlayerColors`).
    pub player_materials: Vec<String>,
    /// Player colour index → UI material id.
    pub ui_materials: Vec<String>,
    pub team_colors: Vec<TeamColor>,
    body: Option<RendererInfo>,
    trail: Option<(RendererInfo, Mat4, f32, f32)>,
    death: Option<EffectDef>,
    water: Option<EffectDef>,
    splat: Option<RendererInfo>,
    indicator: Option<RendererInfo>,
    /// `AbilityReadyIndicator.ResetSizeAnimation` keys.
    indicator_size: Vec<[f32; 4]>,
    arrow: Option<RendererInfo>,
    /// The time-stop `ForceArrow` prefab.
    force_arrow: Option<ForceArrowDef>,
    /// `DuplicatePlatformEffect`'s renderers and curves.
    dup_effect: Option<DupEffectDef>,
    /// Prefabs whose root references a `ForceArrowPrefab` (BoplBody,
    /// BlackHole), by content prefab index.
    has_force_arrow: RefCell<HashMap<u32, bool>>,
    /// Opaque texel points per sprite (eat particles).
    sprite_points: RefCell<HashMap<String, Option<Arc<Vec<Vec2>>>>>,
    prefabs: RefCell<HashMap<(u32, u16), Rc<PrefabVisual>>>,
    /// Presentation prefabs (effects).
    pub fxlib: RefCell<FxLib>,
    /// The rope's sprite (`Rope.sprite`): its texture is the strip's.
    rope_sprite: Option<String>,
    /// `Rope.color` (non-grass levels; grass ropes are black).
    rope_color: Color,
    /// `BeamColorList`: white, yellow, orange, red per colour index.
    pub beam_colors: Vec<[Color; 4]>,
}

/// `ForceArrow` (shown on BoplBodies and black holes during a time stop,
/// pointing along their velocity): a rectangle stretched to the length and
/// a triangle at its end.
#[derive(Clone, Debug)]
struct ForceArrowDef {
    triangle: RendererInfo,
    triangle_local: Mat4,
    rect: RendererInfo,
    rect_local: Mat4,
    multiplier: f32,
    min_length: f32,
}

impl ForceArrowDef {
    fn load(data: &GameData) -> Option<Self> {
        let id = data
            .prefabs
            .roots
            .iter()
            .find(|(_, e)| e.name == "ForceArrow")
            .map(|(id, _)| id.clone())?;
        let p = data.prefab(&id).ok()?;
        let fa = p.objects.first()?.component("ForceArrow")?;
        let node = |field: &str| -> Option<&Object> {
            let r = fa.fields[field]["$ref"].as_str()?;
            p.objects
                .iter()
                .find(|o| o.id == r || o.components.iter().any(|c| c.id == r))
        };
        let (t, r) = (node("Triangle")?, node("rectangle")?);
        Some(Self {
            triangle: RendererInfo::of(t)?,
            triangle_local: local_trs(t),
            rect: RendererInfo::of(r)?,
            rect_local: local_trs(r),
            multiplier: fa.fields["forceMultiplier"].as_f64().unwrap_or(0.1) as f32,
            min_length: fa.fields["minLength"].as_f64().unwrap_or(1.0) as f32,
        })
    }

    /// `SetForce(velocity)` at `position`: the arrow's draws (none below
    /// the minimum length). The root cancels its parent's scale.
    fn draws(&self, position: Vec2, velocity: Vec2, out: &mut Vec<SpriteDraw>) {
        let force = velocity * self.multiplier;
        let len = force.length();
        if len < self.min_length {
            return;
        }
        let angle = force.y.atan2(force.x) - std::f32::consts::FRAC_PI_2;
        let root =
            Mat4::from_rotation_translation(Quat::from_rotation_z(angle), position.extend(0.0));
        let (ts, tr, tp) = self.triangle_local.to_scale_rotation_translation();
        let tri = Mat4::from_scale_rotation_translation(ts, tr, Vec3::new(tp.x, len - 0.2, 0.0));
        let (rs, rr, rp) = self.rect_local.to_scale_rotation_translation();
        let rect = Mat4::from_scale_rotation_translation(Vec3::new(rs.x, len, 1.0), rr, rp);
        for (info, m) in [(&self.rect, rect), (&self.triangle, tri)] {
            if let Some(sp) = &info.sprite {
                out.push(info.draw(sp, root * m));
            }
        }
    }
}

fn asset_by_script<'a>(data: &'a GameData, script: &str) -> Option<&'a serde_json::Value> {
    data.assets
        .values()
        .find(|a| a.script == script)
        .map(|a| &a.fields)
}

fn color_of(j: &serde_json::Value) -> Color {
    let f = |k: &str| j[k].as_f64().unwrap_or(0.0) as f32;
    Color::rgba(f("r"), f("g"), f("b"), f("a"))
}

impl MatchVisuals {
    pub fn new(data: Rc<GameData>, content: Arc<Content>) -> Self {
        let mut player_materials = Vec::new();
        let mut ui_materials = Vec::new();
        if let Some(pc) = asset_by_script(&data, "PlayerColors") {
            for c in pc["playerColors"].as_array().into_iter().flatten() {
                player_materials.push(c["playerMaterial"]["$ref"].as_str().unwrap_or("").into());
                ui_materials.push(c["uiMaterial"]["$ref"].as_str().unwrap_or("").into());
            }
        }
        let mut team_colors: Vec<TeamColor> = asset_by_script(&data, "TeamColors")
            .and_then(|t| t["teamColors"].as_array())
            .map(|a| {
                a.iter()
                    .map(|t| TeamColor {
                        fill: color_of(&t["fill"]),
                        border: color_of(&t["border"]),
                        saturated: color_of(&t["saturated"]),
                    })
                    .collect()
            })
            .unwrap_or_default();
        extend_team_colors(&mut team_colors, &data, &player_materials);
        let player = data
            .player
            .prefab
            .as_deref()
            .and_then(|p| data.prefab(p).ok());
        let (mut body, mut trail) = (None, None);
        if let Some(p) = &player {
            let flip = p.objects.iter().position(|o| o.name == "flipTransform");
            for o in &p.objects {
                if o.name == "PlayerSprite" {
                    body = RendererInfo::of(o);
                }
                if o.name == "slimeTrail"
                    && let Some(info) = RendererInfo::of(o)
                {
                    let vary = o.component("VaryScaleWithWorldPos");
                    let st =
                        |k: &str| vary.and_then(|c| c.fields[k].as_f64()).unwrap_or(0.4) as f32;
                    let parent = flip.map_or(Mat4::IDENTITY, |f| local_trs(&p.objects[f]));
                    trail = Some((
                        info,
                        parent * local_trs(o),
                        st("xStrength"),
                        st("yStrength"),
                    ));
                }
            }
        }
        let effect = |id: &str, names: &[&str]| -> Option<EffectDef> {
            let p = data.prefab(id).ok()?;
            let o = p.objects.first()?;
            let info = RendererInfo::of(o)?;
            let anim_ref = o
                .components
                .iter()
                .find_map(|c| {
                    c.fields["animData"]["$ref"]
                        .as_str()
                        .or(c.fields["data"]["$ref"].as_str())
                })?
                .to_string();
            let ad = content.anim_data.iter().find(|a| a.id == anim_ref)?;
            let anims = names
                .iter()
                .filter_map(|n| ad.find(n).map(|i| &ad.animations[i]))
                .map(|a| {
                    a.frames
                        .iter()
                        .filter(|f| f.sprite != SpriteId::NONE)
                        .map(|f| {
                            (
                                content.sprites[f.sprite.0 as usize].clone(),
                                if f.duration == 0 { 0 } else { f.duration },
                                f.duration < 0,
                            )
                        })
                        .collect()
                })
                .collect();
            Some(EffectDef {
                info,
                scale: Vec2::new(o.local.scale[0], o.local.scale[1]),
                anims,
            })
        };
        let bounds = player.as_ref().and_then(|p| {
            p.objects[0]
                .component("DestroyIfOutsideSceneBounds")
                .map(|c| c.fields.clone())
        });
        let r = |k: &str| {
            bounds
                .as_ref()
                .and_then(|b| b[k]["$ref"].as_str().map(str::to_string))
        };
        let death_id = r("XEffect").unwrap_or_else(|| "sharedassets0.assets:1303".into());
        let death = effect(&death_id, &["anticipation", "start", "end"]);
        let water = r("DownEffect").and_then(|id| effect(&id, &["enter"]));
        // The ground splash of the death effect (`groundSplashEffect`).
        let splat = data.prefab(&death_id).ok().and_then(|p| {
            let gs = p.objects[0].component("DeathEffect2")?.fields["groundSplashEffect"]["$ref"]
                .as_str()?
                .to_string();
            let sp = data.prefab(&gs).ok()?;
            RendererInfo::of(sp.objects.first()?)
        });
        let indicator_prefab = data.prefab("sharedassets0.assets:1352").ok();
        let indicator = indicator_prefab
            .as_ref()
            .and_then(|p| p.objects.first().and_then(RendererInfo::of));
        let indicator_size = indicator_prefab
            .as_ref()
            .and_then(|p| p.objects.first()?.component("AbilityReadyIndicator"))
            .map(|c| crate::fx::curve_keys(&c.fields["ResetSizeAnimation"]))
            .unwrap_or_default();
        let arrow = data
            .prefab("sharedassets5.assets:46")
            .ok()
            .and_then(|p| p.objects.first().and_then(RendererInfo::of));
        let force_arrow = ForceArrowDef::load(&data);
        let dup_effect = DupEffectDef::load(&data);
        let rope = data
            .prefabs
            .roots
            .iter()
            .filter(|(_, e)| e.name == "Rope")
            .find_map(|(id, _)| {
                let p = data.prefab(id).ok()?;
                p.objects
                    .first()?
                    .component("Rope")
                    .map(|c| c.fields.clone())
            });
        let rope_sprite = rope
            .as_ref()
            .and_then(|f| f["sprite"]["$ref"].as_str().map(str::to_string));
        let rope_color = rope
            .as_ref()
            .map_or(Color::WHITE, |f| color_of(&f["color"]));
        let fxlib = RefCell::new(FxLib::new(data.clone(), content.clone()));
        let beam_colors = data.globals.beam_colors.as_ref().map_or(Vec::new(), |b| {
            b.fields["beamColors"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| {
                    [
                        color_of(&c["white"]),
                        color_of(&c["yellow"]),
                        color_of(&c["orange"]),
                        color_of(&c["red"]),
                    ]
                })
                .collect()
        });
        Self {
            data,
            content,
            player_materials,
            ui_materials,
            team_colors,
            body,
            trail,
            death,
            water,
            splat,
            indicator,
            indicator_size,
            arrow,
            force_arrow,
            dup_effect,
            has_force_arrow: RefCell::new(HashMap::new()),
            sprite_points: RefCell::new(HashMap::new()),
            prefabs: RefCell::new(HashMap::new()),
            fxlib,
            rope_sprite,
            rope_color,
            beam_colors,
        }
    }

    /// `_MainColor` and `_ShadowColor` of a player colour's material.
    pub fn player_colors(&self, color: i32) -> Option<[Color; 2]> {
        let m = self.player_material(color)?;
        let m = self.data.materials.get(&m)?;
        Some([
            m.colors.get("_MainColor").map(|c| Color::from_array(*c))?,
            m.colors
                .get("_ShadowColor")
                .map(|c| Color::from_array(*c))?,
        ])
    }

    /// The boulder sprite of a platform type (`boulders` list).
    pub fn boulder_sprite(&self, platform_type: usize) -> Option<String> {
        self.data
            .globals
            .sprite_lists
            .get("boulders")?
            .get(platform_type)?
            .sprite
            .clone()
    }

    /// A player's `_ShadowColor` (white for the black slime, as the mine
    /// light and revive markers use it).
    pub fn shadow_color(&self, color: i32) -> Color {
        let Some(id) = self.player_material(color) else {
            return Color::WHITE;
        };
        let Some(m) = self.data.materials.get(&id) else {
            return Color::WHITE;
        };
        if m.name.starts_with("blackSlime") {
            return Color::WHITE;
        }
        m.colors
            .get("_ShadowColor")
            .map_or(Color::WHITE, |c| Color::from_array(*c))
    }

    /// The particle system a prefab's script names as `coreParticleRef`
    /// (TeleportIndicator), as a component id.
    pub fn core_particle_ref(&self, prefab_id: &str) -> Option<String> {
        let p = self.data.prefab(prefab_id).ok()?;
        p.objects.iter().find_map(|o| {
            o.components.iter().find_map(|c| {
                c.fields
                    .get("coreParticleRef")?
                    .get("$ref")?
                    .as_str()
                    .map(str::to_string)
            })
        })
    }

    /// The import prefab id of a content prefab.
    pub fn prefab_id(&self, prefab: u32) -> Option<&str> {
        self.content
            .prefabs
            .get(prefab as usize)
            .map(|p| p.id.as_str())
    }

    pub fn player_material(&self, color: i32) -> Option<String> {
        self.player_materials
            .get(color.max(0) as usize % self.player_materials.len().max(1))
            .cloned()
    }

    pub fn team(&self, team: i32) -> TeamColor {
        let n = self.team_colors.len().max(1);
        self.team_colors
            .get(team.max(0) as usize % n)
            .copied()
            .unwrap_or(TeamColor {
                fill: Color::rgb(0.125, 0.6, 0.933),
                border: Color::rgb(0.084, 0.333, 0.509),
                saturated: Color::rgb(0.0, 0.745, 1.0),
            })
    }

    /// Import sprite id of a sim sprite.
    pub fn sprite_name(&self, id: SpriteId) -> Option<&str> {
        if id == SpriteId::NONE {
            return None;
        }
        self.content.sprites.get(id.0 as usize).map(String::as_str)
    }

    /// Icon sprite of an ability-list entry.
    pub fn ability_icon(&self, ability: usize) -> Option<&str> {
        self.content
            .abilities
            .get(ability)
            .and_then(|a| self.sprite_name(a.icon))
    }

    /// `BlackHole.particlesPerArea` of a prefab (default 10).
    fn particles_per_area(&self, prefab: u32) -> f32 {
        self.content
            .prefabs
            .get(prefab as usize)
            .and_then(|p| self.data.prefab(&p.id).ok())
            .and_then(|ip| {
                ip.objects.iter().find_map(|o| {
                    o.component("BlackHole")
                        .and_then(|c| c.fields["particlesPerArea"].as_f64())
                })
            })
            .unwrap_or(10.0) as f32
    }

    /// Half the sprite's size in world units (`sprite.bounds.extents`).
    fn sprite_extents(&self, sprite: &str) -> Option<Vec2> {
        let s = self.data.sprites.get(sprite)?;
        let ppu = s.pixels_per_unit.max(1e-6);
        Some(Vec2::new(s.rect[2], s.rect[3]) / ppu * 0.5)
    }

    /// Points on the sprite's opaque texels (alpha ≥ 0.5) in sprite units
    /// (pivot at the origin): the particle shape of a sprite (cached).
    pub fn sprite_points(&self, sprite: &str) -> Option<Arc<Vec<Vec2>>> {
        if let Some(p) = self.sprite_points.borrow().get(sprite) {
            return p.clone();
        }
        let pts = (|| {
            let s = self.data.sprites.get(sprite)?;
            let tex = self.data.textures.get(s.texture.as_deref()?)?;
            let img = bopl_render::Image::load_png(self.data.dir.join(&tex.png)).ok()?;
            let ppu = s.pixels_per_unit.max(1e-6);
            let [tx, ty, tw, th] = s.texture_rect;
            let (tx, ty, tw, th) = (tx as i64, ty as i64, tw as i64, th as i64);
            let pivot = Vec2::new(s.pivot[0] * s.rect[2], s.pivot[1] * s.rect[3]);
            let off = Vec2::new(s.texture_rect_offset[0], s.texture_rect_offset[1]);
            // At most ~4000 texels looked at.
            let step = (((tw * th) as f64 / 4000.0).sqrt().ceil() as i64).max(1);
            let mut out = Vec::new();
            let mut y = 0;
            while y < th {
                let mut x = 0;
                while x < tw {
                    let (px, py) = (tx + x, ty + y);
                    if px >= 0
                        && py >= 0
                        && (px as u32) < img.width
                        && (py as u32) < img.height
                        && img.pixel(px as u32, py as u32)[3] >= 128
                    {
                        // PNG rows go down, sprite y goes up.
                        let local = off + Vec2::new(x as f32 + 0.5, (th - 1 - y) as f32 + 0.5);
                        out.push((local - pivot) / ppu);
                    }
                    x += step;
                }
                y += step;
            }
            (!out.is_empty()).then(|| Arc::new(out))
        })();
        self.sprite_points
            .borrow_mut()
            .insert(sprite.to_string(), pts.clone());
        pts
    }

    /// The prefab's root has a component with a `ForceArrowPrefab`.
    fn has_force_arrow(&self, prefab: u32) -> bool {
        if let Some(&b) = self.has_force_arrow.borrow().get(&prefab) {
            return b;
        }
        let b = self
            .content
            .prefabs
            .get(prefab as usize)
            .and_then(|p| self.data.prefab(&p.id).ok())
            .and_then(|ip| {
                ip.objects.first().map(|o| {
                    o.components
                        .iter()
                        .any(|c| c.fields["ForceArrowPrefab"]["$ref"].is_string())
                })
            })
            .unwrap_or(false);
        self.has_force_arrow.borrow_mut().insert(prefab, b);
        b
    }

    pub fn prefab_visual(&self, prefab: u32, object: u16) -> Rc<PrefabVisual> {
        if let Some(v) = self.prefabs.borrow().get(&(prefab, object)) {
            return v.clone();
        }
        let mut v = PrefabVisual::default();
        if let Some(p) = self.content.prefabs.get(prefab as usize)
            && let Ok(ip) = self.data.prefab(&p.id)
            && ip.objects.get(object as usize).is_some()
        {
            v.prefab_id = ip.id.clone();
            v.root_object = object as usize;
            // Depth first, parents before children.
            let mut stack: Vec<(usize, Option<usize>)> = vec![(object as usize, None)];
            while let Some((i, parent)) = stack.pop() {
                let o = &ip.objects[i];
                let k = v.nodes.len();
                v.particles |= o.particles.is_some();
                v.nodes.push(PrefabNode {
                    object: i as u16,
                    parent,
                    local: if parent.is_none() {
                        Mat4::from_scale(Vec3::from(o.local.scale))
                    } else {
                        local_trs(o)
                    },
                    // The root's state is the simulation's.
                    active: parent.is_none() || o.active,
                    renderer: RendererInfo::of(o),
                    anim: o.component("PlayAnimation").map(|c| {
                        let data = c.fields["data"]["$ref"].as_str().unwrap_or("");
                        let a = |k: &str| {
                            crate::fx::anim_frames(
                                &self.content,
                                data,
                                c.fields[k].as_str().unwrap_or(""),
                            )
                        };
                        (a("animationToPlay"), a("animationOnEnd"))
                    }),
                });
                for &c in o.children.iter().rev() {
                    stack.push((c, Some(k)));
                }
            }
            // Presentation scripts.
            let node_of_ref = |r: &serde_json::Value| -> Option<usize> {
                let id = r["$ref"].as_str()?;
                let oi = ip
                    .objects
                    .iter()
                    .position(|o| o.id == id || o.components.iter().any(|c| c.id == id))?;
                v.nodes.iter().position(|n| n.object as usize == oi)
            };
            for (k, n) in v.nodes.iter().enumerate() {
                let o = &ip.objects[n.object as usize];
                if let Some(c) = o.component("DarknessAura") {
                    let f = &c.fields;
                    let ck = |k: &str| crate::fx::curve_keys(&f[k]);
                    let fl = |k: &str, d: f32| f[k].as_f64().map_or(d, |x| x as f32);
                    v.aura = Some(AuraDef {
                        node: k,
                        red_glow: node_of_ref(&f["redGlow"]),
                        black_glow: node_of_ref(&f["blackGlow"]),
                        black_streams: node_of_ref(&f["blackStreams"]),
                        black_fade: node_of_ref(&f["BlackFade"]),
                        orange_flare: node_of_ref(&f["OrangeFlare"]),
                        character: node_of_ref(&f["character"]),
                        red_curve: ck("RedGlowAnimation"),
                        black_curve: ck("BlackGlowAnimation"),
                        streams_curve: ck("BlackStreamsAnimation"),
                        orange_curve: ck("OrangeFlareAnimation"),
                        unstable_curve: ck("UnstableScale"),
                        target_color: color_of(&f["TargetColor"]),
                        fade_from: fl("BlackFadeAlphaFrom", 0.0),
                        fade_to: fl("BlackFadeAlphaTo", 1.0),
                        fade_time: fl("BlackFadeAnimTime", 3.0),
                    });
                }
                if let Some(c) = o.component("BeamEffect") {
                    let f = &c.fields;
                    v.beam = Some(BeamDef {
                        white: node_of_ref(&f["white"]),
                        yellow: node_of_ref(&f["yellow"]),
                        orange: node_of_ref(&f["orange"]),
                        red: node_of_ref(&f["red"]),
                        additive: node_of_ref(&f["additiveGlow"]),
                        visual_offset: o
                            .component("BeamObject")
                            .and_then(|b| b.fields["visualOffset"].as_f64())
                            .unwrap_or(0.5) as f32,
                    });
                }
                if let Some(c) = o.component("TimeStop") {
                    v.sphere = Some(SphereDef {
                        scale: crate::fx::curve_keys(&c.fields["InversionSphereScale"]),
                        exit_alpha: crate::fx::curve_keys(&c.fields["ExitAlpha"]),
                    });
                }
            }
        }
        let v = Rc::new(v);
        self.prefabs
            .borrow_mut()
            .insert((prefab, object), v.clone());
        v
    }
}

/// Team colours beyond the original 4 (match-flow.md 17.2): derived from
/// the player palette's shadow colours (fill), darkened (border).
fn extend_team_colors(teams: &mut Vec<TeamColor>, data: &GameData, player_materials: &[String]) {
    // Player colours not resembling the original 4 teams (blue, red,
    // green, pink), most distinct first: yellow, turquoise, violet,
    // orange, grey, black, white, purple (close to team 3: only used by
    // the twelfth team).
    for idx in [0usize, 2, 1, 7, 8, 11, 3, 5] {
        if teams.len() >= 12 {
            break;
        }
        let c = player_materials
            .get(idx)
            .and_then(|m| data.materials.get(m))
            .and_then(|m| m.colors.get("_ShadowColor"))
            .map(|c| Color::from_array(*c))
            .unwrap_or(Color::rgb(0.6, 0.6, 0.6));
        let fill = Color::rgba(c.r * 0.85, c.g * 0.85, c.b * 0.85, 1.0);
        teams.push(TeamColor {
            fill,
            border: Color::rgba(c.r * 0.45, c.g * 0.45, c.b * 0.45, 1.0),
            saturated: c,
        });
    }
}

#[derive(Clone, Debug)]
struct Effect {
    kind: EffectKind,
    position: Vec2,
    start: i64,
    color: i32,
    scale: f32,
    exploded: bool,
}

#[derive(Clone, Debug)]
struct Stamp {
    position: Vec2,
    rotation: f32,
    scale: Vec2,
    color: i32,
}

#[derive(Clone, Copy, Debug, Default)]
struct TrailState {
    grounded: bool,
    offset: Vec2,
}

/// `SlimeController.uncastableAbilityIndicatorAlpha` (Fix 0.4 in the player
/// prefab).
const UNCASTABLE_INDICATOR_ALPHA: f32 = 0.4;
/// The indicator circle's radius in sprite units: the material's
/// `_CircleExtents.x` (0.0488 of the 2048-texel icon sheet = 100 texels) at
/// 100 pixels per unit.
const INDICATOR_RADIUS: f32 = 1.0;
/// The circle's dark rim (measured on the original: 2.5 px at scale 0.9).
const INDICATOR_RIM: f32 = 0.15;
/// The rim's colour relative to `_CircleColor` (measured).
const INDICATOR_RIM_DARKEN: f32 = 0.57;

#[derive(Clone, Copy, Debug)]
struct IndicatorState {
    ready: bool,
    ability: usize,
    reset_at: f32,
}

/// The presentation state of a running match.
pub struct MatchPresenter {
    pub visuals: Rc<MatchVisuals>,
    pub level_view: LevelView,
    pub level: usize,
    generation: u32,
    camera: CameraFollow,
    camera_frozen: bool,
    pub aspect: f32,
    trauma: f32,
    time: f32,
    seen: BTreeSet<EventId>,
    effects: Vec<Effect>,
    stamps: Vec<Stamp>,
    trails: HashMap<i32, TrailState>,
    indicators: HashMap<(i32, usize), IndicatorState>,
    clear_trails: bool,
    rng: u32,
    tick: i64,
    /// Sound cues are played (off for silent fast-forward).
    pub play_sounds: bool,
    /// Screen shake enabled (setting).
    pub screen_shake: bool,
    /// Zoom out for more than 4 players (setting, ours).
    pub camera_zoom: bool,
    /// The camera follows the players (`PlayerAverageCamera`; disabled in
    /// the tutorial scene, whose camera stays at its authored x).
    pub camera_follow: bool,
    /// Ability slots whose HUD indicator is not drawn at all (the
    /// tutorial's locked slots: no icon until unlocked).
    pub locked_slots: [bool; 3],
    /// Colour index of effects without a player (the tutorial dummies'
    /// death effect in their own material); -1 = the default slime.
    pub orphan_effect_color: i32,
    /// Player ids of the local players of an online match: invisibility
    /// keeps them at least 0.2 visible (`SetAlphaBasedOnPlayerInvisibiltyTime`).
    pub online_local: Vec<i32>,
    /// Presentation effects playing (cue-driven).
    fx: Vec<Fx>,
    /// Particle systems of the view's objects (by object).
    attached: HashMap<ObjId, Fx>,
    /// Presentation prefabs following objects (`Look::attach`).
    followers: HashMap<(ObjId, &'static str), Fx>,
    /// Effects cued since the last frame: (tick, what).
    pending: Vec<(i64, PendingFx)>,
    /// Last tick the effects were advanced to.
    fx_tick: i64,
    fx_seed: u64,
    /// Last tick's objects (the black hole's eat particles take the shape
    /// of what it swallowed, which is gone by the time the cue arrives).
    prev_shapes: Vec<EatShape>,
    /// Duplicator platform effects of this tick.
    dup_fx: Vec<DupFx>,
    /// Force arrows of this tick (time stop): position, velocity.
    force_arrows: Vec<(Vec2, Vec2)>,
    /// Firing beams' staff directions of the last tick (`beamLoopHP`).
    beam_dirs: HashMap<ObjId, (i64, i64, i64, i64)>,
    /// Beam boxes of the last tick.
    beams: HashMap<ObjId, BeamSeen>,
    /// Child particle systems a script plays/stops (black hole
    /// `OnEatParticle`), by object.
    scripted: HashMap<ObjId, bool>,
    /// The level's particle systems (built on the first frame).
    level_fx: Option<LevelFx>,
}

/// Level particle systems: the scene's own (sky sparkles) and the
/// `WaterWaves` rings where platforms cross the water line.
struct LevelFx {
    scene: Vec<Fx>,
    waves: Vec<WaveFx>,
}

/// `WaterWaves`: two wave emitters (the left one mirrored) at the
/// platform's water-line crossings, moved inwards by `waveOffsetX`,
/// stopped when the platform is clear of the water, fully under it, or
/// the crossings are closer than `DespawnOffset`.
struct WaveFx {
    /// Index into `Level::platforms`.
    platform: usize,
    offset: Vec2,
    despawn: f32,
    left: Fx,
    right: Fx,
}

/// An effect cued by the simulation, created when the next frame is drawn.
#[derive(Clone, Debug)]
enum PendingFx {
    /// A cued presentation prefab; eat particles carry the swallowed
    /// object's shape (taken when the cue arrives, from the tick before).
    Cue(FxCue, Option<EatShape>),
    /// `EffectKind::Ability(prefab index)` (Throw's smoke, Smoke's burst).
    Prefab {
        prefab: u32,
        position: Vec2,
        player: i32,
    },
    /// A presentation prefab by name.
    Named { name: &'static str, position: Vec2 },
    /// A prefab reference on the level's `DetPhysics` (beam effects).
    Level {
        field: &'static str,
        root: Mat4,
        style: FxStyle,
    },
}

/// An object's shape for the eat particles: position, rotation (radians),
/// scale, the sprite (opaque texels) or a resizable platform's rectangle.
#[derive(Clone, Debug)]
struct EatShape {
    object: ObjId,
    position: Vec2,
    rotation: f32,
    scale: Vec2,
    sprite: Option<String>,
    rect: Option<Vec2>,
}

/// `DuplicatePlatformEffect`'s platform shader effect this tick: the
/// platform's sprite revealed towards the clone's place (`CloneEffect`
/// with `_Direction`, `_UVOffset`, `_Progress01`), or for a resizable
/// platform its rounded rectangle moving there.
#[derive(Clone, Debug)]
struct DupFx {
    sprite: Option<String>,
    /// Resizable: half extents and radius (unscaled), renderer colour.
    rect: Option<(Vec2, f32, Color)>,
    position: Vec2,
    rotation: f32,
    scale: Vec2,
    progress: f32,
    alpha: f32,
    direction: f32,
    uv_offset: f32,
}

/// The effect prefab's renderers and curves.
#[derive(Clone, Debug)]
struct DupEffectDef {
    shader: RendererInfo,
    resizable: RendererInfo,
    alpha: Vec<[f32; 4]>,
    progress: Vec<[f32; 4]>,
}

impl DupEffectDef {
    fn load(data: &GameData) -> Option<Self> {
        let (id, _) = data
            .prefabs
            .roots
            .iter()
            .find(|(_, e)| e.name == "DuplicatePlatformEffect")?;
        let p = data.prefab(id).ok()?;
        let f = &p
            .objects
            .first()?
            .component("DuplicatePlatformEffect")?
            .fields;
        let renderer = |field: &str| -> Option<RendererInfo> {
            let r = f[field]["$ref"].as_str()?;
            data.prefabs.roots.keys().find_map(|pid| {
                let q = data.prefab(pid).ok()?;
                let o = q
                    .objects
                    .iter()
                    .find(|o| o.id == r || o.components.iter().any(|c| c.id == r))?;
                RendererInfo::of(o)
            })
        };
        Some(Self {
            shader: renderer("platformShaderEffectPrefab")?,
            resizable: renderer("resizablePlatformShaderEffectPrefab")?,
            alpha: crate::fx::curve_keys(&f["alphaAnim"]),
            progress: crate::fx::curve_keys(&f["progressAnim01"]),
        })
    }
}

/// A beam box as last seen (for its end effects).
#[derive(Clone, Copy, Debug)]
struct BeamSeen {
    position: Vec2,
    rotation: f32,
    length: f32,
    width: f32,
    color: i32,
}

impl MatchPresenter {
    pub fn new(visuals: Rc<MatchVisuals>, level: usize, generation: u32) -> anyhow::Result<Self> {
        let lv = visuals.data.level(level)?;
        let level_view = LevelView::new(&visuals.data, lv);
        Ok(Self {
            visuals,
            level_view,
            level,
            generation,
            camera: CameraFollow::new(0.0),
            camera_frozen: false,
            aspect: 16.0 / 9.0,
            trauma: 0.0,
            time: 0.0,
            seen: BTreeSet::new(),
            effects: Vec::new(),
            stamps: Vec::new(),
            trails: HashMap::new(),
            indicators: HashMap::new(),
            clear_trails: true,
            rng: 0x9e37_79b9,
            tick: 0,
            play_sounds: true,
            screen_shake: true,
            camera_zoom: false,
            camera_follow: true,
            locked_slots: [false; 3],
            orphan_effect_color: -1,
            online_local: Vec::new(),
            fx: Vec::new(),
            attached: HashMap::new(),
            followers: HashMap::new(),
            pending: Vec::new(),
            fx_tick: 0,
            fx_seed: 0x5eed,
            prev_shapes: Vec::new(),
            dup_fx: Vec::new(),
            force_arrows: Vec::new(),
            beam_dirs: HashMap::new(),
            beams: HashMap::new(),
            scripted: HashMap::new(),
            level_fx: None,
        })
    }

    /// Rebuilds for a new level (keeps the event history).
    pub fn set_level(&mut self, level: usize, generation: u32) -> anyhow::Result<()> {
        if generation == self.generation {
            return Ok(());
        }
        let lv = self.visuals.data.level(level)?;
        self.level_view = LevelView::new(&self.visuals.data, lv);
        self.level = level;
        self.generation = generation;
        self.camera = CameraFollow::new(0.0);
        self.camera_frozen = false;
        self.time = 0.0;
        self.effects.clear();
        self.stamps.clear();
        self.trails.clear();
        self.indicators.clear();
        self.clear_trails = true;
        self.fx.clear();
        self.level_fx = None;
        self.attached.clear();
        self.scripted.clear();
        self.followers.clear();
        self.beams.clear();
        self.pending.clear();
        Ok(())
    }

    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// After every sim tick: cues (sounds, effects, shake), camera,
    /// trails. Locally the cues are the view's events (`dedup`: each id
    /// once); online, the event gate's first-seen events of every frame
    /// simulated since the last call (`dedup = false`: the gate already
    /// played each once, across rollbacks).
    pub fn observe_events(
        &mut self,
        w: &World,
        v: &View,
        events: &[bopl_sim::view::Event],
        dedup: bool,
        sounds: &mut Sounds,
        audio: &mut Audio,
    ) {
        let dt = 1.0 / 60.0;
        self.time += dt;
        self.tick = v.tick;
        for e in events {
            if dedup && !self.seen.insert(e.id) {
                continue;
            }
            log::debug!("cue {} {:?}", e.id.tick, e.cue);
            self.cue(w, v, &e.cue, sounds, audio);
        }
        if let Some(&first) = self.seen.iter().next()
            && first.tick < v.tick - 600
        {
            let keep = self.seen.split_off(&EventId {
                tick: v.tick - 600,
                seq: 0,
            });
            self.seen = keep;
        }
        // Death effects explode after their anticipation + start animations.
        let death = self.visuals.death.clone();
        for i in 0..self.effects.len() {
            let ef = self.effects[i].clone();
            if ef.kind != EffectKind::Death || ef.exploded {
                continue;
            }
            let Some(d) = &death else { continue };
            let l = d.lengths();
            let at = l.first().copied().unwrap_or(0) + l.get(1).copied().unwrap_or(0);
            if v.tick - ef.start >= at {
                self.effects[i].exploded = true;
                self.explode(w, &ef, sounds, audio);
            }
        }
        let tick = v.tick;
        self.effects.retain(|e| {
            let def = match e.kind {
                EffectKind::WaterSplash => &self.visuals.water,
                _ => &self.visuals.death,
            };
            def.as_ref()
                .is_some_and(|d| d.frame_at(tick - e.start).is_some())
        });
        // Camera: alive players' x (levels.md 7); stops on the fly-out.
        if v.round.winner_screen {
            self.camera_frozen = true;
        }
        if !self.camera_frozen && self.camera_follow {
            let xs: Vec<f32> = v
                .players
                .iter()
                .filter(|p| p.alive)
                .map(|p| p.position.x.to_f32())
                .collect();
            let pts: Vec<(f32, f32)> = v
                .players
                .iter()
                .filter(|p| p.alive)
                .map(|p| (p.position.x.to_f32(), p.position.y.to_f32()))
                .collect();
            self.camera.zoom(
                dt,
                &pts,
                self.aspect,
                self.camera_zoom && v.players.len() > 4,
            );
            self.camera.update(dt, xs, self.aspect);
        }
        self.trauma = (self.trauma - 0.6 * dt).max(0.0);
        // Trails: the offset is re-anchored on every landing.
        for p in &v.players {
            let st = self.trails.entry(p.id).or_default();
            if p.grounded && !st.grounded {
                let pos = Vec2::new(p.position.x.to_f32(), p.position.y.to_f32());
                st.offset = -pos + Vec2::splat(std::f32::consts::PI * 0.5);
            }
            st.grounded = p.grounded;
        }
        // Indicator reset pulses.
        for p in &v.players {
            for (k, s) in p.slots.iter().enumerate() {
                // SlimeController: ready when timer >= cooldown.
                let ready = s.timer >= s.cooldown;
                let st = self.indicators.entry((p.id, k)).or_insert(IndicatorState {
                    ready,
                    ability: s.ability,
                    reset_at: -100.0,
                });
                // Reset pulse: the cooldown completed (outside an ability)
                // or a pickup swapped the slot's ability.
                if (ready && !st.ready && !p.in_ability) || s.ability != st.ability {
                    st.reset_at = self.time;
                    log::debug!(target: "indicator", "tick {}: player {} slot {k} reset", v.tick, p.id);
                }
                st.ready = ready;
                st.ability = s.ability;
            }
        }
        // Beams (DetPhysics.renderBeams): a beam that ended leaves its red
        // after-glow and hit particles, with `beamEnd`.
        let mut beams: HashMap<ObjId, BeamSeen> = HashMap::new();
        for s in &v.sprites {
            let Visual::Prefab { prefab, object } = s.visual else {
                continue;
            };
            if self.visuals.prefab_visual(prefab, object).beam.is_none()
                || s.look.params[0] <= Fix::ZERO
            {
                continue;
            }
            beams.insert(
                s.object,
                BeamSeen {
                    position: Vec2::new(s.position.x.to_f32(), s.position.y.to_f32()),
                    rotation: s.rotation.to_f32(),
                    length: s.look.params[0].to_f32(),
                    width: s.look.params[1].to_f32(),
                    color: s.color,
                },
            );
        }
        let ended: Vec<BeamSeen> = self
            .beams
            .iter()
            .filter(|(id, _)| !beams.contains_key(id))
            .map(|(_, b)| *b)
            .collect();
        for b in ended {
            let cols = self.beam_colors(b.color);
            let len = (b.length - 0.5).max(0.0);
            let root = Mat4::from_scale_rotation_translation(
                Vec3::new(len / 40.3, 6.953 * b.width / 5.562, 1.0),
                Quat::from_rotation_z(b.rotation),
                b.position.extend(0.0),
            );
            let mut node_colors = Vec::new();
            if let Some(c) = cols {
                node_colors = vec![
                    ("beamRed".to_string(), c[3]),
                    ("additiveGlow".to_string(), c[3]),
                ];
            }
            self.pending.push((
                v.tick,
                PendingFx::Level {
                    field: "beamAfterEffectPrefab",
                    root,
                    style: FxStyle {
                        node_colors,
                        params: Some([len, 0.0, 0.0, 0.0]),
                        ..Default::default()
                    },
                },
            ));
            let end = b.position + Vec2::from_angle(b.rotation) * b.length;
            self.pending.push((
                v.tick,
                PendingFx::Level {
                    field: "beamAfterHitEffectPrefab",
                    root: root_at(end, b.rotation, 1.0),
                    style: FxStyle {
                        particle_color: cols.map(|c| {
                            let o = c[2];
                            Color::rgba(o.r, o.g, o.b, 0.25)
                        }),
                        ..Default::default()
                    },
                },
            ));
            if self.play_sounds {
                sounds.play(audio, "beamEnd");
            }
        }
        self.beams = beams;
        // Force arrows (`ForceArrow`): BoplBodies and black holes show
        // their velocity while time is stopped.
        self.force_arrows.clear();
        if v.round.time_stopped {
            use bopl_sim::abilities::{Behaviour, black_hole};
            use bopl_sim::world::Kind;
            for s in &v.sprites {
                let Visual::Prefab { prefab, object: 0 } = s.visual else {
                    continue;
                };
                if !self.visuals.has_force_arrow(prefab) {
                    continue;
                }
                let vel = match w.objects.get(s.object).map(|o| &o.kind) {
                    Some(Kind::Behaviour(b)) => match &**b {
                        Behaviour::BlackHole(black_hole::Spawned::Hole(h)) => Some(h.velocity),
                        _ => None,
                    },
                    _ => None,
                }
                .or_else(|| w.physics.body(s.object).map(|b| b.velocity));
                if let Some(vel) = vel {
                    self.force_arrows.push((
                        Vec2::new(s.position.x.to_f32(), s.position.y.to_f32()),
                        Vec2::new(vel.x.to_f32(), vel.y.to_f32()),
                    ));
                }
            }
        }
        if !self.force_arrows.is_empty() {
            log::debug!("force arrows {:?}", self.force_arrows);
        }
        // Nothing is drawn while the window is minimised or occluded: drop
        // effects cued long ago (they would only pile up) and keep the
        // trail stamps bounded.
        self.pending.retain(|(at, _)| v.tick - at <= 120);
        if self.stamps.len() > 512 {
            let extra = self.stamps.len() - 512;
            self.stamps.drain(..extra);
        }
        // Looping sounds (`PlayOrContinueLoop`): asked for every tick while
        // an object wants them; `Sounds::update` starts, fades and stops.
        if self.play_sounds && !v.round.winner_screen {
            for s in &v.sprites {
                if let Some(name) = s.look.loop_sound {
                    sounds.want_loop(name);
                }
            }
            for name in self.state_loops(w) {
                sounds.want_loop(name);
            }
        }
        let shapes = self.eat_shapes(v);
        self.dup_fx = self.dup_effects(w, &shapes);
        self.prev_shapes = shapes;
        // Music ducking (`FadeOutMusicTick`): the time-stop cast (0.4 per
        // second) and the time-stop sphere (0.2).
        if v.sprites
            .iter()
            .any(|s| s.look.loop_sound == Some("CreepyAmbientElectrical"))
        {
            sounds.fade_music(0.4);
        }
        if v.round.time_stopped {
            sounds.fade_music(0.2);
        }
    }

    /// The shape of the object a black hole swallowed at `pos` (its
    /// position on the tick before).
    fn eaten_shape(&self, pos: Vec2) -> Option<EatShape> {
        self.prev_shapes
            .iter()
            .filter(|e| e.position.distance(pos) < 1.5)
            .min_by(|a, b| {
                a.position
                    .distance(pos)
                    .total_cmp(&b.position.distance(pos))
            })
            .cloned()
    }

    /// The shapes of the view's objects and level platforms (for the
    /// next tick's eat particles).
    fn eat_shapes(&self, v: &View) -> Vec<EatShape> {
        let vis = &self.visuals;
        let mut out = Vec::new();
        let f = |x: Fix| x.to_f32();
        for s in &v.sprites {
            let Visual::Prefab { prefab, object } = s.visual else {
                continue;
            };
            let pv = vis.prefab_visual(prefab, object);
            let Some(root) = pv.nodes.first() else {
                continue;
            };
            let (sc, _, _) = root.local.to_scale_rotation_translation();
            let k = f(s.scale);
            // Resizable platforms: params = extents and radius (unscaled).
            let resizable = !pv.prefab_id.is_empty()
                && s.look.params[0] > Fix::ZERO
                && root
                    .renderer
                    .as_ref()
                    .and_then(|r| r.material_name.as_deref())
                    == Some("ResizablePlatform");
            let rect = resizable.then(|| {
                let (ex, ey, r) = (
                    f(s.look.params[0]),
                    f(s.look.params[1]),
                    f(s.look.params[2]),
                );
                let d = r * std::f32::consts::SQRT_2;
                Vec2::new(ex * 2.0 + d, ey * 2.0 + d)
            });
            let btype = s.look.params[3];
            let sprite = if s.sprite != SpriteId::NONE {
                vis.sprite_name(s.sprite).map(str::to_string)
            } else {
                root.renderer.as_ref().and_then(|r| r.sprite.clone())
            }
            .or_else(|| {
                // Throw's boulders: the platform type's boulder sprite.
                (btype > Fix::ZERO && rect.is_none())
                    .then(|| vis.boulder_sprite(btype.to_f32() as usize - 1))
                    .flatten()
            });
            out.push(EatShape {
                object: s.object,
                position: Vec2::new(f(s.position.x), f(s.position.y)),
                rotation: f(s.rotation),
                scale: sc.truncate() * k,
                sprite,
                rect,
            });
        }
        let lv = &self.level_view.level;
        for p in &v.platforms {
            let Some(o) = lv.platforms.get(p.index).and_then(|&i| lv.objects.get(i)) else {
                continue;
            };
            let sc = Vec2::new(o.world.trs.scale[0], o.world.trs.scale[1]) * f(p.scale);
            out.push(EatShape {
                object: p.object,
                position: Vec2::new(f(p.position.x), f(p.position.y)),
                rotation: f(p.rotation),
                scale: sc,
                sprite: o.sprite.as_ref().and_then(|r| r.sprite.clone()),
                rect: None,
            });
        }
        out
    }

    /// `DuplicatePlatformEffect.UpdateSim` presentation of every running
    /// platform duplication (duplicator.md).
    fn dup_effects(&mut self, w: &World, shapes: &[EatShape]) -> Vec<DupFx> {
        use bopl_sim::abilities::{Behaviour, duplicator};
        use bopl_sim::world::Kind;
        let Some(def) = self.visuals.dup_effect.clone() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let f = |x: Fix| x.to_f32();
        for id in w.objects.ids() {
            let Some(Kind::Behaviour(b)) = w.objects.get(id).map(|o| &o.kind) else {
                continue;
            };
            let Behaviour::Duplicator(duplicator::Spawned::PlatformEffect(e)) = &**b else {
                continue;
            };
            if !e.initialized || !w.objects.alive(e.platform) {
                continue;
            }
            let Some(shape) = shapes.iter().find(|s| s.object == e.platform) else {
                continue;
            };
            let t = f(e.time);
            let progress = crate::level_view::eval_curve(&def.progress, t);
            let alpha = crate::level_view::eval_curve(&def.alpha, progress);
            let start = Vec2::new(f(e.orig_start.x), f(e.orig_start.y));
            let target = Vec2::new(f(e.new_pos.x), f(e.new_pos.y));
            let rotation = f(e.rotation_on_init);
            let k = f(e.effect_scale);
            // `effectShake`: up to 0.3 × progress while running.
            let shake = if e.time < e.duration {
                let a = self.rand() * std::f32::consts::TAU;
                Vec2::from_angle(a) * self.rand().sqrt() * 0.3 * progress
            } else {
                Vec2::ZERO
            };
            if shape.rect.is_some() {
                let r = f(e.original_radius);
                let half = Vec2::new(f(e.original_extents.x), f(e.original_extents.y));
                out.push(DupFx {
                    sprite: None,
                    rect: Some((half, r, def.resizable.color)),
                    position: start.lerp(target, progress) + shake,
                    rotation,
                    scale: Vec2::splat(k),
                    progress,
                    alpha,
                    direction: 0.0,
                    uv_offset: 0.0,
                });
                continue;
            }
            let Some(sprite) = shape.sprite.clone() else {
                continue;
            };
            let data = &self.visuals.data;
            let Some(sp) = data.sprites.get(&sprite) else {
                continue;
            };
            let (tw, th) = sp
                .texture
                .as_deref()
                .and_then(|t| data.textures.get(t))
                .map_or((1.0, 1.0), |t| {
                    (t.width.max(1) as f32, t.height.max(1) as f32)
                });
            let mut vec = (target - start) * sp.pixels_per_unit;
            vec = Vec2::from_angle(-rotation).rotate(vec);
            vec /= Vec2::new(tw, th);
            // The platform's authored scale × the effect scale.
            let sim_k = w
                .objects
                .get(e.platform)
                .map_or(1.0, |o| f(o.tf.scale))
                .max(1e-4);
            out.push(DupFx {
                sprite: Some(sprite),
                rect: None,
                position: start + shake,
                rotation,
                scale: shape.scale / sim_k * k,
                progress,
                alpha,
                direction: vec.y.atan2(vec.x),
                uv_offset: vec.length() / k.max(1e-4),
            });
        }
        out
    }

    /// Loops the original asks for from state the view does not carry:
    /// `beamLoopBig` while a firing beam is merged with another (its beam
    /// role is a child), `beamLoopHP` while the firing beam turned this
    /// tick (`beamMovedThisFrame`), `reelInLoop` while a rope's caster
    /// holds the ability button with the rope attached
    /// (`SlimeController.playReelInSoundOnFrame`).
    fn state_loops(&mut self, w: &World) -> Vec<&'static str> {
        use bopl_sim::abilities::beam::{BeamState, Role};
        use bopl_sim::abilities::{self, AbilityState};
        use bopl_sim::world::Kind;
        let mut out = Vec::new();
        let mut dirs = HashMap::new();
        for id in w.objects.ids() {
            let Some(o) = w.objects.get(id) else { continue };
            match &o.kind {
                Kind::Ability(a) if !o.destroyed => {
                    let AbilityState::Beam(b) = &a.state else {
                        continue;
                    };
                    if b.state != BeamState::Loop {
                        continue;
                    }
                    let roles = &w.ability_globals.beams.roles;
                    if b.beam_index > 0
                        && roles.get(b.beam_index as usize).is_some_and(|r| {
                            matches!(r, Role::Child | Role::MaybeChild | Role::ParentAndChild)
                        })
                    {
                        out.push("beamLoopBig");
                    }
                    let dir = (
                        b.staff_dir.x.0,
                        b.staff_dir.y.0,
                        b.relative_staff_dir.x.0,
                        b.relative_staff_dir.y.0,
                    );
                    if self.beam_dirs.get(&id).is_some_and(|d| *d != dir) {
                        out.push("beamLoopHP");
                    }
                    dirs.insert(id, dir);
                }
                Kind::Bopl(bopl) => {
                    let c = &bopl.ctrl;
                    if c.is_in_ability || c.suppress_input || !c.has_spawned {
                        continue;
                    }
                    let Some(p) = w.player(w.objects.owner_id(id)) else {
                        continue;
                    };
                    if !p.can_use_abilities {
                        continue;
                    }
                    for (k, &a) in c.abilities.iter().enumerate() {
                        if w.objects.alive(a)
                            && abilities::is_rope(w, a)
                            && abilities::cooldown(w, a) == Fix::ZERO
                            && p.inputs.input.ability(k)
                            && (!abilities::only_castable_on_ground(w, a)
                                || bopl.physics.is_grounded)
                            && abilities::is_castable(w, a, p.id)
                        {
                            out.push("reelInLoop");
                        }
                    }
                }
                _ => {}
            }
        }
        self.beam_dirs = dirs;
        out
    }

    /// Sizes of the presentation's collections (leak checks in long runs).
    pub fn stats(&self) -> String {
        let particles: usize = self
            .fx
            .iter()
            .chain(self.attached.values())
            .chain(self.followers.values())
            .map(Fx::particle_count)
            .sum();
        format!(
            "seen {} effects {} stamps {} trails {} indicators {} fx {} attached {} followers {} pending {} beams {} scripted {} particles {}",
            self.seen.len(),
            self.effects.len(),
            self.stamps.len(),
            self.trails.len(),
            self.indicators.len(),
            self.fx.len(),
            self.attached.len(),
            self.followers.len(),
            self.pending.len(),
            self.beams.len(),
            self.scripted.len(),
            particles
        )
    }

    /// Stops the looping sounds (leaving the match).
    pub fn stop_loops(&mut self, sounds: &mut Sounds, audio: &mut Audio) {
        sounds.stop_loops(audio);
    }

    fn cue(&mut self, w: &World, v: &View, cue: &Cue, sounds: &mut Sounds, audio: &mut Audio) {
        let play = self.play_sounds;
        match cue {
            Cue::Sound(name) if play => {
                sounds.play(audio, name);
            }
            Cue::SoundPrefix(prefix) if play => {
                sounds.play_prefix(audio, prefix);
            }
            Cue::PlayerSound(name, id) if play => {
                // ORIGINAL LIMIT: ids above 4 reuse variants 1..4 (audio.md 4).
                let n = (id - 1).rem_euclid(4) + 1;
                sounds.play(audio, &format!("{name}{n}"));
            }
            Cue::Effect {
                kind,
                position,
                player_id,
            } => {
                let pos = Vec2::new(position.x.to_f32(), position.y.to_f32());
                let pl = w.player(*player_id);
                let color = pl.map_or(self.orphan_effect_color, |p| p.color);
                let scale = pl.map_or(1.0, |p| p.scale().to_f32());
                let kind = match kind {
                    EffectKind::WaterSplash => EffectKind::WaterSplash,
                    EffectKind::Death | EffectKind::BlastZone => EffectKind::Death,
                    k => *k,
                };
                if let EffectKind::Ability(code) = kind {
                    if code == bopl_sim::abilities::grappling_hook::HIT_PARTICLE {
                        self.pending.push((
                            v.tick,
                            PendingFx::Named {
                                name: "ropeHook_hitParticle",
                                position: pos,
                            },
                        ));
                    } else {
                        self.pending.push((
                            v.tick,
                            PendingFx::Prefab {
                                prefab: code as u32,
                                position: pos,
                                player: *player_id,
                            },
                        ));
                    }
                }
                if matches!(kind, EffectKind::Death | EffectKind::WaterSplash) {
                    if kind == EffectKind::Death && play {
                        sounds.play(audio, "slimeDeath");
                    }
                    self.effects.push(Effect {
                        kind,
                        position: pos,
                        start: v.tick,
                        color,
                        scale,
                        exploded: false,
                    });
                }
            }
            Cue::Shake(s) => {
                if self.screen_shake {
                    self.trauma = (self.trauma + *s as f32 / 1000.0).min(1.0);
                }
            }
            // `OnEatParticle.Play()/Stop()` (scale 0 = stop); any eat stops it.
            Cue::Fx(f) if f.field == "OnEatParticle" => {
                self.scripted.insert(f.source, f.scale != Fix::ZERO);
            }
            Cue::Fx(f) => {
                if f.field == "OnEatParticlePrefab" {
                    self.scripted.insert(f.source, false);
                }
                let shape = (f.field == "OnEatParticlePrefab")
                    .then(|| {
                        self.eaten_shape(Vec2::new(f.position.x.to_f32(), f.position.y.to_f32()))
                    })
                    .flatten();
                self.pending.push((v.tick, PendingFx::Cue(*f, shape)))
            }
            Cue::RoundWon { .. } if play => sounds.match_ended(audio, false),
            Cue::RoundDraw if play => sounds.match_ended(audio, true),
            Cue::SuddenDeath if play => sounds.set_music_pitch(audio, 1.1),
            _ => {}
        }
    }

    /// `DeathEffect2.OnExplode`: 15 rays to the closest platform within
    /// `gibbMaxDistance`; each hit paints a splash into the trail texture.
    fn explode(&mut self, w: &World, ef: &Effect, sounds: &mut Sounds, audio: &mut Audio) {
        let mut ph = w.physics.clone();
        let origin =
            bopl_sim::Vec2::new(Fix::from_f32(ef.position.x), Fix::from_f32(ef.position.y));
        let n = 15;
        let dist = Fix::from_int(20);
        for i in 0..n {
            let a = Fix::from_int(i) * Fix::ONE / Fix::from_int(n) * Fix::PI_TIMES_2;
            let dir = bopl_sim::Vec2::new(a.cos(), a.sin());
            let hit = ph.raycast_closest_rr(&w.objects, origin, dir, dist);
            if hit.hit() {
                let p = Vec2::new(hit.near_pos.x.to_f32(), hit.near_pos.y.to_f32());
                let rot = self.rand() * std::f32::consts::TAU;
                let sc = Vec2::new(self.rand() / 2.0 + 0.5, self.rand() / 2.0 + 0.5) * ef.scale;
                self.stamps.push(Stamp {
                    position: p,
                    rotation: rot,
                    scale: sc,
                    color: ef.color,
                });
                if self.play_sounds {
                    sounds.play_prefix(audio, "slimeSplash");
                }
            }
        }
    }

    fn shake(&self) -> Vec2 {
        let t = self.time;
        // Smooth noise in -1..1 (sum of sines standing in for Perlin).
        let n = (t * 37.0).sin() * 0.6 + (t * 61.0 + 1.3).sin() * 0.4;
        Vec2::new(0.15 * self.trauma * self.trauma * n * 10.0, 0.0)
    }

    /// Builds the world view of `v`.
    pub fn world_view(&mut self, v: &View, gfx: &mut Gfx<'_>) -> WorldView {
        let vis = self.visuals.clone();
        let mut out = WorldView {
            time: self.time,
            camera_x: Some(self.camera.x),
            camera_size: (self.camera.size != CameraFollow::SIZE).then_some(self.camera.size),
            shake: self.shake(),
            time_stopped: v.round.time_stopped,
            clear_trails: std::mem::take(&mut self.clear_trails),
            ..Default::default()
        };
        for p in &v.platforms {
            out.platforms.push(PlatformPose {
                platform: p.index,
                position: Vec2::new(p.position.x.to_f32(), p.position.y.to_f32()),
                rotation: p.rotation.to_f32(),
                scale: p.scale.to_f32(),
                visible: p.visible,
            });
        }
        let players: HashMap<i32, &PlayerView> = v.players.iter().map(|p| (p.id, p)).collect();
        if std::env::var_os("BOPL_DUMP_SPRITES").is_some() {
            for s in &v.sprites {
                let pid = match s.visual {
                    Visual::Prefab { prefab, .. } => self.visuals.prefab_id(prefab).unwrap_or(""),
                    _ => "",
                };
                log::info!(
                    "tick {} {:?} {:?} {:?} {pid} sprite {:?} pid {} colour {} pos ({:.2},{:.2}) layer {} look {:?}",
                    v.tick,
                    s.object,
                    s.kind,
                    s.visual,
                    self.visuals.sprite_name(s.look.sprite),
                    s.player_id,
                    s.color,
                    s.position.x.to_f32(),
                    s.position.y.to_f32(),
                    s.layer,
                    s.look
                );
            }
        }
        for s in &v.sprites {
            let pos = Vec2::new(s.position.x.to_f32(), s.position.y.to_f32());
            let rot = s.rotation.to_f32();
            let scale = s.scale.to_f32();
            match s.kind {
                SpriteKind::Bopl => {
                    let Some(body) = &vis.body else { continue };
                    let material = vis.player_material(s.color);
                    let facing = if s.facing_right { 1.0 } else { -1.0 };
                    let root = Mat4::from_scale_rotation_translation(
                        Vec3::new(scale, scale, 1.0),
                        Quat::from_rotation_z(rot),
                        pos.extend(0.0),
                    );
                    let flip = root * Mat4::from_scale(Vec3::new(facing, 1.0, 1.0));
                    let sprite = vis
                        .sprite_name(s.sprite)
                        .map(str::to_string)
                        .or_else(|| body.sprite.clone());
                    if let Some(sp) = sprite {
                        let mut d = body.draw(&sp, flip);
                        d.flip_x = s.flip_x;
                        d.material = material.clone();
                        d.color.a *= self.invisibility_alpha(s.player_id, s.look.alpha.to_f32());
                        out.sprites.push(d);
                    }
                    // Slime trail blob while grounded (SlimeTrailHandler).
                    if let (Some((info, local, xs, ys)), Some(p)) =
                        (&vis.trail, players.get(&s.player_id))
                        && p.grounded
                        && p.spawned
                        && let Some(sp) = &info.sprite
                    {
                        let st = self.trails.get(&p.id).copied().unwrap_or_default();
                        let m = flip * *local;
                        let wp = m.w_axis.truncate();
                        let wobble =
                            (wp.x + st.offset.x).sin() * xs + (wp.y + st.offset.y).sin() * ys;
                        let base_y = local.y_axis.truncate().length();
                        let m = m * Mat4::from_scale(Vec3::new(
                            1.0,
                            (base_y + wobble) / base_y.max(1e-6),
                            1.0,
                        ));
                        let mut d = info.draw(sp, m);
                        d.material = material.clone();
                        d.layer = SLIME_TRAIL_LAYER;
                        out.sprites.push(d);
                    }
                }
                SpriteKind::Ability | SpriteKind::Spawned => {
                    draw_object(&vis, s, v.tick, &mut out);
                }
            }
        }
        // Ropes: mesh strips through the segments (RopeMesh).
        self.draw_ropes(v, &mut out, gfx);
        // Presentation effects.
        self.update_fx(v, gfx);
        for f in self
            .fx
            .iter()
            .chain(self.attached.values())
            .chain(self.followers.values())
        {
            f.draw(&mut out);
        }
        if let Some(l) = &self.level_fx {
            for f in &l.scene {
                f.draw(&mut out);
            }
            for w in &l.waves {
                w.left.draw(&mut out);
                w.right.draw(&mut out);
            }
        }
        // Effects.
        for e in &self.effects {
            let def = match e.kind {
                EffectKind::WaterSplash => &vis.water,
                _ => &vis.death,
            };
            let Some(def) = def else { continue };
            let Some((sp, mirrored)) = def.frame_at(self.tick - e.start) else {
                continue;
            };
            let s = def.scale * e.scale;
            let m = Mat4::from_scale_rotation_translation(
                Vec3::new(s.x, s.y, 1.0),
                Quat::IDENTITY,
                e.position.extend(0.0),
            );
            let mut d = def.info.draw(sp, m);
            d.flip_x ^= mirrored;
            if e.kind == EffectKind::Death {
                d.material = vis.player_material(e.color).or(d.material);
            }
            out.sprites.push(d);
        }
        // Off-screen arrows (levels.md 7): alive, non-winning players
        // outside the view, clamped 1.3 units inside it, pointing at them.
        if let Some(info) = &vis.arrow
            && let Some(sp) = &info.sprite
        {
            let size = self.camera.size;
            let half = Vec2::new(size * self.aspect, size);
            let centre = Vec2::new(self.camera.x, self.camera.y());
            for p in &v.players {
                if !p.alive || !p.spawned || p.won_this_round || v.round.winner_screen {
                    continue;
                }
                let pos = Vec2::new(p.position.x.to_f32(), p.position.y.to_f32());
                let d = pos - centre;
                let wiggle = 2.0;
                if d.x.abs() <= half.x + wiggle && d.y <= half.y + wiggle {
                    continue;
                }
                let inner = half - Vec2::splat(1.3);
                let at =
                    centre + Vec2::new(d.x.clamp(-inner.x, inner.x), d.y.clamp(-inner.y, inner.y));
                let dir = pos - at;
                let mut jitter = Vec2::ZERO;
                if pos.x.abs() > 105.0 - 3.5 || pos.y > 58.0 - 3.5 {
                    jitter = Vec2::new(self.rand() - 0.5, self.rand() - 0.5) * 0.3;
                }
                let m = Mat4::from_scale_rotation_translation(
                    Vec3::ONE,
                    Quat::from_rotation_z(dir.y.atan2(dir.x) - std::f32::consts::FRAC_PI_2),
                    (at + jitter).extend(0.0),
                );
                let mut d = info.draw(sp, m);
                d.material = vis.player_material(p.color).or(d.material);
                out.sprites.push(d);
            }
        }
        if let Some(def) = &vis.dup_effect {
            for d in &self.dup_fx {
                let root = Mat4::from_scale_rotation_translation(
                    d.scale.extend(1.0),
                    Quat::from_rotation_z(d.rotation),
                    d.position.extend(0.0),
                );
                match (&d.rect, &d.sprite) {
                    (Some((half, r, color)), _) => {
                        let Some(sp) = &def.resizable.sprite else {
                            continue;
                        };
                        // The quad covers the shape (as ability platforms).
                        let cover = 2.0 * (half.x.max(half.y) + r) + 0.4;
                        let m = root * Mat4::from_scale(Vec3::new(cover, cover, 1.0));
                        let mut s = def.resizable.draw(sp, m);
                        s.params = [half.x, half.y, *r, 0.0];
                        s.color = Color::rgba(color.r, color.g, color.b, d.alpha);
                        out.sprites.push(s);
                    }
                    (None, Some(sprite)) => {
                        let mut s = def.shader.draw(sprite, root);
                        s.color.a = d.alpha;
                        // CloneEffect: progress, direction (radians), uv offset.
                        s.params = [d.progress, d.direction, d.uv_offset, 2.0];
                        out.sprites.push(s);
                    }
                    _ => {}
                }
            }
        }
        if let Some(fa) = &vis.force_arrow {
            for &(p, vel) in &self.force_arrows {
                fa.draws(p, vel, &mut out.sprites);
            }
        }
        // Death splashes: drawn for one frame into the trail textures.
        if let Some(info) = &vis.splat
            && let Some(sp) = &info.sprite
        {
            for st in self.stamps.drain(..) {
                let m = Mat4::from_scale_rotation_translation(
                    st.scale.extend(1.0),
                    Quat::from_rotation_z(st.rotation),
                    st.position.extend(0.0),
                );
                let mut d = info.draw(sp, m);
                d.material = vis.player_material(st.color).or(d.material);
                d.layer = SLIME_TRAIL_LAYER;
                out.sprites.push(d);
            }
        }
        out
    }

    /// Creates the cued effects, keeps the objects' particle systems and
    /// advances everything to the view's tick.
    fn update_fx(&mut self, v: &View, gfx: &mut Gfx<'_>) {
        let vis = self.visuals.clone();
        let steps = (v.tick - self.fx_tick).clamp(0, 30);
        self.fx_tick = v.tick;
        // New one-shot effects.
        for (at, p) in std::mem::take(&mut self.pending) {
            let mut lib = vis.fxlib.borrow_mut();
            let (def, root, style) = match p {
                PendingFx::Cue(c, eaten) => {
                    let r = if c.prefab == bopl_sim::view::LEVEL_PREFAB {
                        self.level_ref(c.field)
                    } else {
                        vis.prefab_id(c.prefab)
                            .and_then(|id| prefab_ref(&vis.data, id, c.field))
                    };
                    let Some(def) = r.and_then(|r| lib.def_of_ref(&r, gfx)) else {
                        log::debug!("fx {}: no prefab", c.field);
                        continue;
                    };
                    let mut pos = Vec2::new(c.position.x.to_f32(), c.position.y.to_f32());
                    let mut rot = c.rotation.to_f32();
                    let mut def = def;
                    let mut eat_scale: Option<Vec2> = None;
                    match c.field {
                        // Placed at (prefab x + hit x, prefab y): the ring
                        // sits on the water line.
                        "WaterRing" => {
                            pos = Vec2::new(def.root_position.x + pos.x, def.root_position.y);
                        }
                        // Teleport: towards the other portal (0.25 s).
                        "betweenPortalsParticlePrefab" => {
                            let v = Vec2::new(c.value.x.to_f32(), c.value.y.to_f32()) / 0.25;
                            let v = if rot.abs() > 1.0 { -v } else { v };
                            def = Rc::new(def.with_velocity(v));
                            rot = 0.0;
                        }
                        // Black hole: the swallowed object's shape, its
                        // scale, orbiting the hole (`SpawnParticle`).
                        "OnEatParticlePrefab" => {
                            let hole = Vec2::new(c.value.x.to_f32(), c.value.y.to_f32());
                            if let Some(e) = eaten {
                                pos = e.position;
                                let per_area = vis.particles_per_area(c.prefab);
                                let pts = e.sprite.as_deref().and_then(|sp| vis.sprite_points(sp));
                                let count = e.sprite.as_deref().and_then(|sp| {
                                    let ext = vis.sprite_extents(sp)?;
                                    Some((4.0 * ext.x * ext.y * per_area).min(1000.0))
                                });
                                let sc = Vec2::new(
                                    if e.scale.x.abs() < 1e-4 {
                                        1.0
                                    } else {
                                        e.scale.x
                                    },
                                    if e.scale.y.abs() < 1e-4 {
                                        1.0
                                    } else {
                                        e.scale.y
                                    },
                                );
                                let (orbit, root_scale) = if e.rect.is_some() {
                                    (hole - pos, Vec2::ONE)
                                } else {
                                    ((hole - pos) / sc, sc)
                                };
                                def = Rc::new(def.eaten(
                                    pts,
                                    e.rect,
                                    e.rotation.to_degrees(),
                                    if e.rect.is_some() { None } else { count },
                                    orbit,
                                ));
                                eat_scale = Some(root_scale);
                            }
                        }
                        // Ray trails: stretched over the ray, centred.
                        "RaycastParticlePrefab" => {
                            let dist = c.value.x.to_f32();
                            def = Rc::new(def.stretched(dist));
                            pos += Vec2::from_angle(rot) * dist * 0.5;
                        }
                        _ => {}
                    }
                    let style = FxStyle {
                        material: c
                            .player_id
                            .ne(&0)
                            .then(|| vis.player_material(self.player_color(v, c.player_id)))
                            .flatten(),
                        particle_color: c.color.map(fix_color),
                        color: c.color.map(fix_color),
                        ..Default::default()
                    };
                    let root = match eat_scale {
                        Some(k) => Mat4::from_scale_rotation_translation(
                            k.extend(1.0),
                            Quat::IDENTITY,
                            pos.extend(0.0),
                        ),
                        None => root_at(pos, rot, c.scale.to_f32()),
                    };
                    (def, root, style)
                }
                PendingFx::Prefab {
                    prefab,
                    position,
                    player,
                } => {
                    let Some(def) = vis.prefab_id(prefab).and_then(|id| lib.def(id, 0, gfx)) else {
                        continue;
                    };
                    let style = FxStyle {
                        material: vis.player_material(self.player_color(v, player)),
                        ..Default::default()
                    };
                    (def, root_at(position, 0.0, 1.0), style)
                }
                PendingFx::Level { field, root, style } => {
                    let Some(def) = self.level_ref(field).and_then(|r| lib.def_of_ref(&r, gfx))
                    else {
                        continue;
                    };
                    (def, root, style)
                }
                PendingFx::Named { name, position } => {
                    let Some(id) = vis
                        .data
                        .prefabs
                        .roots
                        .iter()
                        .find(|(_, e)| e.name == name)
                        .map(|(id, _)| id.clone())
                    else {
                        continue;
                    };
                    let Some(def) = lib.def(&id, 0, gfx) else {
                        continue;
                    };
                    (def, root_at(position, 0.0, 1.0), FxStyle::default())
                }
            };
            self.fx_seed = self.fx_seed.wrapping_add(0x9E37_79B9);
            let mut f = Fx::played(def, root, self.fx_seed, style);
            // Catch up with the ticks since it was cued.
            for _ in 0..(v.tick - at).clamp(0, 30) {
                f.step();
            }
            self.fx.push(f);
        }
        // Particle systems of the view's objects follow them.
        let mut seen: BTreeSet<ObjId> = BTreeSet::new();
        for s in &v.sprites {
            let Visual::Prefab { prefab, object } = s.visual else {
                continue;
            };
            let pv = vis.prefab_visual(prefab, object);
            if !pv.particles {
                continue;
            }
            seen.insert(s.object);
            let root = object_root(s);
            if let Some(f) = self.attached.get_mut(&s.object) {
                f.set_root(root);
                // Systems the module switches off (`PartLook::visible`).
                for n in &pv.nodes {
                    if let Some(p) = s.look.parts.iter().find(|p| p.object == n.object) {
                        f.set_emitting(n.object as usize, p.visible);
                    }
                }
                // Child systems a script plays (all but the root's).
                if let Some(&on) = self.scripted.get(&s.object) {
                    for n in &pv.nodes {
                        if n.object as usize != pv.root_object {
                            f.set_emitting(n.object as usize, on);
                        }
                    }
                }
                continue;
            }
            let Some(def) = vis
                .fxlib
                .borrow_mut()
                .def(&pv.prefab_id, pv.root_object, gfx)
            else {
                continue;
            };
            self.fx_seed = self.fx_seed.wrapping_add(0x9E37_79B9);
            // `TeleportIndicator.SetParticleColor`: only the system its
            // `coreParticleRef` names gets lerp(white, colour, 0.8); other
            // markers colour all their particles.
            let core = s
                .look
                .shadow_color
                .then(|| vis.core_particle_ref(&pv.prefab_id))
                .flatten();
            let style = FxStyle {
                particle_color: s.look.shadow_color.then(|| {
                    let c = vis.shadow_color(s.color);
                    if core.is_some() {
                        Color::WHITE.lerp(c, 0.8)
                    } else {
                        c
                    }
                }),
                particle_color_component: core,
                ..Default::default()
            };
            let mut f = Fx::new(def, root, self.fx_seed, style);
            f.particles_only = true;
            self.attached.insert(s.object, f);
        }
        self.scripted.retain(|id, _| seen.contains(id));
        for (id, f) in self.attached.iter_mut() {
            if !seen.contains(id) && !f.stopped {
                f.stop();
            }
        }
        // Followers (trails, auras).
        let mut listed: BTreeSet<(ObjId, &'static str)> = BTreeSet::new();
        for s in &v.sprites {
            let Visual::Prefab { prefab, .. } = s.visual else {
                continue;
            };
            for a in &s.look.attach {
                let key = (s.object, a.field);
                listed.insert(key);
                let root = root_at(
                    Vec2::new(a.position.x.to_f32(), a.position.y.to_f32()),
                    a.rotation.to_f32(),
                    a.scale.to_f32(),
                );
                let restart = self.followers.get(&key).is_some_and(|f| f.stopped);
                if let Some(f) = self.followers.get_mut(&key)
                    && !restart
                {
                    f.set_root(root);
                    continue;
                }
                let Some(def) = vis
                    .prefab_id(prefab)
                    .and_then(|id| prefab_ref(&vis.data, id, a.field))
                    .and_then(|r| vis.fxlib.borrow_mut().def_of_ref(&r, gfx))
                else {
                    continue;
                };
                self.fx_seed = self.fx_seed.wrapping_add(0x9E37_79B9);
                let mut def = def;
                let mut particle_color = a.color.map(fix_color);
                if a.shadow_color {
                    if a.field == "DashTrailPrefab" {
                        // Roll: lerp(c, complement, 0.65) → c → black.
                        let c = vis.shadow_color(s.color);
                        let c2 = hue_complement(c);
                        let m = |a: f32, b: f32| a + (b - a) * 0.65;
                        def = Rc::new(def.with_color_over_lifetime(&[
                            (0.0, [m(c.r, c2.r), m(c.g, c2.g), m(c.b, c2.b)]),
                            (0.167, [c.r, c.g, c.b]),
                            (1.0, [0.0, 0.0, 0.0]),
                        ]));
                    } else {
                        particle_color = Some(vis.shadow_color(s.color));
                    }
                }
                let style = FxStyle {
                    particle_color,
                    ..Default::default()
                };
                // A stopped follower keeps its particles; a new one starts.
                if let Some(old) = self.followers.remove(&key) {
                    self.fx.push(old);
                }
                self.followers
                    .insert(key, Fx::played(def, root, self.fx_seed, style));
            }
        }
        // Beam hit animations at the beams' ends.
        let hits: Vec<(ObjId, BeamSeen)> = self.beams.iter().map(|(k, b)| (*k, *b)).collect();
        for (id, b) in hits {
            let key = (id, "hitEffectPrefab");
            listed.insert(key);
            let end = b.position + Vec2::from_angle(b.rotation) * b.length;
            let root = root_at(end, b.rotation, 1.0);
            if let Some(f) = self.followers.get_mut(&key) {
                f.set_root(root);
                continue;
            }
            let Some(def) = self
                .level_ref("hitEffectPrefab")
                .and_then(|r| vis.fxlib.borrow_mut().def_of_ref(&r, gfx))
            else {
                continue;
            };
            let mut node_colors = Vec::new();
            if let Some(c) = self.beam_colors(b.color) {
                node_colors = vec![
                    ("hitAnim".to_string(), c[0]),
                    ("child".to_string(), c[1]),
                    ("child (1)".to_string(), c[2]),
                    ("child (2)".to_string(), c[3]),
                ];
            }
            self.fx_seed = self.fx_seed.wrapping_add(0x9E37_79B9);
            let mut f = Fx::new(
                def,
                root,
                self.fx_seed,
                FxStyle {
                    node_colors,
                    ..Default::default()
                },
            );
            f.looping = true;
            self.followers.insert(key, f);
        }
        for (key, f) in self.followers.iter_mut() {
            if !listed.contains(key) && !f.stopped {
                f.stop();
            }
        }
        if self.level_fx.is_none() {
            self.level_fx = Some(self.build_level_fx(gfx));
        }
        self.place_waves(v);
        for _ in 0..steps {
            for f in self
                .fx
                .iter_mut()
                .chain(self.attached.values_mut())
                .chain(self.followers.values_mut())
            {
                f.step();
            }
            // Level systems pause while time is stopped (`WaterWaves`).
            if let Some(l) = self.level_fx.as_mut()
                && !v.round.time_stopped
            {
                for f in l.scene.iter_mut() {
                    f.step();
                }
                for w in l.waves.iter_mut() {
                    w.left.step();
                    w.right.step();
                }
            }
        }
        if std::env::var_os("BOPL_DUMP_SPRITES").is_some() {
            for (id, f) in &self.attached {
                log::info!(
                    "attached {:?} {} particles {}",
                    id,
                    f.def.name,
                    f.particle_count()
                );
            }
            if let Some(l) = &self.level_fx {
                for f in l
                    .scene
                    .iter()
                    .chain(l.waves.iter().flat_map(|w| [&w.left, &w.right]))
                {
                    log::info!("level fx {} particles {}", f.def.name, f.particle_count());
                }
            }
            for f in &self.fx {
                log::info!(
                    "fx {} particles {} age {}",
                    f.def.name,
                    f.particle_count(),
                    f.age
                );
            }
        }
        self.fx.retain(|f| f.alive());
        self.attached.retain(|_, f| !f.stopped || f.alive());
        self.followers.retain(|_, f| !f.stopped || f.alive());
    }

    fn build_level_fx(&mut self, gfx: &mut Gfx<'_>) -> LevelFx {
        let vis = self.visuals.clone();
        let mut lib = vis.fxlib.borrow_mut();
        let level = &self.level_view.level;
        let objs = &level.objects;
        let mut scene = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            // Roots of particle subtrees only.
            if o.particles.is_none()
                || !o.active_in_hierarchy
                || o.parent.is_some_and(|p| objs[p].particles.is_some())
            {
                continue;
            }
            let def = lib.def_scene(&format!("{}#scene", level.name), objs, i, gfx);
            let mut root = crate::particles::EffectDef::world_of(objs, i);
            // Authored z scale 0 (flat 2D objects) would make it singular.
            if root.z_axis.truncate().length() < 1e-6 {
                root.z_axis = glam::Vec4::Z;
            }
            self.fx_seed = self.fx_seed.wrapping_add(0x9E37_79B9);
            let mut f = Fx::new(def, root, self.fx_seed, FxStyle::default());
            f.particles_only = true;
            scene.push(f);
        }
        let mut waves = Vec::new();
        if level.level_type != 2 {
            for (pi, &oi) in level.platforms.iter().enumerate() {
                let Some(c) = objs[oi].component("WaterWaves") else {
                    continue;
                };
                let f = &c.fields;
                let Some(def) = f["WavePrefab"]["$ref"]
                    .as_str()
                    .and_then(|r| lib.def_of_ref(r, gfx))
                else {
                    continue;
                };
                let num = |k: &str, d: f64| f[k].as_f64().unwrap_or(d) as f32;
                let mut mk = |sx: f32| {
                    self.fx_seed = self.fx_seed.wrapping_add(0x9E37_79B9);
                    let root = Mat4::from_scale(Vec3::new(sx, 1.0, 1.0));
                    let mut f = Fx::new(def.clone(), root, self.fx_seed, FxStyle::default());
                    f.particles_only = true;
                    f.set_all_emitting(false);
                    f
                };
                let left = mk(-1.0);
                let right = mk(1.0);
                waves.push(WaveFx {
                    platform: pi,
                    offset: Vec2::new(num("waveOffsetX", 0.2), num("waveOffsetY", -0.2)),
                    despawn: num("DespawnOffset", 0.3),
                    left,
                    right,
                });
            }
        }
        LevelFx { scene, waves }
    }

    /// Moves and switches the wave emitters (`WaterWaves.Update`; the
    /// crossings use the platform's bounding box).
    fn place_waves(&mut self, v: &View) {
        let Some(l) = self.level_fx.as_mut() else {
            return;
        };
        let level = &self.level_view.level;
        let wh = bopl_sim::consts::CONST_WATER_HEIGHT.to_f32();
        let raw = |r: i64| (r as f64 / 4_294_967_296.0) as f32;
        for w in l.waves.iter_mut() {
            let pose = v.platforms.iter().find(|p| p.index == w.platform);
            let shape = level
                .platforms
                .get(w.platform)
                .and_then(|&oi| level.objects[oi].shape.as_ref());
            let half = match shape.map(|s| &s.kind) {
                Some(bopl_import::format::DetShapeKind::RoundedRect { extents, radius }) => {
                    Vec2::new(
                        raw(extents[0]) + raw(*radius),
                        raw(extents[1]) + raw(*radius),
                    )
                }
                Some(bopl_import::format::DetShapeKind::Box { extents, .. }) => {
                    Vec2::new(raw(extents[0]), raw(extents[1]))
                }
                Some(bopl_import::format::DetShapeKind::Circle { radius }) => {
                    Vec2::splat(raw(*radius))
                }
                None => Vec2::ZERO,
            };
            let mut xs: Vec<f32> = Vec::new();
            if let Some(p) = pose.filter(|p| p.visible) {
                let c = Vec2::new(p.position.x.to_f32(), p.position.y.to_f32());
                let rot = Vec2::from_angle(p.rotation.to_f32());
                let h = half * p.scale.to_f32();
                let corners = [
                    Vec2::new(-h.x, h.y),
                    Vec2::new(h.x, h.y),
                    Vec2::new(h.x, -h.y),
                    Vec2::new(-h.x, -h.y),
                ]
                .map(|k| c + rot.rotate(k));
                for i in 0..4 {
                    let (a, b) = (corners[i], corners[(i + 1) % 4]);
                    if (a.y - wh) * (b.y - wh) <= 0.0 && (a.y - b.y).abs() > 1e-6 {
                        xs.push(a.x + (b.x - a.x) * (wh - a.y) / (b.y - a.y));
                    }
                }
            }
            let (lo, hi) = xs
                .iter()
                .fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
            let y = wh + w.offset.y;
            let left = lo - w.offset.x;
            let right = hi + w.offset.x;
            let on = xs.len() >= 2 && right - left >= w.despawn;
            if on {
                w.left.set_root(
                    Mat4::from_translation(Vec3::new(left, y, 0.0))
                        * Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)),
                );
                w.right
                    .set_root(Mat4::from_translation(Vec3::new(right, y, 0.0)));
            }
            w.left.set_all_emitting(on);
            w.right.set_all_emitting(on);
        }
    }

    /// A beam's colour set (white, yellow, orange, red).
    fn beam_colors(&self, color: i32) -> Option<[Color; 4]> {
        let b = &self.visuals.beam_colors;
        (!b.is_empty()).then(|| b[color.max(0) as usize % b.len()])
    }

    /// A prefab reference on the level's `DetPhysics` component.
    fn level_ref(&self, field: &str) -> Option<String> {
        self.level_view.level.objects.iter().find_map(|o| {
            o.component("DetPhysics")
                .and_then(|c| c.fields[field]["$ref"].as_str().map(str::to_string))
        })
    }

    fn player_color(&self, v: &View, id: i32) -> i32 {
        v.players
            .iter()
            .find(|p| p.id == id)
            .map_or(-1, |p| p.color)
    }

    /// The ropes as textured strips (RopeMesh: half width 0.2, v spacing
    /// 0.25 per segment, kept above the water line except in space).
    fn draw_ropes(&self, v: &View, out: &mut WorldView, gfx: &mut Gfx<'_>) {
        let vis = &self.visuals;
        let Some(sp) = vis.rope_sprite.as_deref() else {
            return;
        };
        let Some(g) = gfx.assets.sprite(gfx.gpu, gfx.renderer, sp) else {
            return;
        };
        let grass = self.level_view.level.level_type == 0;
        let space = self.level_view.level.level_type == 2;
        let color = if grass { Color::BLACK } else { vis.rope_color };
        let water = bopl_sim::consts::CONST_WATER_HEIGHT.to_f32();
        for r in &v.ropes {
            let pts: Vec<Vec2> = r
                .points
                .iter()
                .map(|p| Vec2::new(p.x.to_f32(), p.y.to_f32()))
                .collect();
            let n = pts.len();
            if n < 3 {
                continue;
            }
            let side = |i: usize| -> Vec2 {
                let d = if i == 0 {
                    pts[1] - pts[0]
                } else if i == n - 1 {
                    pts[n - 1] - pts[n - 3]
                } else {
                    pts[i] - pts[i - 1]
                };
                let d = d.normalize_or_zero();
                Vec2::new(d.y, -d.x) * 0.2
            };
            let clamp = |p: Vec2| {
                if space {
                    p
                } else {
                    Vec2::new(p.x, p.y.max(water))
                }
            };
            for i in 0..n - 1 {
                let (a, b) = (side(i), side(i + 1));
                let c00 = clamp(pts[i] + a);
                let c10 = clamp(pts[i] - a);
                let c01 = clamp(pts[i + 1] + b);
                let c11 = clamp(pts[i + 1] - b);
                out.quads.push(QuadDraw {
                    quad: Quad {
                        transform: Mat4::from_cols(
                            (c10 - c00).extend(0.0).extend(0.0),
                            (c01 - c00).extend(0.0).extend(0.0),
                            glam::Vec4::Z,
                            c00.extend(0.0).extend(1.0),
                        ),
                        texture: g.texture,
                        size: Vec2::ONE,
                        pivot: Vec2::ZERO,
                        // The texture repeats along the rope every four
                        // segments (v spacing 0.25).
                        uv: bopl_render::UvRect {
                            min: Vec2::new(0.0, (i % 4) as f32 * 0.25),
                            max: Vec2::new(1.0, (i % 4 + 1) as f32 * 0.25),
                        },
                        color,
                        shading: Shading::SpriteCorner {
                            corner: c11.extend(0.0),
                        },
                        ..Default::default()
                    },
                    material: None,
                    params: [0.0; 4],
                    layer: 24,
                    // The rope's `SortingGroup`: background, order 1.
                    sorting_layer: 0,
                    sorting_order: 1,
                    z: 0.0,
                    mask_interaction: 0,
                });
            }
        }
    }

    /// A player's invisibility alpha as drawn: online, local players stay
    /// at least 0.2 visible.
    fn invisibility_alpha(&self, player_id: i32, a: f32) -> f32 {
        if self.online_local.contains(&player_id) {
            a.max(0.2)
        } else {
            a
        }
    }

    /// HUD ability indicators (ui.md 5): three per spawned player, around
    /// it, team-coloured circle, icon, cooldown pie. World-space quads for
    /// the main camera pass.
    pub fn indicator_quads(
        &self,
        v: &View,
        gpu: &Gpu,
        renderer: &mut Renderer,
        assets: &mut Assets,
    ) -> Vec<Quad> {
        let vis = &self.visuals;
        let mut out = Vec::new();
        const OFFSETS: [Vec2; 3] = [
            Vec2::new(-2.31, 1.98),
            Vec2::new(2.31, 1.98),
            Vec2::new(0.0, 3.91),
        ];
        let cd = vis
            .indicator
            .as_ref()
            .and_then(|i| i.material.as_ref())
            .and_then(|m| vis.data.materials.get(m))
            .and_then(|m| m.colors.get("_CDColor"))
            .map_or(Color::rgba(0.659, 0.439, 0.0, 0.49), |c| {
                Color::from_array(*c)
            });
        for p in &v.players {
            if !p.spawned || !p.alive || p.in_ability {
                continue;
            }
            // Invisibility fades the indicators with the bopl.
            let fade = self.invisibility_alpha(p.id, p.alpha.to_f32());
            let team = vis.team(p.team);
            let center = Vec2::new(p.position.x.to_f32(), p.position.y.to_f32());
            for (k, s) in p.slots.iter().enumerate().take(3) {
                if self.locked_slots[k] {
                    continue;
                }
                let Some(icon) = vis.ability_icon(s.ability) else {
                    continue;
                };
                let Some(g) = assets.sprite(gpu, renderer, icon) else {
                    continue;
                };
                let st = self.indicators.get(&(p.id, k));
                let since = st.map_or(100.0, |st| self.time - st.reset_at);
                // AbilityReadyIndicator: the size pulse (the reset colour is
                // white, so no colour change), `ResetSizeAnimation + 0.2`.
                let scale = if since < 10.0 && !vis.indicator_size.is_empty() {
                    crate::level_view::eval_curve(&vis.indicator_size, since) + 0.2
                } else {
                    0.9
                };
                let pos = center + OFFSETS[k];
                // SlimeController.LateUpdate: an uncastable ability's
                // indicator is dimmed to `uncastableAbilityIndicatorAlpha`.
                let alpha = if s.castable {
                    fade
                } else {
                    fade.min(UNCASTABLE_INDICATOR_ALPHA)
                };
                let progress = if s.cooldown.to_f32() <= 0.0 {
                    1.0
                } else {
                    (s.timer.to_f32() / s.cooldown.to_f32()).clamp(0.0, 1.0)
                };
                // One quad like the original's single sprite: a square that
                // holds both the circle and the icon, the icon's uv rect
                // widened to the square and clamped in the shader.
                let side = g.size.x.max(g.size.y).max(2.0 * INDICATOR_RADIUS + 0.1);
                let c = (g.uv.min + g.uv.max) * 0.5;
                let h = (g.uv.max - g.uv.min) * 0.5;
                let k_uv = Vec2::new(side / g.size.x.max(1e-6), side / g.size.y.max(1e-6));
                out.push(Quad {
                    texture: g.texture,
                    uv: UvRect {
                        min: c - h * k_uv,
                        max: c + h * k_uv,
                    },
                    transform: Mat4::from_translation(pos.extend(0.0))
                        * Mat4::from_scale(Vec3::new(scale, scale, 1.0)),
                    size: Vec2::splat(side),
                    pivot: Vec2::splat(0.5),
                    color: Color::rgba(1.0, 1.0, 1.0, alpha),
                    shading: Shading::AbilityIndicator {
                        circle: team.fill,
                        rim: Color::rgba(
                            team.fill.r * INDICATOR_RIM_DARKEN,
                            team.fill.g * INDICATOR_RIM_DARKEN,
                            team.fill.b * INDICATOR_RIM_DARKEN,
                            team.fill.a,
                        ),
                        cd,
                        radius: INDICATOR_RADIUS,
                        rim_width: INDICATOR_RIM,
                        progress,
                        uv_clamp: g.uv,
                    },
                    ..Default::default()
                });
            }
        }
        out
    }

    /// Main-camera world position of a screen pixel (mouse aim).
    pub fn screen_to_world(&self, v: &WorldView, pixel: Vec2, size: (u32, u32)) -> Vec2 {
        self.level_view.main_camera(v).screen_to_world(pixel, size)
    }

    /// Screen pixel of a world position (HUD placement, tests).
    #[allow(dead_code)]
    pub fn world_to_screen(&self, v: &WorldView, p: Vec2, size: (u32, u32)) -> Vec2 {
        self.level_view
            .main_camera(v)
            .world_to_screen(p.extend(0.0), size)
    }
}

/// The colour with its hue turned half way (`HSVToRGB((H + 0.5) % 1, S, V)`).
fn hue_complement(c: Color) -> Color {
    let (r, g, b) = (c.r, c.g, c.b);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let v = max;
    let s = if max > 0.0 { d / max } else { 0.0 };
    let mut h = if d <= 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    h = (h + 0.5) % 1.0;
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color::rgba(r, g, b, c.a)
}

fn fix_color(c: [Fix; 4]) -> Color {
    Color::rgba(c[0].to_f32(), c[1].to_f32(), c[2].to_f32(), c[3].to_f32())
}

/// The root transform of a view object (position, rotation, uniform scale
/// times the module's stretch).
fn object_root(s: &SpriteView) -> Mat4 {
    let scale = s.scale.to_f32();
    let st = Vec2::new(s.look.stretch.x.to_f32(), s.look.stretch.y.to_f32());
    Mat4::from_scale_rotation_translation(
        Vec3::new(scale * st.x, scale * st.y, 1.0),
        Quat::from_rotation_z(s.rotation.to_f32()),
        Vec3::new(s.position.x.to_f32(), s.position.y.to_f32(), 0.0),
    )
}

/// The `$ref` of a serialized prefab field on any component of a prefab
/// (`dustParticlePrefab`, ...).
pub fn prefab_ref(data: &GameData, prefab: &str, field: &str) -> Option<String> {
    let p = data.prefab(prefab).ok()?;
    p.objects.iter().find_map(|o| {
        o.components
            .iter()
            .find_map(|c| c.fields[field]["$ref"].as_str().map(str::to_string))
    })
}

/// Draws an ability or spawned object: its prefab visual's renderers, with
/// the module's [`Look`] (frames, visibility, poses, alpha, parameters)
/// and the owner's player material where the original applies it.
fn draw_object(vis: &MatchVisuals, s: &SpriteView, tick: i64, out: &mut WorldView) {
    let Visual::Prefab { prefab, object } = s.visual else {
        return;
    };
    let pv = vis.prefab_visual(prefab, object);
    let look: &Look = &s.look;
    let player_mat = vis.player_material(s.color);
    let mut root = object_root(s);
    let n = pv.nodes.len();
    // Script-driven overrides: (scale, alpha, colour, params) per node.
    let mut s_scale = vec![1.0f32; n];
    let mut s_alpha: Vec<Option<f32>> = vec![None; n];
    let mut s_color: Vec<Option<Color>> = vec![None; n];
    let mut s_params: Vec<Option<[f32; 4]>> = vec![None; n];
    let ev = crate::level_view::eval_curve;
    if let Some(a) = &pv.aura
        && look.params[3] == Fix::ONE
    {
        let t = look.params[0].to_f32();
        let set = |v: &mut Vec<Option<[f32; 4]>>, i: Option<usize>, r: f32| {
            if let Some(i) = i {
                v[i] = Some([0.0, 0.0, r, 1.0]);
            }
        };
        set(&mut s_params, a.red_glow, ev(&a.red_curve, t));
        set(&mut s_params, a.black_glow, ev(&a.black_curve, t));
        set(&mut s_params, a.black_streams, ev(&a.streams_curve, t));
        if let Some(i) = a.orange_flare {
            s_alpha[i] = Some(ev(&a.orange_curve, t));
        }
        let end = a.red_curve.last().map_or(1.0, |k| k[0]).max(1e-3);
        if let Some(i) = a.character {
            let k = (t / end).clamp(0.0, 1.0);
            let c = a.target_color;
            s_color[i] = Some(Color::rgba(
                1.0 + (c.r - 1.0) * k,
                1.0 + (c.g - 1.0) * k,
                1.0 + (c.b - 1.0) * k,
                1.0 + (c.a - 1.0) * k,
            ));
        }
        if let Some(i) = a.black_fade {
            let k = (t / a.fade_time.max(1e-3)).clamp(0.0, 1.0);
            s_alpha[i] = Some(a.fade_from + (a.fade_to - a.fade_from) * k);
        }
        let since = look.params[1].to_f32();
        if since >= 0.0 {
            s_scale[a.node] = ev(&a.unstable_curve, since);
        }
    }
    // Beam boxes (`BeamObject.SetDimensions`, `BeamEffect`): the root's y
    // scale is the width, the layers stretch over the beam length.
    let mut s_xscale: Vec<Option<f32>> = vec![None; n];
    if let Some(b) = &pv.beam {
        let len = look.params[0].to_f32() - b.visual_offset;
        let width = look.params[1].to_f32();
        let (_, rot, t) = root.to_scale_rotation_translation();
        root = Mat4::from_scale_rotation_translation(Vec3::new(1.0, width, 1.0), rot, t);
        let cols = if vis.beam_colors.is_empty() {
            None
        } else {
            vis.beam_colors
                .get(s.color.max(0) as usize % vis.beam_colors.len())
        };
        for (i, x, ci) in [
            (b.white, len, 0usize),
            (b.yellow, len, 1),
            (b.orange, len + 2.8, 2),
            (b.red, len, 3),
            (b.additive, len, 3),
        ] {
            if let Some(i) = i {
                s_xscale[i] = Some(x.max(0.0));
                s_params[i] = Some([x.max(0.0), 0.0, 0.0, 0.0]);
                if let Some(c) = cols {
                    s_color[i] = Some(c[ci]);
                }
            }
        }
        if len <= 2.0
            && let Some(i) = b.additive
        {
            s_alpha[i] = Some(0.0);
        }
    }
    if let Some(sp) = &pv.sphere {
        let t = look.params[0].to_f32();
        let d = look.params[1].to_f32();
        let k = ev(&sp.scale, t).max(0.0);
        root *= Mat4::from_scale(Vec3::new(k, k, 1.0));
        s_alpha[0] = Some(ev(&sp.exit_alpha, t - d));
    }
    // Ability-made platforms: the resizable platform's quad covers its
    // shape (`ResizablePlatform` scales the 1-unit sprite).
    let is_resizable = pv
        .nodes
        .first()
        .and_then(|n| n.renderer.as_ref())
        .is_some_and(|r| r.material_name.as_deref() == Some("ResizablePlatform"));
    if is_resizable && look.params[0] > Fix::ZERO {
        let (hx, hy, r) = (
            look.params[0].to_f32(),
            look.params[1].to_f32(),
            look.params[2].to_f32(),
        );
        let cover = 2.0 * (hx.max(hy) + r) + 0.4;
        let (_, rot, t) = root.to_scale_rotation_translation();
        root = Mat4::from_scale_rotation_translation(Vec3::new(cover, cover, 1.0), rot, t);
    }
    let mut world = vec![Mat4::IDENTITY; n];
    let mut active = vec![true; n];
    for (k, node) in pv.nodes.iter().enumerate() {
        let part = look.parts.iter().find(|p| p.object == node.object);
        let local = match part.and_then(|p| p.pose) {
            Some(pose) => Mat4::from_scale_rotation_translation(
                Vec3::new(pose.scale.x.to_f32(), pose.scale.y.to_f32(), 1.0),
                Quat::from_rotation_z(pose.rotation.to_f32()),
                Vec3::new(pose.position.x.to_f32(), pose.position.y.to_f32(), 0.0),
            ),
            None => node.local,
        };
        let mut local = if (is_resizable || pv.beam.is_some())
            && node.parent.is_none()
            && look.params[0] > Fix::ZERO
        {
            Mat4::IDENTITY
        } else {
            local
        } * Mat4::from_scale(Vec3::new(s_scale[k], s_scale[k], 1.0));
        if let Some(x) = s_xscale[k] {
            let (sc, r, t) = local.to_scale_rotation_translation();
            local = Mat4::from_scale_rotation_translation(Vec3::new(x, sc.y, sc.z), r, t);
        }
        world[k] = node.parent.map_or(root, |p| world[p]) * local;
        let parent_active = node.parent.is_none_or(|p| active[p]);
        active[k] = parent_active && part.map_or(node.active, |p| p.visible);
    }
    for (k, node) in pv.nodes.iter().enumerate() {
        if !active[k] {
            continue;
        }
        let Some(r) = &node.renderer else { continue };
        let part = look.parts.iter().find(|p| p.object == node.object);
        let is_root = node.parent.is_none();
        if is_root && look.hidden {
            continue;
        }
        if part.is_none() && !r.enabled {
            continue;
        }
        let (frame, mut flip, alpha) = if is_root {
            (look.sprite, look.flip_x, look.alpha)
        } else {
            part.map_or((SpriteId::NONE, false, Fix::ONE), |p| {
                (p.sprite, p.flip_x, p.alpha)
            })
        };
        // Children with their own `PlayAnimation` loop it (the original
        // starts it when the object spawns; the phase is the clock's).
        let auto = (frame == SpriteId::NONE)
            .then_some(node.anim.as_ref())
            .flatten()
            .and_then(|(a, b)| {
                let l = if b.is_empty() { a } else { b };
                crate::fx::frame_at(l, tick, true)
            });
        if let Some((_, f, _)) = auto {
            flip ^= f;
        }
        // Throw's boulders (and the boulder dummy): the platform type's
        // boulder sprite on a renderer without one.
        let btype = if is_root {
            look.params[3]
        } else {
            part.map_or(Fix::ZERO, |p| p.params[3])
        };
        let boulder = (btype > Fix::ZERO && !is_resizable && r.sprite.is_none())
            .then(|| vis.boulder_sprite(btype.to_f32() as usize - 1))
            .flatten();
        let Some(sprite) = vis
            .sprite_name(frame)
            .map(str::to_string)
            .or_else(|| auto.map(|a| a.0.to_string()))
            .or(boulder)
            .or_else(|| r.sprite.clone())
        else {
            continue;
        };
        let mut d = r.draw(&sprite, world[k]);
        d.flip_x ^= flip;
        d.color.a *= alpha.to_f32();
        if (is_root && look.player_material)
            || part.is_some_and(|p| p.player_material)
            || r.material_name.as_deref() == Some(YELLOW_SLIME)
        {
            d.material = player_mat.clone().or(d.material);
        }
        if is_root {
            d.params = look.params.map(|p| p.to_f32());
        } else if let Some(p) = part {
            d.params = p.params.map(|p| p.to_f32());
        }
        d.colors = vis.player_colors(s.color);
        if let Some(t) = part.and_then(|p| p.tint) {
            d.color = fix_color(t);
        }
        if let Some(c) = s_color[k] {
            d.color = if pv.beam.is_some() { c } else { d.color * c };
        }
        if let Some(a) = s_alpha[k] {
            d.color.a = a;
        }
        if let Some(p) = s_params[k] {
            d.params = p;
        }
        if (is_root && look.shadow_color) || part.is_some_and(|p| p.shadow_color) {
            let c = vis.shadow_color(s.color);
            d.color = Color::rgba(c.r, c.g, c.b, d.color.a);
        }
        out.sprites.push(d);
    }
}

/// Piecewise-linear evaluation of `(time, value)` keys (UI curves).
pub fn eval_keys(keys: &[(f32, f32)], t: f32) -> f32 {
    let Some(first) = keys.first() else {
        return 0.0;
    };
    if t <= first.0 {
        return first.1;
    }
    for w in keys.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.0 {
            let k = if b.0 > a.0 {
                (t - a.0) / (b.0 - a.0)
            } else {
                1.0
            };
            // Smoothstep between keys (the curves are eased).
            let k = k * k * (3.0 - 2.0 * k);
            return a.1 + (b.1 - a.1) * k;
        }
    }
    keys[keys.len() - 1].1
}

/// Unused texture handle (keeps imports tidy when features are off).
#[allow(dead_code)]
const _NO_TEX: TextureId = TextureId::TRANSPARENT;

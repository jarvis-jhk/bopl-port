//! Presentation instances of the original's effect prefabs: what
//! `Instantiate(prefab, position, rotation)` of a presentation-only prefab
//! shows (dust, sparks, flares, explosion layers, water rings, ...).
//!
//! An [`FxDef`] is read once per prefab object from the import: its subtree's
//! sprite renderers, the presentation components that animate them
//! (`PlayAnimation` / `SimpleSpriteAnimator` frame animations from
//! `AnimationData`, Mecanim `Animator` clips: sprite swaps, scale, colour,
//! active flags and the `SelfDestruct` event, `AlphaFade`,
//! `AnimateWaveRing`, `GrowthFlare`, `portalIndicatorParticle`,
//! `beamAfterEffect`) and its particle systems. An [`Fx`] plays one at a
//! root transform, one simulation tick at a time, until everything it
//! shows is finished.
//!
//! Presentation only: floats, its own particle seeds; nothing feeds back
//! into the simulation.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use bopl_import::GameData;
use bopl_import::format::{Object, path_hash};
use bopl_render::{Color, Gpu, Renderer, TextureId, UvRect};
use bopl_sim::data::{Content, SpriteId};
use glam::{Mat4, Quat, Vec2, Vec3};
use serde_json::Value as J;

use crate::assets::Assets;
use crate::level_view::eval_curve;
use crate::match_view::{AnimFrames, RendererInfo};
use crate::particles::{self, Play, Res};
use crate::present::WorldView;

/// Graphics access for loading textures.
pub struct Gfx<'a> {
    pub gpu: &'a Gpu,
    pub renderer: &'a mut Renderer,
    pub assets: &'a mut Assets,
}

impl Gfx<'_> {
    /// Resolves a texture or sprite reference for the particle runtime.
    pub fn resolve(&mut self, r: Res<'_>) -> Option<(TextureId, UvRect)> {
        match r {
            Res::Texture(id) => self
                .assets
                .texture(self.gpu, self.renderer, id)
                .map(|t| (t.0, UvRect::FULL)),
            Res::Sprite(id) => self
                .assets
                .sprite(self.gpu, self.renderer, id)
                .map(|g| (g.texture, g.uv)),
        }
    }
}

/// A Mecanim curve: cubic segments `[time, a, b, c, d]`.
#[derive(Clone, Debug, Default)]
pub struct Curve3 {
    segments: Vec<[f32; 5]>,
}

impl Curve3 {
    pub fn eval(&self, t: f32) -> f32 {
        let Some(first) = self.segments.first() else {
            return 0.0;
        };
        if t <= first[0] {
            return first[4];
        }
        let mut seg = first;
        for s in &self.segments {
            if s[0] <= t {
                seg = s;
            }
        }
        let dt = t - seg[0];
        seg[1] * dt * dt * dt + seg[2] * dt * dt + seg[3] * dt + seg[4]
    }
}

/// What a clip curve drives.
#[derive(Clone, Debug)]
pub enum ClipTarget {
    Sprite(Vec<Option<String>>),
    Scale(usize),
    Color(usize),
    Active,
    Enabled,
}

/// A Mecanim clip on an `Animator` (default state of layer 0).
#[derive(Clone, Debug)]
pub struct ClipAnim {
    pub length: f32,
    pub looping: bool,
    pub speed: f32,
    /// (target node, what, curve).
    pub curves: Vec<(usize, ClipTarget, Curve3)>,
    /// `SelfDestruct` event time (clip seconds).
    pub destroy_at: Option<f32>,
}

/// A node's own animation.
#[derive(Clone, Debug, Default)]
pub enum NodeAnim {
    #[default]
    None,
    /// Frame animation (`PlayAnimation`, `SimpleSpriteAnimator`, the
    /// `SpriteAnimator` default): frames, then a looping follow-up.
    Frames {
        frames: AnimFrames,
        then: AnimFrames,
        looping: bool,
        destroy_on_end: bool,
        deactivate_on_end: bool,
    },
    Clip(Rc<ClipAnim>),
}

/// One object of an effect prefab.
#[derive(Clone, Debug)]
pub struct FxNode {
    /// Index in the prefab's object list.
    pub object: usize,
    pub name: String,
    /// Parent node (`None` = the root).
    pub parent: Option<usize>,
    /// Local transform (the root: its authored scale only).
    pub local: Mat4,
    pub local_scale: Vec3,
    pub active: bool,
    pub renderer: Option<RendererInfo>,
    pub anim: NodeAnim,
    /// `AlphaFade`: curve keys, destroy when done.
    pub fade: Option<(Vec<[f32; 4]>, bool)>,
    /// `AnimateWaveRing`: `_Progress` curve, self destruct.
    pub wave_ring: Option<(Vec<[f32; 4]>, bool)>,
    /// `GrowthFlare`: opacity curve, flare size.
    pub flare: Option<(Vec<[f32; 4]>, f32)>,
    /// `portalIndicatorParticle`: scale curve.
    pub scale_curve: Option<Vec<[f32; 4]>>,
    /// `beamAfterEffect`: alpha curve.
    pub alpha_curve: Option<Vec<[f32; 4]>>,
}

/// An effect prefab (subtree of one prefab object).
#[derive(Clone, Debug)]
pub struct FxDef {
    pub name: String,
    pub nodes: Vec<FxNode>,
    pub particles: Option<Rc<particles::EffectDef>>,
    /// The root's authored position (some scripts keep parts of it).
    pub root_position: Vec3,
}

impl FxDef {
    /// `spawnRayCastEffect`: the particle shapes stretched to `dist` along
    /// x, burst counts scaled to keep their density.
    pub fn stretched(&self, dist: f32) -> FxDef {
        let mut d = self.clone();
        if let Some(p) = &self.particles {
            let mut e = (**p).clone();
            for sd in &mut e.systems {
                let mut sys = (*sd.sys).clone();
                if let Some(sh) = sys.shape.as_mut() {
                    let s0 = sh.scale[0].abs().max(1e-3);
                    let k = dist / s0;
                    sh.scale[0] = dist;
                    for b in &mut sys.emission.bursts {
                        scale_curve(&mut b.count, k);
                    }
                }
                sd.sys = Arc::new(sys);
            }
            d.particles = Some(Rc::new(e));
        }
        d
    }
}

impl FxDef {
    /// The particles' colour over lifetime replaced (Roll's dash trail).
    pub fn with_color_over_lifetime(&self, keys: &[(f32, [f32; 3])]) -> FxDef {
        use bopl_import::particles as ip;
        let mut d = self.clone();
        if let Some(p) = &self.particles {
            let mut e = (**p).clone();
            for sd in &mut e.systems {
                let mut sys = (*sd.sys).clone();
                sys.color_over_lifetime = Some(ip::MinMaxGradient::Gradient {
                    gradient: ip::Gradient {
                        mode: 0,
                        colors: keys
                            .iter()
                            .map(|(t, c)| ip::ColorKey {
                                time: *t,
                                color: *c,
                            })
                            .collect(),
                        alphas: vec![
                            ip::AlphaKey {
                                time: 0.0,
                                alpha: 1.0,
                            },
                            ip::AlphaKey {
                                time: 1.0,
                                alpha: 1.0,
                            },
                        ],
                    },
                });
                sd.sys = Arc::new(sys);
            }
            d.particles = Some(Rc::new(e));
        }
        d
    }
}

impl FxDef {
    /// The particles given a constant world velocity (Teleport's particles
    /// flying between the two portals: the original pulls them with a
    /// radial velocity about the other end).
    pub fn with_velocity(&self, v: Vec2) -> FxDef {
        use bopl_import::particles as ip;
        let mut d = self.clone();
        if let Some(p) = &self.particles {
            let mut e = (**p).clone();
            for sd in &mut e.systems {
                let mut sys = (*sd.sys).clone();
                sys.velocity_over_lifetime = Some(ip::VelocityOverLifetime {
                    x: ip::MinMaxCurve::Constant { value: v.x },
                    y: ip::MinMaxCurve::Constant { value: v.y },
                    speed_modifier: ip::MinMaxCurve::Constant { value: 1.0 },
                    world_space: true,
                    ..Default::default()
                });
                sd.sys = Arc::new(sys);
            }
            d.particles = Some(Rc::new(e));
        }
        d
    }
}

impl FxDef {
    /// `BlackHole.SpawnParticle`: the swallow particles in the eaten
    /// object's shape: emitted from `points` (the sprite's opaque texels,
    /// sprite units) or a `rect` (resizable platforms: the shape scale),
    /// shape rotated by `rotation` (degrees), the first burst set to
    /// `count`, orbiting `orbit` (the hole relative to the effect, in its
    /// local units).
    pub fn eaten(
        &self,
        points: Option<Arc<Vec<Vec2>>>,
        rect: Option<Vec2>,
        rotation: f32,
        count: Option<f32>,
        orbit: Vec2,
    ) -> FxDef {
        use bopl_import::particles as ip;
        let mut d = self.clone();
        if let Some(p) = &self.particles {
            let mut e = (**p).clone();
            for sd in &mut e.systems {
                let mut sys = (*sd.sys).clone();
                if let Some(sh) = sys.shape.as_mut() {
                    sh.rotation[2] = rotation;
                    if let Some(r) = rect {
                        sh.kind = 18;
                        sh.kind_name = "rectangle".into();
                        sh.scale = [r.x, r.y, 1.0];
                    } else {
                        sd.points = points.clone();
                    }
                }
                if let (Some(c), Some(b)) = (count, sys.emission.bursts.first_mut()) {
                    b.count = ip::MinMaxCurve::Constant { value: c };
                }
                if let Some(v) = sys.velocity_over_lifetime.as_mut() {
                    v.orbital_offset[0] = ip::MinMaxCurve::Constant { value: orbit.x };
                    v.orbital_offset[1] = ip::MinMaxCurve::Constant { value: orbit.y };
                }
                sd.sys = Arc::new(sys);
            }
            d.particles = Some(Rc::new(e));
        }
        d
    }
}

fn scale_curve(c: &mut bopl_import::particles::MinMaxCurve, k: f32) {
    use bopl_import::particles::MinMaxCurve as M;
    match c {
        M::Constant { value } => *value *= k,
        M::TwoConstants { min, max } => {
            *min *= k;
            *max *= k;
        }
        M::Curve { multiplier, .. } | M::TwoCurves { multiplier, .. } => *multiplier *= k,
    }
}

pub fn curve_keys(f: &J) -> Vec<[f32; 4]> {
    f["m_Curve"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|k| {
                    [
                        k["time"].as_f64().unwrap_or(0.0) as f32,
                        k["value"].as_f64().unwrap_or(0.0) as f32,
                        k["inSlope"].as_f64().map_or(f32::INFINITY, |v| v as f32),
                        k["outSlope"].as_f64().map_or(f32::INFINITY, |v| v as f32),
                    ]
                })
                .collect()
        })
        .unwrap_or_default()
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

/// Frames of a named animation of an `AnimationData` asset.
pub fn anim_frames(content: &Content, data_ref: &str, name: &str) -> AnimFrames {
    let Some(ad) = content.anim_data.iter().find(|a| a.id == data_ref) else {
        return Vec::new();
    };
    let Some(i) = ad.find(name) else {
        return Vec::new();
    };
    ad.animations[i]
        .frames
        .iter()
        .filter(|f| f.sprite != SpriteId::NONE)
        .map(|f| {
            (
                content.sprites[f.sprite.0 as usize].clone(),
                f.duration.abs().max(1),
                f.duration < 0,
            )
        })
        .collect()
}

/// Sprite of a frame list at `age` ticks (looping or holding the last).
pub fn frame_at(frames: &AnimFrames, age: i64, looping: bool) -> Option<(&str, bool, bool)> {
    let total: i64 = frames.iter().map(|f| f.1 as i64).sum();
    if total <= 0 {
        return None;
    }
    let ended = age >= total;
    let t = if looping { age.rem_euclid(total) } else { age };
    let mut acc = 0i64;
    for f in frames {
        acc += f.1 as i64;
        if t < acc {
            return Some((&f.0, f.2, ended));
        }
    }
    frames.last().map(|f| (f.0.as_str(), f.2, ended))
}

/// Effect definitions by (prefab id, object index), read on first use.
pub struct FxLib {
    data: Rc<GameData>,
    content: Arc<Content>,
    defs: HashMap<(String, usize), Rc<FxDef>>,
}

impl FxLib {
    pub fn new(data: Rc<GameData>, content: Arc<Content>) -> Self {
        Self {
            data,
            content,
            defs: HashMap::new(),
        }
    }

    /// The effect of prefab object `root` (`id` = any id inside the
    /// prefab, resolved through the prefab index).
    pub fn def(&mut self, id: &str, root: usize, gfx: &mut Gfx<'_>) -> Option<Rc<FxDef>> {
        let p = self.data.prefab(id).ok()?;
        let key = (p.id.clone(), root);
        if let Some(d) = self.defs.get(&key) {
            return Some(d.clone());
        }
        let d = Rc::new(self.build(&p.name, &p.objects, root, gfx));
        self.defs.insert(key, d.clone());
        Some(d)
    }

    /// The effect of a scene object's subtree (level particle systems),
    /// cached under `key`.
    pub fn def_scene(
        &mut self,
        key: &str,
        objs: &[Object],
        root: usize,
        gfx: &mut Gfx<'_>,
    ) -> Rc<FxDef> {
        let k = (key.to_string(), root);
        if let Some(d) = self.defs.get(&k) {
            return d.clone();
        }
        let d = Rc::new(self.build(key, objs, root, gfx));
        self.defs.insert(k, d.clone());
        d
    }

    /// The effect a `$ref` (object, component or prefab id) points to.
    pub fn def_of_ref(&mut self, r: &str, gfx: &mut Gfx<'_>) -> Option<Rc<FxDef>> {
        let p = self.data.prefab(r).ok()?;
        // A reference to a component or object inside the prefab: that
        // object's subtree.
        let root = p
            .objects
            .iter()
            .position(|o| {
                o.id == r
                    || o.transform.as_deref() == Some(r)
                    || o.components.iter().any(|c| c.id == r)
            })
            .unwrap_or(0);
        self.def(r, root, gfx)
    }

    fn build(&self, name: &str, objs: &[Object], root: usize, gfx: &mut Gfx<'_>) -> FxDef {
        let mut nodes: Vec<FxNode> = Vec::new();
        let mut index: HashMap<usize, usize> = HashMap::new();
        let mut stack = vec![(root, None::<usize>)];
        while let Some((i, parent)) = stack.pop() {
            let o = &objs[i];
            let local = if parent.is_none() {
                Mat4::from_scale(Vec3::from(o.local.scale))
            } else {
                local_trs(o)
            };
            let mut n = FxNode {
                object: i,
                name: o.name.clone(),
                parent,
                local,
                local_scale: Vec3::from(o.local.scale),
                active: o.active || parent.is_none(),
                renderer: RendererInfo::of(o),
                anim: NodeAnim::None,
                fade: None,
                wave_ring: None,
                flare: None,
                scale_curve: None,
                alpha_curve: None,
            };
            for c in &o.components {
                let f = &c.fields;
                match c.script.as_deref().unwrap_or(c.kind.as_str()) {
                    "PlayAnimation" => {
                        let data = f["data"]["$ref"].as_str().unwrap_or("");
                        let play = f["animationToPlay"].as_str().unwrap_or("");
                        let then = f["animationOnEnd"].as_str().unwrap_or("");
                        n.anim = NodeAnim::Frames {
                            frames: anim_frames(&self.content, data, play),
                            then: anim_frames(&self.content, data, then),
                            looping: false,
                            destroy_on_end: f["SelfDestructOnAnimationEnd"].as_i64() == Some(1),
                            deactivate_on_end: f["SelfDeactivateOnAnimationEnd"].as_i64()
                                == Some(1),
                        };
                    }
                    "SimpleSpriteAnimator" if f["autoPlay"].as_i64() == Some(1) => {
                        let data = f["animData"]["$ref"].as_str().unwrap_or("");
                        let play = f["animationToPlay"].as_str().unwrap_or("");
                        n.anim = NodeAnim::Frames {
                            frames: anim_frames(&self.content, data, play),
                            then: Vec::new(),
                            looping: false,
                            destroy_on_end: false,
                            deactivate_on_end: false,
                        };
                    }
                    "Animator" if c.enabled != Some(false) => {
                        if let Some(clip) = self.clip_of(f, objs, i) {
                            n.anim = NodeAnim::Clip(Rc::new(clip));
                        }
                    }
                    "AlphaFade" if c.enabled != Some(false) => {
                        n.fade = Some((
                            curve_keys(&f["fadeAnimation"]),
                            f["destroyWhenDone"].as_i64() == Some(1),
                        ));
                    }
                    "AnimateWaveRing" => {
                        n.wave_ring = Some((
                            curve_keys(&f["animation"]),
                            f["selfDestruct"].as_i64() == Some(1),
                        ));
                    }
                    "GrowthFlare" => {
                        n.flare = Some((
                            curve_keys(&f["opacityAnim01"]),
                            f["flareSize"].as_f64().unwrap_or(0.075) as f32,
                        ));
                    }
                    "portalIndicatorParticle" => {
                        n.scale_curve = Some(curve_keys(&f["animCurve"]));
                    }
                    "beamAfterEffect" => {
                        n.alpha_curve = Some(curve_keys(&f["anim"]));
                    }
                    _ => {}
                }
            }
            let k = nodes.len();
            index.insert(i, k);
            nodes.push(n);
            for &c in o.children.iter().rev() {
                stack.push((c, Some(k)));
            }
        }
        // Clip targets by object index → node index.
        for n in &mut nodes {
            if let NodeAnim::Clip(c) = &mut n.anim {
                let mut c2 = (**c).clone();
                c2.curves.retain_mut(|(t, _, _)| {
                    if let Some(&k) = index.get(t) {
                        *t = k;
                        true
                    } else {
                        false
                    }
                });
                *c = Rc::new(c2);
            }
        }
        let has_particles = nodes.iter().any(|n| objs[n.object].particles.is_some());
        let particles = has_particles.then(|| {
            Rc::new(particles::EffectDef::from_objects(
                name,
                objs,
                root,
                &self.data,
                &mut |r| gfx.resolve(r),
            ))
        });
        FxDef {
            name: name.to_string(),
            nodes,
            particles,
            root_position: Vec3::from(objs[root].local.position),
        }
    }

    /// The default clip of an `Animator` (curves target object indices).
    fn clip_of(&self, f: &J, objs: &[Object], me: usize) -> Option<ClipAnim> {
        let ctrl = self
            .data
            .animations
            .controllers
            .get(f["m_Controller"]["$ref"].as_str()?)?;
        let layer = ctrl.layers.first()?;
        let state = layer.states.get(layer.default_state)?;
        let clip = self.data.animations.clips.get(state.clip.as_deref()?)?;
        // Paths below the animator's object.
        let mut paths: Vec<(u32, usize)> = vec![(0, me)];
        let mut stack: Vec<(usize, String)> = objs[me]
            .children
            .iter()
            .map(|&c| (c, objs[c].name.clone()))
            .collect();
        while let Some((i, p)) = stack.pop() {
            paths.push((path_hash(&p), i));
            for &c in &objs[i].children {
                stack.push((c, format!("{p}/{}", objs[c].name)));
            }
        }
        let mut curves = Vec::new();
        for cv in &clip.curves {
            let Some(&(_, target)) = paths.iter().find(|(h, _)| *h == cv.path) else {
                continue;
            };
            let what = match (cv.class_id, cv.property.as_str()) {
                (212, "#0") if cv.pptr => ClipTarget::Sprite(clip.pptr.clone()),
                (4, "scale.x") => ClipTarget::Scale(0),
                (4, "scale.y") => ClipTarget::Scale(1),
                (4, "scale.z") => ClipTarget::Scale(2),
                (212, "m_Color.r") => ClipTarget::Color(0),
                (212, "m_Color.g") => ClipTarget::Color(1),
                (212, "m_Color.b") => ClipTarget::Color(2),
                (212, "m_Color.a") => ClipTarget::Color(3),
                (1, "m_IsActive") => ClipTarget::Active,
                (212, "m_Enabled") => ClipTarget::Enabled,
                _ => continue,
            };
            curves.push((
                target,
                what,
                Curve3 {
                    segments: cv.segments.clone(),
                },
            ));
        }
        let destroy_at = clip
            .events
            .as_array()
            .and_then(|a| {
                a.iter()
                    .find(|e| e["functionName"].as_str() == Some("SelfDestruct"))
            })
            .and_then(|e| e["time"].as_f64())
            .map(|t| t as f32);
        Some(ClipAnim {
            length: (clip.stop - clip.start).max(1e-3),
            looping: state.looping || clip.looping,
            speed: if state.speed == 0.0 { 1.0 } else { state.speed },
            curves,
            destroy_at,
        })
    }
}

/// Per-instance overrides.
#[derive(Clone, Debug, Default)]
pub struct FxStyle {
    /// Material for renderers using the default player material
    /// (`yellowSlime`) or every renderer when `force_material`.
    pub material: Option<String>,
    pub force_material: bool,
    /// Multiplies every renderer colour.
    pub tint: Option<Color>,
    /// Replaces every renderer's rgb.
    pub color: Option<Color>,
    /// Renderer colours by object name (replace, e.g. beam hit layers).
    pub node_colors: Vec<(String, Color)>,
    /// Shader values for every renderer (`SpriteDraw::params`).
    pub params: Option<[f32; 4]>,
    /// Replaces the particles' start colour.
    pub particle_color: Option<Color>,
    /// Only for this particle system (component id; all when `None`).
    pub particle_color_component: Option<String>,
    /// Sorting order offset.
    pub order: i32,
    /// `portalIndicatorParticle.SetScale` (radius multiplier, default 1).
    pub shape_scale: Option<f32>,
}

/// A playing effect.
pub struct Fx {
    pub def: Rc<FxDef>,
    pub root: Mat4,
    pub style: FxStyle,
    /// Ticks since it started.
    pub age: i64,
    particles: Option<particles::Effect>,
    destroyed: bool,
    /// Particles stop emitting (e.g. their object is gone).
    pub stopped: bool,
    /// Only the particle systems are drawn (the object draws its sprites).
    pub particles_only: bool,
    /// Frame animations loop (persistent effects like beam hits).
    pub looping: bool,
}

/// The default player material (the prefabs' slime renderers use it).
pub const YELLOW_SLIME: &str = "yellowSlime";

impl Fx {
    pub fn new(def: Rc<FxDef>, root: Mat4, seed: u64, style: FxStyle) -> Fx {
        Self::with_play(def, root, seed, style, Play::Awake)
    }

    /// An effect whose particle systems are `Play()`ed (all active ones).
    pub fn played(def: Rc<FxDef>, root: Mat4, seed: u64, style: FxStyle) -> Fx {
        Self::with_play(def, root, seed, style, Play::All)
    }

    fn with_play(def: Rc<FxDef>, root: Mat4, seed: u64, style: FxStyle, play: Play) -> Fx {
        let mut particles = def
            .particles
            .clone()
            .map(|p| particles::Effect::new(p, root, seed, play));
        if let (Some(p), Some(c)) = (particles.as_mut(), style.particle_color) {
            let only = style.particle_color_component.as_deref();
            p.set_start_color(|sd| only.is_none_or(|id| sd.sys.component == id), c);
        }
        Fx {
            def,
            root,
            style,
            age: 0,
            particles,
            destroyed: false,
            stopped: false,
            particles_only: false,
            looping: false,
        }
    }

    /// Moves the effect (attached effects follow their object).
    pub fn set_root(&mut self, root: Mat4) {
        self.root = root;
        if let Some(p) = self.particles.as_mut() {
            p.root = root;
        }
    }

    /// Emission of the particle systems on prefab object `object`.
    pub fn set_emitting(&mut self, object: usize, on: bool) {
        if let Some(p) = self.particles.as_mut() {
            p.set_emitting(object, on);
        }
    }

    /// `Play()` / `Stop()` of all its particle systems (toggles).
    pub fn set_all_emitting(&mut self, on: bool) {
        if let Some(p) = self.particles.as_mut() {
            p.set_all_emitting(on);
        }
    }

    pub fn stop(&mut self) {
        self.stopped = true;
        if let Some(p) = self.particles.as_mut() {
            p.stop();
        }
    }

    /// One simulation tick.
    pub fn step(&mut self) {
        self.age += 1;
        let dt = 1.0 / 60.0;
        if let Some(p) = self.particles.as_mut() {
            // `portalIndicatorParticle`: shape radius = curve(time / duration).
            for n in &self.def.nodes {
                if let Some(keys) = &n.scale_curve {
                    p.set_shape_radius(n.object, |t| {
                        eval_curve(keys, t) * self.style.shape_scale.unwrap_or(1.0)
                    });
                }
            }
            p.update(dt, dt);
        }
        let t = self.age as f32 * dt;
        for n in &self.def.nodes {
            match &n.anim {
                NodeAnim::Frames {
                    frames,
                    destroy_on_end: true,
                    ..
                } => {
                    let total: i64 = frames.iter().map(|f| f.1 as i64).sum();
                    if self.age > total {
                        self.destroyed = true;
                    }
                }
                NodeAnim::Clip(c) => {
                    if let Some(d) = c.destroy_at
                        && t * c.speed >= d
                    {
                        self.destroyed = true;
                    }
                }
                _ => {}
            }
            if let Some((keys, true)) = &n.fade
                && t >= keys.last().map_or(0.0, |k| k[0])
            {
                self.destroyed = true;
            }
            if let Some((keys, true)) = &n.wave_ring
                && t >= keys.last().map_or(0.0, |k| k[0])
            {
                self.destroyed = true;
            }
        }
    }

    pub fn particle_count(&self) -> usize {
        self.particles.as_ref().map_or(0, |p| p.particle_count())
    }

    /// Something is still shown.
    pub fn alive(&self) -> bool {
        if self.destroyed {
            return self
                .particles
                .as_ref()
                .is_some_and(|p| p.particle_count() > 0);
        }
        if self.particles.as_ref().is_some_and(|p| p.alive()) {
            return true;
        }
        // Renderers: shown while an animation runs (or for a while).
        let t = self.age as f32 / 60.0;
        let animated = self.def.nodes.iter().any(|n| {
            n.renderer.is_some()
                && match &n.anim {
                    NodeAnim::Frames { looping: true, .. } => true,
                    NodeAnim::Frames { frames, then, .. } => {
                        !then.is_empty()
                            || self.age <= frames.iter().map(|f| f.1 as i64).sum::<i64>()
                    }
                    NodeAnim::Clip(c) => c.looping || t * c.speed <= c.length,
                    NodeAnim::None => {
                        n.fade.is_some() || n.flare.is_some() || n.alpha_curve.is_some()
                    }
                }
        });
        animated && t < 20.0
    }

    /// Adds the effect's sprites and particles to the view.
    pub fn draw(&self, out: &mut WorldView) {
        if let Some(p) = &self.particles {
            p.quads(&mut out.quads);
        }
        if self.destroyed || self.particles_only || (self.looping && self.stopped) {
            return;
        }
        let t = self.age as f32 / 60.0;
        let n = self.def.nodes.len();
        let mut world = vec![Mat4::IDENTITY; n];
        let mut active = vec![true; n];
        let mut scale_ovr: Vec<Vec3> = self.def.nodes.iter().map(|n| n.local_scale).collect();
        let mut color_ovr: Vec<[Option<f32>; 4]> = vec![[None; 4]; n];
        let mut sprite_ovr: Vec<Option<(String, bool)>> = vec![None; n];
        let mut active_ovr: Vec<Option<bool>> = vec![None; n];
        let mut enabled_ovr: Vec<Option<bool>> = vec![None; n];
        for (k, node) in self.def.nodes.iter().enumerate() {
            match &node.anim {
                NodeAnim::Clip(c) => {
                    let mut ct = t * c.speed;
                    if c.looping {
                        ct = ct.rem_euclid(c.length);
                    }
                    for (target, what, curve) in &c.curves {
                        let v = curve.eval(ct);
                        match what {
                            ClipTarget::Sprite(list) => {
                                let i = v.floor().max(0.0) as usize;
                                if let Some(Some(s)) = list.get(i) {
                                    sprite_ovr[*target] = Some((s.clone(), false));
                                }
                            }
                            ClipTarget::Scale(a) => scale_ovr[*target][*a] = v,
                            ClipTarget::Color(a) => color_ovr[*target][*a] = Some(v),
                            ClipTarget::Active => active_ovr[*target] = Some(v > 0.5),
                            ClipTarget::Enabled => enabled_ovr[*target] = Some(v > 0.5),
                        }
                    }
                }
                NodeAnim::Frames {
                    frames,
                    then,
                    looping,
                    deactivate_on_end,
                    ..
                } => {
                    let looping = &(*looping || self.looping);
                    let total: i64 = frames.iter().map(|f| f.1 as i64).sum();
                    let s = if self.age >= total && !then.is_empty() {
                        frame_at(then, self.age - total, true)
                    } else {
                        frame_at(frames, self.age, *looping)
                    };
                    if let Some((sp, flip, ended)) = s {
                        if ended && *deactivate_on_end && then.is_empty() {
                            active_ovr[k] = Some(false);
                        }
                        sprite_ovr[k] = Some((sp.to_string(), flip));
                    }
                }
                NodeAnim::None => {}
            }
            // `GrowthFlare`: sized to its target's sprite (a bopl's, about
            // 3.9 × 3.4 units) × `flareSize`, centred on it.
            if let Some((_, size)) = &node.flare {
                scale_ovr[k] = Vec3::new(3.9 * size, 3.4 * size, 1.0);
            }
        }
        for (k, node) in self.def.nodes.iter().enumerate() {
            let (_, rot, mut pos) = node.local.to_scale_rotation_translation();
            if node.flare.is_some() && node.parent.is_none() {
                pos.y += 1.4;
            }
            let local = Mat4::from_scale_rotation_translation(scale_ovr[k], rot, pos);
            let parent_m = node.parent.map_or(self.root, |p| world[p]);
            world[k] = parent_m * local;
            let parent_active = node.parent.is_none_or(|p| active[p]);
            active[k] = parent_active && active_ovr[k].unwrap_or(node.active);
        }
        for (k, node) in self.def.nodes.iter().enumerate() {
            if !active[k] {
                continue;
            }
            let Some(r) = &node.renderer else { continue };
            if !enabled_ovr[k].unwrap_or(r.enabled) {
                continue;
            }
            let Some((sprite, flip)) = sprite_ovr[k]
                .clone()
                .or_else(|| r.sprite.clone().map(|s| (s, false)))
            else {
                continue;
            };
            let mut d = r.draw(&sprite, world[k]);
            d.flip_x ^= flip;
            let mut c = d.color;
            for (a, v) in color_ovr[k].iter().enumerate() {
                if let Some(v) = v {
                    match a {
                        0 => c.r = *v,
                        1 => c.g = *v,
                        2 => c.b = *v,
                        _ => c.a = *v,
                    }
                }
            }
            if let Some((keys, _)) = &node.fade {
                c.a *= eval_curve(keys, t);
            }
            if let Some(keys) = &node.alpha_curve {
                c.a *= eval_curve(keys, t);
            }
            if let Some((keys, size)) = &node.flare {
                c.a *= eval_curve(keys, t);
                let _ = size;
            }
            if let Some(tint) = self.style.tint {
                c = c * tint;
            }
            if let Some(rc) = self.style.color {
                c = Color::rgba(rc.r, rc.g, rc.b, c.a);
            }
            let name = &self.def.nodes[k].name;
            if let Some((_, nc)) = self.style.node_colors.iter().find(|(n, _)| n == name) {
                c = Color::rgba(nc.r, nc.g, nc.b, nc.a * c.a);
            }
            d.color = c;
            if let Some((keys, _)) = &node.wave_ring {
                d.params = [eval_curve(keys, t), 0.0, 0.0, 1.0];
            }
            if let Some(p) = self.style.params {
                d.params = p;
            }
            if let Some(m) = &self.style.material
                && (self.style.force_material || r.material_name.as_deref() == Some(YELLOW_SLIME))
            {
                d.material = Some(m.clone());
            }
            d.sorting_order += self.style.order;
            out.sprites.push(d);
        }
    }
}

/// A root transform from a position, rotation (radians) and scale.
/// A negative scale mirrors along x only.
pub fn root_at(pos: Vec2, rotation: f32, scale: f32) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        Vec3::new(scale, scale.abs(), 1.0),
        Quat::from_rotation_z(rotation),
        pos.extend(0.0),
    )
}

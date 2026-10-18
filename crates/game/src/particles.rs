//! Particle effects from the import's typed particle systems
//! (`Object.particles`, docs/formats/import.md): a Unity-like runtime for the
//! subset the original's ability prefabs and levels use (adapted from
//! rounds-port's `psys.rs`, ours: gamma space, materials applied by the
//! level view, sprite-mode texture sheets).
//!
//! An [`EffectDef`] holds every particle system of one object hierarchy (a
//! prefab, or a subtree of a map / the player prefab) with its transform
//! relative to the hierarchy's root and its look (texture, blend, sorting).
//! An [`Effect`] plays it at a root transform: emission over time and
//! distance, bursts with cycles, start delay, duration and looping,
//! prewarm, the emitter shapes (sphere, hemisphere, cone, circle, box,
//! edge, donut, rectangle; others approximated by a box), local or world
//! simulation space, 3D gravity, velocity / limit-velocity / force /
//! colour / size / rotation over lifetime, simple noise, texture-sheet
//! animation, billboard and stretched rendering, and death sub-emitters.
//! Not done: trails, collisions, mesh shapes, lights.
//!
//! Presentation only: own seeded RNG, never the simulation's state.

use std::rc::Rc;
use std::sync::Arc;

use bopl_import::GameData;
use bopl_import::format::Object;
use bopl_import::particles as ip;
use bopl_render::{Color, Quad, TextureId, UvRect};
use glam::{EulerRot, Mat4, Quat, Vec2, Vec3};

use crate::present::QuadDraw;

/// How a system is drawn.
#[derive(Clone, Debug)]
pub struct Look {
    pub texture: TextureId,
    /// Sprite-mode texture sheet: the frames' textures and uv rects.
    pub sheet: Vec<(TextureId, UvRect)>,
    /// Material id (its shader shades the quads).
    pub material: Option<String>,
    pub mask_interaction: i32,
    pub sort_mode: i32,
    /// 0 billboard, 1 stretched, others drawn as billboards.
    pub render_mode: i32,
    pub velocity_scale: f32,
    pub length_scale: f32,
    /// Unity render alignment (0 view, 4 velocity, ...).
    pub alignment: i32,
    /// Sorting layer index and order in layer.
    pub layer: usize,
    pub order: i32,
    /// Unity layer of the system's object (camera culling).
    pub unity_layer: u32,
}

/// What a texture or sprite reference resolves to.
pub enum Res<'a> {
    Texture(&'a str),
    Sprite(&'a str),
}

/// One particle system of an effect.
#[derive(Clone, Debug)]
pub struct SystemDef {
    /// Index of the object in the source list.
    pub object: usize,
    pub sys: Arc<ip::ParticleSystem>,
    /// Transform relative to the effect's root.
    pub rel: Mat4,
    /// Active in the source hierarchy.
    pub active: bool,
    pub look: Look,
    /// Death sub-emitters (indices into the effect's systems).
    pub sub_death: Vec<usize>,
    /// Emission points (shape space) replacing the shape (a sprite shape
    /// given a sprite's opaque texels: the black hole's eat particles).
    pub points: Option<Arc<Vec<Vec2>>>,
}

impl SystemDef {}

/// Every particle system of an object hierarchy.
#[derive(Clone, Debug, Default)]
pub struct EffectDef {
    /// Root object name (debug output).
    #[allow(dead_code)]
    pub name: String,
    pub systems: Vec<SystemDef>,
}

fn trs(o: &Object) -> Mat4 {
    let w = &o.world.trs;
    // A zero scale (flat 2D objects authored with z = 0) would make the
    // matrix singular.
    let s = Vec3::from(w.scale);
    let s = Vec3::new(s.x, s.y, if s.z == 0.0 { 1.0 } else { s.z });
    Mat4::from_scale_rotation_translation(
        s,
        Quat::from_array(w.rotation).normalize(),
        Vec3::from(w.position),
    )
}

fn descends(objs: &[Object], mut i: usize, root: usize) -> bool {
    loop {
        if i == root {
            return true;
        }
        match objs[i].parent {
            Some(p) => i = p,
            None => return false,
        }
    }
}

impl EffectDef {
    /// The systems of `objs[root]` and its descendants. `texture` loads a
    /// texture by import id (returns its renderer id).
    pub fn from_objects(
        name: &str,
        objs: &[Object],
        root: usize,
        data: &GameData,
        texture: &mut dyn FnMut(Res<'_>) -> Option<(TextureId, UvRect)>,
    ) -> EffectDef {
        Self::build(name, objs, root, data, texture, false)
    }

    /// The world transform of `objs[i]` (for placing an effect built from
    /// a scene's object list where it was authored).
    pub fn world_of(objs: &[Object], i: usize) -> Mat4 {
        trs(&objs[i])
    }

    fn build(
        name: &str,
        objs: &[Object],
        root: usize,
        data: &GameData,
        texture: &mut dyn FnMut(Res<'_>) -> Option<(TextureId, UvRect)>,
        only_root: bool,
    ) -> EffectDef {
        let inv = trs(&objs[root]).inverse();
        let mut systems = Vec::new();
        let mut by_component: Vec<(String, usize)> = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            let Some(ps) = &o.particles else { continue };
            if (only_root && i != root) || !descends(objs, i, root) {
                continue;
            }
            // A disabled renderer draws nothing (as in Unity).
            let Some(r) = ps.renderer.as_ref().filter(|r| r.enabled) else {
                continue;
            };
            let _ = data;
            let tex = r
                .texture
                .as_deref()
                .and_then(|t| texture(Res::Texture(t)))
                .map_or(TextureId::WHITE, |t| t.0);
            let sheet: Vec<(TextureId, UvRect)> = ps
                .texture_sheet
                .as_ref()
                .filter(|ts| ts.mode == 1)
                .map(|ts| {
                    ts.sprites
                        .iter()
                        .flatten()
                        .filter_map(|sp| texture(Res::Sprite(sp)))
                        .collect()
                })
                .unwrap_or_default();
            by_component.push((ps.component.clone(), systems.len()));
            systems.push(SystemDef {
                object: i,
                sys: Arc::new((**ps).clone()),
                rel: inv * trs(o),
                active: o.active_in_hierarchy,
                look: Look {
                    texture: tex,
                    sheet,
                    material: r.material.clone(),
                    mask_interaction: r.mask_interaction,
                    sort_mode: r.sort_mode,
                    render_mode: r.render_mode,
                    velocity_scale: r.velocity_scale,
                    length_scale: r.length_scale,
                    alignment: r.render_alignment,
                    layer: r.sorting_layer_index,
                    order: r.sorting_order,
                    unity_layer: o.layer,
                },
                sub_death: Vec::new(),
                points: None,
            });
        }
        // Death sub-emitters by component id.
        for sd in &mut systems {
            let subs: Vec<usize> = sd
                .sys
                .sub_emitters
                .iter()
                .filter(|s| s.kind == 2)
                .filter_map(|s| {
                    let e = s.emitter.as_ref()?;
                    by_component.iter().find(|(c, _)| c == e).map(|x| x.1)
                })
                .collect();
            sd.sub_death = subs;
        }
        EffectDef {
            name: name.to_owned(),
            systems,
        }
    }

    /// Systems that are sub-emitters of another (not played on their own).
    fn is_sub(&self, i: usize) -> bool {
        self.systems.iter().any(|s| s.sub_death.contains(&i))
    }
}

/// Which systems an [`Effect`] starts.
#[derive(Clone, Copy, Debug)]
pub enum Play {
    /// Active systems with `play_on_awake` (an instantiated prefab).
    Awake,
    /// Every active system (the hierarchy's root was `Play()`ed).
    All,
}

#[derive(Clone, Debug)]
struct Particle {
    /// World space, or emitter space for local simulation.
    pos: Vec3,
    vel: Vec3,
    /// Last effective velocity (orbital and module velocities included).
    dir: Vec3,
    age: f32,
    life: f32,
    size: Vec3,
    rot: f32,
    color: [f32; 4],
    r: [f32; 4],
    born: u64,
}

#[derive(Clone, Debug)]
struct Emitter {
    sys: usize,
    particles: Vec<Particle>,
    time: f32,
    delay: f32,
    emit_acc: f32,
    dist_acc: f32,
    last_pos: Option<Vec3>,
    stopped: bool,
    /// Plays one cycle then dies (sub-emitter spawned at a point).
    origin: Option<Mat4>,
    start_color: Option<[f32; 4]>,
    /// Random between two colours (player skin fill).
    start_range: Option<([f32; 4], [f32; 4])>,
    /// Shape radius override (scripts that animate `shape.radius`).
    radius: Option<f32>,
    born: u64,
}

/// Small xorshift64* RNG (presentation only).
#[derive(Clone, Debug)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn f(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }
    fn unit3(&mut self) -> Vec3 {
        let z = self.f() * 2.0 - 1.0;
        let a = self.f() * std::f32::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * a.cos(), r * a.sin(), z)
    }
}

/// A playing effect.
#[derive(Clone, Debug)]
pub struct Effect {
    pub def: Rc<EffectDef>,
    /// World transform of the root.
    pub root: Mat4,
    /// Per-system transform overrides (relative to the root).
    rel: Vec<Mat4>,
    emitters: Vec<Emitter>,
    rng: Rng,
    /// 3D gravity (y) for `gravity_source` 0 and the 2D one for 1.
    gravity: (f32, f32),
    /// Speed multiplier (e.g. `ParticleTime`, slow motion handled by the
    /// caller's dt).
    pub speed: f32,
}

/// Fixed simulation sub-step.
const STEP: f32 = 1.0 / 60.0;

fn hash3(x: i32, y: i32, z: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 32767.5 - 1.0
}

fn value_noise(p: Vec3) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec3::splat(3.0) - 2.0 * f);
    let (x, y, z) = (i.x as i32, i.y as i32, i.z as i32);
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let c = |dx, dy, dz| hash3(x + dx, y + dy, z + dz);
    l(
        l(
            l(c(0, 0, 0), c(1, 0, 0), u.x),
            l(c(0, 1, 0), c(1, 1, 0), u.x),
            u.y,
        ),
        l(
            l(c(0, 0, 1), c(1, 0, 1), u.x),
            l(c(0, 1, 1), c(1, 1, 1), u.x),
            u.y,
        ),
        u.z,
    )
}

fn shape_rotation(r: [f32; 3]) -> Quat {
    // Unity's Quaternion.Euler: z, then x, then y.
    Quat::from_euler(
        EulerRot::YXZ,
        r[1].to_radians(),
        r[0].to_radians(),
        r[2].to_radians(),
    )
}

/// Uniform scale magnitude of a matrix (mean of the x/y axis lengths).
fn scale_of(m: &Mat4) -> f32 {
    0.5 * (m.x_axis.truncate().length() + m.y_axis.truncate().length())
}

impl Effect {
    /// Starts `def` at `root` (world transform of the hierarchy's root).
    pub fn new(def: Rc<EffectDef>, root: Mat4, seed: u64, play: Play) -> Effect {
        let rel = def.systems.iter().map(|s| s.rel).collect();
        let mut e = Effect {
            def: def.clone(),
            root,
            rel,
            emitters: Vec::new(),
            rng: Rng::new(seed),
            gravity: (-9.81, -9.81),
            speed: 1.0,
        };
        for (i, s) in def.systems.iter().enumerate() {
            if !s.active || def.is_sub(i) {
                continue;
            }
            let on = match play {
                Play::Awake => s.sys.main.play_on_awake,
                Play::All => true,
            };
            if on {
                e.start(i, None);
            }
        }
        e
    }

    fn start(&mut self, i: usize, origin: Option<Mat4>) {
        let m = &self.def.systems[i].sys.main;
        let delay = m.start_delay.eval(0.0, self.rng.f()).max(0.0);
        let mut em = Emitter {
            sys: i,
            particles: Vec::new(),
            time: 0.0,
            delay,
            emit_acc: 0.0,
            dist_acc: 0.0,
            last_pos: None,
            stopped: false,
            origin,
            start_color: None,
            start_range: None,
            radius: None,
            born: 0,
        };
        if m.prewarm && m.looping && origin.is_none() {
            let n = (m.duration.max(0.01) / STEP).ceil().min(600.0) as usize;
            em.delay = 0.0;
            self.emitters.push(em);
            let k = self.emitters.len() - 1;
            for _ in 0..n {
                self.step_emitter(k, STEP);
            }
            return;
        }
        self.emitters.push(em);
    }

    /// Replaces the start colour of systems matching `pick` (SetTeamColor,
    /// DeathEffect).
    pub fn set_start_color(&mut self, pick: impl Fn(&SystemDef) -> bool, c: Color) {
        for em in &mut self.emitters {
            if pick(&self.def.systems[em.sys]) {
                em.start_color = Some([c.r, c.g, c.b, c.a]);
            }
        }
    }

    /// Starts or stops the emission of the systems on prefab object
    /// `object` (a script's `Play()` / `Stop()`).
    pub fn set_emitting(&mut self, object: usize, on: bool) {
        let mut found = false;
        for em in &mut self.emitters {
            if self.def.systems[em.sys].object == object && em.origin.is_none() {
                em.stopped = !on;
                found = true;
            }
        }
        // `Play()` of a system that did not play on awake.
        if on && !found {
            let def = self.def.clone();
            for (i, sd) in def.systems.iter().enumerate() {
                if sd.object == object && !def.is_sub(i) {
                    self.start(i, None);
                }
            }
        }
    }

    /// `Play()` / `Stop()` of every system (one-cycle sub-emitters aside).
    pub fn set_all_emitting(&mut self, on: bool) {
        for em in &mut self.emitters {
            if em.origin.is_none() {
                em.stopped = !on;
            }
        }
        // `Play()` of systems that did not play on awake.
        if on {
            let def = self.def.clone();
            for (i, sd) in def.systems.iter().enumerate() {
                if sd.active
                    && !def.is_sub(i)
                    && !self
                        .emitters
                        .iter()
                        .any(|e| e.sys == i && e.origin.is_none())
                {
                    self.start(i, None);
                }
            }
        }
    }

    /// Sets the shape radius of the systems on prefab object `object` from
    /// their normalised system time (`time / duration`).
    pub fn set_shape_radius(&mut self, object: usize, f: impl Fn(f32) -> f32) {
        for em in &mut self.emitters {
            let sd = &self.def.systems[em.sys];
            if sd.object == object && em.origin.is_none() {
                let d = sd.sys.main.duration.max(0.01);
                em.radius = Some(f(em.time / d));
            }
        }
    }

    /// Stops emitting (particles finish their lives).
    pub fn stop(&mut self) {
        for em in &mut self.emitters {
            em.stopped = true;
        }
    }

    /// Something is still emitting or alive.
    pub fn alive(&self) -> bool {
        self.emitters
            .iter()
            .any(|e| !e.particles.is_empty() || !e.stopped)
    }

    pub fn particle_count(&self) -> usize {
        self.emitters.iter().map(|e| e.particles.len()).sum()
    }

    fn emitter_world(&self, em: &Emitter) -> Mat4 {
        em.origin.unwrap_or(self.root * self.rel[em.sys])
    }

    /// Advances by `real` / `game` seconds (systems with
    /// `use_unscaled_time` take the real time).
    pub fn update(&mut self, real: f32, game: f32) {
        let mut k = 0;
        while k < self.emitters.len() {
            let sys = self.emitters[k].sys;
            let m = &self.def.systems[sys].sys.main;
            let dt = if m.use_unscaled_time { real } else { game };
            let mut left = dt.max(0.0) * m.simulation_speed.max(0.0) * self.speed;
            // Long hitches: cap the catch-up work.
            left = left.min(0.5);
            while left > 1e-6 {
                let h = left.min(STEP);
                self.step_emitter(k, h);
                left -= h;
            }
            k += 1;
        }
        self.emitters
            .retain(|e| !(e.origin.is_some() && e.stopped && e.particles.is_empty()));
    }

    fn emit(&mut self, k: usize, t_norm: f32, world: &Mat4, at: Option<Vec3>) {
        let def = self.def.clone();
        let sd = &def.systems[self.emitters[k].sys];
        let s = &sd.sys;
        let m = &s.main;
        if self.emitters[k].particles.len() >= m.max_particles.clamp(0, 20000) as usize {
            return;
        }
        let rng = &mut self.rng;
        let mut r = [0.0f32; 8];
        for x in &mut r {
            *x = rng.f();
        }
        let life = m.start_lifetime.eval(t_norm, r[0]).max(1e-3);
        let sx = m.start_size.eval(t_norm, r[1]);
        let size = if m.size_3d {
            Vec3::new(
                sx,
                m.start_size_y.eval(t_norm, r[1]),
                m.start_size_z.eval(t_norm, r[1]),
            )
        } else {
            Vec3::splat(sx)
        };
        let speed = m.start_speed.eval(t_norm, r[2]);
        let mut rot = m.start_rotation.eval(t_norm, r[3]);
        let em = &self.emitters[k];
        let color = match (em.start_color, em.start_range) {
            (Some(c), _) => c,
            (None, Some((a, b))) => {
                let t = r[4];
                [
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + (b[2] - a[2]) * t,
                    a[3] + (b[3] - a[3]) * t,
                ]
            }
            _ => m.start_color.eval(t_norm, r[4]),
        };
        // Shape (emitter space).
        let (mut pos, mut dir) = match &s.shape {
            Some(sh) if sd.points.as_ref().is_some_and(|p| !p.is_empty()) => {
                let pts = sd.points.as_ref().unwrap();
                let i = ((rng.f() * pts.len() as f32) as usize).min(pts.len() - 1);
                let sc = Vec3::from(sh.scale);
                let p = pts[i].extend(0.0) * sc;
                let q = shape_rotation(sh.rotation);
                (Vec3::from(sh.position) + q * p, q * Vec3::Z)
            }
            Some(sh) => {
                let (p, d) = match em.radius {
                    Some(r) => {
                        let mut sh2 = sh.clone();
                        sh2.radius.value = r;
                        shape_point(&sh2, rng)
                    }
                    None => shape_point(sh, rng),
                };
                let q = shape_rotation(sh.rotation);
                let o = Vec3::from(sh.position);
                (o + q * p, q * d)
            }
            None => (Vec3::ZERO, Vec3::Z),
        };
        if let Some(sh) = &s.shape {
            if sh.random_direction > 0.0 {
                dir = dir.lerp(rng.unit3(), sh.random_direction).normalize_or(dir);
            }
            if sh.spherical_direction > 0.0 && pos.length() > 1e-5 {
                dir = dir
                    .lerp(pos.normalize(), sh.spherical_direction)
                    .normalize_or(dir);
            }
        }
        // Align to direction: the billboard turns with the emission
        // direction, so a rotated emitter turns its particles (the meteor's
        // flipped dust); start rotations stay relative to it.
        if s.shape.as_ref().is_some_and(|sh| sh.align_to_direction) {
            let wd = world.transform_vector3(dir);
            if wd.truncate().length() > 1e-5 && dir.truncate().length() > 1e-5 {
                rot -= wd.y.atan2(wd.x) - dir.y.atan2(dir.x);
            }
        }
        let scale_mode = m.scaling_mode;
        let k_scale = if scale_mode == 2 {
            1.0
        } else {
            scale_of(world)
        };
        let world_space = m.simulation_space != 0;
        let (p, v) = if world_space {
            let p = at.unwrap_or_else(|| world.transform_point3(pos));
            let v = world.transform_vector3(dir).normalize_or(Vec3::Z) * speed * k_scale;
            (p, v)
        } else {
            if let Some(a) = at {
                pos = world.inverse().transform_point3(a);
            }
            (pos, dir * speed)
        };
        let em = &mut self.emitters[k];
        em.born += 1;
        em.particles.push(Particle {
            pos: p,
            vel: v,
            dir: v,
            age: 0.0,
            life,
            size: if world_space { size * k_scale } else { size },
            rot,
            color,
            r: [r[5], r[6], r[7], rng.f()],
            born: em.born,
        });
    }

    fn step_emitter(&mut self, k: usize, h: f32) {
        let def = self.def.clone();
        let sd = &def.systems[self.emitters[k].sys];
        let s = &sd.sys;
        let m = &s.main;
        let world = self.emitter_world(&self.emitters[k]);
        if self.emitters[k].delay > 0.0 {
            self.emitters[k].delay -= h;
            return;
        }
        let duration = m.duration.max(0.01);
        let sys_t = self.emitters[k].time;
        let world_space = m.simulation_space != 0;
        let g = m.gravity_modifier.eval(sys_t / duration, 0.5)
            * if m.gravity_source == 1 {
                self.gravity.1
            } else {
                self.gravity.0
            };
        let gravity = if world_space {
            Vec3::new(0.0, g, 0.0)
        } else {
            world.inverse().transform_vector3(Vec3::new(0.0, g, 0.0))
        };
        let inv = (!world_space).then(|| world.inverse());
        let mut deaths: Vec<Vec3> = Vec::new();
        {
            let em = &mut self.emitters[k];
            let mut i = 0;
            while i < em.particles.len() {
                let p = &mut em.particles[i];
                p.age += h;
                if p.age >= p.life {
                    if !sd.sub_death.is_empty() {
                        deaths.push(if world_space {
                            p.pos
                        } else {
                            world.transform_point3(p.pos)
                        });
                    }
                    em.particles.swap_remove(i);
                    continue;
                }
                let t = p.age / p.life;
                p.vel += gravity * h;
                if let Some(f) = &s.force_over_lifetime {
                    let fv = Vec3::new(
                        f.x.eval(t, p.r[0]),
                        f.y.eval(t, p.r[1]),
                        f.z.eval(t, p.r[2]),
                    );
                    let fv = match (&inv, f.world_space) {
                        (Some(inv), true) => inv.transform_vector3(fv),
                        (None, false) => world.transform_vector3(fv),
                        _ => fv,
                    };
                    p.vel += fv * h;
                }
                if let Some(l) = &s.limit_velocity_over_lifetime {
                    let lim = l.limit.eval(t, p.r[3]);
                    let speed = p.vel.length();
                    if speed > lim && speed > 0.0 {
                        let kk = (1.0 - l.dampen.clamp(0.0, 1.0)).powf(h * 60.0);
                        p.vel *= (lim + (speed - lim) * kk) / speed;
                    }
                    let drag = l.drag.eval(t, p.r[2]);
                    if drag > 0.0 {
                        p.vel *= (1.0 - drag * h).max(0.0);
                    }
                }
                let mut v = p.vel;
                let mut orbit: Option<(Vec3, Vec3)> = None;
                if let Some(vo) = &s.velocity_over_lifetime {
                    let lv = Vec3::new(
                        vo.x.eval(t, p.r[0]),
                        vo.y.eval(t, p.r[1]),
                        vo.z.eval(t, p.r[2]),
                    );
                    let lv = match (&inv, vo.world_space) {
                        (Some(inv), true) => inv.transform_vector3(lv),
                        (None, false) => world.transform_vector3(lv),
                        _ => lv,
                    };
                    v += lv;
                    // Orbital (z) and radial around the emitter, moved by
                    // the orbital offset (local units).
                    let off = Vec3::new(
                        vo.orbital_offset[0].eval(t, 0.5),
                        vo.orbital_offset[1].eval(t, 0.5),
                        vo.orbital_offset[2].eval(t, 0.5),
                    );
                    let centre = if world_space {
                        world.transform_point3(off)
                    } else {
                        off
                    };
                    let rel = p.pos - centre;
                    // Angular velocity about each axis: w x rel.
                    let w = Vec3::new(
                        vo.orbital_x.eval(t, p.r[0]),
                        vo.orbital_y.eval(t, p.r[2]),
                        vo.orbital_z.eval(t, p.r[1]),
                    );
                    if w != Vec3::ZERO {
                        orbit = Some((w, centre));
                    }
                    let rad = vo.radial.eval(t, p.r[2]);
                    if rad != 0.0 {
                        v += rel.normalize_or_zero() * rad;
                    }
                    v *= vo.speed_modifier.eval(t, p.r[3]);
                }
                if let Some(n) = &s.noise {
                    let st = n.strength.eval(t, p.r[1]);
                    let q = p.pos * n.frequency.max(1e-3)
                        + Vec3::splat(n.scroll_speed.eval(t, 0.5) * sys_t);
                    v += Vec3::new(
                        value_noise(q + Vec3::new(17.0, 0.0, 0.0)),
                        value_noise(q + Vec3::new(0.0, 31.0, 0.0)),
                        0.0,
                    ) * st
                        // Damping: the strength scales with the field's size
                        // (1 / frequency), so a scaled field behaves alike.
                        * if n.damping { 1.0 / n.frequency.max(0.05) } else { 1.0 }
                        * n.position_amount.eval(t, p.r[1])
                        // Our value noise's typical amplitude is about half
                        // of Unity's noise field's.
                        * 2.0;
                }
                let before = p.pos;
                p.pos += v * h;
                if let Some((w, centre)) = orbit {
                    // Exact rotation about the centre (a linear step would
                    // spiral outwards).
                    let rel = p.pos - centre;
                    p.pos = centre + Quat::from_scaled_axis(w * h) * rel;
                }
                p.dir = (p.pos - before) / h.max(1e-6);
                if let Some(rl) = &s.rotation_over_lifetime {
                    p.rot += rl.z.eval(t, p.r[2]) * h;
                }
                i += 1;
            }
        }
        for at in deaths {
            for &sub in &sd.sub_death {
                self.start(sub, Some(Mat4::from_translation(at)));
                let n = self.emitters.len() - 1;
                // Its bursts at time 0 fire on the next step.
                self.emitters[n].delay = 0.0;
            }
        }
        // Emission.
        let em = &self.emitters[k];
        if em.stopped {
            self.emitters[k].time += h;
            return;
        }
        let t_in = em.time % duration;
        let t_norm = t_in / duration;
        let mut n = 0.0f32;
        let prev = em.last_pos;
        if s.emission.enabled {
            let rate = s.emission.rate_over_time.eval(t_norm, self.rng.f());
            let here = world.transform_point3(Vec3::ZERO);
            let moved = em.last_pos.map_or(0.0, |l| (here - l).truncate().length());
            let rd = s.emission.rate_over_distance.eval(t_norm, self.rng.f());
            let em = &mut self.emitters[k];
            em.emit_acc += rate * h;
            em.dist_acc += rd * moved;
            em.last_pos = Some(here);
            n = em.emit_acc.floor() + em.dist_acc.floor();
            em.emit_acc -= em.emit_acc.floor();
            em.dist_acc -= em.dist_acc.floor();
            for b in &s.emission.bursts {
                let cycles = if b.cycles <= 0 { 64 } else { b.cycles.min(64) };
                for c in 0..cycles {
                    let bt = b.time + c as f32 * b.interval.max(1e-3);
                    if bt > duration {
                        break;
                    }
                    // Window [t_in, t_in + h), wrapping into the next
                    // cycle for looping systems.
                    let hit = (bt >= t_in && bt < t_in + h)
                        || (m.looping && bt + duration >= t_in && bt + duration < t_in + h);
                    if hit && (b.probability >= 1.0 || self.rng.f() <= b.probability) {
                        n += b.count.eval(t_norm, self.rng.f()).round().max(0.0);
                    }
                }
            }
        }
        // World-space particles of a moving emitter are spread along its
        // path within the step (Unity emits at sub-step positions; Roll
        // even simulates per movement sub-step).
        let here = world.transform_point3(Vec3::ZERO);
        let path = prev
            .filter(|_| s.main.simulation_space != 0)
            .map(|p| p - here)
            .filter(|d| d.length() > 1e-4 && d.length() < 8.0);
        let count = (n as usize).min(2000);
        for i in 0..count {
            let w = match path {
                Some(d) => {
                    let f = (i + 1) as f32 / count as f32;
                    Mat4::from_translation(d * (1.0 - f)) * world
                }
                None => world,
            };
            self.emit(k, t_norm, &w, None);
        }
        let em = &mut self.emitters[k];
        em.time += h;
        if (!m.looping || em.origin.is_some()) && em.time >= duration {
            em.stopped = true;
        }
    }

    /// The particles as quads for the level view (sorted with the sprites
    /// by their system's sorting layer and order).
    pub fn quads(&self, out: &mut Vec<QuadDraw>) {
        for em in &self.emitters {
            let sd = &self.def.systems[em.sys];
            let s = &sd.sys;
            let look = &sd.look;
            let world = self.emitter_world(em);
            let world_space = s.main.simulation_space != 0;
            let k_scale = if s.main.scaling_mode == 2 || world_space {
                1.0
            } else {
                scale_of(&world)
            };
            let mut order: Vec<usize> = (0..em.particles.len()).collect();
            match look.sort_mode {
                2 => order.sort_by_key(|&i| std::cmp::Reverse(em.particles[i].born)),
                3 => order.sort_by_key(|&i| em.particles[i].born),
                _ => {}
            }
            for i in order {
                let p = &em.particles[i];
                let t = p.age / p.life;
                let mut c = p.color;
                if let Some(g) = &s.color_over_lifetime {
                    let m = g.eval(t, p.r[0]);
                    c = [c[0] * m[0], c[1] * m[1], c[2] * m[2], c[3] * m[3]];
                }
                if c[3] <= 0.002 {
                    continue;
                }
                let mut size = p.size * k_scale;
                if let Some(sl) = &s.size_over_lifetime {
                    if sl.separate_axes {
                        size.x *= sl.x.eval(t, p.r[1]);
                        size.y *= sl.y.eval(t, p.r[1]);
                    } else {
                        size *= sl.x.eval(t, p.r[1]);
                    }
                }
                if size.x.abs() < 1e-4 && size.y.abs() < 1e-4 {
                    continue;
                }
                let (pos, vel) = if world_space {
                    (p.pos, p.vel)
                } else {
                    (
                        world.transform_point3(p.pos),
                        world.transform_vector3(p.vel),
                    )
                };
                let (texture, uv) = match sheet_frame(s.texture_sheet.as_ref(), t, p.r[3]) {
                    Frame::Sprite(f) if !look.sheet.is_empty() => {
                        look.sheet[f.min(look.sheet.len() - 1)]
                    }
                    Frame::Grid(uv) => (look.texture, uv),
                    _ => (look.texture, UvRect::FULL),
                };
                let (rx, ry) = if s.main.rotation_3d {
                    (
                        s.main.start_rotation_x.eval(0.0, p.r[2]),
                        s.main.start_rotation_y.eval(0.0, p.r[2]),
                    )
                } else {
                    (0.0, 0.0)
                };
                let flat = rx.cos().abs();
                let dir3 = if world_space {
                    p.dir
                } else {
                    world.transform_vector3(p.dir)
                };
                let (rotation, size2) =
                    if look.alignment == 4 && look.render_mode == 0 && s.main.rotation_3d {
                        // Velocity alignment (Unity `ParticleSystemRenderSpace.
                        // Velocity`): the quad's frame has its forward axis along
                        // the particle's velocity and its side axis in the
                        // screen plane; the particle's 3D rotation (Unity order
                        // Z, X, Y) turns the quad inside that frame and the
                        // orthographic camera projects it. With x = 90° (the
                        // teleport rings) the sprite's width runs along the
                        // motion, face-on: full-height dashes along the ring,
                        // shortened where the motion leaves the screen plane.
                        let local = Quat::from_rotation_y(ry)
                            * Quat::from_rotation_x(rx)
                            * Quat::from_rotation_z(-p.rot);
                        let frame = velocity_frame(dir3).unwrap_or_else(|| {
                            // At rest: around the emitter centre.
                            let c = world.transform_point3(Vec3::ZERO);
                            let rel = pos - c;
                            velocity_frame(Vec3::new(-rel.y, rel.x, 0.0)).unwrap_or(Quat::IDENTITY)
                        });
                        (frame * local, Vec2::new(size.x, size.y))
                    } else if look.render_mode == 1 {
                        let v2 = vel.truncate();
                        let speed = v2.length();
                        let len = size.x * look.length_scale + speed * look.velocity_scale;
                        let ang = if speed > 1e-4 { v2.y.atan2(v2.x) } else { 0.0 };
                        (Quat::from_rotation_z(ang), Vec2::new(len.max(1e-4), size.y))
                    } else {
                        // Unity's particle rotation turns clockwise on screen;
                        // a 3D x rotation foreshortens the quad (orthographic).
                        (
                            Quat::from_rotation_z(-p.rot),
                            Vec2::new(size.x, size.y * flat.max(0.02)),
                        )
                    };
                out.push(QuadDraw {
                    quad: Quad {
                        texture,
                        uv,
                        transform: Mat4::from_rotation_translation(rotation, pos),
                        size: size2,
                        pivot: Vec2::splat(0.5),
                        color: Color::rgba(c[0], c[1], c[2], c[3].clamp(0.0, 1.0)),
                        ..Default::default()
                    },
                    material: look.material.clone(),
                    // Age fraction (custom vertex stream; ExpandingCircle).
                    params: [t, 0.0, 0.0, 2.0],
                    layer: look.unity_layer,
                    sorting_layer: look.layer,
                    sorting_order: look.order,
                    z: pos.z,
                    mask_interaction: look.mask_interaction,
                });
            }
        }
    }
}

/// The frame of a velocity-aligned particle: forward (z) along `v`, the
/// side axis (x) in the screen plane perpendicular to the motion, up (y)
/// completing it. `None` at rest or when moving straight along the view.
fn velocity_frame(v: Vec3) -> Option<Quat> {
    let f = v.try_normalize()?;
    let x = Vec3::Z.cross(f).try_normalize()?;
    let y = f.cross(x);
    Some(Quat::from_mat3(&glam::Mat3::from_cols(x, y, f)))
}

/// A texture-sheet frame.
enum Frame {
    None,
    Grid(UvRect),
    /// Index into the sprite list.
    Sprite(usize),
}

fn sheet_frame(ts: Option<&ip::TextureSheet>, t: f32, r: f32) -> Frame {
    let Some(ts) = ts else {
        return Frame::None;
    };
    if ts.mode == 1 {
        let n = ts.sprites.iter().flatten().count().max(1) as f32;
        let cyc = ts.cycles.max(1.0);
        let tt = (t * cyc).fract();
        let v = ts.frame_over_time.eval(tt, r) + ts.start_frame.eval(0.0, r);
        return Frame::Sprite((v * n).floor().rem_euclid(n) as usize);
    }
    Frame::Grid(sheet_uv(Some(ts), t, r))
}

/// Texture-sheet frame (grid mode) at normalised age `t`.
fn sheet_uv(ts: Option<&ip::TextureSheet>, t: f32, r: f32) -> UvRect {
    let Some(ts) = ts else {
        return UvRect::FULL;
    };
    if ts.mode != 0 {
        return UvRect::FULL;
    }
    let (tx, ty) = (ts.tiles[0].max(1) as f32, ts.tiles[1].max(1) as f32);
    let (frames, row0) = if ts.animation_type == 1 {
        let row = if ts.row_mode == 1 {
            (r * ty).floor()
        } else {
            ts.row_index as f32
        };
        (tx, row.clamp(0.0, ty - 1.0))
    } else {
        (tx * ty, 0.0)
    };
    let cyc = ts.cycles.max(1.0);
    let tt = (t * cyc).fract();
    let v = ts.frame_over_time.eval(tt, r) + ts.start_frame.eval(0.0, r);
    let f = (v * frames).floor().rem_euclid(frames);
    let (col, row) = if ts.animation_type == 1 {
        (f, row0)
    } else {
        (f % tx, (f / tx).floor())
    };
    UvRect {
        min: Vec2::new(col / tx, row / ty),
        max: Vec2::new((col + 1.0) / tx, (row + 1.0) / ty),
    }
}

/// A point and direction of an emitter shape (shape space, before its
/// position/rotation; scale applied).
fn shape_point(sh: &ip::ParticleShape, rng: &mut Rng) -> (Vec3, Vec3) {
    let sc = Vec3::from(sh.scale);
    let rad = sh.radius.value;
    let thick = sh.radius_thickness.clamp(0.0, 1.0);
    let arc = sh.arc.value.to_radians().max(1e-4);
    let (p, d) = match sh.kind_name.as_str() {
        "sphere" | "sphere_shell" | "hemisphere" | "hemisphere_shell" => {
            let mut d = rng.unit3();
            if sh.kind_name.starts_with("hemi") {
                d.z = d.z.abs();
            }
            let dist = rad * (1.0 - thick * (1.0 - rng.f().cbrt()));
            (d * dist, d)
        }
        "circle" | "circle_edge" => {
            let a = rng.f() * arc;
            let e = Vec3::new(a.cos(), a.sin(), 0.0);
            let dist = rad * (1.0 - thick * (1.0 - rng.f().sqrt()));
            (e * dist, e)
        }
        "edge" | "single_sided_edge" => (Vec3::new((rng.f() * 2.0 - 1.0) * rad, 0.0, 0.0), Vec3::Y),
        "donut" => {
            let a = rng.f() * arc;
            let ring = Vec3::new(a.cos(), a.sin(), 0.0);
            let b = rng.f() * std::f32::consts::TAU;
            let tube = (ring * b.cos() + Vec3::Z * b.sin()) * sh.donut_radius * rng.f().sqrt();
            (ring * rad + tube, (tube).normalize_or(ring))
        }
        "rectangle" => (Vec3::new(rng.f() - 0.5, rng.f() - 0.5, 0.0), Vec3::Z),
        n if n.starts_with("cone") => {
            let a = rng.f() * arc;
            let rr = 1.0 - thick * (1.0 - rng.f().sqrt());
            let q = Vec2::new(a.cos(), a.sin()) * rr;
            let tan = sh.angle.clamp(0.0, 89.0).to_radians().tan();
            let d = Vec3::new(q.x * tan, q.y * tan, 1.0).normalize();
            let mut p = Vec3::new(q.x * rad, q.y * rad, 0.0);
            if n.contains("volume") {
                p += d * sh.length * rng.f();
            }
            (p, d)
        }
        _ => (
            // box, box shell/edge, mesh, sprite: a box of the shape scale.
            Vec3::new(rng.f() - 0.5, rng.f() - 0.5, rng.f() - 0.5),
            Vec3::Z,
        ),
    };
    (p * sc, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn burst_def(count: f32, looping: bool) -> Rc<EffectDef> {
        let sys = ip::ParticleSystem {
            main: ip::ParticleMain {
                duration: 1.0,
                looping,
                play_on_awake: true,
                start_lifetime: ip::MinMaxCurve::Constant { value: 0.5 },
                start_size: ip::MinMaxCurve::Constant { value: 1.0 },
                start_speed: ip::MinMaxCurve::Constant { value: 2.0 },
                start_color: ip::MinMaxGradient::Color {
                    color: [1.0, 1.0, 1.0, 1.0],
                },
                simulation_space: 1,
                simulation_speed: 1.0,
                max_particles: 1000,
                ..Default::default()
            },
            emission: ip::ParticleEmission {
                enabled: true,
                bursts: vec![ip::Burst {
                    time: 0.0,
                    count: ip::MinMaxCurve::Constant { value: count },
                    cycles: 1,
                    interval: 0.01,
                    probability: 1.0,
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        Rc::new(EffectDef {
            name: "t".into(),
            systems: vec![SystemDef {
                object: 0,
                sys: Arc::new(sys),
                rel: Mat4::IDENTITY,
                active: true,
                look: Look {
                    texture: TextureId::WHITE,
                    sheet: Vec::new(),
                    material: None,
                    mask_interaction: 0,
                    sort_mode: 0,
                    render_mode: 0,
                    velocity_scale: 0.0,
                    length_scale: 1.0,
                    alignment: 0,
                    layer: 3,
                    order: 0,
                    unity_layer: 12,
                },
                sub_death: Vec::new(),
                points: None,
            }],
        })
    }

    #[test]
    fn burst_emits_once_and_dies() {
        let mut fx = Effect::new(burst_def(12.0, false), Mat4::IDENTITY, 1, Play::Awake);
        fx.update(0.05, 0.05);
        assert_eq!(fx.particle_count(), 12);
        let mut out = Vec::new();
        fx.quads(&mut out);
        assert_eq!(out.len(), 12);
        assert_eq!((out[0].sorting_layer, out[0].sorting_order), (3, 0));
        fx.update(0.6, 0.6);
        assert_eq!(fx.particle_count(), 0, "lifetime 0.5 s");
        fx.update(1.0, 1.0);
        assert!(!fx.alive(), "not looping: done after its duration");
    }

    #[test]
    fn looping_bursts_repeat() {
        let mut fx = Effect::new(burst_def(5.0, true), Mat4::IDENTITY, 1, Play::Awake);
        let mut total = 0;
        for _ in 0..(2.5 / 0.05) as usize {
            let before = fx.particle_count();
            fx.update(0.05, 0.05);
            total += fx.particle_count().saturating_sub(before);
        }
        assert!(fx.alive());
        assert!((10..=15).contains(&total), "{total}");
    }

    #[test]
    fn start_colour_override() {
        let mut fx = Effect::new(burst_def(3.0, false), Mat4::IDENTITY, 1, Play::All);
        fx.set_start_color(|_| true, Color::rgb(1.0, 0.0, 0.0));
        fx.update(0.05, 0.05);
        let mut out = Vec::new();
        fx.quads(&mut out);
        assert!(
            out.iter()
                .all(|q| q.quad.color.g == 0.0 && q.quad.color.r > 0.9)
        );
    }

    #[test]
    fn sheet_frames() {
        let ts = ip::TextureSheet {
            tiles: [2, 2],
            frame_over_time: ip::MinMaxCurve::Constant { value: 0.6 },
            cycles: 1.0,
            ..Default::default()
        };
        let uv = sheet_uv(Some(&ts), 0.3, 0.0);
        // frame floor(0.6 * 4) = 2 -> column 0, row 1.
        assert_eq!(uv.min, Vec2::new(0.0, 0.5));
        assert_eq!(uv.max, Vec2::new(0.5, 1.0));
    }
}

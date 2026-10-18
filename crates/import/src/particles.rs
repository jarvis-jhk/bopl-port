//! Typed Unity particle systems (`Object::particles`, revision 2): the
//! modules of a `ParticleSystem` and its `ParticleSystemRenderer` that
//! matter for a 2D game, converted from their raw serialized fields, with
//! faithful curves (`AnimationCurve` keyframes with tangents) and gradients
//! (colour and alpha keys), plus evaluation helpers.
//!
//! Units are Unity's: seconds, world units, **radians** for rotations
//! (start rotation, rotation over lifetime, in rad / rad/s), degrees for
//! the shape's angle, arc and rotation. Modules that are disabled are
//! `None` (emission keeps its `enabled` flag instead).
//! See `docs/formats/import.md` ("Particle systems").

use serde::{Deserialize, Serialize};
use serde_json::Value as J;

use crate::format::ObjId;

/// Writes non-finite floats as `null` and reads `null` back as +infinity
/// (Unity uses infinite tangents for stepped keys).
mod inf_null {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
        if v.is_finite() {
            s.serialize_f32(*v)
        } else {
            s.serialize_none()
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
        Ok(Option::<f32>::deserialize(d)?.unwrap_or(f32::INFINITY))
    }
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

/// An `AnimationCurve` key.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Keyframe {
    /// Time (normalised lifetime for over-lifetime curves).
    pub time: f32,
    /// Value.
    pub value: f32,
    /// Incoming and outgoing tangent (slope); infinite = stepped (written
    /// as `null`).
    #[serde(with = "inf_null")]
    pub in_slope: f32,
    /// See [`Keyframe::in_slope`].
    #[serde(with = "inf_null")]
    pub out_slope: f32,
    /// Unity `WeightedMode` (0 none, 1 in, 2 out, 3 both); weights only
    /// present when non-zero.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub weighted_mode: i32,
    /// Tangent weights (used with `weighted_mode`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<[f32; 2]>,
}

/// A Unity `AnimationCurve`: Hermite keys plus wrap modes outside the
/// key range.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Curve {
    /// Keys sorted by time.
    pub keys: Vec<Keyframe>,
    /// Wrap mode before the first / after the last key (internal enum:
    /// 0 ping-pong, 1 loop, 2 clamp).
    pub pre_wrap: i32,
    /// See [`Curve::pre_wrap`].
    pub post_wrap: i32,
}

impl Curve {
    /// Value at `t` (Hermite interpolation; tangent weights are ignored,
    /// infinite tangents step).
    pub fn eval(&self, t: f32) -> f32 {
        let k = &self.keys;
        match k.len() {
            0 => return 0.0,
            1 => return k[0].value,
            _ => {}
        }
        let (t0, t1) = (k[0].time, k[k.len() - 1].time);
        let span = t1 - t0;
        let wrap = |t: f32, mode: i32| -> f32 {
            if span <= 0.0 {
                return t0;
            }
            match mode {
                0 => {
                    let x = ((t - t0) / span).rem_euclid(2.0);
                    t0 + if x > 1.0 { 2.0 - x } else { x } * span
                }
                1 => t0 + (t - t0).rem_euclid(span),
                _ => t.clamp(t0, t1),
            }
        };
        let t = if t < t0 {
            wrap(t, self.pre_wrap)
        } else if t > t1 {
            wrap(t, self.post_wrap)
        } else {
            t
        };
        let i = k.partition_point(|x| x.time <= t).clamp(1, k.len() - 1);
        let (a, b) = (&k[i - 1], &k[i]);
        let dt = b.time - a.time;
        if dt <= 0.0 {
            return b.value;
        }
        if !a.out_slope.is_finite() || !b.in_slope.is_finite() {
            return a.value;
        }
        let s = (t - a.time) / dt;
        let (s2, s3) = (s * s, s * s * s);
        let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
        let h10 = s3 - 2.0 * s2 + s;
        let h01 = -2.0 * s3 + 3.0 * s2;
        let h11 = s3 - s2;
        h00 * a.value + h10 * dt * a.out_slope + h01 * b.value + h11 * dt * b.in_slope
    }
}

/// A `MinMaxCurve` (particle property: constant, curve, random between
/// two curves or two constants).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum MinMaxCurve {
    /// `minMaxState` 0.
    Constant {
        /// The value.
        value: f32,
    },
    /// `minMaxState` 1: `curve(t) * multiplier`.
    Curve {
        /// Curve multiplier (`scalar`).
        multiplier: f32,
        /// The curve.
        curve: Curve,
    },
    /// `minMaxState` 2: lerp(min(t), max(t), random) * multiplier.
    TwoCurves {
        /// Curve multiplier.
        multiplier: f32,
        /// Lower curve.
        min: Curve,
        /// Upper curve.
        max: Curve,
    },
    /// `minMaxState` 3: lerp(min, max, random).
    TwoConstants {
        /// Lower value (`minScalar`).
        min: f32,
        /// Upper value (`scalar`).
        max: f32,
    },
}

impl Default for MinMaxCurve {
    fn default() -> Self {
        MinMaxCurve::Constant { value: 0.0 }
    }
}

impl MinMaxCurve {
    /// Value at curve time `t` (normalised lifetime or system time) with
    /// the particle's random factor `r` in [0, 1].
    pub fn eval(&self, t: f32, r: f32) -> f32 {
        match self {
            MinMaxCurve::Constant { value } => *value,
            MinMaxCurve::Curve { multiplier, curve } => curve.eval(t) * multiplier,
            MinMaxCurve::TwoCurves {
                multiplier,
                min,
                max,
            } => {
                let (a, b) = (min.eval(t), max.eval(t));
                (a + (b - a) * r) * multiplier
            }
            MinMaxCurve::TwoConstants { min, max } => min + (max - min) * r,
        }
    }

    /// Largest value it can take (for buffers / bounds; curves sampled).
    pub fn max_value(&self) -> f32 {
        match self {
            MinMaxCurve::Constant { value } => *value,
            MinMaxCurve::TwoConstants { min, max } => min.max(*max),
            _ => (0..=32)
                .map(|i| {
                    let t = i as f32 / 32.0;
                    self.eval(t, 0.0).max(self.eval(t, 1.0))
                })
                .fold(f32::MIN, f32::max),
        }
    }
}

/// A colour key of a [`Gradient`].
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ColorKey {
    /// Time in [0, 1].
    pub time: f32,
    /// RGB as authored (sRGB).
    pub color: [f32; 3],
}

/// An alpha key of a [`Gradient`].
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AlphaKey {
    /// Time in [0, 1].
    pub time: f32,
    /// Alpha.
    pub alpha: f32,
}

/// A Unity `Gradient`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Gradient {
    /// 0 blend, 1 fixed (step to the next key), 2 perceptual blend.
    pub mode: i32,
    /// Colour keys sorted by time.
    pub colors: Vec<ColorKey>,
    /// Alpha keys sorted by time.
    pub alphas: Vec<AlphaKey>,
}

fn keyed<const N: usize>(keys: &[(f32, [f32; N])], t: f32, fixed: bool) -> [f32; N] {
    let Some(first) = keys.first() else {
        return [1.0; N];
    };
    if t <= first.0 {
        return first.1;
    }
    for w in keys.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if t <= b.0 {
            if fixed {
                return b.1;
            }
            let s = if b.0 > a.0 {
                (t - a.0) / (b.0 - a.0)
            } else {
                1.0
            };
            return std::array::from_fn(|i| a.1[i] + (b.1[i] - a.1[i]) * s);
        }
    }
    keys[keys.len() - 1].1
}

impl Gradient {
    /// RGBA at `t` in [0, 1] (blend and perceptual modes interpolate
    /// linearly in the authored space).
    pub fn eval(&self, t: f32) -> [f32; 4] {
        let fixed = self.mode == 1;
        let c: Vec<(f32, [f32; 3])> = self.colors.iter().map(|k| (k.time, k.color)).collect();
        let a: Vec<(f32, [f32; 1])> = self.alphas.iter().map(|k| (k.time, [k.alpha])).collect();
        let [r, g, b] = keyed(&c, t, fixed);
        [r, g, b, keyed(&a, t, fixed)[0]]
    }
}

/// A `MinMaxGradient` (particle colour property).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum MinMaxGradient {
    /// `minMaxState` 0: one colour.
    Color {
        /// RGBA.
        color: [f32; 4],
    },
    /// 1: a gradient over time.
    Gradient {
        /// The gradient.
        gradient: Gradient,
    },
    /// 2: random between two colours.
    TwoColors {
        /// First colour.
        min: [f32; 4],
        /// Second colour.
        max: [f32; 4],
    },
    /// 3: random between two gradients.
    TwoGradients {
        /// First gradient.
        min: Gradient,
        /// Second gradient.
        max: Gradient,
    },
    /// 4: a random point of the gradient (picked once per particle).
    RandomColor {
        /// The gradient sampled at the random factor.
        gradient: Gradient,
    },
}

impl Default for MinMaxGradient {
    fn default() -> Self {
        MinMaxGradient::Color { color: [1.0; 4] }
    }
}

impl MinMaxGradient {
    /// Colour at time `t` with random factor `r` in [0, 1].
    pub fn eval(&self, t: f32, r: f32) -> [f32; 4] {
        let lerp = |a: [f32; 4], b: [f32; 4]| -> [f32; 4] {
            std::array::from_fn(|i| a[i] + (b[i] - a[i]) * r)
        };
        match self {
            MinMaxGradient::Color { color } => *color,
            MinMaxGradient::Gradient { gradient } => gradient.eval(t),
            MinMaxGradient::TwoColors { min, max } => lerp(*min, *max),
            MinMaxGradient::TwoGradients { min, max } => lerp(min.eval(t), max.eval(t)),
            MinMaxGradient::RandomColor { gradient } => gradient.eval(r),
        }
    }
}

/// The main module (`InitialModule` plus the system's own fields).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ParticleMain {
    /// System duration in seconds (`lengthInSec`).
    pub duration: f32,
    /// Loops after `duration`.
    pub looping: bool,
    /// Starts as if one cycle had already run.
    pub prewarm: bool,
    /// Plays when the object becomes active.
    pub play_on_awake: bool,
    /// Delay before the system starts (curve over nothing: use t = 0).
    pub start_delay: MinMaxCurve,
    /// Particle lifetime; evaluated at the system's normalised time.
    pub start_lifetime: MinMaxCurve,
    /// Initial speed along the shape's emission direction.
    pub start_speed: MinMaxCurve,
    /// Initial size (x when `size_3d`).
    pub start_size: MinMaxCurve,
    /// Per-axis start size (`startSizeY`, `startSizeZ`), when `size_3d`.
    pub size_3d: bool,
    /// See [`ParticleMain::size_3d`].
    pub start_size_y: MinMaxCurve,
    /// See [`ParticleMain::size_3d`].
    pub start_size_z: MinMaxCurve,
    /// Initial rotation about z, radians.
    pub start_rotation: MinMaxCurve,
    /// Per-axis rotation (x, y in `start_rotation_x/y`) when set.
    pub rotation_3d: bool,
    /// See [`ParticleMain::rotation_3d`].
    pub start_rotation_x: MinMaxCurve,
    /// See [`ParticleMain::rotation_3d`].
    pub start_rotation_y: MinMaxCurve,
    /// Fraction of particles rotating the other way (`randomizeRotationDirection`).
    pub flip_rotation: f32,
    /// Initial colour (sRGB).
    pub start_color: MinMaxGradient,
    /// Multiplier of gravity (see `gravity_source`).
    pub gravity_modifier: MinMaxCurve,
    /// 0 = 3D physics gravity (`ProjectSettings::gravity_3d`, -9.81), 1 = 2D
    /// physics gravity (`ProjectSettings::gravity`).
    pub gravity_source: i32,
    /// 0 local, 1 world, 2 custom (`custom_simulation_space`).
    pub simulation_space: i32,
    /// Transform id for the custom simulation space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_simulation_space: Option<ObjId>,
    /// Playback speed multiplier.
    pub simulation_speed: f32,
    /// Uses unscaled time.
    pub use_unscaled_time: bool,
    /// 0 hierarchy, 1 local, 2 shape.
    pub scaling_mode: i32,
    /// Maximum live particles.
    pub max_particles: i64,
    /// Emitter velocity source: 0 transform, 1 rigidbody, 2 custom.
    pub emitter_velocity_mode: i32,
    /// 0 none, 1 disable, 2 destroy, 3 callback (when finished).
    pub stop_action: i32,
    /// New random seed per play (`autoRandomSeed`) or fixed `random_seed`.
    pub auto_random_seed: bool,
    /// See [`ParticleMain::auto_random_seed`].
    pub random_seed: i64,
    /// Culling mode (0 automatic, 1 pause and catch up, 2 pause, 3 always simulate).
    pub culling_mode: i32,
}

/// A burst of the emission module.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Burst {
    /// Time in the system cycle, seconds.
    pub time: f32,
    /// Particle count (evaluated once per burst).
    pub count: MinMaxCurve,
    /// Repetitions (0 = infinite).
    pub cycles: i64,
    /// Seconds between repetitions.
    pub interval: f32,
    /// Chance the burst fires.
    pub probability: f32,
}

/// The emission module.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ParticleEmission {
    /// Module enabled.
    pub enabled: bool,
    /// Particles per second (curve over the system's normalised time).
    pub rate_over_time: MinMaxCurve,
    /// Particles per world unit the emitter moves.
    pub rate_over_distance: MinMaxCurve,
    /// Bursts.
    pub bursts: Vec<Burst>,
}

/// A shape-module parameter with a distribution mode (`radius`, `arc`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct MultiMode {
    /// The value.
    pub value: f32,
    /// 0 random, 1 loop, 2 ping-pong, 3 burst spread.
    pub mode: i32,
    /// Spread (0 = continuous).
    pub spread: f32,
    /// Loop speed.
    pub speed: MinMaxCurve,
}

/// The shape module.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ParticleShape {
    /// `ParticleSystemShapeType`: 0 sphere, 2 hemisphere, 4 cone, 5 box,
    /// 8 cone volume, 10 circle, 12 single-sided edge, 15 box shell,
    /// 16 box edge, 17 donut, 18 rectangle, 19 sprite, 20 sprite renderer,
    /// 6/13/14 meshes (see `kind_name`).
    pub kind: i32,
    /// Readable name of `kind` (`sphere`, `cone`, `box`, `circle`,
    /// `edge`, ...).
    pub kind_name: String,
    /// Cone angle, degrees.
    pub angle: f32,
    /// Radius (sphere, circle, cone, edge half length).
    pub radius: MultiMode,
    /// Emission from the volume (1) down to the surface only (0).
    pub radius_thickness: f32,
    /// Arc (circle, cone, donut), degrees.
    pub arc: MultiMode,
    /// Cone length (volume modes).
    pub length: f32,
    /// Box shell thickness per axis.
    pub box_thickness: [f32; 3],
    /// Donut tube radius.
    pub donut_radius: f32,
    /// Shape transform: position, rotation (degrees, euler) and scale (the
    /// box size).
    pub position: [f32; 3],
    /// See [`ParticleShape::position`].
    pub rotation: [f32; 3],
    /// See [`ParticleShape::position`].
    pub scale: [f32; 3],
    /// Align particles to their initial direction.
    pub align_to_direction: bool,
    /// Blend towards a random direction.
    pub random_direction: f32,
    /// Blend towards the direction from the shape centre.
    pub spherical_direction: f32,
    /// Random position offset.
    pub random_position: f32,
    /// Sprite for sprite shapes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite: Option<ObjId>,
}

/// Velocity over lifetime (curves over normalised particle age).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct VelocityOverLifetime {
    /// Linear velocity.
    pub x: MinMaxCurve,
    /// See [`VelocityOverLifetime::x`].
    pub y: MinMaxCurve,
    /// See [`VelocityOverLifetime::x`].
    pub z: MinMaxCurve,
    /// Orbital velocity (rad/s about each axis) and its centre offset.
    pub orbital_x: MinMaxCurve,
    /// See [`VelocityOverLifetime::orbital_x`].
    pub orbital_y: MinMaxCurve,
    /// See [`VelocityOverLifetime::orbital_x`].
    pub orbital_z: MinMaxCurve,
    /// See [`VelocityOverLifetime::orbital_x`].
    pub orbital_offset: [MinMaxCurve; 3],
    /// Radial velocity away from the centre.
    pub radial: MinMaxCurve,
    /// Speed multiplier.
    pub speed_modifier: MinMaxCurve,
    /// Linear velocity in world space (else local).
    pub world_space: bool,
}

/// Limit velocity over lifetime (`ClampVelocityModule`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct LimitVelocity {
    /// Limit per axis (`x`, `y`, `z`) instead of the speed.
    pub separate_axes: bool,
    /// Speed limit.
    pub limit: MinMaxCurve,
    /// Per-axis limits.
    pub x: MinMaxCurve,
    /// See [`LimitVelocity::x`].
    pub y: MinMaxCurve,
    /// See [`LimitVelocity::x`].
    pub z: MinMaxCurve,
    /// Fraction of the excess speed removed per... (Unity: per frame at 30 fps
    /// equivalent).
    pub dampen: f32,
    /// Linear drag.
    pub drag: MinMaxCurve,
    /// Drag scales with particle size / velocity.
    pub drag_by_size: bool,
    /// See [`LimitVelocity::drag_by_size`].
    pub drag_by_velocity: bool,
    /// Per-axis limits in world space.
    pub world_space: bool,
}

/// Force over lifetime.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ForceOverLifetime {
    /// Acceleration per axis.
    pub x: MinMaxCurve,
    /// See [`ForceOverLifetime::x`].
    pub y: MinMaxCurve,
    /// See [`ForceOverLifetime::x`].
    pub z: MinMaxCurve,
    /// World space (else local).
    pub world_space: bool,
    /// New random value every frame (random modes).
    pub randomize_per_frame: bool,
}

/// Size over lifetime (multiplies the start size).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SizeOverLifetime {
    /// Per-axis curves.
    pub separate_axes: bool,
    /// Size (x when separate).
    pub x: MinMaxCurve,
    /// See [`SizeOverLifetime::separate_axes`].
    pub y: MinMaxCurve,
    /// See [`SizeOverLifetime::separate_axes`].
    pub z: MinMaxCurve,
}

/// Rotation over lifetime (angular velocity, rad/s).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RotationOverLifetime {
    /// Per-axis rotation.
    pub separate_axes: bool,
    /// About x and y (separate axes).
    pub x: MinMaxCurve,
    /// See [`RotationOverLifetime::x`].
    pub y: MinMaxCurve,
    /// About z (the 2D rotation).
    pub z: MinMaxCurve,
}

/// Noise (basic parameters).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Noise {
    /// Strength (x when separate).
    pub strength: MinMaxCurve,
    /// Per-axis strength.
    pub separate_axes: bool,
    /// See [`Noise::separate_axes`].
    pub strength_y: MinMaxCurve,
    /// See [`Noise::separate_axes`].
    pub strength_z: MinMaxCurve,
    /// Spatial frequency.
    pub frequency: f32,
    /// Scroll speed over time.
    pub scroll_speed: MinMaxCurve,
    /// Strength proportional to frequency.
    pub damping: bool,
    /// Octaves and their multiplier / scale.
    pub octaves: i64,
    /// See [`Noise::octaves`].
    pub octave_multiplier: f32,
    /// See [`Noise::octaves`].
    pub octave_scale: f32,
    /// 0 low (1D), 1 medium (2D), 2 high (3D).
    pub quality: i32,
    /// How much the noise moves, rotates and scales particles.
    pub position_amount: MinMaxCurve,
    /// See [`Noise::position_amount`].
    pub rotation_amount: MinMaxCurve,
    /// See [`Noise::position_amount`].
    pub size_amount: MinMaxCurve,
    /// Remap curve (when enabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remap: Option<MinMaxCurve>,
}

/// Texture sheet animation (`UVModule`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TextureSheet {
    /// 0 grid, 1 sprites.
    pub mode: i32,
    /// Grid tiles (x, y).
    pub tiles: [i64; 2],
    /// 0 whole sheet, 1 single row.
    pub animation_type: i32,
    /// Row selection: 0 custom (`row_index`), 1 random, 2 mesh index.
    pub row_mode: i32,
    /// See [`TextureSheet::row_mode`].
    pub row_index: i64,
    /// Frame over the particle's lifetime (normalised 0..1 of the frames).
    pub frame_over_time: MinMaxCurve,
    /// Start frame.
    pub start_frame: MinMaxCurve,
    /// Animation cycles per lifetime.
    pub cycles: f32,
    /// 0 lifetime, 1 speed, 2 fps.
    pub time_mode: i32,
    /// Frames per second (fps mode).
    pub fps: f32,
    /// Speed range (speed mode).
    pub speed_range: [f32; 2],
    /// Fraction of particles flipped in U / V.
    pub flip_u: f32,
    /// See [`TextureSheet::flip_u`].
    pub flip_v: f32,
    /// Sprites (sprite mode).
    pub sprites: Vec<Option<ObjId>>,
}

/// Trails (basic).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Trails {
    /// 0 per particle, 1 ribbon.
    pub mode: i32,
    /// Fraction of particles with a trail.
    pub ratio: f32,
    /// Trail lifetime as a fraction of the particle lifetime.
    pub lifetime: MinMaxCurve,
    /// Minimum distance between trail vertices.
    pub min_vertex_distance: f32,
    /// Trail points in world space.
    pub world_space: bool,
    /// Trails die with their particle.
    pub die_with_particles: bool,
    /// 0 stretch, 1 tile, 2 distribute per segment, 3 repeat per segment.
    pub texture_mode: i32,
    /// Width scales with particle size; lifetime scales with size.
    pub size_affects_width: bool,
    /// See [`Trails::size_affects_width`].
    pub size_affects_lifetime: bool,
    /// Multiplies with the particle colour.
    pub inherit_particle_color: bool,
    /// Colour over the trail's lifetime and along the trail.
    pub color_over_lifetime: MinMaxGradient,
    /// See [`Trails::color_over_lifetime`].
    pub color_over_trail: MinMaxGradient,
    /// Width along the trail (multiplier).
    pub width_over_trail: MinMaxCurve,
    /// Ribbon count (ribbon mode).
    pub ribbon_count: i64,
}

/// Collision module (basic parameters; the flag is what matters).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Collision {
    /// 0 planes, 1 world.
    pub kind: i32,
    /// 0 3D, 1 2D.
    pub mode: i32,
    /// Bounce, dampen and lifetime loss per collision.
    pub bounce: MinMaxCurve,
    /// See [`Collision::bounce`].
    pub dampen: MinMaxCurve,
    /// See [`Collision::bounce`].
    pub lifetime_loss: MinMaxCurve,
    /// Particles slower than this after a collision die.
    pub min_kill_speed: f32,
    /// Collider radius as a fraction of the particle size.
    pub radius_scale: f32,
    /// Layer mask of colliders.
    pub collides_with: u32,
    /// Sends collision messages.
    pub send_messages: bool,
}

/// A sub-emitter reference.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SubEmitter {
    /// The sub-emitter's `ParticleSystem` component id.
    pub emitter: Option<ObjId>,
    /// Index of its object in the same object list, if there.
    pub object: Option<usize>,
    /// 0 birth, 1 collision, 2 death, 3 trigger, 4 manual.
    pub kind: i32,
    /// Inherit bits: 1 colour, 2 size, 4 rotation, 8 lifetime, 16 duration.
    pub properties: i32,
    /// Emission probability.
    pub probability: f32,
}

/// The `ParticleSystemRenderer`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ParticleRenderer {
    /// Component enabled.
    pub enabled: bool,
    /// 0 billboard, 1 stretched billboard, 2 horizontal billboard,
    /// 3 vertical billboard, 4 mesh, 5 none.
    pub render_mode: i32,
    /// Particle material (`materials.json` key), its name, shader and main
    /// texture (`_MainTex`, `textures.json` key).
    pub material: Option<ObjId>,
    /// See [`ParticleRenderer::material`].
    pub material_name: String,
    /// See [`ParticleRenderer::material`].
    pub shader: String,
    /// See [`ParticleRenderer::material`].
    pub texture: Option<ObjId>,
    /// Trail material and its main texture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trail_material: Option<ObjId>,
    /// See [`ParticleRenderer::trail_material`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trail_texture: Option<ObjId>,
    /// Sorting layer name, its draw index and the order in layer.
    pub sorting_layer: String,
    /// See [`ParticleRenderer::sorting_layer`].
    pub sorting_layer_index: usize,
    /// See [`ParticleRenderer::sorting_layer`].
    pub sorting_order: i32,
    /// 0 none, 1 by distance, 2 oldest first, 3 youngest first.
    pub sort_mode: i32,
    /// Sorting fudge.
    pub sorting_fudge: f32,
    /// Stretched billboard: camera velocity scale, velocity scale, length
    /// scale.
    pub camera_velocity_scale: f32,
    /// See [`ParticleRenderer::camera_velocity_scale`].
    pub velocity_scale: f32,
    /// See [`ParticleRenderer::camera_velocity_scale`].
    pub length_scale: f32,
    /// Rotate with the stretch direction (stretched billboard).
    pub rotate_with_stretch: bool,
    /// Particle size limits as fractions of the viewport.
    pub min_particle_size: f32,
    /// See [`ParticleRenderer::min_particle_size`].
    pub max_particle_size: f32,
    /// 0 view, 1 world, 2 local, 3 facing, 4 velocity.
    pub render_alignment: i32,
    /// Pivot offset (in particle sizes).
    pub pivot: [f32; 3],
    /// Fraction of particles flipped per axis.
    pub flip: [f32; 3],
    /// 0 none, 1 visible inside mask, 2 outside.
    pub mask_interaction: i32,
}

/// A particle system with its renderer.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ParticleSystem {
    /// `ParticleSystem` component id.
    pub component: ObjId,
    /// The renderer on the same object, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer: Option<ParticleRenderer>,
    /// Main module.
    pub main: ParticleMain,
    /// Emission module.
    pub emission: ParticleEmission,
    /// Enabled modules (`None` = disabled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ParticleShape>,
    /// See [`ParticleSystem::shape`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity_over_lifetime: Option<VelocityOverLifetime>,
    /// See [`ParticleSystem::shape`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_velocity_over_lifetime: Option<LimitVelocity>,
    /// See [`ParticleSystem::shape`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_over_lifetime: Option<ForceOverLifetime>,
    /// Colour multiplier over normalised age.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_over_lifetime: Option<MinMaxGradient>,
    /// See [`ParticleSystem::shape`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_over_lifetime: Option<SizeOverLifetime>,
    /// See [`ParticleSystem::shape`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_over_lifetime: Option<RotationOverLifetime>,
    /// See [`ParticleSystem::shape`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise: Option<Noise>,
    /// See [`ParticleSystem::shape`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_sheet: Option<TextureSheet>,
    /// See [`ParticleSystem::shape`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trails: Option<Trails>,
    /// See [`ParticleSystem::shape`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collision: Option<Collision>,
    /// Sub-emitters (empty when the module is disabled).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_emitters: Vec<SubEmitter>,
}

// ---- conversion from raw fields -----------------------------------------

fn num(j: &J, k: &str) -> f32 {
    match j.get(k) {
        Some(J::Number(n)) => n.as_f64().unwrap_or(0.0) as f32,
        Some(J::Bool(b)) => *b as i32 as f32,
        Some(J::Null) => f32::INFINITY,
        _ => 0.0,
    }
}

fn int(j: &J, k: &str) -> i64 {
    j.get(k)
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_f64().map(|f| f as i64))
                .or_else(|| v.as_bool().map(|b| b as i64))
        })
        .unwrap_or(0)
}

fn flag(j: &J, k: &str) -> bool {
    int(j, k) != 0
}

fn vec3(j: &J, k: &str) -> [f32; 3] {
    let v = &j[k];
    [num(v, "x"), num(v, "y"), num(v, "z")]
}

fn rgba(j: &J) -> [f32; 4] {
    [num(j, "r"), num(j, "g"), num(j, "b"), num(j, "a")]
}

fn rf(j: &J) -> Option<ObjId> {
    j.get("$ref").and_then(J::as_str).map(String::from)
}

/// A raw `AnimationCurve`.
pub fn curve(j: &J) -> Curve {
    let keys = j["m_Curve"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|k| {
                    let wm = int(k, "weightedMode") as i32;
                    Keyframe {
                        time: num(k, "time"),
                        value: num(k, "value"),
                        in_slope: num(k, "inSlope"),
                        out_slope: num(k, "outSlope"),
                        weighted_mode: wm,
                        weights: (wm != 0).then(|| [num(k, "inWeight"), num(k, "outWeight")]),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Curve {
        keys,
        pre_wrap: int(j, "m_PreInfinity") as i32,
        post_wrap: int(j, "m_PostInfinity") as i32,
    }
}

/// A raw `MinMaxCurve`.
pub fn min_max_curve(j: &J) -> MinMaxCurve {
    match int(j, "minMaxState") {
        1 => MinMaxCurve::Curve {
            multiplier: num(j, "scalar"),
            curve: curve(&j["maxCurve"]),
        },
        2 => MinMaxCurve::TwoCurves {
            multiplier: num(j, "scalar"),
            min: curve(&j["minCurve"]),
            max: curve(&j["maxCurve"]),
        },
        3 => MinMaxCurve::TwoConstants {
            min: num(j, "minScalar"),
            max: num(j, "scalar"),
        },
        _ => MinMaxCurve::Constant {
            value: num(j, "scalar"),
        },
    }
}

/// A raw `Gradient` (`key0..7`, `ctime0..7`, `atime0..7` as 0..65535).
pub fn gradient(j: &J) -> Gradient {
    let nc = int(j, "m_NumColorKeys").clamp(0, 8) as usize;
    let na = int(j, "m_NumAlphaKeys").clamp(0, 8) as usize;
    let key = |i: usize| rgba(&j[format!("key{i}").as_str()]);
    let time = |p: &str, i: usize| num(j, &format!("{p}{i}")) / 65535.0;
    Gradient {
        mode: int(j, "m_Mode") as i32,
        colors: (0..nc)
            .map(|i| {
                let [r, g, b, _] = key(i);
                ColorKey {
                    time: time("ctime", i),
                    color: [r, g, b],
                }
            })
            .collect(),
        alphas: (0..na)
            .map(|i| AlphaKey {
                time: time("atime", i),
                alpha: key(i)[3],
            })
            .collect(),
    }
}

/// A raw `MinMaxGradient`.
pub fn min_max_gradient(j: &J) -> MinMaxGradient {
    match int(j, "minMaxState") {
        1 => MinMaxGradient::Gradient {
            gradient: gradient(&j["maxGradient"]),
        },
        2 => MinMaxGradient::TwoColors {
            min: rgba(&j["minColor"]),
            max: rgba(&j["maxColor"]),
        },
        3 => MinMaxGradient::TwoGradients {
            min: gradient(&j["minGradient"]),
            max: gradient(&j["maxGradient"]),
        },
        4 => MinMaxGradient::RandomColor {
            gradient: gradient(&j["maxGradient"]),
        },
        _ => MinMaxGradient::Color {
            color: rgba(&j["maxColor"]),
        },
    }
}

fn shape_name(kind: i64) -> &'static str {
    match kind {
        0 => "sphere",
        1 => "sphere_shell",
        2 => "hemisphere",
        3 => "hemisphere_shell",
        4 => "cone",
        5 => "box",
        6 => "mesh",
        7 => "cone_shell",
        8 => "cone_volume",
        9 => "cone_volume_shell",
        10 => "circle",
        11 => "circle_edge",
        12 => "edge",
        13 => "mesh_renderer",
        14 => "skinned_mesh_renderer",
        15 => "box_shell",
        16 => "box_edge",
        17 => "donut",
        18 => "rectangle",
        19 => "sprite",
        20 => "sprite_renderer",
        _ => "unknown",
    }
}

fn multi_mode(j: &J) -> MultiMode {
    MultiMode {
        value: num(j, "value"),
        mode: int(j, "mode") as i32,
        spread: num(j, "spread"),
        speed: min_max_curve(&j["speed"]),
    }
}

/// Material facts the renderer conversion needs: (name, shader, main
/// texture).
pub type MaterialInfo = (String, String, Option<ObjId>);

/// Converts the raw fields of a `ParticleSystem` (`ps`) and its
/// `ParticleSystemRenderer` (`psr`). `material` resolves a material id;
/// `sorting` maps a sorting layer id to (name, draw index).
pub fn from_raw(
    component: ObjId,
    ps: &J,
    psr: Option<&J>,
    material: &dyn Fn(&str) -> Option<MaterialInfo>,
    sorting: &dyn Fn(i32) -> (String, usize),
) -> ParticleSystem {
    let m = &ps["InitialModule"];
    let c = min_max_curve;
    let on = |k: &str| flag(&ps[k], "enabled");
    let main = ParticleMain {
        duration: num(ps, "lengthInSec"),
        looping: flag(ps, "looping"),
        prewarm: flag(ps, "prewarm"),
        play_on_awake: flag(ps, "playOnAwake"),
        start_delay: c(&ps["startDelay"]),
        start_lifetime: c(&m["startLifetime"]),
        start_speed: c(&m["startSpeed"]),
        start_size: c(&m["startSize"]),
        size_3d: flag(m, "size3D"),
        start_size_y: c(&m["startSizeY"]),
        start_size_z: c(&m["startSizeZ"]),
        start_rotation: c(&m["startRotation"]),
        rotation_3d: flag(m, "rotation3D"),
        start_rotation_x: c(&m["startRotationX"]),
        start_rotation_y: c(&m["startRotationY"]),
        flip_rotation: num(m, "randomizeRotationDirection"),
        start_color: min_max_gradient(&m["startColor"]),
        gravity_modifier: c(&m["gravityModifier"]),
        gravity_source: int(m, "gravitySource") as i32,
        simulation_space: int(ps, "moveWithTransform") as i32,
        custom_simulation_space: rf(&ps["moveWithCustomTransform"]),
        simulation_speed: num(ps, "simulationSpeed"),
        use_unscaled_time: flag(ps, "useUnscaledTime"),
        scaling_mode: int(ps, "scalingMode") as i32,
        max_particles: int(m, "maxNumParticles"),
        emitter_velocity_mode: int(ps, "emitterVelocityMode") as i32,
        stop_action: int(ps, "stopAction") as i32,
        auto_random_seed: flag(ps, "autoRandomSeed"),
        random_seed: int(ps, "randomSeed"),
        culling_mode: int(ps, "cullingMode") as i32,
    };
    let e = &ps["EmissionModule"];
    let burst_count = int(e, "m_BurstCount").max(0) as usize;
    let emission = ParticleEmission {
        enabled: flag(e, "enabled"),
        rate_over_time: c(&e["rateOverTime"]),
        rate_over_distance: c(&e["rateOverDistance"]),
        bursts: e["m_Bursts"]
            .as_array()
            .map(|a| {
                a.iter()
                    .take(burst_count)
                    .map(|b| Burst {
                        time: num(b, "time"),
                        count: c(&b["countCurve"]),
                        cycles: int(b, "cycleCount"),
                        interval: num(b, "repeatInterval"),
                        probability: num(b, "probability"),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    };
    let s = &ps["ShapeModule"];
    let shape = on("ShapeModule").then(|| ParticleShape {
        kind: int(s, "type") as i32,
        kind_name: shape_name(int(s, "type")).to_owned(),
        angle: num(s, "angle"),
        radius: multi_mode(&s["radius"]),
        radius_thickness: num(s, "radiusThickness"),
        arc: multi_mode(&s["arc"]),
        length: num(s, "length"),
        box_thickness: vec3(s, "boxThickness"),
        donut_radius: num(s, "donutRadius"),
        position: vec3(s, "m_Position"),
        rotation: vec3(s, "m_Rotation"),
        scale: vec3(s, "m_Scale"),
        align_to_direction: flag(s, "alignToDirection"),
        random_direction: num(s, "randomDirectionAmount"),
        spherical_direction: num(s, "sphericalDirectionAmount"),
        random_position: num(s, "randomPositionAmount"),
        sprite: rf(&s["m_Sprite"]),
    });
    let v = &ps["VelocityModule"];
    let velocity_over_lifetime = on("VelocityModule").then(|| VelocityOverLifetime {
        x: c(&v["x"]),
        y: c(&v["y"]),
        z: c(&v["z"]),
        orbital_x: c(&v["orbitalX"]),
        orbital_y: c(&v["orbitalY"]),
        orbital_z: c(&v["orbitalZ"]),
        orbital_offset: [
            c(&v["orbitalOffsetX"]),
            c(&v["orbitalOffsetY"]),
            c(&v["orbitalOffsetZ"]),
        ],
        radial: c(&v["radial"]),
        speed_modifier: c(&v["speedModifier"]),
        world_space: flag(v, "inWorldSpace"),
    });
    let l = &ps["ClampVelocityModule"];
    let limit_velocity_over_lifetime = on("ClampVelocityModule").then(|| LimitVelocity {
        separate_axes: flag(l, "separateAxis"),
        limit: c(&l["magnitude"]),
        x: c(&l["x"]),
        y: c(&l["y"]),
        z: c(&l["z"]),
        dampen: num(l, "dampen"),
        drag: c(&l["drag"]),
        drag_by_size: flag(l, "multiplyDragByParticleSize"),
        drag_by_velocity: flag(l, "multiplyDragByParticleVelocity"),
        world_space: flag(l, "inWorldSpace"),
    });
    let f = &ps["ForceModule"];
    let force_over_lifetime = on("ForceModule").then(|| ForceOverLifetime {
        x: c(&f["x"]),
        y: c(&f["y"]),
        z: c(&f["z"]),
        world_space: flag(f, "inWorldSpace"),
        randomize_per_frame: flag(f, "randomizePerFrame"),
    });
    let color_over_lifetime =
        on("ColorModule").then(|| min_max_gradient(&ps["ColorModule"]["gradient"]));
    let z = &ps["SizeModule"];
    let size_over_lifetime = on("SizeModule").then(|| SizeOverLifetime {
        separate_axes: flag(z, "separateAxes"),
        x: c(&z["curve"]),
        y: c(&z["y"]),
        z: c(&z["z"]),
    });
    let r = &ps["RotationModule"];
    let rotation_over_lifetime = on("RotationModule").then(|| RotationOverLifetime {
        separate_axes: flag(r, "separateAxes"),
        x: c(&r["x"]),
        y: c(&r["y"]),
        z: c(&r["curve"]),
    });
    let n = &ps["NoiseModule"];
    let noise = on("NoiseModule").then(|| Noise {
        strength: c(&n["strength"]),
        separate_axes: flag(n, "separateAxes"),
        strength_y: c(&n["strengthY"]),
        strength_z: c(&n["strengthZ"]),
        frequency: num(n, "frequency"),
        scroll_speed: c(&n["scrollSpeed"]),
        damping: flag(n, "damping"),
        octaves: int(n, "octaves"),
        octave_multiplier: num(n, "octaveMultiplier"),
        octave_scale: num(n, "octaveScale"),
        quality: int(n, "quality") as i32,
        position_amount: c(&n["positionAmount"]),
        rotation_amount: c(&n["rotationAmount"]),
        size_amount: c(&n["sizeAmount"]),
        remap: flag(n, "remapEnabled").then(|| c(&n["remap"])),
    });
    let u = &ps["UVModule"];
    let texture_sheet = on("UVModule").then(|| TextureSheet {
        mode: int(u, "mode") as i32,
        tiles: [int(u, "tilesX"), int(u, "tilesY")],
        animation_type: int(u, "animationType") as i32,
        row_mode: int(u, "rowMode") as i32,
        row_index: int(u, "rowIndex"),
        frame_over_time: c(&u["frameOverTime"]),
        start_frame: c(&u["startFrame"]),
        cycles: num(u, "cycles"),
        time_mode: int(u, "timeMode") as i32,
        fps: num(u, "fps"),
        speed_range: [num(&u["speedRange"], "x"), num(&u["speedRange"], "y")],
        flip_u: num(u, "flipU"),
        flip_v: num(u, "flipV"),
        sprites: u["sprites"]
            .as_array()
            .map(|a| a.iter().map(|s| rf(&s["sprite"])).collect())
            .unwrap_or_default(),
    });
    let t = &ps["TrailModule"];
    let trails = on("TrailModule").then(|| Trails {
        mode: int(t, "mode") as i32,
        ratio: num(t, "ratio"),
        lifetime: c(&t["lifetime"]),
        min_vertex_distance: num(t, "minVertexDistance"),
        world_space: flag(t, "worldSpace"),
        die_with_particles: flag(t, "dieWithParticles"),
        texture_mode: int(t, "textureMode") as i32,
        size_affects_width: flag(t, "sizeAffectsWidth"),
        size_affects_lifetime: flag(t, "sizeAffectsLifetime"),
        inherit_particle_color: flag(t, "inheritParticleColor"),
        color_over_lifetime: min_max_gradient(&t["colorOverLifetime"]),
        color_over_trail: min_max_gradient(&t["colorOverTrail"]),
        width_over_trail: c(&t["widthOverTrail"]),
        ribbon_count: int(t, "ribbonCount"),
    });
    let k = &ps["CollisionModule"];
    let collision = on("CollisionModule").then(|| Collision {
        kind: int(k, "type") as i32,
        mode: int(k, "collisionMode") as i32,
        bounce: c(&k["m_Bounce"]),
        dampen: c(&k["m_Dampen"]),
        lifetime_loss: c(&k["m_EnergyLossOnCollision"]),
        min_kill_speed: num(k, "minKillSpeed"),
        radius_scale: num(k, "radiusScale"),
        collides_with: int(&k["collidesWith"], "m_Bits") as u32,
        send_messages: flag(k, "collisionMessages"),
    });
    let sub = &ps["SubModule"];
    let sub_emitters = if flag(sub, "enabled") {
        sub["subEmitters"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|s| SubEmitter {
                        emitter: rf(&s["emitter"]),
                        object: None,
                        kind: int(s, "type") as i32,
                        properties: int(s, "properties") as i32,
                        probability: num(s, "emitProbability"),
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        vec![]
    };
    let renderer = psr.map(|r| {
        let mats: Vec<Option<ObjId>> = r["m_Materials"]
            .as_array()
            .map(|a| a.iter().map(rf).collect())
            .unwrap_or_default();
        let mat = mats.first().cloned().flatten();
        let info = mat.as_deref().and_then(material).unwrap_or_default();
        let trail_mat = if trails.is_some() {
            mats.get(1).cloned().flatten()
        } else {
            None
        };
        let trail_texture = trail_mat
            .as_deref()
            .and_then(material)
            .and_then(|(_, _, t)| t);
        let (layer, index) = sorting(int(r, "m_SortingLayerID") as i32);
        ParticleRenderer {
            enabled: flag(r, "m_Enabled"),
            render_mode: int(r, "m_RenderMode") as i32,
            material: mat,
            material_name: info.0,
            shader: info.1,
            texture: info.2,
            trail_material: trail_mat,
            trail_texture,
            sorting_layer: layer,
            sorting_layer_index: index,
            sorting_order: int(r, "m_SortingOrder") as i32,
            sort_mode: int(r, "m_SortMode") as i32,
            sorting_fudge: num(r, "m_SortingFudge"),
            camera_velocity_scale: num(r, "m_CameraVelocityScale"),
            velocity_scale: num(r, "m_VelocityScale"),
            length_scale: num(r, "m_LengthScale"),
            rotate_with_stretch: flag(r, "m_RotateWithStretchDirection"),
            min_particle_size: num(r, "m_MinParticleSize"),
            max_particle_size: num(r, "m_MaxParticleSize"),
            render_alignment: int(r, "m_RenderAlignment") as i32,
            pivot: vec3(r, "m_Pivot"),
            flip: vec3(r, "m_Flip"),
            mask_interaction: int(r, "m_MaskInteraction") as i32,
        }
    });
    ParticleSystem {
        component,
        renderer,
        main,
        emission,
        shape,
        velocity_over_lifetime,
        limit_velocity_over_lifetime,
        force_over_lifetime,
        color_over_lifetime,
        size_over_lifetime,
        rotation_over_lifetime,
        noise,
        texture_sheet,
        trails,
        collision,
        sub_emitters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mmc(state: i64, scalar: f32, min_scalar: f32, keys: J) -> J {
        json!({"minMaxState": state, "scalar": scalar, "minScalar": min_scalar,
               "maxCurve": {"m_Curve": keys, "m_PreInfinity": 2, "m_PostInfinity": 2},
               "minCurve": {"m_Curve": [], "m_PreInfinity": 2, "m_PostInfinity": 2}})
    }

    #[test]
    fn curves_evaluate_like_unity() {
        let keys = json!([
            {"time": 0.0, "value": 0.0, "inSlope": 0.0, "outSlope": 2.0, "weightedMode": 0},
            {"time": 1.0, "value": 1.0, "inSlope": 0.0, "outSlope": 0.0, "weightedMode": 0}
        ]);
        let m = min_max_curve(&mmc(1, 3.0, 0.0, keys));
        let MinMaxCurve::Curve { curve, multiplier } = &m else {
            panic!("{m:?}")
        };
        assert_eq!(*multiplier, 3.0);
        assert_eq!(curve.eval(0.0), 0.0);
        assert_eq!(curve.eval(1.0), 1.0);
        assert_eq!(curve.eval(2.0), 1.0); // clamp
        // Hermite: h10 * 2 + h01 at s = 0.5 -> 0.125 * 2 + 0.5
        assert!((curve.eval(0.5) - 0.75).abs() < 1e-6);
        assert!((m.eval(0.5, 0.0) - 2.25).abs() < 1e-6);
        let r = min_max_curve(&mmc(3, 4.0, 2.0, json!([])));
        assert_eq!(r.eval(0.0, 0.5), 3.0);
        assert_eq!(r.max_value(), 4.0);
        // Stepped key (infinite tangent written as null).
        let step = min_max_curve(&mmc(
            1,
            1.0,
            0.0,
            json!([
                {"time": 0.0, "value": 5.0, "inSlope": 0.0, "outSlope": null},
                {"time": 1.0, "value": 9.0, "inSlope": null, "outSlope": 0.0}
            ]),
        ));
        assert_eq!(step.eval(0.7, 0.0), 5.0);
        let back: MinMaxCurve =
            serde_json::from_str(&serde_json::to_string(&step).unwrap()).unwrap();
        assert_eq!(back.eval(0.7, 0.0), 5.0);
        let MinMaxCurve::Curve { curve, .. } = back else {
            panic!()
        };
        assert!(curve.keys[0].out_slope.is_infinite());
    }

    #[test]
    fn gradients_have_color_and_alpha_keys() {
        let mut g = json!({"m_Mode": 0, "m_NumColorKeys": 2, "m_NumAlphaKeys": 3,
            "ctime0": 0, "ctime1": 65535, "atime0": 0, "atime1": 32768, "atime2": 65535});
        g["key0"] = json!({"r": 1.0, "g": 0.0, "b": 0.0, "a": 0.0});
        g["key1"] = json!({"r": 0.0, "g": 0.0, "b": 1.0, "a": 1.0});
        g["key2"] = json!({"r": 0.0, "g": 0.0, "b": 0.0, "a": 0.0});
        let mmg = min_max_gradient(&json!({"minMaxState": 1, "maxGradient": g}));
        let MinMaxGradient::Gradient { gradient } = &mmg else {
            panic!()
        };
        assert_eq!(gradient.colors.len(), 2);
        assert_eq!(gradient.alphas.len(), 3);
        let c = mmg.eval(0.25, 0.0);
        assert!((c[0] - 0.75).abs() < 1e-4 && (c[2] - 0.25).abs() < 1e-4);
        // Alpha 0 -> 1 at t 0.5 -> 0 at 1.
        assert!((mmg.eval(0.5, 0.0)[3] - 1.0).abs() < 1e-3);
        assert!(mmg.eval(1.0, 0.0)[3].abs() < 1e-6);
        let two = min_max_gradient(&json!({"minMaxState": 2,
            "minColor": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0},
            "maxColor": {"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0}}));
        assert_eq!(two.eval(0.0, 0.5), [0.5, 0.5, 0.5, 1.0]);
    }

    #[test]
    fn system_modules_convert() {
        let zero = mmc(0, 0.0, 0.0, json!([]));
        let ps = json!({
            "lengthInSec": 2.0, "looping": true, "moveWithTransform": 1, "scalingMode": 1,
            "startDelay": zero,
            "InitialModule": {"startLifetime": mmc(3, 2.0, 1.0, json!([])), "startSpeed": mmc(0, 5.0, 0.0, json!([])),
                "startSize": zero, "startRotation": zero, "gravityModifier": mmc(0, 0.5, 0.0, json!([])),
                "maxNumParticles": 100, "startColor": {"minMaxState": 0, "maxColor": {"r": 1.0, "g": 0.5, "b": 0.0, "a": 1.0}}},
            "EmissionModule": {"enabled": true, "rateOverTime": mmc(0, 10.0, 0.0, json!([])), "m_BurstCount": 1,
                "m_Bursts": [{"time": 0.0, "countCurve": mmc(0, 30.0, 0.0, json!([])), "cycleCount": 1, "repeatInterval": 0.01, "probability": 1.0},
                             {"time": 9.0, "countCurve": zero, "cycleCount": 1, "repeatInterval": 0.01, "probability": 1.0}]},
            "ShapeModule": {"enabled": true, "type": 10, "radius": {"value": 1.5, "mode": 0, "spread": 0.0, "speed": zero},
                "arc": {"value": 360.0, "mode": 0, "spread": 0.0, "speed": zero}, "m_Scale": {"x": 1.0, "y": 1.0, "z": 1.0}},
            "VelocityModule": {"enabled": false},
            "SubModule": {"enabled": true, "subEmitters": [{"emitter": {"$ref": "f:9"}, "type": 2, "properties": 0, "emitProbability": 1.0}]},
            "CollisionModule": {"enabled": true, "type": 1, "collisionMode": 1, "collidesWith": {"m_Bits": 4294967295u32}}
        });
        let psr = json!({"m_Enabled": true, "m_RenderMode": 1, "m_Materials": [{"$ref": "m:1"}],
            "m_SortingLayerID": 7, "m_SortingOrder": 3, "m_LengthScale": 2.0});
        let sys = from_raw(
            "f:1".into(),
            &ps,
            Some(&psr),
            &|id| {
                (id == "m:1").then(|| {
                    (
                        "Mat".into(),
                        "Particles/Additive".into(),
                        Some("t:5".into()),
                    )
                })
            },
            &|id| (format!("L{id}"), id as usize),
        );
        assert_eq!(sys.main.duration, 2.0);
        assert_eq!(sys.main.simulation_space, 1);
        assert_eq!(
            sys.main.start_lifetime,
            MinMaxCurve::TwoConstants { min: 1.0, max: 2.0 }
        );
        assert_eq!(sys.main.start_color.eval(0.0, 0.0), [1.0, 0.5, 0.0, 1.0]);
        assert_eq!(sys.emission.bursts.len(), 1);
        assert_eq!(sys.emission.bursts[0].count.eval(0.0, 0.0), 30.0);
        let sh = sys.shape.as_ref().unwrap();
        assert_eq!(sh.kind_name, "circle");
        assert_eq!(sh.radius.value, 1.5);
        assert!(sys.velocity_over_lifetime.is_none());
        assert_eq!(sys.sub_emitters[0].emitter.as_deref(), Some("f:9"));
        assert_eq!(sys.collision.as_ref().unwrap().mode, 1);
        let r = sys.renderer.as_ref().unwrap();
        assert_eq!(r.texture.as_deref(), Some("t:5"));
        assert_eq!((r.sorting_layer.as_str(), r.sorting_order), ("L7", 3));
        let j = serde_json::to_value(&sys).unwrap();
        assert_eq!(serde_json::from_value::<ParticleSystem>(j).unwrap(), sys);
    }
}

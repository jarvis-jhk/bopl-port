//! Static game data the simulation reads (built by `bopl-content` from the
//! import; raw Q32.32 values, never Unity floats). Shared immutably between
//! world clones through `Arc`.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::curve::Curve;
use crate::fix::Fix;
use crate::physics::{BodyDef, ColliderDef, PhysParams};
use crate::vec2::Vec2;

/// Index into [`Content::sprites`] (the import's sprite ids).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpriteId(pub u32);

impl SpriteId {
    pub const NONE: SpriteId = SpriteId(u32::MAX);
}

/// One frame of a frame animation (`frameDuration` in ticks; negative = mirrored).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Frame {
    pub sprite: SpriteId,
    pub duration: i32,
}

/// A named frame animation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Animation {
    pub name: String,
    pub frames: Vec<Frame>,
}

impl Animation {
    /// `Σ|duration|`.
    pub fn total(&self) -> i32 {
        self.frames.iter().map(|f| f.duration.abs()).sum()
    }
}

/// One entry of a vertical-speed sprite map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpeedFrame {
    pub from: Fix,
    pub to: Fix,
    pub sprite: SpriteId,
}

/// A named speed map (`velocityAnimations`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SpeedAnimation {
    pub name: String,
    pub frames: Vec<SpeedFrame>,
}

/// An `AnimationData` asset.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct AnimationData {
    pub id: String,
    pub name: String,
    pub animations: Vec<Animation>,
    pub speed_animations: Vec<SpeedAnimation>,
    pub default_frame_duration: i32,
}

impl AnimationData {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.animations.iter().position(|a| a.name == name)
    }
}

/// Reference to an animation: `(AnimationData index, animation index)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnimRef {
    pub data: u16,
    pub anim: u16,
}

/// Generic serialized value of a component field (raw Fix stays raw).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    /// An `f32` field (bits). Presentation only; the sim never computes with it.
    Float(u32),
    Str(String),
    Fix(Fix),
    Vec2(Vec2),
    Curve(Arc<Curve>),
    /// Reference to another object by import id (prefab, asset, sprite, component).
    Ref(String),
    List(Vec<Value>),
    Map(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn fix(&self, key: &str) -> Option<Fix> {
        match self.get(key)? {
            Value::Fix(f) => Some(*f),
            _ => None,
        }
    }

    pub fn fix_or(&self, key: &str, d: Fix) -> Fix {
        self.fix(key).unwrap_or(d)
    }

    pub fn vec2(&self, key: &str) -> Option<Vec2> {
        match self.get(key)? {
            Value::Vec2(v) => Some(*v),
            _ => None,
        }
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            Value::Int(i) => Some(*i),
            Value::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    pub fn boolean(&self, key: &str) -> Option<bool> {
        self.int(key).map(|i| i != 0)
    }

    pub fn curve(&self, key: &str) -> Option<Arc<Curve>> {
        match self.get(key)? {
            Value::Curve(c) => Some(c.clone()),
            _ => None,
        }
    }

    pub fn reference(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Ref(r) => Some(r),
            _ => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn list(&self, key: &str) -> &[Value] {
        match self.get(key) {
            Some(Value::List(l)) => l,
            _ => &[],
        }
    }
}

/// A component of a prefab object.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PrefabComponent {
    /// Script class (`Dash`, `PlayerPhysics`, ...) or engine class.
    pub class: String,
    pub id: String,
    pub enabled: bool,
    /// `hierarchyNumber` for updatables.
    pub hierarchy: Option<i32>,
    pub fields: Value,
}

/// Serialized `FixTransform` values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TransformDef {
    pub position: Vec2,
    pub rotation: Fix,
    pub right: Vec2,
    pub up: Vec2,
    pub offset: Vec2,
}

/// Presentation of an imported object (renderer looks it up in the import).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct VisualDef {
    /// Import object id (`"<file>:<path id>"` of the GameObject).
    pub object: String,
    /// Sprite drawn at rest.
    pub sprite: Option<SpriteId>,
    pub sorting_layer: i32,
    pub sorting_order: i32,
}

/// One object of a prefab or scene.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PrefabObject {
    pub name: String,
    pub parent: Option<usize>,
    pub active: bool,
    pub layer: u8,
    pub tag: String,
    pub fix: Option<TransformDef>,
    pub collider: Option<ColliderDef>,
    /// Hierarchy number of the collider component.
    pub collider_hier: Option<i32>,
    pub collider_enabled: bool,
    pub components: Vec<PrefabComponent>,
    pub visual: VisualDef,
}

impl PrefabObject {
    pub fn component(&self, class: &str) -> Option<&PrefabComponent> {
        self.components.iter().find(|c| c.class == class)
    }
}

/// A prefab (or a scene's object list).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Prefab {
    pub id: String,
    pub name: String,
    pub objects: Vec<PrefabObject>,
}

impl Prefab {
    /// Number of updatable components (hierarchy numbers), i.e. how many
    /// numbers `InstantiateFixed` of this prefab consumes.
    pub fn updatable_count(&self) -> i32 {
        self.objects
            .iter()
            .map(|o| {
                o.components
                    .iter()
                    .filter(|c| c.hierarchy.is_some())
                    .count() as i32
            })
            .sum()
    }

    /// The first object with a component of the given class.
    pub fn find_component(&self, class: &str) -> Option<(usize, &PrefabComponent)> {
        self.objects
            .iter()
            .enumerate()
            .find_map(|(i, o)| o.component(class).map(|c| (i, c)))
    }
}

/// The bopl prefab, typed (player.md §1.1).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlayerPrefab {
    pub prefab: String,
    // hierarchy numbers inside the prefab
    pub hier_physics: i32,
    pub hier_controller: i32,
    pub hier_collision: i32,
    pub hier_bounds: i32,
    pub hier_body: i32,
    pub hier_hurtbox: i32,
    pub hier_animator: i32,
    // PlayerPhysics
    pub max_speed: Fix,
    pub jump_strength: Fix,
    pub accel: Fix,
    pub radius: Fix,
    pub gravity_accel: Fix,
    pub gravity_max_fall_speed: Fix,
    pub gravity_modifier: Fix,
    pub jump_extra_x_strength: Fix,
    pub jump_kept_momentum: Fix,
    pub jump_normal_scale_factor: Fix,
    pub extra_jump_teleport_multiplier: Fix,
    pub air_accel: Fix,
    pub inverse_mass01: Fix,
    pub apply_gravity: bool,
    pub sync_rotation_to_platform: bool,
    pub cannot_be_ungrounded_by_ropes: bool,
    // SlimeController
    pub rope_circle_cast_radius: Fix,
    pub extra_swing_force: Fix,
    pub push_overlapping_players_str: Fix,
    pub scale_diff_before_nom_nom: Fix,
    pub scale_increase_on_nom: Fix,
    pub anim_data: u16,
    // PlayerCollision
    pub is_squishable: bool,
    pub platform_squish_point_offset: Vec2,
    // PlayerBody
    pub rope_player_mass01: Fix,
    pub ignore_gravity_when_roped: bool,
    // bounds
    pub destroy_if_too_far_x: bool,
    pub destroy_if_too_far_y: bool,
    pub do_not_destroy_up: bool,
    /// Updatable counts of the effect prefabs (they consume hierarchy numbers).
    pub death_effect_updatables: i32,
    pub down_effect_updatables: i32,
    pub up_effect_updatables: i32,
    pub x_effect_updatables: i32,
    /// Fixed-transform y of the down (water splash) effect prefab.
    pub down_effect_offset: Vec2,
    pub hurtbox: ColliderDef,
    pub layer: u8,
    pub visual: VisualDef,
}

/// The kind of an ability's base component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AbilityKind {
    /// `Ability`: the player becomes this object while it runs.
    Object,
    /// `InstantAbility`: fires and returns in the same tick.
    Instant,
}

/// One entry of the ability list (abilities.md §2).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AbilityDef {
    pub index: usize,
    pub name: String,
    pub offensive: bool,
    pub kind: AbilityKind,
    pub cooldown: Fix,
    pub only_castable_on_ground: bool,
    pub kill_players_on_contact: bool,
    pub is_pushable: bool,
    pub cooldown_stopped_during_time_stop: bool,
    pub is_rope: bool,
    pub icon: SpriteId,
    /// Index into [`Content::prefabs`].
    pub prefab: Option<usize>,
    /// Raw fields of the base component (`Ability` / `InstantAbility`).
    pub fields: Value,
}

/// Grass, snow or space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LevelWorld {
    #[default]
    Grass,
    Snow,
    Space,
}

/// `GameSessionHandler` values of a level.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SessionDef {
    pub hier: i32,
    pub level_animation_radius: Fix,
    pub max_random_delay: Fix,
    pub time_before_two: Fix,
    pub time_before_go: Fix,
    pub level_animation_time: Fix,
    pub time_before_sudden_death: Fix,
    pub delay_before_next_level_input: Fix,
    pub start_progress: Fix,
    pub block_in_delay: Fix,
    pub block_out_delay: Fix,
    pub animate_in: Arc<Curve>,
    pub animate_out: Arc<Curve>,
}

/// `AnimateVelocity` parameters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SpringDef {
    pub hier: i32,
    pub enabled: bool,
    pub home_is_movable: bool,
    pub speed: Fix,
    pub mu: Fix,
    pub rotation_speed: Fix,
    pub rotation_mu: Fix,
    pub interpolate_rotation_home: bool,
    pub rotation_resistence: Fix,
    pub sudden_death_curve: Arc<Curve>,
    pub sudden_death_speed: Fix,
    pub sudden_death_acceleration: Fix,
    pub max_random_sudden_death_delay: Fix,
    pub home_rotation: Fix,
}

/// `VectorFieldPlatform` parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OrbitDef {
    pub hier: i32,
    pub delay: Fix,
    pub orbit_speed: Fix,
    pub expand_speed: Fix,
    pub center: Vec2,
    pub normal_speed_friction: Fix,
    pub dead_zone: Fix,
    pub orbit_accel: Fix,
    pub target_radius: Fix,
}

/// `AntiLockPlatform` parameters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PathDef {
    pub hier: i32,
    pub points: Vec<Vec2>,
    pub force: Fix,
    pub delay: Fix,
}

/// A platform of a level (or spawned by an ability).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlatformDef {
    pub name: String,
    /// Unity tag (`ground` for level platforms).
    pub tag: String,
    pub layer: u8,
    pub transform: TransformDef,
    pub collider: ColliderDef,
    pub hier_collider: i32,
    pub body: BodyDef,
    pub hier_body: i32,
    pub hier_sticky: i32,
    pub platform_type: i32,
    pub spring: Option<SpringDef>,
    pub hier_shake: Option<i32>,
    pub shake_strength: Fix,
    pub grow_on_start: Option<(i32, Fix)>,
    pub orbit: Option<OrbitDef>,
    pub path: Option<PathDef>,
    pub visual: VisualDef,
}

/// A static collider object of a level (the water box).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StaticDef {
    pub name: String,
    /// Unity tag (e.g. `waterKillZone`).
    pub tag: String,
    pub layer: u8,
    pub transform: TransformDef,
    pub collider: ColliderDef,
    pub hier: i32,
    pub enabled: bool,
}

/// An ability spawner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpawnerDef {
    pub hier: i32,
    pub position: Vec2,
}

/// A level, ready for the simulation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LevelData {
    pub build_index: usize,
    pub name: String,
    pub world: LevelWorld,
    pub team_spawns: [Vec2; 4],
    pub teammate_spacing: Fix,
    /// Spawn points for more than 4 teams (generated, match-flow.md §17.2).
    pub extra_spawns: Vec<Vec2>,
    pub session: SessionDef,
    pub physics: PhysParams,
    /// `DetPhysics.beamWidthAnim` (beam half-width over the beam's age).
    pub beam_width_anim: Arc<Curve>,
    /// In the session handler's collection order.
    pub platforms: Vec<PlatformDef>,
    pub statics: Vec<StaticDef>,
    pub spawners: Vec<SpawnerDef>,
    /// The tutorial scene's handler and objects (docs/spec/tutorial.md);
    /// `None` for levels.
    pub tutorial: Option<Arc<TutorialDef>>,
}

/// A trigger object of the tutorial (arrow, pickup, dummy).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TutorialObjDef {
    pub name: String,
    /// Index of the scene object (presentation).
    pub object: usize,
    pub layer: u8,
    pub transform: TransformDef,
    pub collider: ColliderDef,
    pub hier_collider: i32,
    /// Hierarchy number of the tutorial component.
    pub hier: i32,
    /// `activeSelf` in the scene.
    pub active: bool,
}

/// `TutorialArrowPickup`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TutorialPickupDef {
    pub obj: TutorialObjDef,
    /// Ability slot unlocked (`abilityToUnlock`).
    pub slot: usize,
    pub hover_speed: Fix,
    pub hover_amplitude: Fix,
}

/// `TutorialTargetDummy` (with the dummy's player body).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TutorialDummyDef {
    pub obj: TutorialObjDef,
    /// Index into [`TutorialDef::grounds`] of the ground it stands on.
    pub ground: usize,
    /// Layers whose hits count (`layerMask`).
    pub mask: u32,
    pub unlock_grenade: bool,
    pub unlock_dash: bool,
}

/// `AnimateGround` of a tutorial platform.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AnimateGroundDef {
    /// Index into [`LevelData::platforms`].
    pub platform: usize,
    pub keys: Vec<Vec2>,
    pub curve: Arc<Curve>,
    pub delay: Fix,
    pub hier: i32,
}

/// `TutorialGameHandler` and the tutorial scene's objects.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TutorialDef {
    pub hier: i32,
    pub player_spawn: Vec2,
    /// Ability-list indices of the player's slots 0..2 (Bow, Grenade, Dash).
    pub abilities: [usize; 3],
    /// The abilities' cooldown until unlocked (100000 s: locked).
    pub locked_cooldown: Fix,
    /// Cooldowns once unlocked (`cooldowns`).
    pub cooldowns: [Fix; 3],
    pub jingle_delay: Fix,
    pub exit_delay: Fix,
    /// `arrows`, in order.
    pub arrows: Vec<TutorialObjDef>,
    /// Bow, grenade and dash pickups.
    pub pickups: Vec<TutorialPickupDef>,
    pub dummies: Vec<TutorialDummyDef>,
    /// `groundsWithDummies`, in order.
    pub grounds: Vec<AnimateGroundDef>,
}

/// Everything static the simulation needs.
#[derive(Clone, Debug, Default)]
pub struct Content {
    /// SpriteId → import sprite id.
    pub sprites: Vec<String>,
    pub sprite_index: BTreeMap<String, SpriteId>,
    pub anim_data: Vec<AnimationData>,
    pub player: Option<PlayerPrefab>,
    pub abilities: Vec<AbilityDef>,
    pub prefabs: Vec<Prefab>,
    pub prefab_index: BTreeMap<String, usize>,
    /// Layer collision matrix (`Physics2DSettings.m_LayerCollisionMatrix`).
    pub layer_matrix: [u32; 32],
    /// The ability pickup prefab (`AbilityPickup_Dynamic`), typed.
    pub pickup: Option<crate::abilities::pickup::PickupDef>,
    /// Index of the pickup prefab in `prefabs`.
    pub pickup_prefab: Option<usize>,
}

impl Content {
    pub fn player(&self) -> &PlayerPrefab {
        self.player
            .as_ref()
            .expect("content without a player prefab")
    }

    pub fn anim(&self, r: AnimRef) -> &Animation {
        &self.anim_data[r.data as usize].animations[r.anim as usize]
    }

    /// Finds an animation by name in an `AnimationData`.
    pub fn anim_ref(&self, data: u16, name: &str) -> Option<AnimRef> {
        let d = self.anim_data.get(data as usize)?;
        d.find(name).map(|a| AnimRef {
            data,
            anim: a as u16,
        })
    }

    pub fn prefab(&self, id: &str) -> Option<&Prefab> {
        self.prefab_index.get(id).map(|&i| &self.prefabs[i])
    }

    pub fn sprite(&self, id: &str) -> SpriteId {
        self.sprite_index.get(id).copied().unwrap_or(SpriteId::NONE)
    }

    pub fn ability_by_name(&self, name: &str) -> Option<&AbilityDef> {
        self.abilities.iter().find(|a| a.name == name)
    }
}

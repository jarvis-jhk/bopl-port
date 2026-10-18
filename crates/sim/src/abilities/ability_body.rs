//! The player body of ability objects and the prefab-object spawn path,
//! shared by every ability module.
//!
//! * [`AbilityBody`]: an ability prefab's own `PlayerPhysics`, `PlayerBody`
//!   (with the rope coupling of `abilities/rope.rs`), `PlayerCollision`
//!   kill rules and `DestroyIfOutsideSceneBounds`, configured by the prefab
//!   ([`BodyCfg`]); the algorithms are the bopl's (player.md §6, §8,
//!   engine.md §7.1, blink-gun.md "Ability object player body"). The
//!   framework hands the entry snapshot's rope to the body on enter and
//!   takes it back on exit (abilities.md §7.1 step 2.3, §8.2 step 1).
//! * [`Bounds`] / [`Effect`]: `DestroyIfOutsideSceneBounds` for ability
//!   objects and spawned objects, including live explosion effects.
//! * [`add_parts`]: the parts of an ability object's prefab components
//!   (for modules that dispatch them per part through `update_part`).
//! * [`spawn_prefab`]: `FixTransform.InstantiateFixed` of a prefab as a
//!   module object (the one spawn path for all modules).
//! * Animator helpers ([`advance_anim`], [`play_then`], ...), the spawned
//!   `BoplBody` update ([`bopl_body_update`]) and small physics helpers.
//!
//! A module embeds an `AbilityBody` and runs its components in the
//! prefab's hierarchy order, either inside `AbilityImpl::tick` (the object's
//! updatables are contiguous in the updater's list) or as separate parts
//! dispatched to `AbilityImpl::update_part` ([`AbilityBody::update_part`]
//! handles the standard ones).

use std::sync::Arc;

use super::prefab::{self, root_fields};
use super::{AbilityExitInfo, AbilityImpl, Cx, Framework, PlayerInfo, lethal_info, rope};
use crate::anim::{AnimCallback, SpriteAnimator};
use crate::bopl::{PlayerBody, PlayerCollision, PlayerPhysics, RopeLink};
use crate::consts::*;
use crate::data::{AbilityDef, AnimRef, Content, Prefab, PrefabObject, Value};
use crate::fix::Fix;
use crate::object::{FixTransform, ObjId, PartKind};
use crate::physics::{Contact, ShapeSnap};
use crate::player::{CauseOfDeath, ExtraLife};
use crate::surface::{Surface, move_along_surface};
use crate::vec2::Vec2;
use crate::view::{Cue, EffectKind, Visual};
use crate::world::{Kind, World};

// -- DestroyIfOutsideSceneBounds ---------------------------------------------------

/// An effect prefab of the bounds or collision component.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Effect {
    pub prefab: Option<usize>,
    /// Hierarchy numbers it consumes when spawned.
    pub updatables: i32,
    /// Its root fixed-transform position (added to the spawn point).
    pub offset: Vec2,
    /// A live explosion (has an `Explosion` component): simulated.
    pub live: bool,
    pub water: bool,
}

impl Effect {
    pub fn from_ref(w: &World, fields: &Value, key: &str) -> Effect {
        Self::of_ref(&w.content, fields, key)
    }

    pub fn of_ref(c: &Content, fields: &Value, key: &str) -> Effect {
        let Some(p) = prefab::prefab_by_ref(c, fields.reference(key)) else {
            return Effect::default();
        };
        let pf = &c.prefabs[p];
        Effect {
            prefab: Some(p),
            updatables: pf.updatable_count(),
            offset: prefab::root_position(c, Some(p)),
            live: pf.find_component("Explosion").is_some(),
            water: pf.name.starts_with("WaterDeathEffect"),
        }
    }

    /// `InstantiateFixed(effect, pos)`: a live explosion is simulated
    /// (owner 0, scale 1); presentation effects consume their hierarchy
    /// numbers and become a cue.
    pub fn spawn(&self, w: &mut World, pos: Vec2, kind: EffectKind, player_id: i32) {
        let Some(p) = self.prefab else {
            return;
        };
        if self.live {
            super::explosion::spawn(w, p, pos, None, 0);
            return;
        }
        w.consume_hierarchy(self.updatables);
        let kind = if self.water {
            EffectKind::WaterSplash
        } else {
            kind
        };
        w.emit(Cue::Effect {
            kind,
            position: pos,
            player_id,
        });
    }
}

/// `DestroyIfOutsideSceneBounds` settings (engine.md §7.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Bounds {
    pub present: bool,
    pub x: bool,
    pub y: bool,
    pub do_not_destroy_up: bool,
    pub x_effect: Effect,
    pub up_effect: Effect,
    pub down_effect: Effect,
}

impl Bounds {
    pub fn from_fields(w: &World, f: &Value) -> Bounds {
        Self::of_fields(&w.content, f)
    }

    pub fn of_fields(c: &Content, f: &Value) -> Bounds {
        Bounds {
            present: true,
            x: f.boolean("DestroyIfTooFarX").unwrap_or(true),
            y: f.boolean("DestroyIfTooFarY").unwrap_or(true),
            do_not_destroy_up: f.boolean("DoNotDestroyUp").unwrap_or(false),
            x_effect: Effect::of_ref(c, f, "XEffect"),
            up_effect: Effect::of_ref(c, f, "UpEffect"),
            down_effect: Effect::of_ref(c, f, "DownEffect"),
        }
    }

    /// The component of a prefab object (not present if it has none).
    pub fn of_object(w: &World, o: &PrefabObject) -> Bounds {
        Self::of_object_c(&w.content, o)
    }

    pub fn of_object_c(c: &Content, o: &PrefabObject) -> Bounds {
        o.component("DestroyIfOutsideSceneBounds")
            .map_or_else(Bounds::default, |k| Bounds::of_fields(c, &k.fields))
    }

    /// From the root object of a prefab.
    pub fn of_prefab(w: &World, prefab: usize) -> Bounds {
        match w
            .content
            .prefabs
            .get(prefab)
            .and_then(|p| p.objects.first())
        {
            Some(o) => Bounds::of_object(w, o),
            None => Bounds::default(),
        }
    }

    /// `UpdateSim`: spawns the effect and returns true if the object must
    /// self-destruct (the caller does that, engine.md §7.1).
    pub fn check(&self, w: &mut World, me: ObjId) -> bool {
        if !self.present {
            return false;
        }
        let Some(t) = w.objects.get(me).map(|o| o.tf) else {
            return false;
        };
        let p = t.position;
        let space = w.is_space();
        let pid = w.objects.owner_id(me);
        if !space && self.y && p.y < WATER_HEIGHT {
            let mut at = p + self.down_effect.offset;
            at.y = WATER_HEIGHT - F_1_47 * t.scale;
            self.down_effect.spawn(w, at, EffectKind::WaterSplash, pid);
            if self.down_effect.prefab.is_some() && self.down_effect.water {
                w.emit(Cue::Sound("waterSplash"));
            }
        } else if space && self.y && p.y < CAMERA_Y_MIN {
            let mut at = p + self.up_effect.offset;
            at.y = CAMERA_Y_MIN;
            self.up_effect.spawn(w, at, EffectKind::BlastZone, pid);
        } else if !self.do_not_destroy_up && self.y && p.y > BLAST_Y_MAX {
            let mut at = p + self.up_effect.offset;
            at.y = BLAST_Y_MAX;
            self.up_effect.spawn(w, at, EffectKind::BlastZone, pid);
        } else if self.x && (p.x < BLAST_X_MIN || p.x > BLAST_X_MAX) {
            let mut at = p;
            at.x = if p.x < BLAST_X_MIN {
                BLAST_X_MIN
            } else {
                BLAST_X_MAX
            };
            at += self.x_effect.offset;
            self.x_effect.spawn(w, at, EffectKind::BlastZone, pid);
        } else {
            return false;
        }
        true
    }

    /// Self-destruct of spawned items and projectiles (layer Projectile or
    /// item): destroyed.
    pub fn check_and_destroy(&self, w: &mut World, me: ObjId) -> bool {
        if self.check(w, me) {
            w.destroy(me);
            return true;
        }
        false
    }
}

// -- the body ------------------------------------------------------------------------

/// Prefab values of an ability object's body components (root object).
/// Missing fields take the components' code defaults (player.md §1.1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BodyCfg {
    pub has_physics: bool,
    pub has_body: bool,
    pub has_collision: bool,
    pub has_animator: bool,
    /// Hierarchy numbers of the root's components (for per-part modules).
    pub hier_animator: Option<i32>,
    pub hier_physics: Option<i32>,
    pub hier_collision: Option<i32>,
    pub hier_bounds: Option<i32>,
    pub hier_body: Option<i32>,
    pub always_stop_during_time_stop: bool,
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
    // PlayerBody
    pub rope_player_mass01: Fix,
    pub ignore_gravity_when_roped: bool,
    // PlayerCollision
    pub is_squishable: bool,
    pub squish_offset: Vec2,
    pub death_effect: Effect,
    // DestroyIfOutsideSceneBounds
    pub bounds: Bounds,
}

impl BodyCfg {
    /// From the root object of a prefab.
    pub fn from_prefab(w: &World, prefab: usize) -> BodyCfg {
        let Some(p) = w.content.prefabs.get(prefab) else {
            return BodyCfg::default();
        };
        Self::from_root(&w.content, p)
    }

    pub fn from_root(c: &Content, p: &Prefab) -> BodyCfg {
        let empty = Value::Map(Vec::new());
        let root = p.objects.first();
        let comp = |c: &str| root.and_then(|o| o.component(c));
        let hier = |c: &str| comp(c).and_then(|c| c.hierarchy);
        let pf = root_fields(p, "PlayerPhysics").unwrap_or(&empty);
        let cf = root_fields(p, "PlayerCollision").unwrap_or(&empty);
        let bf = root_fields(p, "PlayerBody").unwrap_or(&empty);
        let fx = prefab::fx;
        BodyCfg {
            has_physics: comp("PlayerPhysics").is_some(),
            has_body: comp("PlayerBody").is_some(),
            has_collision: comp("PlayerCollision").is_some(),
            has_animator: comp("SpriteAnimator").is_some(),
            hier_animator: hier("SpriteAnimator"),
            hier_physics: hier("PlayerPhysics"),
            hier_collision: hier("PlayerCollision"),
            hier_bounds: hier("DestroyIfOutsideSceneBounds"),
            hier_body: hier("PlayerBody"),
            always_stop_during_time_stop: prefab::root_always_stop(p),
            max_speed: fx(pf, "maxSpeed", Fix::from_int(19)),
            jump_strength: fx(pf, "jumpStrength", Fix::from_int(30)),
            accel: fx(pf, "accel", Fix::from_int(180)),
            radius: fx(pf, "radius", Fix(3_264_175_104)),
            gravity_accel: fx(pf, "gravity_accel", Fix(6_871_947_776)),
            gravity_max_fall_speed: fx(pf, "gravity_maxFallSpeed", Fix::from_int(27)),
            gravity_modifier: fx(pf, "gravity_modifier", Fix::ONE),
            jump_extra_x_strength: fx(pf, "jumpExtraXStrength", Fix(2_576_980_480)),
            jump_kept_momentum: fx(pf, "jumpKeptMomentum", Fix(2_791_728_640)),
            jump_normal_scale_factor: fx(pf, "jumpNormalScaleFactor", Fix::ONE),
            extra_jump_teleport_multiplier: fx(pf, "extraJumpTeleportMultiplier", Fix(85_899_344)),
            air_accel: fx(pf, "airAccel", Fix::from_int(10)),
            inverse_mass01: fx(pf, "inverseMass01", Fix::from_int(35)),
            apply_gravity: pf.boolean("ApplyGravity").unwrap_or(true),
            sync_rotation_to_platform: pf.boolean("SyncRotationToPlatform").unwrap_or(true),
            cannot_be_ungrounded_by_ropes: pf.boolean("cannotBeUngroundedByRopes").unwrap_or(false),
            rope_player_mass01: fx(bf, "RopePlayerMass01", F_0_5),
            ignore_gravity_when_roped: bf.boolean("ignoreGravityWhenRoped").unwrap_or(false),
            is_squishable: cf.boolean("isSquishable").unwrap_or(true),
            squish_offset: prefab::vec2_field(cf, "platformSquishPointOffset").unwrap_or_default(),
            death_effect: Effect::of_ref(c, cf, "deathEffect"),
            bounds: root.map_or_else(Bounds::default, |o| Bounds::of_object_c(c, o)),
        }
    }

    pub fn rope_cfg(&self) -> rope::RopeCfg {
        rope::RopeCfg {
            rope_player_mass01: self.rope_player_mass01,
            ignore_gravity_when_roped: self.ignore_gravity_when_roped,
            cannot_be_ungrounded_by_ropes: self.cannot_be_ungrounded_by_ropes,
        }
    }
}

/// The player body of an ability object.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct AbilityBody {
    pub cfg: Arc<BodyCfg>,
    /// The owning player (set by [`AbilityBody::install`]).
    pub owner: i32,
    pub physics: PlayerPhysics,
    pub body: PlayerBody,
    /// `PlayerPhysics.gravity_modifier` (abilities change it at run time; it
    /// persists between uses of the slot).
    pub gravity_modifier: Fix,
    /// `PlayerCollision.isSquishable` (some abilities change it).
    pub squishable: bool,
    /// `OnAttachedToGround` fired (by `CollisionRaycasts`) since the module
    /// last cleared the flag.
    pub attached_event: bool,
    /// `OnNoLongerGrounded` fired (by `UnGround`) since the flag was cleared.
    pub ungrounded_event: bool,
}

impl AbilityBody {
    /// The body of an ability prefab (its root object's components).
    pub fn new(w: &World, def: &AbilityDef) -> AbilityBody {
        match def.prefab {
            Some(p) => Self::from_prefab(w, p),
            None => Self::with_cfg(BodyCfg::default()),
        }
    }

    pub fn from_prefab(w: &World, prefab: usize) -> AbilityBody {
        Self::with_cfg(BodyCfg::from_prefab(w, prefab))
    }

    pub fn with_cfg(cfg: BodyCfg) -> AbilityBody {
        Self::from_cfg(Arc::new(cfg))
    }

    /// A body sharing a configuration (modules that keep it in their config).
    pub fn from_cfg(cfg: Arc<BodyCfg>) -> AbilityBody {
        AbilityBody {
            physics: PlayerPhysics {
                radius: cfg.radius,
                prev_scale: Fix::ONE,
                ..PlayerPhysics::default()
            },
            body: PlayerBody {
                simulated: true,
                ..PlayerBody::default()
            },
            gravity_modifier: cfg.gravity_modifier,
            squishable: cfg.is_squishable,
            cfg,
            owner: 0,
            attached_event: false,
            ungrounded_event: false,
        }
    }

    /// `build`: remembers the owner and gives the framework its
    /// `PlayerCollision` state if the prefab has one. (The root collider is
    /// added by the framework's `instantiate`.)
    pub fn install(&mut self, fw: &mut Framework) {
        self.owner = fw.owner;
        if self.cfg.has_collision && fw.collision.is_none() {
            fw.collision = Some(PlayerCollision {
                ignored_beam_id: -1,
                ..PlayerCollision::default()
            });
        }
    }

    /// `build` for modules that dispatch the body components per part: also
    /// adds the standard parts (SpriteAnimator, PlayerPhysics,
    /// PlayerCollision, bounds, PlayerBody) with their hierarchy numbers.
    pub fn install_parts(&mut self, w: &mut World, me: ObjId, fw: &mut Framework) {
        self.install(fw);
        let c = self.cfg.clone();
        if let Some(o) = w.objects.get_mut(me) {
            for (k, h) in [
                (PartKind::Animator, c.hier_animator),
                (PartKind::PlayerPhysics, c.hier_physics),
                (PartKind::PlayerCollision, c.hier_collision),
                (PartKind::Bounds, c.hier_bounds),
                (PartKind::PlayerBody, c.hier_body),
            ] {
                if let Some(h) = h
                    && o.part(k).is_none()
                {
                    o.add_part(k, h);
                }
            }
        }
    }

    // -- transform helpers ----------------------------------------------------------

    pub fn pos(w: &World, me: ObjId) -> Vec2 {
        w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position)
    }

    pub fn set_pos(w: &mut World, me: ObjId, p: Vec2) {
        if let Some(o) = w.objects.get_mut(me) {
            o.tf.position = p;
        }
    }

    pub fn rotation(w: &World, me: ObjId) -> Fix {
        w.objects.get(me).map_or(Fix::ZERO, |o| o.tf.rotation)
    }

    pub fn set_rotation(w: &mut World, me: ObjId, r: Fix) {
        if let Some(o) = w.objects.get_mut(me) {
            o.tf.set_rotation(r);
        }
    }

    pub fn set_up(w: &mut World, me: ObjId, u: Vec2) {
        if let Some(o) = w.objects.get_mut(me) {
            o.tf.set_up(u);
        }
    }

    pub fn scale(w: &World, me: ObjId) -> Fix {
        w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale)
    }

    pub fn tf(w: &World, me: ObjId) -> FixTransform {
        w.objects.get(me).map(|o| o.tf).unwrap_or_default()
    }

    /// `UpdatePhysicsPositions()` of the hurt box.
    pub fn update_collider(w: &mut World, me: ObjId) {
        w.physics.update_physics_positions(&w.objects, me);
    }

    // -- state queries ----------------------------------------------------------------

    pub fn velocity(&self) -> Vec2 {
        self.body.velocity()
    }

    pub fn is_grounded(&self) -> bool {
        self.physics.is_grounded
    }

    /// The attached platform if it still exists (Unity `!= null`).
    pub fn ground(&self, w: &World) -> Option<ObjId> {
        self.physics
            .attached_ground
            .filter(|g| w.objects.exists(*g))
    }

    /// The attached platform if it is not flagged destroyed either.
    pub fn ground_alive(&self, w: &World) -> Option<ObjId> {
        self.physics.attached_ground.filter(|g| w.objects.alive(*g))
    }

    pub fn surface(&self, w: &World) -> Option<Surface> {
        self.ground(w).and_then(|g| Surface::of(w, g))
    }

    /// `attachedGround.currentNormal(body)`.
    pub fn current_normal(&self, w: &World, me: ObjId) -> Option<Vec2> {
        Some(self.surface(w)?.normal(Self::pos(w, me)))
    }

    /// `attachedGround.forwardsDirection(body)`.
    pub fn forward(&self, w: &World, me: ObjId) -> Option<Vec2> {
        Some(self.surface(w)?.forward(Self::pos(w, me)))
    }

    /// `InputGroundAlignment(v)` (uses the attached ground even airborne).
    pub fn input_ground_alignment(&self, w: &World, me: ObjId, v: Vec2) -> Fix {
        v.dot(self.forward(w, me).unwrap_or(Vec2::RIGHT))
    }

    /// `externalForceAlongSurface(body)`.
    pub fn external_force_along_surface(&self, w: &World, me: ObjId) -> Fix {
        match self.forward(w, me) {
            Some(f) => self.body.external_velocity.dot(f),
            None => Fix::ZERO,
        }
    }

    // -- PlayerPhysics ------------------------------------------------------------------

    /// `UpdateScale()`.
    pub fn update_scale(&mut self, w: &World, me: ObjId) {
        let s = Self::scale(w, me);
        if self.physics.prev_scale != s {
            let f = s / self.physics.prev_scale;
            self.physics.radius = self.physics.radius * f;
            self.physics.prev_scale = s;
        }
    }

    /// `SyncPhysicsTo(PlayerInfo)` (the entry snapshot; blink-gun.md "Ability
    /// object player body"): grounded entries snap onto the snapshot's
    /// platform at its local position with this body's radius (no check that
    /// the platform still exists), keeping the snapshot's external velocity
    /// and grounded speed, self-imposed velocity 0, rotation aligned to the
    /// ground; airborne entries copy position and both velocities,
    /// rotation 0, no ground, grounded speed 0.
    pub fn sync_physics_to(&mut self, w: &mut World, me: ObjId, info: &PlayerInfo) {
        self.physics.is_grounded = info.is_grounded;
        self.update_scale(w, me);
        if self.physics.is_grounded {
            self.physics.attached_ground = info.ground;
            self.physics.local_platform_pos = info.grounded_local_position;
            if let Some(s) = info.ground.and_then(|g| Surface::of(w, g)) {
                let p = s.position_from_param(self.physics.local_platform_pos, self.physics.radius);
                Self::set_pos(w, me, p);
            }
            self.body.self_imposed_velocity = Vec2::ZERO;
            self.body.external_velocity = info.external_velocity;
            self.physics.grounded_speed = info.grounded_speed;
            self.align_rotation(w, me);
        } else {
            self.body.external_velocity = info.external_velocity;
            self.body.self_imposed_velocity = info.self_imposed_velocity;
            Self::set_pos(w, me, info.position);
            Self::set_rotation(w, me, Fix::ZERO);
            self.physics.attached_ground = None;
            self.physics.grounded_speed = Fix::ZERO;
        }
    }

    /// `PlayerPhysics.UpdateSim`: returns whether the gravity probe hit.
    pub fn physics_update(&mut self, w: &mut World, me: ObjId, dt: Fix) -> bool {
        if !self.cfg.has_physics {
            return false;
        }
        if self.physics.skip_next_update {
            self.physics.skip_next_update = false;
            return false;
        }
        self.update_scale(w, me);
        if !self.physics.is_grounded && self.cfg.apply_gravity {
            self.add_gravity_factor(w);
            return self.velocity_based_raycasts(w, me, true, dt);
        }
        false
    }

    /// `PlayerPhysics.LateUpdateSim` (grounded: follow the platform, the
    /// arrived-rope pull).
    pub fn physics_late_update(&mut self, w: &mut World, me: ObjId) {
        if !self.cfg.has_physics {
            return;
        }
        if self.physics.is_grounded
            && let Some(s) = self.ground_alive(w).and_then(|g| Surface::of(w, g))
        {
            let p = s.position_from_param(self.physics.local_platform_pos, self.physics.radius);
            Self::set_pos(w, me, p);
            if self.cfg.sync_rotation_to_platform {
                self.align_rotation(w, me);
            }
            let n = s.normal(p);
            if let Some(un) =
                rope::grounded_late_update(w, &mut self.body, p, n, self.cfg.rope_cfg())
            {
                if un {
                    self.unground(w, me, true, false);
                }
                if let Some(l) = self.body.rope {
                    rope::pin_end(w, l, Self::pos(w, me));
                }
            }
        }
        self.physics.jumped_this_frame = false;
    }

    /// `AddGravityFactor()` (per tick, no `dt`).
    pub fn add_gravity_factor(&mut self, w: &World) {
        let c = &self.cfg;
        let g = if w.is_space() {
            self.gravity_modifier * F_0_5
        } else {
            self.gravity_modifier
        };
        let v = &mut self.body.self_imposed_velocity;
        *v += Vec2::DOWN * c.gravity_accel * g;
        let k = c.gravity_accel * g / c.gravity_max_fall_speed;
        let vy = v.y;
        *v += Vec2::DOWN * k * vy;
    }

    /// `AirealMovement(h)`.
    pub fn aireal_movement(&mut self, h: Fix) {
        let c = &self.cfg;
        let v = &mut self.body.self_imposed_velocity;
        *v += Vec2::new(h * c.air_accel, Fix::ZERO);
        let k = c.air_accel / (c.max_speed + c.air_accel);
        let vx = v.x;
        *v += Vec2::LEFT * k * vx;
    }

    /// `VelocityBasedRaycasts(attach, dt)`: whether a probe hit.
    pub fn velocity_based_raycasts(
        &mut self,
        w: &mut World,
        me: ObjId,
        attach: bool,
        dt: Fix,
    ) -> bool {
        let v = self.body.velocity();
        self.collision_raycasts(w, me, attach, v, dt).is_some()
    }

    /// `CollisionRaycasts(attach, v, dt)` (player.md §6.1): the rounded rect
    /// hit (attached to if `attach` and it is a sticky platform).
    pub fn collision_raycasts(
        &mut self,
        w: &mut World,
        me: ObjId,
        attach: bool,
        v: Vec2,
        dt: Fix,
    ) -> Option<ObjId> {
        let s = v.magnitude();
        let len = s * w.fixed_delta_time(self.owner, dt);
        let p = Self::pos(w, me);
        let d = if s.0 == 0 { Vec2::ZERO } else { v / s };
        let n = Vec2::new(-d.y, d.x);
        let r = self.physics.radius;
        let probes = [
            (p + d * r, Vec2::ZERO),
            (p - n * r, n * r),
            (p + n * r, -(n * r)),
        ];
        for (origin, off) in probes {
            let hit = w.physics.raycast_closest_rr(&w.objects, origin, d, len);
            if hit.hit() {
                if attach && Surface::of(w, hit.obj).is_some() {
                    self.attach_to_ground(w, me, hit.obj, hit.near_pos + off);
                    self.attached_event = true;
                }
                return Some(hit.obj);
            }
        }
        None
    }

    /// `AttachToGround(platform, location)`.
    pub fn attach_to_ground(&mut self, w: &mut World, me: ObjId, plat: ObjId, loc: Vec2) {
        attach_parts(w, me, &mut self.physics, &mut self.body, plat, loc);
    }

    /// `alignRotation(body)`: `up = currentNormal`.
    pub fn align_rotation(&mut self, w: &mut World, me: ObjId) {
        if let Some(n) = self.current_normal(w, me) {
            Self::set_up(w, me, n);
        }
    }

    /// `AlignToGround()` (no grounded check).
    pub fn align_to_ground(&mut self, w: &mut World, me: ObjId) {
        if let Some(s) = self.surface(w) {
            let p = s.position_from_param(self.physics.local_platform_pos, self.physics.radius);
            Self::set_pos(w, me, p);
            self.align_rotation(w, me);
        }
    }

    /// `UnGround(nullRotation, playSoundEffect)`.
    pub fn unground(&mut self, w: &mut World, me: ObjId, null_rotation: bool, sound: bool) {
        self.physics.grounded_speed = Fix::ZERO;
        self.physics.is_grounded = false;
        if let Some(n) = self.current_normal(w, me) {
            let p = Self::pos(w, me) + n * F_0_05;
            Self::set_pos(w, me, p);
        }
        if null_rotation {
            Self::set_rotation(w, me, Fix::ZERO);
        }
        self.ungrounded_event = true;
        if sound {
            w.emit(Cue::PlayerSound("slimeJump", self.owner));
        }
    }

    fn increment_towards(c: Fix, target: Fix, a: Fix, h: Fix) -> Fix {
        let c = if c * target < Fix::ZERO { Fix::ZERO } else { c };
        let n = c + h * target.sign_fix() * a;
        if n.abs() > target.abs() { target } else { n }
    }

    /// `PlayerPhysics.Move(input, dt)` (player.md §6.4; `along` false skips
    /// the surface movement). Returns the drop-through record of
    /// `DropPlatformTest` (the caller hands it to its first ability
    /// component's exit).
    pub fn move_(
        &mut self,
        w: &mut World,
        me: ObjId,
        input: Vec2,
        dt: Fix,
        along: bool,
    ) -> Option<AbilityExitInfo> {
        self.move_with(w, me, input, dt, along.then_some(Fix::ONE))
    }

    /// `MachoMove(input, dt, roundedness)`: `Move` with the surface speed
    /// scaled by `roundedness`.
    pub fn macho_move(
        &mut self,
        w: &mut World,
        me: ObjId,
        input: Vec2,
        dt: Fix,
        roundedness: Fix,
    ) -> Option<AbilityExitInfo> {
        self.move_with(w, me, input, dt, Some(roundedness))
    }

    fn move_with(
        &mut self,
        w: &mut World,
        me: ObjId,
        input: Vec2,
        dt: Fix,
        along: Option<Fix>,
    ) -> Option<AbilityExitInfo> {
        let id = self.owner;
        if !self.physics.is_grounded {
            self.aireal_movement(input.x);
            let fdt = w.fixed_delta_time(id, dt);
            self.velocity_based_raycasts(w, me, true, fdt);
            return None;
        }
        let al = self.input_ground_alignment(w, me, input.normalized_safe());
        let nd = self.current_normal(w, me).unwrap_or(Vec2::UP).dot(input);
        let dir =
            if al.abs() >= GROUND_ALIGNMENT_DEADZONE || (nd <= Fix::ZERO && al.abs() >= F_0_01) {
                al.sign_fix()
            } else {
                Fix::ZERO
            };
        let ground = self.ground(w);
        if ground.is_some() && dir == Fix::ZERO {
            let ice = ground
                .and_then(|g| match &w.objects.get(g)?.kind {
                    Kind::Platform(p) => Some(p.sticky.platform_type == 2),
                    _ => None,
                })
                .unwrap_or(false);
            self.physics.grounded_speed =
                self.physics.grounded_speed * if ice { ICE_SLIP } else { NORMAL_SLIP };
        } else {
            let fdt = w.fixed_delta_time(id, dt);
            self.physics.grounded_speed = Self::increment_towards(
                self.physics.grounded_speed,
                dir * self.cfg.max_speed,
                self.cfg.accel,
                fdt,
            );
        }
        let live =
            ground.filter(|g| w.objects.alive(*g) && w.objects.get(*g).is_some_and(|o| o.active));
        let Some(g) = live else {
            self.unground(w, me, true, true);
            return None;
        };
        if self.cfg.sync_rotation_to_platform {
            self.align_rotation(w, me);
        }
        let ext = self.external_force_along_surface(w, me);
        if let Some(k) = along {
            let fdt = w.fixed_delta_time(id, dt);
            let pos = Self::pos(w, me);
            let speed = if k == Fix::ONE {
                self.physics.grounded_speed + ext
            } else {
                (self.physics.grounded_speed + ext) * k
            };
            self.physics.local_platform_pos = move_along_surface(
                w,
                g,
                pos,
                self.physics.radius,
                speed,
                fdt,
                self.physics.local_platform_pos,
            );
        }
        self.drop_platform_test(w, me)
    }

    /// `DropPlatformTest()`: the exit record if the body dropped through
    /// (groundedSpeed 0, airborne, the new velocity and position).
    pub fn drop_platform_test(&mut self, w: &mut World, me: ObjId) -> Option<AbilityExitInfo> {
        let g = self.ground_alive(w)?;
        if !w.objects.get(g).is_some_and(|o| o.active) {
            return None;
        }
        let force = crate::platform::am_i_dropped(w, g, self.owner)?;
        let n = self.current_normal(w, me).unwrap_or(Vec2::UP);
        self.unground(w, me, true, false);
        self.body.self_imposed_velocity += n * force;
        let p = Self::pos(w, me) + self.body.self_imposed_velocity / Fix::from_int(100);
        Self::set_pos(w, me, p);
        Some(AbilityExitInfo {
            grounded_speed: Fix::ZERO,
            is_grounded: false,
            self_imposed_velocity: self.body.self_imposed_velocity,
            position: p,
            ..AbilityExitInfo::default()
        })
    }

    /// `MoveAlongCurrentPlatForm(dt)`: carried along the surface by the
    /// external velocity only (which is zeroed).
    pub fn move_along_current_platform(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        let e = self.external_force_along_surface(w, me);
        self.body.external_velocity = Vec2::ZERO;
        let Some(g) = self.ground(w) else {
            return;
        };
        let fdt = w.fixed_delta_time(self.owner, dt);
        let pos = Self::pos(w, me);
        self.physics.local_platform_pos = move_along_surface(
            w,
            g,
            pos,
            self.physics.radius,
            Fix::ZERO + e,
            fdt,
            self.physics.local_platform_pos,
        );
    }

    /// `MoveAlongCurrentPlatFormSwapIfBlocked(body, r, speed, dt, buffer)`
    /// (player.md §5.5): moves along the attached platform, hopping onto a
    /// blocking platform (at most once per free buffer slot per chain). The
    /// recursion passes the remaining movement, already multiplied by `dt`,
    /// and multiplies it by `dt` again (literal).
    pub fn move_along_swap_if_blocked(
        &mut self,
        w: &mut World,
        me: ObjId,
        r: Fix,
        speed: Fix,
        dt: Fix,
        buffer: &mut [Option<ObjId>],
    ) {
        let Some(g) = self.physics.attached_ground else {
            return;
        };
        if let Some(slot) = buffer.iter_mut().find(|s| s.is_none()) {
            *slot = Some(g);
        }
        let Some(s) = Surface::of(w, g) else {
            return;
        };
        let speed = speed * dt;
        let fwd = s.forward(Self::pos(w, me));
        let p = s.position_from_param(self.physics.local_platform_pos, r);
        let dir = (speed * fwd).normalized_safe();
        let mut near = Fix::from_long(1_000_000_000);
        let mut blocker = None;
        for i in 0..w.physics.rrs.len() {
            let Some((dn, _)) = crate::physics::shapes::ray_rr(p, dir, &w.physics.rrs.shapes[i])
            else {
                continue;
            };
            let o = w.physics.rrs.parents[i].obj;
            let ok = w
                .objects
                .get(o)
                .is_some_and(|ob| !ob.tf.destroyed && ob.active)
                && buffer.iter().all(|b| *b != Some(o));
            if ok && dn < near {
                near = dn;
                blocker = matches!(w.objects.get(o).map(|ob| &ob.kind), Some(Kind::Platform(_)))
                    .then_some(o);
            }
        }
        if speed.abs() < near {
            blocker = None;
        }
        let step = speed.abs().min(near);
        let remaining = (step - speed.abs()).abs() * speed.sign_fix();
        let step = step * speed.sign_fix();
        self.physics.local_platform_pos =
            (self.physics.local_platform_pos + step / s.path_len(r) + Fix::ONE).slow_mod(Fix::ONE);
        if let Some(b) = blocker {
            let at = s.position_from_param(self.physics.local_platform_pos, self.physics.radius);
            self.attach_to_ground(w, me, b, at);
            self.move_along_swap_if_blocked(w, me, r, remaining, dt, buffer);
        }
    }

    /// `PlayerPhysics.Jump()`.
    pub fn jump(&mut self, w: &mut World, me: ObjId) {
        let c = self.cfg.clone();
        self.physics.jumped_this_frame = true;
        let n = if self.physics.is_grounded {
            self.current_normal(w, me).unwrap_or(Vec2::UP)
        } else {
            Vec2::UP
        };
        let side = Vec2::new(n.x, Fix::ZERO);
        let b = Fix::ONE
            .min((n.y + Fix::ONE) / c.jump_normal_scale_factor)
            .max(Fix::ZERO);
        let up = Vec2::UP * b + side * c.jump_extra_x_strength;
        let tangent = Vec2::new(n.y, -n.x) * self.physics.grounded_speed;
        self.body.self_imposed_velocity = tangent * c.jump_kept_momentum + up * c.jump_strength;
        let p =
            Self::pos(w, me) + self.body.self_imposed_velocity * c.extra_jump_teleport_multiplier;
        Self::set_pos(w, me, p);
        self.unground(w, me, true, true);
    }

    /// `PlayerPhysics.AddForce(f)`: `externalVelocity += f · inverseMass01`.
    pub fn add_force(&mut self, f: Vec2) {
        self.body.external_velocity += f * self.cfg.inverse_mass01;
    }

    /// `PlayerBody.AddForce(f)`: `externalVelocity += f · 35`.
    pub fn body_add_force(&mut self, f: Vec2) {
        self.body.external_velocity += f * Fix::from_int(35);
    }

    /// The force entry of other objects (black-hole pull, shockwaves):
    /// `PlayerPhysics.AddForce` if the object has a PlayerPhysics, else
    /// `PlayerBody.AddForce`.
    pub fn external_force(&mut self, f: Vec2) {
        if self.cfg.has_physics {
            self.add_force(f);
        } else {
            self.body_add_force(f);
        }
    }

    // -- PlayerBody ---------------------------------------------------------------------

    /// `PlayerBody.UpdateSim` (rope coupling, integration, external decay).
    pub fn body_update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        if !self.body.simulated {
            return;
        }
        let phys = self.cfg.has_physics.then_some(&self.physics);
        if !rope::body_update(
            w,
            me,
            self.owner,
            &mut self.body,
            phys,
            self.cfg.rope_cfg(),
            dt,
        ) {
            let fdt = w.fixed_delta_time(self.owner, dt);
            let p = Self::pos(w, me) + self.body.velocity() * fdt;
            Self::set_pos(w, me, p);
        }
        self.body.external_velocity = self.body.external_velocity * F_0_95;
    }

    /// `PlayerBody.LateUpdateSim`.
    pub fn body_late_update(&mut self, w: &mut World, me: ObjId) {
        if !self.body.simulated {
            return;
        }
        rope::body_late_update(w, &mut self.body, self.cfg.rope_cfg());
        self.body.prev_position = Self::pos(w, me);
    }

    /// `PlayerBody.AttachRope(rope, top)`.
    pub fn attach_rope(&mut self, w: &mut World, rope_idx: Option<u16>, top: bool) {
        rope::attach_rope(w, &mut self.body, self.cfg.rope_cfg(), rope_idx, top);
    }

    /// Enter (abilities.md §7.1 step 2.3): the body takes over the
    /// snapshot's rope at the same end.
    pub fn take_snapshot_rope(&mut self, w: &mut World, info: &PlayerInfo) {
        if self.cfg.has_body
            && let Some(l) = info.rope
        {
            self.attach_rope(w, Some(l.rope), l.top);
        }
    }

    /// Exit (abilities.md §8.2 step 1): the held rope goes into the record
    /// and the body's reference is cleared (no detach).
    pub fn hand_back_rope(&mut self, info: &mut AbilityExitInfo) {
        if let Some(l) = self.body.rope {
            info.rope = Some(l);
            self.body.prev_player_vel = self.body.velocity();
            self.body.prev_imposed_vel = self.body.self_imposed_velocity;
            self.body.prev_external_vel = self.body.external_velocity;
            self.body.rope = None;
        }
    }

    /// `EnterAbilityClearRope()`: remember the velocities, forget the rope
    /// (no detach).
    pub fn clear_rope(&mut self) {
        self.body.prev_player_vel = self.body.velocity();
        self.body.prev_imposed_vel = self.body.self_imposed_velocity;
        self.body.prev_external_vel = self.body.external_velocity;
        self.body.rope = None;
    }

    pub fn rope(&self) -> Option<RopeLink> {
        self.body.rope
    }

    // -- PlayerCollision --------------------------------------------------------------

    /// `PlayerCollision.UpdateSim`.
    pub fn collision_update(fw: &mut Framework) {
        if let Some(c) = fw.collision.as_mut() {
            c.was_killed_this_frame = false;
        }
    }

    /// `PlayerCollision.OnCollide(contact)`: the kill rules unless the body
    /// was already killed this tick. Returns `(killer, cause)` when the body
    /// dies; the module then runs `AbilityImpl::kill` (or [`hurt_contact`]).
    pub fn kill_on_collision(
        &mut self,
        w: &mut World,
        me: ObjId,
        fw: &mut Framework,
        c: &Contact,
    ) -> Option<(i32, CauseOfDeath)> {
        if fw.collision.as_ref()?.was_killed_this_frame {
            return None;
        }
        self.kill_player_on_collision(w, me, fw, c)
    }

    /// `KillPlayerOnCollision(contact)` called directly (no
    /// `wasKilledThisFrame` test): the kill rules (player.md §8.1).
    pub fn kill_player_on_collision(
        &mut self,
        w: &mut World,
        me: ObjId,
        fw: &mut Framework,
        c: &Contact,
    ) -> Option<(i32, CauseOfDeath)> {
        let col = fw.collision.as_ref()?;
        if col.is_invulnerable() {
            return None;
        }
        let id = self.owner;
        if w.player(id).is_some_and(|p| !p.is_alive()) {
            return None;
        }
        let other = w.objects.get(c.other)?;
        if other.tf.destroyed {
            return None;
        }
        for ig in col.ignored.iter().flatten() {
            if let Some(h) = w
                .objects
                .get(*ig)
                .and_then(|o| o.part(PartKind::Collider))
                .filter(|p| !p.destroyed)
                && h.hier == c.other_hier
                && c.other_hier != 0
            {
                return None;
            }
        }
        let ignored_beam = col.ignored_beam_id;
        let mut squished = false;
        if c.layer == 11 {
            let other_is_platform = matches!(other.kind, Kind::Platform(_));
            if self.squishable
                && self.cfg.has_physics
                && other_is_platform
                && self.physics.is_grounded
                && let Some(g) = self.ground_alive(w)
                && g != c.other
            {
                if crate::platform::picks_up_players(w, c.other) {
                    let p = Self::pos(w, me);
                    self.attach_to_ground(w, me, c.other, p);
                } else {
                    let t = Self::tf(w, me);
                    let p = t.position + t.right.complex_mul(self.cfg.squish_offset);
                    if c.collider.contains(p) {
                        squished = true;
                    }
                }
            }
        }
        let other_layer = w.objects.get(c.other).map_or(0, |o| o.layer);
        let lethal_layer = matches!(other_layer, 9 | 18 | 8 | 16 | 26);
        if !(lethal_layer || squished) {
            return None;
        }
        let k = lethal_info(w, c.other, other_layer, squished, ignored_beam)?;
        if k.owner == id && !k.can_hurt_owner {
            return None;
        }
        if let Some(col) = fw.collision.as_mut() {
            col.was_killed_this_frame = true;
        }
        let cause = if squished {
            CauseOfDeath::Squished
        } else if k.owner == id && k.can_hurt_owner {
            CauseOfDeath::Accident
        } else if k.beam {
            CauseOfDeath::Beamed
        } else {
            CauseOfDeath::Other
        };
        Some((k.owner, cause))
    }

    /// `PlayerCollision.OnCollide` followed by `killPlayer(killer, effect,
    /// respect invulnerability, cause)` up to the exit: `(killer, cause)` if
    /// the body died (the module then runs its first ability component's
    /// dead exit).
    pub fn collision_contact(
        &mut self,
        w: &mut World,
        me: ObjId,
        fw: &mut Framework,
        c: &Contact,
    ) -> Option<(i32, CauseOfDeath)> {
        let k = self.kill_on_collision(w, me, fw, c)?;
        self.kill(w, me, fw, true, false).then_some(k)
    }

    /// `PlayerCollision.killPlayer` on an ability object up to the exit
    /// (player.md §8.2, blink-gun.md): releases the rope; not killed if
    /// invulnerable (unless ignoring) or the object is inactive; the death
    /// effect (consumes its hierarchy numbers) unless the player is being
    /// revived. Returns whether the ability must now exit as dead.
    pub fn kill(
        &mut self,
        w: &mut World,
        me: ObjId,
        fw: &Framework,
        effect: bool,
        ignore_invulnerability: bool,
    ) -> bool {
        if let Some(l) = self.body.rope {
            self.attach_rope(w, None, l.top);
        }
        let inv = fw.collision.as_ref().is_some_and(|c| c.is_invulnerable());
        if (inv && !ignore_invulnerability) || !w.objects.get(me).is_some_and(|o| o.active) {
            return false;
        }
        let reviving = w
            .player(self.owner)
            .is_some_and(|p| p.extra_life == ExtraLife::IsBeingRevived);
        if effect && !reviving {
            if self.cfg.death_effect.prefab.is_some() {
                w.consume_hierarchy(self.cfg.death_effect.updatables);
            }
            let p = Self::pos(w, me);
            w.emit(Cue::Effect {
                kind: EffectKind::Death,
                position: p,
                player_id: self.owner,
            });
        }
        true
    }

    // -- DestroyIfOutsideSceneBounds --------------------------------------------------

    /// The bounds check of the ability object: the dead exit record (killer
    /// = environment, default cause Other) when it left the scene.
    pub fn bounds_update(&mut self, w: &mut World, me: ObjId) -> Option<AbilityExitInfo> {
        if !self.cfg.bounds.check(w, me) {
            return None;
        }
        Some(dead_record(w, me, ENVIRONMENT_ID, CauseOfDeath::Other))
    }

    /// The standard parts of a per-part module ([`AbilityBody::install_parts`]):
    /// PlayerPhysics, PlayerCollision, bounds, PlayerBody. Returns the dead
    /// record when the bounds check fired (the module hands it to its first
    /// ability component's exit). The animator is the module's.
    pub fn update_part(
        &mut self,
        cx: &mut Cx<'_>,
        k: PartKind,
        dt: Fix,
        late: bool,
    ) -> Option<AbilityExitInfo> {
        let me = cx.me;
        match (k, late) {
            (PartKind::PlayerPhysics, false) => {
                self.physics_update(cx.w, me, dt);
            }
            (PartKind::PlayerPhysics, true) => self.physics_late_update(cx.w, me),
            (PartKind::PlayerCollision, false) => Self::collision_update(cx.fw),
            (PartKind::Bounds, false) => return self.bounds_update(cx.w, me),
            (PartKind::PlayerBody, false) => self.body_update(cx.w, me, dt),
            (PartKind::PlayerBody, true) => self.body_late_update(cx.w, me),
            _ => {}
        }
        None
    }
}

/// `AttachToGround(platform, location)` on any `PlayerPhysics` + `PlayerBody`
/// pair (a bopl or an ability body).
pub fn attach_parts(
    w: &mut World,
    me: ObjId,
    phys: &mut PlayerPhysics,
    body: &mut PlayerBody,
    plat: ObjId,
    loc: Vec2,
) {
    phys.attached_ground = Some(plat);
    phys.is_grounded = true;
    let Some(s) = Surface::of(w, plat) else {
        return;
    };
    let n = s.normal(AbilityBody::pos(w, me));
    phys.grounded_speed = body.self_imposed_velocity.cross(n);
    phys.local_platform_pos = s.param_from_position(loc, phys.radius);
    let p = s.position_from_param(phys.local_platform_pos, phys.radius);
    AbilityBody::set_pos(w, me, p);
    body.self_imposed_velocity = Vec2::ZERO;
    body.external_velocity = Vec2::ZERO;
    AbilityBody::set_up(w, me, s.normal(p));
}

/// The record of a dead exit at the object's position (bounds, kills).
pub fn dead_record(w: &World, me: ObjId, killer: i32, cause: CauseOfDeath) -> AbilityExitInfo {
    AbilityExitInfo {
        is_dead: true,
        killer_id: killer,
        cause,
        position: AbilityBody::pos(w, me),
        ..AbilityExitInfo::default()
    }
}

/// Standard handling of a hurt-box contact for a module with an
/// [`AbilityBody`]: the kill rules, then `AbilityImpl::kill` (death effect,
/// then the first ability component's dead exit).
pub fn hurt_contact<A: AbilityImpl>(a: &mut A, cx: &mut Cx<'_>, c: &Contact) {
    let Some(b) = a.body_mut() else {
        return;
    };
    if let Some((killer, cause)) = b.kill_on_collision(cx.w, cx.me, cx.fw, c) {
        a.kill(cx, killer, true, false, cause);
    }
}

// -- prefab parts and spawning -----------------------------------------------------------

/// Marker for [`spawn_prefab`] part lists: the root's DPhysics collider.
pub const COLLIDER: &str = "@collider";

/// Standard component classes → part kinds.
fn standard_part(class: &str) -> Option<PartKind> {
    Some(match class {
        "SpriteAnimator" => PartKind::Animator,
        "PlayerCollision" => PartKind::PlayerCollision,
        "PlayerPhysics" => PartKind::PlayerPhysics,
        "DestroyIfOutsideSceneBounds" => PartKind::Bounds,
        "PlayerBody" => PartKind::PlayerBody,
        "BoplBody" => PartKind::BoplBody,
        _ => return None,
    })
}

/// Parts of a prefab object: the standard components and the module
/// components named in `module` (class → part, a class listed n times maps
/// its n-th occurrence to the n-th entry), with their hierarchy numbers and
/// enabled flags; `collider` adds the DPhysics collider. Components without
/// a hierarchy number and `Ability` are skipped.
pub fn parts_of(
    o: &PrefabObject,
    module: &[(&str, PartKind)],
    collider: bool,
) -> Vec<(PartKind, i32, bool)> {
    let mut out = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for c in &o.components {
        let Some(h) = c.hierarchy else {
            continue;
        };
        if c.class == "Ability" {
            continue;
        }
        let nth = seen.iter().filter(|s| **s == c.class.as_str()).count();
        seen.push(c.class.as_str());
        let k = module
            .iter()
            .filter(|(n, _)| *n == c.class)
            .nth(nth)
            .map(|(_, k)| *k)
            .or_else(|| standard_part(&c.class));
        if let Some(k) = k {
            out.push((k, h, c.enabled));
        }
    }
    if collider
        && o.collider.is_some()
        && let Some(h) = o.collider_hier
    {
        out.push((PartKind::Collider, h, o.collider_enabled));
    }
    out.sort_by_key(|p| p.1);
    out
}

/// `build` of a per-part ability module: adds the prefab root's component
/// parts (standard ones plus `module`) to the ability object, and the
/// framework's `PlayerCollision` state if the root has one.
pub fn add_parts(
    w: &mut World,
    me: ObjId,
    fw: &mut Framework,
    def: &AbilityDef,
    module: &[(&str, PartKind)],
) {
    let Some(root) = prefab::ability_root(w, def).cloned() else {
        return;
    };
    let parts = parts_of(&root, module, false);
    if root.component("PlayerCollision").is_some() && fw.collision.is_none() {
        fw.collision = Some(PlayerCollision {
            ignored_beam_id: -1,
            ..PlayerCollision::default()
        });
    }
    if let Some(o) = w.objects.get_mut(me) {
        for (k, h, en) in parts {
            if o.part(k).is_none() || matches!(k, PartKind::Behaviour(_)) {
                let i = o.add_part(k, h);
                o.parts[i].enabled = en;
            }
        }
    }
}

/// `FixTransform.InstantiateFixed(prefab, pos[, rot])` of a prefab whose
/// simulated part is its root object (engine.md §3, §6.3): one object with
/// the given payload, the root's layer, transform offset and collider, and
/// one part per `(component class, part kind)` with that component's
/// hierarchy number and enabled flag (looked up on the root, else anywhere
/// in the prefab; [`COLLIDER`] = the root collider). The prefab's other
/// updatables consume their numbers.
pub fn spawn_prefab(
    w: &mut World,
    prefab: usize,
    kind: Kind,
    parts: &[(&str, PartKind)],
    pos: Vec2,
    rot: Option<Fix>,
) -> ObjId {
    let Some(p) = w.content.prefabs.get(prefab) else {
        return ObjId::NONE;
    };
    let Some(root) = p.objects.first() else {
        return ObjId::NONE;
    };
    let mut list = Vec::new();
    for (class, pk) in parts {
        if *class == COLLIDER {
            if root.collider.is_some()
                && let Some(h) = root.collider_hier
            {
                list.push((*pk, h, root.collider_enabled));
            }
        } else if let Some(c) = root
            .component(class)
            .or_else(|| p.find_component(class).map(|(_, c)| c))
        {
            list.push((*pk, c.hierarchy.unwrap_or(0), c.enabled));
        }
    }
    spawn_with_parts(w, prefab, kind, &list, pos, rot)
}

/// The parts of a prefab's root object for [`spawn_with_parts`] (standard
/// components, `module` components and the collider).
pub fn prefab_parts(
    w: &World,
    prefab: usize,
    module: &[(&str, PartKind)],
) -> Vec<(PartKind, i32, bool)> {
    w.content
        .prefabs
        .get(prefab)
        .and_then(|p| p.objects.first())
        .map_or_else(Vec::new, |o| parts_of(o, module, true))
}

/// [`spawn_with_parts`] for an object owned by a player (`player_id` set
/// before the object's components initialise).
pub fn spawn_owned(
    w: &mut World,
    prefab: usize,
    kind: Kind,
    parts: &[(PartKind, i32, bool)],
    pos: Vec2,
    rot: Option<Fix>,
    player_id: i32,
) -> ObjId {
    let id = spawn_with_parts(w, prefab, kind, parts, pos, rot);
    if let Some(o) = w.objects.get_mut(id) {
        o.player_id = player_id;
    }
    id
}

/// [`spawn_prefab`] with an explicit part list `(kind, hierarchy, enabled)`.
pub fn spawn_with_parts(
    w: &mut World,
    prefab: usize,
    kind: Kind,
    parts: &[(PartKind, i32, bool)],
    pos: Vec2,
    rot: Option<Fix>,
) -> ObjId {
    let Some(p) = w.content.prefabs.get(prefab) else {
        return ObjId::NONE;
    };
    let total = p.updatable_count();
    let Some(root) = p.objects.first().cloned() else {
        return ObjId::NONE;
    };
    w.open_bracket();
    let id = w.objects.create(kind, root.layer);
    if let Some(o) = w.objects.get_mut(id) {
        o.visual = Visual::Prefab {
            prefab: prefab as u32,
            object: 0,
        };
        if let Some(t) = root.fix {
            o.tf.offset = t.offset;
        }
        // `BoplBody.Awake`: `isResizable` from the tag.
        o.bb.resizable = root.tag == "ResizablePlatform";
        for &(k, h, en) in parts {
            if k == PartKind::Collider {
                o.collider = root.collider;
            }
            let i = o.add_part(k, h);
            o.parts[i].enabled = en;
        }
    }
    let n = w.objects.get(id).map_or(0, |o| o.n_parts as i32);
    w.register_all(id);
    w.close_bracket();
    w.consume_hierarchy((total - n).max(0));
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.position = pos;
        match rot {
            Some(r) => o.tf.set_rotation(r),
            None => o.tf.reset_rotation(),
        }
    }
    id
}

// -- animators ---------------------------------------------------------------------------

/// A fresh animator with the `AlwaysStopDuringTimeStop` flag.
pub fn animator(always_stop: bool) -> SpriteAnimator {
    SpriteAnimator {
        always_stop_during_time_stop: always_stop,
        ..SpriteAnimator::default()
    }
}

/// One `SpriteAnimator.UpdateSim` of an animator owned by player `owner`
/// (engine.md §10a time-stop rule); returns the callback to dispatch.
pub fn advance_anim(w: &World, owner: i32, anim: &mut SpriteAnimator) -> Option<AnimCallback> {
    let advance = if w.is_time_stopped() {
        !anim.always_stop_during_time_stop
            && w.player(owner)
                .is_some_and(|p| p.is_protected_from_time_stop)
    } else {
        true
    };
    if !advance {
        return None;
    }
    anim.advance(&w.content, Fix::ZERO)
}

/// [`advance_anim`] returning the module code of an `Ability` callback.
pub fn advance_code(w: &World, owner: i32, anim: &mut SpriteAnimator) -> Option<u16> {
    match advance_anim(w, owner, anim) {
        Some(AnimCallback::Ability(c)) => Some(c),
        _ => None,
    }
}

/// `beginAnimThenDoAction(a, code)`.
pub fn play_then(w: &World, anim: &mut SpriteAnimator, a: AnimRef, code: u16) {
    anim.begin_then_do(&w.content, a, AnimCallback::Ability(code));
}

/// `beginAnimThenDoAction(name, code)` by animation name.
pub fn play_named_then(w: &World, anim: &mut SpriteAnimator, data: u16, name: &str, code: u16) {
    let a = prefab::anim(w, data, name);
    play_then(w, anim, a, code);
}

/// `beginAnimThenPlayAnim(a, b, code)`.
pub fn play_then_loop(
    w: &World,
    anim: &mut SpriteAnimator,
    a: AnimRef,
    b: AnimRef,
    code: Option<u16>,
) {
    anim.begin_then_play(&w.content, a, b, code.map(AnimCallback::Ability));
}

/// `beginAnimation(a)` (looped).
pub fn play(w: &World, anim: &mut SpriteAnimator, a: AnimRef) {
    anim.begin(&w.content, a);
}

/// `beginAnimation(name)` by animation name.
pub fn play_named(w: &World, anim: &mut SpriteAnimator, data: u16, name: &str) {
    let a = prefab::anim(w, data, name);
    play(w, anim, a);
}

// -- BoplBody ------------------------------------------------------------------------------

/// `BoplBody.UpdateSim` of any object (physics.md §2.4, magnet-gun.md): the
/// hold counter, then the inverse-mass rescaling on scale changes. Called
/// by the updater at the object's `PartKind::BoplBody` part.
pub fn bopl_body_update(w: &mut World, me: ObjId) {
    use crate::physics::shapes::ShapeKind;
    let Some(o) = w.objects.get_mut(me) else {
        return;
    };
    o.bb.tick_hold();
    let scale = o.tf.scale;
    let bb = o.bb;
    if bb.resizable || bb.prev_scale == scale {
        return;
    }
    let Some(bd) = o.collider.filter(|c| c.initialized).and_then(|c| c.body) else {
        return;
    };
    if bd.mass == Fix::ZERO || bd.moment_of_inertia == Fix::ZERO {
        return;
    }
    let composite_part = w
        .physics
        .find(me)
        .is_some_and(|(k, _)| k != ShapeKind::RoundedRect)
        && own_body(w, me).is_some_and(|b| b.composite.is_some());
    if !composite_part {
        let k = bb.prev_scale / scale;
        // `physicsCollider.inverseMass` (a rounded rect in a composite
        // writes the composite's combined body).
        if let Some(b) = w.physics.body_mut(me) {
            b.inverse_mass = b.inverse_mass * k;
            b.inverse_moi = b.inverse_moi * k;
        }
    }
    if let Some(o) = w.objects.get_mut(me) {
        o.bb.prev_scale = scale;
    }
}

/// `BoplBody.IsBeingHeld()` of an object.
pub fn is_held(w: &World, obj: ObjId) -> bool {
    w.objects.get(obj).is_some_and(|o| o.bb.is_held())
}

/// `BoplBody.magnetGunHolder` while held.
pub fn holder_of(w: &World, obj: ObjId) -> Option<ObjId> {
    w.objects
        .get(obj)
        .filter(|o| o.bb.is_held())
        .and_then(|o| o.bb.holder)
}

// -- physics helpers -----------------------------------------------------------------------

/// `ManualInit()` of an object's collider (joins the physics world now).
pub fn manual_init(w: &mut World, me: ObjId) {
    let hier = collider_hier(w, me);
    w.physics.register(&mut w.objects, me, hier);
}

/// Whether the object's collider has joined the physics world.
pub fn collider_initialized(w: &World, me: ObjId) -> bool {
    w.objects
        .get(me)
        .and_then(|o| o.collider)
        .is_some_and(|c| c.initialized)
}

/// Hierarchy number of an object's collider (its physics parent number).
pub fn collider_hier(w: &World, obj: ObjId) -> i32 {
    w.objects
        .get(obj)
        .and_then(|o| o.part(PartKind::Collider))
        .map_or(0, |p| p.hier)
}

/// `DetPhysics.IsActive(collider)` (physics.md §3.1).
pub fn collider_active(w: &World, obj: ObjId) -> bool {
    w.objects
        .get(obj)
        .is_some_and(|o| o.part(PartKind::Collider).is_some_and(|p| p.enabled))
        && w.objects.active_in_hierarchy(obj)
}

/// The collider's own body (never redirected to its composite).
pub fn own_body(w: &World, obj: ObjId) -> Option<crate::physics::Body> {
    use crate::physics::shapes::ShapeKind;
    let (k, i) = w.physics.find(obj)?;
    Some(match k {
        ShapeKind::Circle => w.physics.circles.bodies[i],
        ShapeKind::Box => w.physics.boxes.bodies[i],
        _ => w.physics.rrs.bodies[i],
    })
}

/// The collider shape of an object as a contact snapshot.
pub fn shape_snap(w: &World, obj: ObjId) -> Option<ShapeSnap> {
    use crate::physics::shapes::ShapeKind;
    match w.physics.find(obj)? {
        (ShapeKind::Circle, i) => Some(ShapeSnap::Circle(w.physics.circles.shapes[i])),
        (ShapeKind::Box, i) => Some(ShapeSnap::Box(w.physics.boxes.shapes[i])),
        (_, i) => Some(ShapeSnap::RoundedRect(w.physics.rrs.shapes[i])),
    }
}

/// Invokes another object's collision callback with a synthetic contact.
pub fn touch(w: &mut World, c: Contact) {
    w.dispatch_contact(c);
}

/// `SetRelativePositionOfChildBody(relPos, relRight)` (box or circle):
/// `relativePos = relPos + ComplexMul(relRight, transform.offset)`.
pub fn set_relative_position(w: &mut World, obj: ObjId, rel: Vec2, right: Vec2) {
    let off = w.objects.get(obj).map_or(Vec2::ZERO, |o| o.tf.offset);
    let v = rel + right.complex_mul(off);
    if let Some(b) = w.physics.own_body_mut(obj) {
        b.relative_pos = v;
    }
}

/// `SetRelativeOrientationOfChildBody(r)`.
pub fn set_relative_orientation(w: &mut World, obj: ObjId, r: Fix) {
    if let Some(b) = w.physics.own_body_mut(obj) {
        b.relative_orientation = r;
    }
}

/// `gameObject.layer = l; collider.UpdateLayer()`.
pub fn set_layer(w: &mut World, obj: ObjId, l: u8) {
    use crate::physics::shapes::ShapeKind;
    if let Some(o) = w.objects.get_mut(obj) {
        o.layer = l;
    }
    if let Some((k, i)) = w.physics.find(obj) {
        match k {
            ShapeKind::Circle => w.physics.circles.shapes[i].layer = l as i32,
            ShapeKind::Box => w.physics.boxes.shapes[i].layer = l as i32,
            _ => w.physics.rrs.shapes[i].layer = l as i32,
        }
    }
}

/// `AddForceAtPosition(f, pos, mode)` (physics.md §2.3): Impulse mode
/// multiplies by 60; either way an instantaneous impulse with lever
/// `NormalizedSafe(pos − position)` (composite aware).
pub fn add_force_at_position(w: &mut World, obj: ObjId, f: Vec2, at: Vec2, impulse: bool) {
    let Some(p) = w.objects.get(obj).map(|o| o.tf.position) else {
        return;
    };
    let f = if impulse { f * Fix::from_int(60) } else { f };
    let lever = (at - p).normalized_safe();
    w.physics.apply_impulse(obj, f, lever);
}

/// `GetComponent<IPlatformAttachable>() != null`: the object was built from
/// a prefab whose root carries one of the attachable components (arrows,
/// rocket engines, rope anchors, spark nodes, spikes).
pub fn is_platform_attachable(w: &World, obj: ObjId) -> bool {
    prefab::prefab_object(w, obj).is_some_and(|p| {
        p.components.iter().any(|c| {
            matches!(
                c.class.as_str(),
                "Arrow" | "RocketEngine" | "RopeAttachment" | "SimpleSparkNode" | "SpikeAttack"
            )
        })
    })
}

/// `SceneBounds.WaterHeight` of the current level.
pub fn water_height(w: &World) -> Fix {
    if w.is_space() {
        SPACE_WATER_HEIGHT
    } else {
        WATER_HEIGHT
    }
}

/// `SceneBounds.IsInSceneBounds(p)` (camera box).
pub fn in_scene_bounds(p: Vec2) -> bool {
    p.x >= CAMERA_X_MIN && p.x <= CAMERA_X_MAX && p.y >= CAMERA_Y_MIN && p.y <= CAMERA_Y_MAX
}

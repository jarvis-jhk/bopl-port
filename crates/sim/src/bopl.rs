//! The bopl: `PlayerPhysics`, `SlimeController`, `PlayerCollision`,
//! `DestroyIfOutsideSceneBounds`, `PlayerBody` and its `SpriteAnimator`
//! (player.md §4–§9, engine.md §7.1).

use crate::abilities::{self, AbilityExitInfo, PlayerInfo};
use crate::anim::{AnimCallback, SpriteAnimator};
use crate::consts::*;
use crate::data::{AnimRef, PlayerPrefab};
use crate::fix::Fix;
use crate::object::{ObjId, PartKind};
use crate::physics::Contact;
use crate::player::{CauseOfDeath, ExtraLife};
use crate::surface::{Surface, move_along_surface};
use crate::vec2::Vec2;
use crate::view::{Cue, EffectKind, Visual};
use crate::world::{Kind, World};

/// `SlimeController.State`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum State {
    Idle = 0,
    Run = 1,
    #[default]
    Fall = 2,
    Walk = 3,
    Land = 4,
    Ability = 5,
    Turn = 6,
    Stop = 7,
    Nothing = 8,
    Biting = 9,
    Ragdoll = 10,
}

/// `PlayerPhysics`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PlayerPhysics {
    pub is_grounded: bool,
    /// Not cleared when ungrounding.
    pub attached_ground: Option<ObjId>,
    pub local_platform_pos: Fix,
    pub grounded_speed: Fix,
    pub radius: Fix,
    pub prev_scale: Fix,
    pub jumped_this_frame: bool,
    pub skip_next_update: bool,
}

/// A rope held by the body (rope system: grappling-hook module).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RopeLink {
    pub rope: u16,
    pub top: bool,
}

/// `PlayerBody`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PlayerBody {
    pub self_imposed_velocity: Vec2,
    pub external_velocity: Vec2,
    pub simulated: bool,
    pub prev_position: Vec2,
    pub prev_player_vel: Vec2,
    pub prev_imposed_vel: Vec2,
    pub prev_external_vel: Vec2,
    pub rope: Option<RopeLink>,
    pub extra_swing_force: Vec2,
}

impl PlayerBody {
    /// `Velocity = externalVelocity + selfImposedVelocity`.
    pub fn velocity(&self) -> Vec2 {
        self.external_velocity + self.self_imposed_velocity
    }
}

/// `SlimeController`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SlimeController {
    pub player_number: i32,
    pub state: State,
    pub input_vector: Vec2,
    pub prev_input: Fix,
    pub suppress_input: bool,
    pub has_spawned: bool,
    pub is_in_ability: bool,
    pub is_abilityless_clone: bool,
    /// Ability objects per slot.
    pub abilities: Vec<ObjId>,
    pub cooldown_timers: Vec<Fix>,
    pub was_upside_down: bool,
    pub time_in_ragdoll: Fix,
    pub ragdoll_duration: Fix,
    pub player_to_eat: Option<ObjId>,
    pub swallowing: bool,
    /// `FlippingTransform` facing (initially right).
    pub facing_right: bool,
    pub was_invisible: bool,
    pub was_invisible_when_entered_ability: bool,
    pub next_slot_on_pickup: i32,
    pub magnet_holder: Option<ObjId>,
    /// `ropeCastBuffer`: the last rope target found (kept when none is found).
    pub rope_target: Option<ObjId>,
    /// Number of HUD indicators (3 for spawned players, 0 for clones).
    pub indicators: usize,
}

/// `PlayerCollision`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PlayerCollision {
    pub was_killed_this_frame: bool,
    /// Invulnerability bits: 1 zone, 2 blocking, 4 spawn/revive.
    pub invulnerable: u32,
    /// Ignored colliders (meteor, magnet, roll, combo 0–2).
    pub ignored: [Option<ObjId>; 6],
    pub ignored_beam_id: i32,
}

impl PlayerCollision {
    pub fn is_invulnerable(&self) -> bool {
        self.invulnerable != 0
    }

    pub fn set_invulnerable(&mut self, on: bool, bit: u32) {
        if on {
            self.invulnerable |= bit;
        } else {
            self.invulnerable &= !bit;
        }
    }
}

/// The bopl object payload.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Bopl {
    pub physics: PlayerPhysics,
    pub ctrl: SlimeController,
    pub collision: PlayerCollision,
    pub body: PlayerBody,
    pub anim: SpriteAnimator,
}

/// Animation names of the bopl's `MCAnimData`.
fn anim(w: &World, name: &str) -> AnimRef {
    let p = w.content.player();
    w.content.anim_ref(p.anim_data, name).unwrap_or(AnimRef {
        data: p.anim_data,
        anim: 0,
    })
}

fn speed_map(w: &World, name: &str) -> u16 {
    let p = w.content.player();
    w.content.anim_data[p.anim_data as usize]
        .speed_animations
        .iter()
        .position(|a| a.name == name)
        .unwrap_or(0) as u16
}

impl Bopl {
    fn cfg(w: &World) -> &PlayerPrefab {
        w.content.player()
    }

    /// Creates a bopl (`InstantiateFixed(PlayerPrefab, pos)`) for a player.
    /// Ability objects are spawned separately by the caller.
    pub fn spawn(w: &mut World, pos: Vec2, player_id: i32) -> ObjId {
        let cfg = w.content.player().clone();
        let bopl = Bopl {
            physics: PlayerPhysics {
                radius: cfg.radius,
                prev_scale: Fix::ONE,
                ..PlayerPhysics::default()
            },
            ctrl: SlimeController {
                player_number: player_id,
                // Awake: state fall, input suppressed.
                state: State::Fall,
                suppress_input: true,
                facing_right: true,
                cooldown_timers: vec![Fix::ZERO; 3],
                indicators: 3,
                ..SlimeController::default()
            },
            collision: PlayerCollision {
                ignored_beam_id: -1,
                ..PlayerCollision::default()
            },
            // The prefab has `simulated = 1`; `BeforeSpawnInit` clears it at Init.
            body: PlayerBody {
                simulated: true,
                ..PlayerBody::default()
            },
            anim: SpriteAnimator::default(),
        };
        w.open_bracket();
        let id = w.objects.create(Kind::Bopl(Box::new(bopl)), cfg.layer);
        {
            let o = w.objects.get_mut(id).unwrap();
            o.player_id = player_id;
            o.visual = Visual::Bopl;
            o.collider = Some(cfg.hurtbox);
            o.add_part(PartKind::PlayerPhysics, cfg.hier_physics);
            o.add_part(PartKind::SlimeController, cfg.hier_controller);
            o.add_part(PartKind::PlayerCollision, cfg.hier_collision);
            o.add_part(PartKind::Bounds, cfg.hier_bounds);
            o.add_part(PartKind::PlayerBody, cfg.hier_body);
            o.add_part(PartKind::Collider, cfg.hier_hurtbox);
            o.add_part(PartKind::Animator, cfg.hier_animator);
        }
        w.register_all(id);
        w.close_bracket();
        if let Some(o) = w.objects.get_mut(id) {
            o.tf.position = pos;
            o.tf.reset_rotation();
        }
        id
    }

    // -- dispatch -------------------------------------------------------------

    pub fn init(&mut self, w: &mut World, me: ObjId, k: PartKind) {
        match k {
            PartKind::SlimeController => {
                self.ctrl.next_slot_on_pickup = 0;
                self.ctrl.cooldown_timers = vec![F_NEG_1_5; 3];
                if !self.ctrl.is_abilityless_clone {
                    self.before_spawn_init(w, me);
                }
            }
            PartKind::PlayerBody => {
                self.body.prev_player_vel = self.body.velocity();
                self.body.prev_imposed_vel = self.body.self_imposed_velocity;
                self.body.prev_external_vel = self.body.external_velocity;
            }
            _ => {}
        }
    }

    pub fn update(&mut self, w: &mut World, me: ObjId, k: PartKind, dt: Fix, late: bool) {
        match (k, late) {
            (PartKind::PlayerPhysics, false) => self.physics_update(w, me, dt),
            (PartKind::PlayerPhysics, true) => self.physics_late_update(w, me),
            (PartKind::SlimeController, false) => self.controller_update(w, me, dt),
            (PartKind::PlayerCollision, false) => self.collision.was_killed_this_frame = false,
            (PartKind::Bounds, false) => self.bounds_update(w, me),
            (PartKind::PlayerBody, false) => self.body_update(w, me, dt),
            (PartKind::PlayerBody, true) => self.body_late_update(w, me),
            (PartKind::Animator, false) => self.animator_update(w, me),
            _ => {}
        }
    }

    // -- helpers ----------------------------------------------------------------

    fn pos(w: &World, me: ObjId) -> Vec2 {
        w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position)
    }

    fn set_pos(w: &mut World, me: ObjId, p: Vec2) {
        if let Some(o) = w.objects.get_mut(me) {
            o.tf.position = p;
        }
    }

    fn set_rotation(w: &mut World, me: ObjId, r: Fix) {
        if let Some(o) = w.objects.get_mut(me) {
            o.tf.set_rotation(r);
        }
    }

    fn id(&self) -> i32 {
        self.ctrl.player_number
    }

    /// The attached platform if it is alive (Unity `!= null`).
    fn ground(&self, w: &World) -> Option<ObjId> {
        self.physics
            .attached_ground
            .filter(|g| w.objects.exists(*g))
    }

    fn ground_alive(&self, w: &World) -> Option<ObjId> {
        self.physics.attached_ground.filter(|g| w.objects.alive(*g))
    }

    fn surface(&self, w: &World) -> Option<Surface> {
        self.ground(w).and_then(|g| Surface::of(w, g))
    }

    /// `currentNormal(body)` of the attached ground.
    fn current_normal(&self, w: &World, me: ObjId) -> Option<Vec2> {
        let s = self.surface(w)?;
        Some(s.normal(Self::pos(w, me)))
    }

    fn hurtbox_enabled(w: &World, me: ObjId) -> bool {
        w.objects
            .get(me)
            .is_some_and(|o| o.part_enabled(PartKind::Collider))
    }

    fn set_hurtbox_enabled(w: &mut World, me: ObjId, e: bool) {
        w.set_collider_enabled(me, e);
    }

    fn set_physics_enabled(w: &mut World, me: ObjId, e: bool) {
        if let Some(o) = w.objects.get_mut(me) {
            o.set_part_enabled(PartKind::PlayerPhysics, e);
        }
    }

    // -- PlayerPhysics ------------------------------------------------------------

    fn update_scale(&mut self, w: &World, me: ObjId) {
        let s = w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale);
        if self.physics.prev_scale != s {
            let f = s / self.physics.prev_scale;
            self.physics.radius = self.physics.radius * f;
            self.physics.prev_scale = s;
        }
    }

    fn physics_update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        if self.physics.skip_next_update {
            self.physics.skip_next_update = false;
            return;
        }
        self.update_scale(w, me);
        if !self.physics.is_grounded && Self::cfg(w).apply_gravity {
            self.add_gravity_factor(w);
            self.velocity_based_raycasts(w, me, true, dt);
        }
    }

    /// `AddGravityFactor()` (per tick, no `dt`).
    fn add_gravity_factor(&mut self, w: &World) {
        let c = Self::cfg(w);
        let g = if w.is_space() {
            c.gravity_modifier * Fix(2_147_483_648)
        } else {
            c.gravity_modifier
        };
        let v = &mut self.body.self_imposed_velocity;
        *v += Vec2::DOWN * c.gravity_accel * g;
        let k = c.gravity_accel * g / c.gravity_max_fall_speed;
        let vy = v.y;
        *v += Vec2::DOWN * k * vy;
    }

    /// `AirealMovement(h)`.
    fn aireal_movement(&mut self, w: &World, h: Fix) {
        let c = Self::cfg(w);
        let v = &mut self.body.self_imposed_velocity;
        *v += Vec2::new(h * c.air_accel, Fix::ZERO);
        let k = c.air_accel / (c.max_speed + c.air_accel);
        let vx = v.x;
        *v += Vec2::LEFT * k * vx;
    }

    /// `VelocityBasedRaycasts(attach, dt)` = `CollisionRaycasts(attach, Velocity, dt)`.
    pub fn velocity_based_raycasts(
        &mut self,
        w: &mut World,
        me: ObjId,
        attach: bool,
        dt: Fix,
    ) -> bool {
        let v = self.body.velocity();
        self.collision_raycasts(w, me, attach, v, dt)
    }

    fn collision_raycasts(
        &mut self,
        w: &mut World,
        me: ObjId,
        attach: bool,
        v: Vec2,
        dt: Fix,
    ) -> bool {
        let s = v.magnitude();
        let len = s * w.fixed_delta_time(self.id(), dt);
        let pos = Self::pos(w, me);
        let d = if s.0 == 0 { Vec2::ZERO } else { v / s };
        let n = Vec2::new(-d.y, d.x);
        let r = self.physics.radius;
        let probes = [
            (pos + d * r, Vec2::ZERO),
            (pos - n * r, n * r),
            (pos + n * r, -(n * r)),
        ];
        for (origin, off) in probes {
            let hit = w.physics.raycast_closest_rr(&w.objects, origin, d, len);
            if hit.hit() {
                if attach {
                    let loc = hit.near_pos + off;
                    self.collide_with_ground(w, me, hit.obj, loc);
                }
                return true;
            }
        }
        false
    }

    fn collide_with_ground(&mut self, w: &mut World, me: ObjId, plat: ObjId, loc: Vec2) {
        if Surface::of(w, plat).is_none() {
            // A rounded rect without a walkable surface (not in the data).
            return;
        }
        self.attach_to_ground(w, me, plat, loc);
        // OnAttachedToGround → SlimeController.attachToGround.
        if Self::hurtbox_enabled(w, me) && self.body.simulated {
            self.on_land(w, me, plat);
        }
    }

    /// `AttachToGround(platform, location)`.
    pub fn attach_to_ground(&mut self, w: &mut World, me: ObjId, plat: ObjId, loc: Vec2) {
        self.physics.attached_ground = Some(plat);
        self.physics.is_grounded = true;
        let Some(s) = Surface::of(w, plat) else {
            return;
        };
        let n = s.normal(Self::pos(w, me));
        self.physics.grounded_speed = self.body.self_imposed_velocity.cross(n);
        let r = self.physics.radius;
        self.physics.local_platform_pos = s.param_from_position(loc, r);
        let p = s.position_from_param(self.physics.local_platform_pos, r);
        Self::set_pos(w, me, p);
        self.body.self_imposed_velocity = Vec2::ZERO;
        self.body.external_velocity = Vec2::ZERO;
        self.align_rotation(w, me);
    }

    /// `alignRotation(body)`: `up = currentNormal`.
    fn align_rotation(&mut self, w: &mut World, me: ObjId) {
        if let Some(n) = self.current_normal(w, me)
            && let Some(o) = w.objects.get_mut(me)
        {
            o.tf.set_up(n);
        }
    }

    /// `UnGround(nullRotation, sound)`.
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
        self.on_no_longer_grounded(w, me);
        if sound {
            w.emit(Cue::PlayerSound("slimeJump", self.id()));
        }
    }

    fn input_ground_alignment(&self, w: &World, me: ObjId, input: Vec2) -> Fix {
        let u = match self.surface(w) {
            Some(s) => s.forward(Self::pos(w, me)),
            None => Vec2::RIGHT,
        };
        input.dot(u)
    }

    fn increment_towards(c: Fix, target: Fix, a: Fix, h: Fix) -> Fix {
        let c = if c * target < Fix::ZERO { Fix::ZERO } else { c };
        let n = c + h * target.sign_fix() * a;
        if n.abs() > target.abs() { target } else { n }
    }

    /// `PlayerPhysics.Move(input, dt)` (player.md §6.4).
    pub fn move_(&mut self, w: &mut World, me: ObjId, input: Vec2, dt: Fix, along: bool) {
        let id = self.id();
        if !self.physics.is_grounded {
            self.aireal_movement(w, input.x);
            let fdt = w.fixed_delta_time(id, dt);
            self.velocity_based_raycasts(w, me, true, fdt);
            return;
        }
        let c = Self::cfg(w).clone();
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
                dir * c.max_speed,
                c.accel,
                fdt,
            );
        }
        let live =
            ground.filter(|g| w.objects.alive(*g) && w.objects.get(*g).is_some_and(|o| o.active));
        let Some(g) = live else {
            self.unground(w, me, true, true);
            return;
        };
        if c.sync_rotation_to_platform {
            self.align_rotation(w, me);
        }
        let ext = self.external_force_along_surface(w, me);
        if along {
            let fdt = w.fixed_delta_time(id, dt);
            let pos = Self::pos(w, me);
            self.physics.local_platform_pos = move_along_surface(
                w,
                g,
                pos,
                self.physics.radius,
                self.physics.grounded_speed + ext,
                fdt,
                self.physics.local_platform_pos,
            );
        }
        self.drop_platform_test(w, me);
    }

    fn external_force_along_surface(&self, w: &World, me: ObjId) -> Fix {
        match self.surface(w) {
            Some(s) => self.body.external_velocity.dot(s.forward(Self::pos(w, me))),
            None => Fix::ZERO,
        }
    }

    fn drop_platform_test(&mut self, w: &mut World, me: ObjId) {
        let Some(g) = self.ground_alive(w) else {
            return;
        };
        if !w.objects.get(g).is_some_and(|o| o.active) {
            return;
        }
        let Some(force) = crate::platform::am_i_dropped(w, g, self.id()) else {
            return;
        };
        let n = self.current_normal(w, me).unwrap_or(Vec2::UP);
        self.unground(w, me, true, false);
        self.body.self_imposed_velocity += n * force;
        let p = Self::pos(w, me) + self.body.self_imposed_velocity / Fix::from_int(100);
        Self::set_pos(w, me, p);
    }

    /// `Jump()` (PlayerPhysics).
    fn physics_jump(&mut self, w: &mut World, me: ObjId) {
        let c = Self::cfg(w).clone();
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

    /// `PlayerPhysics.AddForce(f)`.
    pub fn physics_add_force(&mut self, w: &World, f: Vec2) {
        self.body.external_velocity += f * Self::cfg(w).inverse_mass01;
    }

    /// `PlayerBody.AddForce(f)`.
    pub fn body_add_force(&mut self, f: Vec2) {
        self.body.external_velocity += f * Fix::from_int(35);
    }

    fn physics_late_update(&mut self, w: &mut World, me: ObjId) {
        if self.physics.is_grounded
            && let Some(s) = self.ground_alive(w).and_then(|g| Surface::of(w, g))
        {
            let p = s.position_from_param(self.physics.local_platform_pos, self.physics.radius);
            Self::set_pos(w, me, p);
            if Self::cfg(w).sync_rotation_to_platform {
                self.align_rotation(w, me);
            }
            abilities::rope_grounded_late_update(w, me, self);
        }
        self.physics.jumped_this_frame = false;
    }

    // -- SlimeController ------------------------------------------------------------

    fn before_spawn_init(&mut self, w: &mut World, me: ObjId) {
        Self::set_physics_enabled(w, me, false);
        self.ctrl.suppress_input = true;
        self.body.simulated = false;
        self.body.self_imposed_velocity = Vec2::ZERO;
        self.body.external_velocity = Vec2::ZERO;
        if self.ctrl.state != State::Nothing {
            Self::set_hurtbox_enabled(w, me, true);
            let a = anim(w, "spawn");
            let c = w.content.clone();
            self.anim.begin_then_do(&c, a, AnimCallback::BoplSpawn);
        }
    }

    /// `Spawn()` (end of the spawn animation; also called directly for
    /// duplicator clones).
    pub fn spawn_done(&mut self, w: &mut World, me: ObjId) {
        Self::set_physics_enabled(w, me, true);
        self.body.simulated = true;
        Self::set_hurtbox_enabled(w, me, true);
        self.unground(w, me, true, true);
        self.body.self_imposed_velocity = Vec2::UP * Self::cfg(w).jump_strength * F_0_25;
        self.ctrl.suppress_input = false;
        self.ctrl.has_spawned = true;
        self.collision.set_invulnerable(false, 4);
        let id = self.id();
        if let Some(p) = w.player_mut(id) {
            p.extra_life = ExtraLife::NoRevive;
            p.in_spark_shot = false;
            p.in_machine_throw = false;
            p.in_platform_transform = false;
        }
        w.emit(Cue::PlayerSpawned { id });
    }

    fn controller_update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        let id = self.id();
        if self.ctrl.state == State::Ragdoll {
            self.ctrl.time_in_ragdoll += w.fixed_delta_time(id, dt);
        }
        if self.ctrl.state == State::Ragdoll
            && self.ctrl.time_in_ragdoll > self.ctrl.ragdoll_duration
        {
            self.ctrl.magnet_holder = None;
            self.body.external_velocity += self.body.self_imposed_velocity;
            self.body.self_imposed_velocity = Vec2::ZERO;
            self.enter_fall(w, me);
        }
        if self.physics.is_grounded && self.ground_alive(w).is_none() {
            Self::set_rotation(w, me, Fix::ZERO);
            self.unground(w, me, false, true);
        }
        self.old_update(w, me, dt);
        if self.ctrl.swallowing && !w.is_time_stopped_for(id) {
            let inc = Self::cfg(w).scale_increase_on_nom;
            if let Some(p) = w.player_mut(id) {
                let s = p.scale() + inc;
                p.set_scale(s);
            }
        }
        let invisible = w.player(id).is_some_and(|p| p.is_invisible);
        if !invisible && self.ctrl.was_invisible {
            abilities::invisibility_ended(w, me, self);
        }
        self.ctrl.was_invisible = invisible;
        let scale = w.player(id).map_or(Fix::ONE, |p| p.scale());
        w.set_collider_scale(me, scale);
        if matches!(
            self.ctrl.state,
            State::Nothing | State::Biting | State::Ragdoll
        ) || !self.ctrl.has_spawned
        {
            return;
        }
        let input = self.ctrl.input_vector;
        self.move_(w, me, input, dt, true);
        if !self.physics.is_grounded && abilities::rope_enabled(w, self.body.rope) {
            self.body.extra_swing_force =
                Vec2::new(input.x * Self::cfg(w).extra_swing_force, Fix::ZERO);
        }
    }

    /// The controller part of the update (`OldUpdate`, player.md §6.3).
    fn old_update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        let id = self.id();
        if w.player(id).is_none() {
            return;
        }
        let pos = Self::pos(w, me);
        set_player_position(w, id, pos, me, self.ctrl.abilities.len());
        if self.ctrl.state == State::Nothing {
            return;
        }
        let mut rope_target = false;
        if !self.ctrl.is_in_ability {
            for i in 0..self.ctrl.abilities.len() {
                let a = self.ctrl.abilities[i];
                if !w.objects.alive(a) {
                    continue;
                }
                let cd = abilities::cooldown(w, a);
                if !(w.is_time_stopped() && abilities::cooldown_stopped_during_time_stop(w, a)) {
                    let fdt = w.fixed_delta_time(id, dt);
                    if let Some(t) = self.ctrl.cooldown_timers.get_mut(i) {
                        *t += fdt;
                    }
                }
                if abilities::is_rope(w, a) {
                    let r = Self::cfg(w).rope_circle_cast_radius;
                    if let Some(t) = w.physics.circle_cast_closest_players(
                        &w.objects,
                        pos,
                        r,
                        F_0_5 + Fix::ONE,
                        me,
                    ) {
                        rope_target = true;
                        self.ctrl.rope_target = Some(t);
                    }
                    abilities::update_rope_cooldowns(w, a, me, self.body.rope);
                }
                if i >= self.ctrl.indicators {
                    continue;
                }
                if cd == Fix::ZERO
                    && let Some(t) = self.ctrl.cooldown_timers.get_mut(i)
                {
                    *t = F_0_01D;
                }
            }
        }
        if matches!(self.ctrl.state, State::Biting | State::Ragdoll) {
            return;
        }
        let (input, jp, buttons) = match w.player(id) {
            Some(p) => (
                p.inputs.input_vector(),
                p.inputs.jump_down_this_frame,
                p.inputs.input,
            ),
            None => (Vec2::ZERO, false, Default::default()),
        };
        self.ctrl.input_vector = input;
        let mut al = self.input_ground_alignment(w, me, input.normalized_safe());
        let mut jp = jp;
        let nd = if self.physics.is_grounded {
            self.current_normal(w, me).unwrap_or(Vec2::UP)
        } else {
            Vec2::UP
        }
        .dot(input);
        let mut moving = input.magnitude() >= DEADZONE
            && (al.abs() >= GROUND_ALIGNMENT_DEADZONE || (nd < Fix::ZERO && al.abs() >= F_0_01));
        if self.ctrl.suppress_input {
            moving = false;
            jp = false;
            self.ctrl.input_vector = Vec2::ZERO;
            al = Fix::ZERO;
        }
        for k in 0..self.ctrl.abilities.len() {
            let a = self.ctrl.abilities[k];
            let down = buttons.ability(k);
            if abilities::ignore_next_cooldown(w, a) && !down {
                let cd = abilities::cooldown(w, a);
                if let Some(t) = self.ctrl.cooldown_timers.get_mut(k) {
                    *t = cd;
                }
            }
            let ready = self
                .ctrl
                .cooldown_timers
                .get(k)
                .is_some_and(|t| *t > abilities::cooldown(w, a));
            if down && ready && self.is_ability_castable(w, k) && !self.ctrl.suppress_input {
                self.enter_ability(w, me, k, rope_target);
                return;
            }
        }
        match self.ctrl.state {
            State::Land => {
                if al.abs() >= GROUND_ALIGNMENT_DEADZONE || nd < Fix::ZERO {
                    self.ctrl.facing_right = al >= Fix::ZERO;
                }
            }
            State::Stop => {
                if moving && self.ctrl.facing_right != (al >= Fix::ZERO) {
                    self.enter_turn(w, al);
                }
            }
            State::Turn => {
                if !moving {
                    self.enter_stop(w);
                }
            }
            State::Idle => {
                if moving {
                    self.enter_run(w, al);
                }
            }
            State::Run if !moving || (al * self.ctrl.prev_input).sign() < 0 => {
                self.enter_stop(w);
            }
            _ => {}
        }
        if self.physics.is_grounded && jp {
            self.ctrl.was_upside_down = self.current_normal(w, me).is_some_and(|n| n.y < Fix::ZERO);
            self.physics_jump(w, me);
            // The original advances the animator once more on the jump tick.
            self.advance_animator(w, me);
        }
        self.ctrl.prev_input = al;
        if !self.physics.is_grounded {
            self.align_transform_air();
        }
    }

    /// `SlimeController.isAbilityCastable` (pure; also read by the view).
    pub(crate) fn is_ability_castable(&self, w: &World, k: usize) -> bool {
        let a = self.ctrl.abilities[k];
        let id = self.id();
        let ground_ok = !abilities::only_castable_on_ground(w, a) || self.physics.is_grounded;
        let can = w.player(id).is_some_and(|p| p.can_use_abilities);
        ground_ok && can && abilities::is_castable(w, a, id)
    }

    fn enter_stop(&mut self, w: &World) {
        self.ctrl.state = State::Stop;
        let a = anim(w, "stop");
        self.anim
            .begin_then_do(&w.content, a, AnimCallback::BoplStopDone);
    }

    fn enter_turn(&mut self, w: &World, al: Fix) {
        self.ctrl.state = State::Turn;
        self.ctrl.facing_right = al >= Fix::ZERO;
        let a = anim(w, "runTurnAround");
        self.anim
            .begin_then_do(&w.content, a, AnimCallback::BoplTurnDone(al));
    }

    fn enter_run(&mut self, w: &mut World, al: Fix) {
        self.ctrl.state = State::Run;
        self.ctrl.facing_right = al >= Fix::ZERO;
        let a = anim(w, "run");
        self.anim.begin(&w.content, a);
        w.emit(Cue::PlayerSound("slimeWalk", self.id()));
    }

    fn enter_idle(&mut self, w: &World) {
        self.ctrl.state = State::Idle;
        let a = anim(w, "idle");
        self.anim.begin(&w.content, a);
    }

    /// `EnterFall(sprite)`.
    pub fn enter_fall(&mut self, w: &mut World, me: ObjId) {
        self.align_transform_air();
        Self::set_rotation(w, me, Fix::ZERO);
        self.ctrl.state = State::Fall;
        let lp = anim(w, "fallLeftLoop");
        let map = speed_map(w, "fallLeft");
        let vy = self.body.velocity().y;
        let c = w.content.clone();
        let data = c.player().anim_data;
        self.anim.begin_speed_y(&c, data, map, lp, vy);
        self.velocity_based_raycasts(w, me, true, STANDARD_FIXED_DT);
    }

    fn align_transform_air(&mut self) {
        if self.ctrl.was_upside_down {
            self.ctrl.facing_right = !self.ctrl.facing_right;
            self.ctrl.was_upside_down = false;
        }
        if self.ctrl.input_vector.x < -MIN_TURNSPEED {
            self.ctrl.facing_right = false;
        } else if self.ctrl.input_vector.x > MIN_TURNSPEED {
            self.ctrl.facing_right = true;
        }
    }

    /// The landing event (`attachToGround(platform)`).
    fn on_land(&mut self, w: &mut World, me: ObjId, plat: ObjId) {
        self.ctrl.state = State::Land;
        let (l, i) = (anim(w, "land"), anim(w, "idle"));
        self.anim
            .begin_then_play(&w.content, l, i, Some(AnimCallback::BoplLandDone));
        w.emit(Cue::PlayerSound("slimeLand", self.id()));
        if let Some(s) = Surface::of(w, plat)
            && s.normal(Self::pos(w, me)).y < Fix::ZERO
        {
            self.ctrl.facing_right = !self.ctrl.facing_right;
        }
    }

    /// `OnNoLongerGrounded`.
    fn on_no_longer_grounded(&mut self, w: &World, _me: ObjId) {
        self.ctrl.state = State::Fall;
        if self.body.simulated {
            let lp = anim(w, "fallLeftLoop");
            let map = speed_map(w, "fallLeft");
            let data = w.content.player().anim_data;
            let vy = self.body.velocity().y;
            self.anim.begin_speed_y(&w.content, data, map, lp, vy);
        }
    }

    /// `ApplyRagDoll(duration, v, fast, resetAnim)`.
    pub fn apply_ragdoll(
        &mut self,
        w: &mut World,
        me: ObjId,
        duration: Fix,
        v: Vec2,
        fast: bool,
        reset_anim: bool,
    ) {
        if reset_anim {
            let a = anim(w, if fast { "ragdollFast" } else { "ragdollSlow" });
            self.anim.begin(&w.content, a);
        }
        self.ctrl.state = State::Ragdoll;
        let mut s = v;
        s.x = s.x / (Fix::THREE / Fix::TWO);
        self.ctrl.ragdoll_duration = duration;
        self.ctrl.time_in_ragdoll = Fix::ZERO;
        self.body.self_imposed_velocity = s;
        let n = v.normalized_safe();
        if n != Vec2::ZERO
            && let Some(o) = w.objects.get_mut(me)
        {
            o.tf.set_right(n);
        }
    }

    // -- abilities (enter / exit) -------------------------------------------------

    fn enter_ability(&mut self, w: &mut World, me: ObjId, k: usize, rope_target: bool) {
        let a = self.ctrl.abilities[k];
        let grounded = self.physics.is_grounded;
        let pos = Self::pos(w, me);
        let surf = self.surface(w);
        let mut fwd = match (grounded, surf) {
            (true, Some(s)) => s.forward(pos),
            _ => {
                Vec2::RIGHT
                    * if self.ctrl.facing_right {
                        Fix::ONE
                    } else {
                        -Fix::ONE
                    }
            }
        };
        let self_imposed = if grounded {
            self.physics.grounded_speed * fwd
        } else {
            self.body.self_imposed_velocity
        };
        let rope = self.body.rope;
        let instant = abilities::is_instant(w, a);
        if !instant {
            // `EnterAbilityClearRope`: remember the velocities, drop the link.
            self.body.prev_player_vel = self.body.velocity();
            self.body.prev_imposed_vel = self.body.self_imposed_velocity;
            self.body.prev_external_vel = self.body.external_velocity;
            self.body.rope = None;
        }
        let up = match (grounded, surf) {
            (true, Some(s)) => s.normal(pos).normalized(),
            _ => Vec2::UP,
        };
        if !self.ctrl.facing_right {
            fwd = fwd * (-Fix::ONE);
        }
        let rotation = w.objects.get(me).map_or(Fix::ZERO, |o| o.tf.rotation);
        let info = PlayerInfo {
            player_id: self.id(),
            slot: k,
            position: pos,
            external_velocity: self.body.external_velocity,
            self_imposed_velocity: self_imposed,
            is_grounded: grounded,
            grounded_speed: self.physics.grounded_speed,
            rotation,
            forwards: fwd,
            up,
            facing_right: self.ctrl.facing_right,
            ground: self.physics.attached_ground,
            grounded_local_position: if grounded {
                self.physics.local_platform_pos
            } else {
                Fix::ZERO
            },
            radius: self.physics.radius,
            rope,
            use_rope_on_other_player: rope_target,
            rope_target: self.ctrl.rope_target,
            controller: me,
            collision_ignored: self.collision.ignored,
        };
        if !w.objects.alive(a) {
            self.ctrl.cooldown_timers[k] = Fix::ZERO;
            return;
        }
        let id = self.id();
        if let Some(p) = w.player_mut(id) {
            p.in_spark_shot = false;
            p.in_machine_throw = false;
            p.in_platform_transform = false;
        }
        if !instant {
            self.ctrl.is_in_ability = true;
            let inv = if let Some(p) = w.player_mut(id) {
                p.time_since_entered_ability = Fix::ZERO;
                p.is_invisible
            } else {
                false
            };
            self.ctrl.was_invisible_when_entered_ability = inv;
            if let Some(o) = w.objects.get_mut(me) {
                o.active = false;
            }
        }
        abilities::enter(w, a, info);
        // An ability that ended inside its enter call (e.g. Random, stubs).
        abilities::apply_pending_exits(w, me, self);
        if let Some(t) = self.ctrl.cooldown_timers.get_mut(k) {
            *t = Fix::ZERO;
        }
    }

    /// `onExitAbility(info)`.
    pub fn on_exit_ability(&mut self, w: &mut World, me: ObjId, info: AbilityExitInfo) {
        if !w.objects.exists(me) {
            return;
        }
        if let Some(o) = w.objects.get_mut(me) {
            o.active = true;
        }
        let id = self.id();
        let invisible = if let Some(p) = w.player_mut(id) {
            p.time_since_entered_ability = Fix::ZERO;
            p.is_invisible
        } else {
            false
        };
        if self.ctrl.was_invisible_when_entered_ability && !invisible {
            abilities::invisibility_ended(w, me, self);
        }
        let scale = w.player(id).map_or(Fix::ONE, |p| p.scale());
        w.set_collider_scale(me, scale);
        if let Some(p) = w.player_mut(id) {
            p.in_spark_shot = false;
            p.in_machine_throw = false;
            p.in_platform_transform = false;
        }
        if let Some(r) = info.rope {
            self.body.rope = Some(r);
        }
        if info.is_dead {
            Self::set_pos(w, me, info.position);
            self.kill_player(w, me, info.killer_id, false, false, info.cause);
            self.ctrl.is_in_ability = false;
            return;
        }
        if info.facing_right != self.ctrl.facing_right {
            self.ctrl.facing_right = !self.ctrl.facing_right;
        }
        self.sync_physics_to_exit(w, me, &info);
        if self.physics.is_grounded {
            if info.enter_turn {
                self.enter_stop(w);
            } else {
                self.enter_idle(w);
            }
            if info.just_landed
                && let Some(g) = info.ground
            {
                self.on_land(w, me, g);
            }
            if info.enter_jump {
                self.ctrl.was_upside_down =
                    self.current_normal(w, me).is_some_and(|n| n.y < Fix::ZERO);
                self.physics_jump(w, me);
            }
        } else {
            self.enter_fall(w, me);
        }
        w.physics.update_physics_positions(&w.objects, me);
        self.ctrl.is_in_ability = false;
    }

    /// `SyncPhysicsTo(AbilityExitInfo)`.
    fn sync_physics_to_exit(&mut self, w: &mut World, me: ObjId, info: &AbilityExitInfo) {
        self.physics.is_grounded = info.is_grounded;
        self.update_scale(w, me);
        let g = info
            .ground
            .filter(|g| w.objects.alive(*g) && w.physics.rr(*g).is_some());
        if let (true, Some(g), Some(s)) = (
            self.physics.is_grounded,
            g,
            g.and_then(|g| Surface::of(w, g)),
        ) {
            self.physics.grounded_speed = info.grounded_speed;
            self.physics.attached_ground = Some(g);
            self.physics.is_grounded = true;
            let p = s.position_from_param(info.grounded_local_position, info.radius);
            self.physics.local_platform_pos = s.param_from_position(p, self.physics.radius);
            Self::set_pos(w, me, p);
            self.body.external_velocity = info.external_velocity;
            self.body.self_imposed_velocity = Vec2::ZERO;
            self.align_rotation(w, me);
        } else {
            self.body.external_velocity = info.external_velocity;
            self.body.self_imposed_velocity = info.self_imposed_velocity;
            Self::set_pos(w, me, info.position);
            self.physics.attached_ground = None;
            self.physics.grounded_speed = Fix::ZERO;
            Self::set_rotation(w, me, Fix::ZERO);
        }
    }

    // -- PlayerBody -------------------------------------------------------------

    fn body_update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        if abilities::rope_body_update(w, me, self, dt) {
            // The rope system integrated the position.
        } else {
            let fdt = w.fixed_delta_time(self.id(), dt);
            let p = Self::pos(w, me) + self.body.velocity() * fdt;
            Self::set_pos(w, me, p);
        }
        self.body.external_velocity = self.body.external_velocity * F_0_95;
    }

    fn body_late_update(&mut self, w: &mut World, me: ObjId) {
        abilities::rope_body_late_update(w, me, self);
        self.body.prev_position = Self::pos(w, me);
    }

    // -- animator ---------------------------------------------------------------

    fn animator_update(&mut self, w: &mut World, me: ObjId) {
        let advance = if w.is_time_stopped() {
            !self.anim.always_stop_during_time_stop
                && w.player(self.id())
                    .is_some_and(|p| p.is_protected_from_time_stop)
        } else {
            true
        };
        if advance {
            self.advance_animator(w, me);
        }
    }

    fn advance_animator(&mut self, w: &mut World, me: ObjId) {
        let vy = self.body.velocity().y;
        let c = w.content.clone();
        if let Some(cb) = self.anim.advance(&c, vy) {
            self.anim_callback(w, me, cb);
        }
    }

    fn anim_callback(&mut self, w: &mut World, me: ObjId, cb: AnimCallback) {
        match cb {
            AnimCallback::BoplSpawn => self.spawn_done(w, me),
            AnimCallback::BoplStopDone => self.enter_idle(w),
            AnimCallback::BoplTurnDone(al) => self.enter_run(w, al),
            AnimCallback::BoplLandDone => self.ctrl.state = State::Idle,
            AnimCallback::BoplChew => self.chew(w, me),
            AnimCallback::BoplSwallow => {
                w.emit(Cue::Sound("swallow"));
                let a = anim(w, "swallow");
                self.anim
                    .begin_then_do(&w.content, a, AnimCallback::BoplFinishEating);
                self.ctrl.swallowing = true;
            }
            AnimCallback::BoplFinishEating => {
                self.ctrl.state = State::Idle;
                self.ctrl.swallowing = false;
                let a = anim(w, "idle");
                self.anim.begin(&w.content, a);
            }
            AnimCallback::Ability(_) => {}
        }
    }

    // -- eating and pushing (hurtbox contacts) ------------------------------------

    pub fn on_contact(&mut self, w: &mut World, me: ObjId, c: Contact) {
        // Listeners in hierarchy order: SlimeController, then PlayerCollision.
        self.controller_on_collide(w, me, &c);
        if !self.collision.was_killed_this_frame {
            self.kill_player_on_collision(w, me, &c);
        }
    }

    fn controller_on_collide(&mut self, w: &mut World, me: ObjId, c: &Contact) {
        if c.layer != 8 {
            return;
        }
        let id = self.id();
        let Some(other_tf) = w.objects.get(c.other).map(|o| o.tf) else {
            return;
        };
        let my_tf = w.objects.get(me).map(|o| o.tf).unwrap_or_default();
        if self.ctrl.state != State::Biting
            && self.physics.is_grounded
            && !w.is_time_stopped_for(id)
            && my_tf.scale - other_tf.scale >= Self::cfg(w).scale_diff_before_nom_nom
        {
            let other_pid = w.objects.get(c.other).map_or(0, |o| o.player_id);
            let other_is_victim =
                matches!(w.objects.get(c.other).map(|o| &o.kind), Some(Kind::Bopl(_)))
                    || abilities::has_player_collision(w, c.other);
            let teams_differ = match (w.player(other_pid), w.player(id)) {
                (Some(a), Some(b)) => a.team != b.team,
                _ => false,
            };
            if other_is_victim && teams_differ {
                self.bite(w, c.other);
            }
        }
        let d = my_tf.position - other_tf.position;
        let l = d.magnitude();
        let str_ = Self::cfg(w).push_overlapping_players_str;
        if l != Fix::ZERO && l < self.physics.radius {
            let d = d / l;
            self.physics_add_force(w, d * str_);
        } else if l == Fix::ZERO {
            let v = my_tf.up.mul_by_i() / Fix::from_int(10);
            if c.my_hier.wrapping_sub(c.other_hier) < 0 {
                self.physics_add_force(w, v * str_);
            } else {
                self.physics_add_force(w, (-v) * str_);
            }
        }
    }

    fn bite(&mut self, w: &mut World, victim: ObjId) {
        // The victim's controller (a bopl, or the bopl behind an ability object).
        let target = match w.objects.get(victim).map(|o| &o.kind) {
            Some(Kind::Bopl(_)) => Some(victim),
            _ => abilities::controller_of(w, victim),
        };
        let Some(t) = target.filter(|t| w.objects.alive(*t)) else {
            return;
        };
        self.ctrl.player_to_eat = Some(t);
        if active_collision_invulnerable(w, t) {
            return;
        }
        w.emit(Cue::Sound("bite"));
        self.ctrl.state = State::Biting;
        let a = anim(w, "bite");
        self.anim
            .begin_then_do(&w.content, a, AnimCallback::BoplChew);
    }

    fn chew(&mut self, w: &mut World, me: ObjId) {
        if let Some(t) = self.ctrl.player_to_eat
            && w.objects.alive(t)
        {
            let id = self.id();
            kill_active_collision(w, t, id, false, false, CauseOfDeath::Eaten);
            if let Some(p) = w.player_mut(id) {
                p.players_eaten_count_this_round += 1;
            }
        }
        let a = anim(w, "chew");
        self.anim
            .begin_then_do(&w.content, a, AnimCallback::BoplSwallow);
        let _ = me;
    }

    // -- PlayerCollision --------------------------------------------------------

    fn kill_player_on_collision(&mut self, w: &mut World, me: ObjId, c: &Contact) -> bool {
        if self.collision.is_invulnerable() {
            return false;
        }
        let id = self.id();
        if w.player(id).is_some_and(|p| !p.is_alive()) {
            return false;
        }
        let Some(other) = w.objects.get(c.other) else {
            return false;
        };
        if other.tf.destroyed {
            return false;
        }
        for ig in self.collision.ignored.iter().flatten() {
            if let Some(h) = w
                .objects
                .get(*ig)
                .and_then(|o| o.part(PartKind::Collider))
                .filter(|p| !p.destroyed)
                && h.hier == c.other_hier
                && c.other_hier != 0
            {
                return false;
            }
        }
        let mut squished = false;
        if c.layer == 11 {
            let other_is_platform = matches!(other.kind, Kind::Platform(_));
            let cfg = Self::cfg(w).clone();
            if cfg.is_squishable
                && other_is_platform
                && self.physics.is_grounded
                && let Some(g) = self.ground_alive(w)
                && g != c.other
            {
                if crate::platform::picks_up_players(w, c.other) {
                    let pos = Self::pos(w, me);
                    self.attach_to_ground(w, me, c.other, pos);
                } else {
                    let t = w.objects.get(me).map(|o| o.tf).unwrap_or_default();
                    let p = t.position + t.right.complex_mul(cfg.platform_squish_point_offset);
                    if c.collider.contains(p) {
                        squished = true;
                    }
                }
            }
        }
        let other_layer = w.objects.get(c.other).map_or(0, |o| o.layer);
        let lethal_layer = matches!(other_layer, 9 | 18 | 8 | 16 | 26);
        if !(lethal_layer || squished) {
            return false;
        }
        let Some(k) = abilities::lethal_info(
            w,
            c.other,
            other_layer,
            squished,
            self.collision.ignored_beam_id,
        ) else {
            return false;
        };
        if (k.owner == id && k.can_hurt_owner) || k.owner != id {
            self.collision.was_killed_this_frame = true;
            let cause = if squished {
                CauseOfDeath::Squished
            } else if k.owner == id && k.can_hurt_owner {
                CauseOfDeath::Accident
            } else if k.beam {
                CauseOfDeath::Beamed
            } else {
                CauseOfDeath::Other
            };
            return self.kill_player(w, me, k.owner, true, false, cause);
        }
        false
    }

    /// `killPlayer(killer, effect, ignoreInvulnerability, cause)` (player.md §8.2).
    pub fn kill_player(
        &mut self,
        w: &mut World,
        me: ObjId,
        killer: i32,
        effect: bool,
        ignore_invulnerability: bool,
        cause: CauseOfDeath,
    ) -> bool {
        if self.body.rope.is_some() {
            abilities::release_rope(w, me, self);
        }
        let id = self.id();
        if (self.collision.is_invulnerable() && !ignore_invulnerability)
            || !w.objects.get(me).is_some_and(|o| o.active)
        {
            return false;
        }
        let pos = Self::pos(w, me);
        let extra = w.player(id).map(|p| p.extra_life);
        if effect && extra != Some(ExtraLife::IsBeingRevived) {
            let n = Self::cfg(w).death_effect_updatables;
            w.consume_hierarchy(n);
            w.emit(Cue::Effect {
                kind: EffectKind::Death,
                position: pos,
                player_id: id,
            });
        }
        w.emit(Cue::Shake(800));
        let Some(player) = w.player(id).cloned() else {
            w.destroy(me);
            return true;
        };
        if !player.respawn_positions.is_empty() {
            abilities::revive_on_death(w, me, self, killer, cause);
        } else if matches!(
            player.extra_life,
            ExtraLife::NoRevive | ExtraLife::IsBeingRevived
        ) {
            if player.players_and_clones_still_alive == 1 {
                self.drop_abilities(w, me);
            }
            kill_record(w, id, killer, cause);
            for a in self.ctrl.abilities.clone() {
                if w.objects.alive(a) {
                    w.destroy(a);
                }
            }
            self.ctrl.abilities.clear();
            w.destroy(me);
        } else if player.extra_life == ExtraLife::PermanentRevive {
            self.unground(w, me, true, true);
            if let Some(p) = w.player_mut(id) {
                p.extra_life = ExtraLife::IsBeingRevived;
            }
            let sp = player.spawn_position;
            self.prepare_to_revive(w, me, sp);
            self.respawn(w, me, sp);
        }
        true
    }

    /// `DropAbilities()`: drop a pickup of the highest slot with an indicator.
    fn drop_abilities(&mut self, w: &mut World, me: ObjId) {
        if !w.round.game_in_progress || self.ctrl.abilities.is_empty() {
            return;
        }
        let mut num = w.settings.number_of_abilities as i32 - 1;
        while num >= 0 && num as usize >= self.ctrl.indicators {
            num -= 1;
        }
        if num < 0 {
            return;
        }
        let r = w.rng.random_fix(Fix(-1_288_490_240), Fix(1_288_490_240));
        let dir = (Vec2::UP + Vec2::new(r, Fix::ZERO)).normalized_safe();
        let id = self.id();
        let ability = w
            .player(id)
            .and_then(|p| p.current_abilities.get(num as usize).copied());
        if let Some(a) = ability {
            let pos = Self::pos(w, me);
            abilities::spawn_pickup(w, pos, Some(a), dir);
        }
    }

    /// `PrepareToRevive(pos)`.
    pub fn prepare_to_revive(&mut self, w: &mut World, me: ObjId, pos: Vec2) {
        let id = self.id();
        if let Some(p) = w.player_mut(id) {
            p.extra_life = ExtraLife::IsBeingRevived;
        }
        Self::set_hurtbox_enabled(w, me, false);
        self.collision.set_invulnerable(true, 4);
        self.unground(w, me, true, true);
        self.ctrl.state = State::Nothing;
        Self::set_rotation(w, me, Fix::ZERO);
        self.body.external_velocity = Vec2::ZERO;
        self.body.self_imposed_velocity = Vec2::ZERO;
        Self::set_pos(w, me, pos);
        self.ctrl.has_spawned = false;
        Self::set_physics_enabled(w, me, false);
        self.body.simulated = false;
        self.ctrl.suppress_input = true;
        abilities::on_prepare_to_revive(w, me, self);
    }

    /// `ReSpawn(pos)`.
    pub fn respawn(&mut self, w: &mut World, me: ObjId, pos: Vec2) {
        Self::set_hurtbox_enabled(w, me, false);
        self.body.simulated = true;
        self.body.external_velocity = Vec2::ZERO;
        self.body.self_imposed_velocity = Vec2::ZERO;
        Self::set_pos(w, me, pos);
        self.ctrl.state = State::Fall;
        let id = self.id();
        if let Some(p) = w.player_mut(id) {
            p.extra_life = ExtraLife::NoRevive;
        }
        self.collision.set_invulnerable(false, 4);
        self.before_spawn_init(w, me);
    }

    // -- bounds (DestroyIfOutsideSceneBounds) --------------------------------------

    fn bounds_update(&mut self, w: &mut World, me: ObjId) {
        let cfg = Self::cfg(w).clone();
        let Some(t) = w.objects.get(me).map(|o| o.tf) else {
            return;
        };
        let p = t.position;
        let space = w.is_space();
        let id = self.id();
        let effect = |w: &mut World, kind: EffectKind, n: i32, at: Vec2| {
            w.consume_hierarchy(n);
            w.emit(Cue::Effect {
                kind,
                position: at,
                player_id: id,
            });
        };
        if !space && cfg.destroy_if_too_far_y && p.y < WATER_HEIGHT {
            let mut at = p + cfg.down_effect_offset;
            at.y = WATER_HEIGHT - F_1_47 * t.scale;
            effect(w, EffectKind::WaterSplash, cfg.down_effect_updatables, at);
            w.emit(Cue::Sound("waterSplash"));
        } else if space && cfg.destroy_if_too_far_y && p.y < CAMERA_Y_MIN {
            effect(
                w,
                EffectKind::BlastZone,
                cfg.up_effect_updatables,
                Vec2::new(p.x, CAMERA_Y_MIN),
            );
        } else if !cfg.do_not_destroy_up && cfg.destroy_if_too_far_y && p.y > BLAST_Y_MAX {
            effect(
                w,
                EffectKind::BlastZone,
                cfg.up_effect_updatables,
                Vec2::new(p.x, BLAST_Y_MAX),
            );
        } else if cfg.destroy_if_too_far_x && (p.x < BLAST_X_MIN || p.x > BLAST_X_MAX) {
            let x = if p.x < BLAST_X_MIN {
                BLAST_X_MIN
            } else {
                BLAST_X_MAX
            };
            effect(
                w,
                EffectKind::BlastZone,
                cfg.x_effect_updatables,
                Vec2::new(x, p.y),
            );
        } else {
            return;
        }
        // selfDestruct(): a bopl with PlayerCollision is killed by the environment.
        self.kill_player(w, me, ENVIRONMENT_ID, false, true, CauseOfDeath::Accident);
    }
}

/// `Player.SetPosition(pos, controller)` with the ability-count priority.
pub fn set_player_position(
    w: &mut World,
    id: i32,
    pos: Vec2,
    controller: ObjId,
    n_abilities: usize,
) {
    let this = w.player(id).and_then(|p| p.setter_this_frame);
    let last = w.player(id).and_then(|p| p.setter_last_frame);
    let count_of = |w: &World, o: ObjId| -> Option<usize> {
        match &w.objects.get(o)?.kind {
            Kind::Bopl(b) => Some(b.ctrl.abilities.len()),
            Kind::Empty => Some(n_abilities), // the caller (taken out)
            _ => None,
        }
    };
    let c = match this.filter(|t| w.objects.alive(*t)) {
        None => 1,
        Some(t) if t == controller => 0,
        Some(t) => n_abilities as i32 - count_of(w, t).unwrap_or(0) as i32,
    };
    if (c > 0 || (c == 0 && last == Some(controller)))
        && let Some(p) = w.player_mut(id)
    {
        p.setter_this_frame = Some(controller);
        p.position = pos;
    }
}

/// `Player.Kill(killer, tick, cause)` (player.md §3).
pub fn kill_record(w: &mut World, id: i32, killer: i32, cause: CauseOfDeath) {
    let tick = w.time.simulation_ticks;
    let Some(p) = w.player(id) else {
        return;
    };
    if p.won_this_round {
        return;
    }
    let last = p.players_and_clones_still_alive == 1;
    let killer_exists = w.player(killer).is_some();
    let p = w.player_mut(id).unwrap();
    p.deaths += 1;
    p.players_and_clones_still_alive -= 1;
    if last {
        p.still_alive_this_round = false;
        p.timestamp_when_killed = tick;
        p.cause_of_death = cause;
    }
    if killer_exists && let Some(k) = w.player_mut(killer) {
        k.kills += 1;
    }
    if last {
        w.emit(Cue::PlayerDied { id, killer, cause });
    }
}

/// `GetActivePlayerCollision().IsInvulnerable` of a controller.
fn active_collision_invulnerable(w: &World, ctrl: ObjId) -> bool {
    match w.objects.get(ctrl).map(|o| &o.kind) {
        Some(Kind::Bopl(b)) => {
            if b.ctrl.is_in_ability {
                abilities::active_collision_invulnerable(w, ctrl)
            } else {
                b.collision.is_invulnerable()
            }
        }
        _ => false,
    }
}

/// Kills through the controller's active collision (bopl or ability object).
pub fn kill_active_collision(
    w: &mut World,
    ctrl: ObjId,
    killer: i32,
    effect: bool,
    ignore_inv: bool,
    cause: CauseOfDeath,
) {
    let in_ability =
        matches!(w.objects.get(ctrl).map(|o| &o.kind), Some(Kind::Bopl(b)) if b.ctrl.is_in_ability);
    if in_ability {
        abilities::kill_active_ability(w, ctrl, killer, effect, ignore_inv, cause);
        return;
    }
    w.with_kind(ctrl, |w, k| {
        if let Kind::Bopl(b) = k {
            b.kill_player(w, ctrl, killer, effect, ignore_inv, cause);
        }
    });
}

//! Blink gun (`docs/spec/abilities/blink-gun.md`).
//!
//! Also the canonical home of
//! - [`AbilityBody`]: the player body of an ability object (`PlayerPhysics`,
//!   `PlayerBody`, `PlayerCollision`, `DestroyIfOutsideSceneBounds`, the
//!   hurt box and the `SpriteAnimator` as configured by the ability prefab);
//! - [`GunTransform`] / [`GunAbility`]: "turn into a gun, aim, release to
//!   fire", shared by Blink gun, Growth ray, Shrink ray and Duplicator, each
//!   with its own [`Gun`] shot;
//! - [`ShootQuantum`] and the [`QuantumTunnel`] it spawns.

use std::sync::Arc;

use super::ability_body::{self, AbilityBody, BodyCfg, attach_parts, collider_active, in_scene_bounds, water_height};
use super::prefab::{
    NULL, ability_prefab, anim_data, anim_named, fields, float_curve_last_time, hier_of, prefab_by_ref,
    static_sound, updatables,
};
use crate::surface::Surface;
use super::prelude::*;
use super::{Behaviour, Framework};
use crate::data::{AnimRef, Content, Prefab};
use crate::physics::RayHit;
use crate::physics::shapes::ShapeKind;
use crate::view::Visual;
use crate::world::Kind;

// ---------------------------------------------------------------------------
// Code literals (float path, fixmath.md §3.1)
// ---------------------------------------------------------------------------

/// `(Fix)0.03f`: gravity modifier of an airborne gun.
pub const F_0_03: Fix = Fix(128_849_016);
/// `(Fix)1E-06f`: "is moving" threshold on the squared speed.
pub const F_1E_6: Fix = Fix(4294);
/// `(Fix)0.5f`.
const F_HALF: Fix = Fix(2_147_483_648);

/// Part codes of gun-like ability objects besides the standard body parts
/// (`PartKind::Behaviour(n)`).
pub mod part {
    /// The ability component (GunTransform, MagnetGun, Beam).
    pub const COMPONENT: u8 = 0;
    /// A child animator (Beam's sparks; presentation).
    pub const CHILD_ANIMATOR: u8 = 6;
}

// ---------------------------------------------------------------------------
// GunTransform (canonical) and the gun abilities
// ---------------------------------------------------------------------------

/// Animation callback codes of the gun body.
const CB_FIRE_DONE: u16 = 1;

/// Prefab values of a gun (blink-gun.md, gun prefab table).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GunCfg {
    pub body: Arc<BodyCfg>,
    pub hier_gun: Option<i32>,
    pub firepoint_offset: Vec2,
    pub knockback: Fix,
    pub aireal_blend: Fix,
    pub max_slow_mo_speed: Fix,
    pub prepare_sound: Option<&'static str>,
    pub anim_transform: Option<AnimRef>,
    pub anim_loop: Option<AnimRef>,
    pub anim_fire: Option<AnimRef>,
}

impl GunCfg {
    pub fn from_def(w: &World, def: &AbilityDef) -> GunCfg {
        let c = &w.content;
        let Some(p) = ability_prefab(w, def) else {
            return GunCfg::from_prefab(c, &Prefab::default());
        };
        GunCfg::from_prefab(c, p)
    }

    pub fn from_prefab(c: &Content, p: &Prefab) -> GunCfg {
        let g = super::prefab::fields(p, "GunTransform");
        let data = anim_data(c, g.reference("animData"));
        GunCfg {
            body: Arc::new(BodyCfg::from_root(c, p)),
            hier_gun: hier_of(p, "GunTransform"),
            firepoint_offset: g.vec2("FirepointOffset").unwrap_or_default(),
            knockback: g.fix_or("KnockBack", Fix::from_int(50)),
            aireal_blend: g.fix_or("airealControlBlend", F_HALF),
            max_slow_mo_speed: g.fix_or("maxSlowMoSpeed", Fix::from_int(9)),
            prepare_sound: g.str("prepareToShootSoundName").and_then(static_sound),
            anim_transform: anim_named(c, data, "gunTransform"),
            anim_loop: anim_named(c, data, "gunTransformLoop"),
            anim_fire: anim_named(c, data, "gunTransformFire"),
        }
    }
}

/// `GunTransform` state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GunTransform {
    pub cfg: Arc<GunCfg>,
    pub b: AbilityBody,
    pub anim: SpriteAnimator,
    /// Aim direction `inputVector`.
    pub d: Vec2,
    pub has_fired: bool,
    /// `FirepointOffset` (rescaled by scale changes, persists).
    pub firepoint_offset: Vec2,
}

impl GunTransform {
    pub fn new(cfg: Arc<GunCfg>) -> GunTransform {
        GunTransform {
            b: AbilityBody::from_cfg(cfg.body.clone()),
            anim: ability_body::animator(cfg.body.always_stop_during_time_stop),
            d: Vec2::RIGHT,
            has_fired: false,
            firepoint_offset: cfg.firepoint_offset,
            cfg,
        }
    }

    /// `CurrentFirePoint()`.
    pub fn fire_point(&self, w: &World, me: ObjId) -> Vec2 {
        let t = AbilityBody::tf(w, me);
        let f = self.firepoint_offset;
        t.position + f.y * t.up + f.x * t.right
    }

    /// `OnEnterAbility()`.
    pub fn enter(&mut self, cx: &mut Cx<'_>) {
        let info = cx.info();
        let me = cx.me;
        self.b.sync_physics_to(cx.w, me, &info);
        cx.w.physics.update_physics_positions(&cx.w.objects, me);
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        self.b.body.external_velocity = Vec2::ZERO;
        if let (Some(a), Some(l)) = (self.cfg.anim_transform, self.cfg.anim_loop) {
            let c = cx.w.content.clone();
            self.anim.begin_then_play(&c, a, l, None);
        }
        if !info.is_grounded {
            self.b.gravity_modifier = F_0_03;
            self.b.body.self_imposed_velocity = info.self_imposed_velocity * F_0_1;
            self.b.body.external_velocity = info.external_velocity * F_0_1;
        }
        self.d = info.forwards;
        self.has_fired = false;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        if let Some(s) = self.cfg.prepare_sound {
            cx.w.emit(Cue::Sound(s));
        }
    }

    /// The landing / exit record fields the gun fills in.
    fn fill(&self, w: &World, me: ObjId, mut r: AbilityExitInfo) -> AbilityExitInfo {
        r.grounded_speed = self.b.physics.grounded_speed;
        r.is_grounded = self.b.physics.is_grounded;
        r.facing_right = self.d.x >= Fix::ZERO;
        r.position = AbilityBody::pos(w, me);
        r.self_imposed_velocity = self.b.body.self_imposed_velocity;
        r.external_velocity = self.b.body.external_velocity;
        r.ground = self.b.physics.attached_ground;
        r.grounded_local_position = self.b.physics.local_platform_pos;
        r.radius = self.b.physics.radius;
        r
    }

    /// `GunTransform.ExitAbility(info)`.
    pub fn exit(&mut self, cx: &mut Cx<'_>, info: AbilityExitInfo) {
        let me = cx.me;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        self.b.unground(cx.w, me, true, false);
        let r = self.fill(cx.w, me, info);
        cx.exit_body(&mut self.b, r);
    }

    /// `Fire(d)`.
    pub fn fire<G: Gun>(&mut self, cx: &mut Cx<'_>, gun: &mut G) {
        let me = cx.me;
        self.has_fired = true;
        if let Some(a) = self.cfg.anim_fire {
            let c = cx.w.content.clone();
            self
                .anim
                .begin_then_do(&c, a, AnimCallback::Ability(CB_FIRE_DONE));
        }
        let d = self.d;
        AbilityBody::set_up(cx.w, me, Vec2::new(-d.y, d.x));
        let fp = self.fire_point(cx.w, me);
        let dir = d.normalized_safe();
        gun.shoot(cx, self, fp, dir);
        self.has_fired = true;
        self.b.gravity_modifier = Fix::ONE;
        if self.b.physics.is_grounded {
            let n = self.b.current_normal(cx.w, me).unwrap_or(Vec2::UP);
            self.b.body.self_imposed_velocity = -n * self.cfg.knockback;
            self.b.unground(cx.w, me, true, false);
        } else {
            self.b.body.self_imposed_velocity = -dir * self.cfg.knockback;
        }
    }

    /// `GunTransform.UpdateSim`.
    pub fn update<G: Gun>(&mut self, cx: &mut Cx<'_>, gun: &mut G, dt: Fix) {
        let me = cx.me;
        let owner = cx.fw.owner;
        let (aim, horizontal) = cx.w.player(owner).map_or((Vec2::ZERO, Fix::ZERO), |p| {
            (p.inputs.aim_vector(), p.inputs.input_vector().x)
        });
        let m = aim.magnitude();
        if m > DEADZONE {
            self.d = aim / m;
        }
        if self.b.physics.is_grounded
            && let Some(r) = self.b.move_(cx.w, me, Vec2::ZERO, dt, true)
        {
            self.exit(cx, r);
        }
        if !cx.button_down() && !self.has_fired {
            self.fire(cx, gun);
        }
        if !cx.w.objects.active_in_hierarchy(me) {
            return;
        }
        if self.b.physics.is_grounded {
            let moving = self.b.body.self_imposed_velocity.sqr_magnitude() > F_1E_6;
            let ground_ok = self.b.ground(cx.w).is_some_and(|g| {
                cx.w.objects.active_in_hierarchy(g)
                    && cx.w.objects.get(g).is_some_and(|o| o.part_enabled(PartKind::Sticky))
            });
            if moving || !ground_ok {
                self.b.gravity_modifier = F_0_03;
                self.b.unground(cx.w, me, true, false);
            }
        }
        if !self.b.physics.is_grounded {
            self.b.add_gravity_factor(cx.w);
            if self.b.velocity_based_raycasts(cx.w, me, true, dt) && self.has_fired {
                AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
                let r = self.fill(
                    cx.w,
                    me,
                    AbilityExitInfo {
                        just_landed: true,
                        ..AbilityExitInfo::default()
                    },
                );
                cx.exit_body(&mut self.b, r);
            }
        }
        if !self.has_fired {
            let d = self.d;
            AbilityBody::set_up(cx.w, me, Vec2::new(-d.y, d.x));
            let v = self.b.body.self_imposed_velocity;
            if v.magnitude() > self.cfg.max_slow_mo_speed {
                self.b.body.self_imposed_velocity = v.normalized() * self.cfg.max_slow_mo_speed;
            }
        } else {
            let v0 = self.b.body.self_imposed_velocity;
            self.b.aireal_movement(horizontal);
            let v1 = self.b.body.self_imposed_velocity;
            self.b.body.self_imposed_velocity = v0.lerp(v1, self.cfg.aireal_blend);
        }
        let v = self.b.body.self_imposed_velocity;
        if v.magnitude() > self.b.cfg.max_speed {
            self.b.body.self_imposed_velocity = v.normalized() * self.b.cfg.max_speed;
        }
        if self.b.physics.is_grounded
            && let Some(r) = self.b.drop_platform_test(cx.w, me)
        {
            self.exit(cx, r);
        }
    }
}

/// The ability-specific shot of a gun (`IGun`).
pub trait Gun: Clone + std::fmt::Debug + std::hash::Hash + PartialEq + Eq {
    fn new(w: &World, def: &AbilityDef) -> Self;
    /// `IGun.Shoot(firepoint, direction, ...)`.
    fn shoot(&mut self, cx: &mut Cx<'_>, gun: &mut GunTransform, fp: Vec2, dir: Vec2);
}

/// An ability made of `Ability` + `GunTransform` + a gun.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GunAbility<G: Gun> {
    pub gun: GunTransform,
    pub shot: G,
}

impl<G: Gun> AbilityImpl for GunAbility<G> {
    fn new(w: &World, def: &AbilityDef) -> Self {
        GunAbility {
            gun: GunTransform::new(Arc::new(GunCfg::from_def(w, def))),
            shot: G::new(w, def),
        }
    }

    fn build(&mut self, w: &mut World, me: ObjId, fw: &mut Framework, _def: &AbilityDef) {
        self.gun.b.install_parts(w, me, fw);
        if let (Some(h), Some(o)) = (self.gun.cfg.hier_gun, w.objects.get_mut(me)) {
            o.add_part(PartKind::Behaviour(part::COMPONENT), h);
        }
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        self.gun.enter(cx);
    }

    fn update_part(&mut self, cx: &mut Cx<'_>, k: PartKind, dt: Fix, late: bool) {
        match (k, late) {
            (PartKind::Behaviour(part::COMPONENT), false) => self.gun.update(cx, &mut self.shot, dt),
            (PartKind::Animator, false) => {
                if ability_body::advance_code(cx.w, cx.fw.owner, &mut self.gun.anim) == Some(CB_FIRE_DONE) {
                    self.gun.exit(cx, AbilityExitInfo::default());
                }
            }
            _ => {
                if let Some(r) = self.gun.b.update_part(cx, k, dt, late) {
                    self.gun.exit(cx, r);
                }
            }
        }
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        ability_body::hurt_contact(self, cx, c);
    }

    fn on_scale_changed(&mut self, _cx: &mut Cx<'_>, m: Fix) {
        self.gun.firepoint_offset = self.gun.firepoint_offset * m;
    }

    fn exit_component(&mut self, cx: &mut Cx<'_>, info: AbilityExitInfo) {
        self.gun.exit(cx, info);
    }

    fn body(&self) -> Option<&AbilityBody> {
        Some(&self.gun.b)
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        Some(&mut self.gun.b)
    }

    fn sprite(&self) -> crate::data::SpriteId {
        self.gun.anim.sprite
    }

    fn look(&self, _w: &World, _me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        l.frame(&self.gun.anim);
    }
}

// ---------------------------------------------------------------------------
// Blink gun: ShootQuantum
// ---------------------------------------------------------------------------

/// The Blink gun ability.
pub type BlinkGun = GunAbility<ShootQuantum>;

/// Objects this module spawns.
pub type Spawned = QuantumTunnel;

/// `maxDistance` (code literal 100).
const MAX_DISTANCE: Fix = Fix(429_496_729_600);
/// `(Fix)0.3` (double literal) in the player lifespan.
const F_0_3_D: Fix = Fix(1_288_490_188);
/// `(Fix)0.1` (double literal) self-hit distance.
pub const SELF_HIT_DIST: Fix = Fix(429_496_729);
/// `(Fix)1.7` (double literal) below the camera top.
const F_1_7_D: Fix = Fix(7_301_444_403);

/// Layer numbers (engine.md §8).
pub mod layer {
    pub const WATER: i32 = 4;
    pub const PLAYER: i32 = 8;
    pub const PROJECTILE: i32 = 9;
    pub const ITEM: i32 = 10;
    pub const WALL: i32 = 11;
    pub const WEAPON: i32 = 13;
    pub const LETHAL_TERRAIN: i32 = 16;
    pub const EXPLOSION: i32 = 18;
    pub const EFFECTOR_ZONE: i32 = 19;
    pub const NON_LETHAL_TERRAIN: i32 = 21;
    pub const SMOKE: i32 = 22;
    pub const RIGID_BODY_AFFECTOR: i32 = 23;
    pub const ORB: i32 = 26;
    pub const SPELL: i32 = 27;
}

/// Static data of `ShootQuantum` and the tunnel prefab.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QuantumCfg {
    pub collision_mask: u32,
    pub wall_duration: Fix,
    pub wall_delay: Fix,
    pub wall_shake: Fix,
    pub min_player_duration: Fix,
    pub on_player_hit_cd: Fix,
    pub base_cd: Fix,
    pub tunnel_prefab: Option<usize>,
    pub tunnel_hier: i32,
    pub tunnel_updatables: i32,
    /// Last `ScaleAnim` key time, `(Fix)time` (float path).
    pub scale_anim_last: Fix,
}

/// `ShootQuantum`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShootQuantum {
    pub cfg: Arc<QuantumCfg>,
}

impl Gun for ShootQuantum {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let c = &w.content;
        let p = ability_prefab(w, def);
        let f = p.map_or(&NULL, |p| fields(p, "ShootQuantum"));
        let tunnel = prefab_by_ref(c, f.reference("QuantumTunnelPrefab"));
        let tf = tunnel
            .and_then(|i| c.prefabs.get(i))
            .map_or(&NULL, |p| fields(p, "QuantumTunnel"));
        ShootQuantum {
            cfg: Arc::new(QuantumCfg {
                collision_mask: f.int("collisionMask").unwrap_or(82_390_800) as u32,
                wall_duration: f.fix_or("WallDuration", Fix::from_int(4)),
                wall_delay: f.fix_or("WallDelay", Fix::ONE),
                wall_shake: f.fix_or("WallShake", Fix(1_288_490_240)),
                min_player_duration: f.fix_or("minPlayerDuration", F_HALF),
                on_player_hit_cd: f.fix_or("onPlayerHitCD", Fix::from_int(5)),
                base_cd: def.cooldown,
                tunnel_prefab: tunnel,
                tunnel_hier: tf.int("hierarchyNumber").unwrap_or(1) as i32,
                tunnel_updatables: updatables(c, tunnel).max(1),
                scale_anim_last: tf
                    .get("ScaleAnim")
                    .and_then(float_curve_last_time)
                    .unwrap_or(Fix(1_292_488_320)),
            }),
        }
    }

    fn shoot(&mut self, cx: &mut Cx<'_>, gun: &mut GunTransform, fp: Vec2, dir: Vec2) {
        self.shoot_q(cx, gun, fp, dir, false);
    }
}

/// The ray hit replaced by the shooter's own box (guns' water rule).
pub fn self_hit(w: &World, me: ObjId, dist: Fix) -> RayHit {
    let pos = AbilityBody::pos(w, me);
    let hier = w
        .objects
        .get(me)
        .and_then(|o| o.part(PartKind::Collider))
        .map_or(0, |p| p.hier);
    RayHit {
        near_pos: pos,
        far_pos: pos,
        near_dist: dist,
        far_dist: dist,
        layer: layer::PLAYER,
        obj: me,
        hierarchy: hier,
        kind: ShapeKind::Box,
    }
}

fn set_active(w: &mut World, id: ObjId, a: bool) {
    if let Some(o) = w.objects.get_mut(id) {
        o.active = a;
    }
}

impl ShootQuantum {
    fn shoot_q(
        &mut self,
        cx: &mut Cx<'_>,
        gun: &mut GunTransform,
        fp: Vec2,
        dir: Vec2,
        bounce: bool,
    ) {
        let me = cx.me;
        cx.w.emit(Cue::Sound("laserShoot"));
        if !bounce {
            set_active(cx.w, me, false);
        }
        let mut hit = cx.w.physics.point_check_all_rrs(&cx.w.objects, fp);
        if !hit.hit() {
            hit = cx.w.physics.raycast_closest(
                &cx.w.objects,
                fp,
                dir,
                MAX_DISTANCE,
                self.cfg.collision_mask,
            );
        }
        if !bounce {
            set_active(cx.w, me, true);
        }
        if (!bounce && fp.y <= water_height(cx.w)) || (bounce && hit.layer == layer::WATER) {
            cx.w.emit(Cue::Sound("waterExplosion"));
            let x = hit.near_pos.x;
            super::fx(cx.w, me, "WaterRing", Vec2::new(x, Fix::ZERO), Fix::ZERO, Fix::ONE, 0, Vec2::ZERO);
            hit = self_hit(cx.w, me, SELF_HIT_DIST);
        }
        if hit.hit() {
            self.blink(cx, gun, &hit, fp, dir);
        } else {
            let o = cx.fw.owner;
            super::ray_fx(
                cx.w,
                me,
                "RaycastParticlePrefab",
                "RaycastParticleHitPrefab",
                fp,
                dir,
                MAX_DISTANCE,
                false,
                None,
                o,
            );
        }
    }

    /// `BlinkObject(hit)`.
    fn blink(&mut self, cx: &mut Cx<'_>, gun: &mut GunTransform, hit: &RayHit, fp: Vec2, dir: Vec2) {
        let c = self.cfg.clone();
        let victim = hit.obj;
        let water = hit.layer == layer::WATER;
        let (me, o) = (cx.me, cx.fw.owner);
        super::ray_fx(
            cx.w,
            me,
            "RaycastParticlePrefab",
            "RaycastParticleHitPrefab",
            fp,
            dir,
            hit.near_dist,
            !water,
            Some(-dir),
            o,
        );
        if water {
            let x = hit.near_pos.x;
            super::fx(cx.w, me, "WaterRing", Vec2::new(x, Fix::ZERO), Fix::ZERO, Fix::ONE, 0, Vec2::ZERO);
        }
        if hit.layer == layer::WATER {
            let d = Vec2::new(dir.x, -dir.y);
            self.shoot_q(cx, gun, hit.near_pos, d, true);
            cx.w.emit(Cue::Sound("waterExplosion"));
            return;
        }
        let vpos = AbilityBody::pos(cx.w, victim);
        if hit.layer == layer::EFFECTOR_ZONE || hit.layer == layer::WEAPON {
            let t = spawn_tunnel(cx.w, &c, vpos);
            // Objects tagged `InvincibilityZone` are not blinked (none in
            // the shipped levels).
            if super::prefab::object_tag(cx.w, victim) != "InvincibilityZone" {
                tunnel_init(cx, gun, t, victim, c.wall_duration, false);
            }
            return;
        }
        // (The orb-layer branch never applies: no object of the game's
        // data is on the orb layer.)
        BlinkGlobals::prune(cx.w);
        let existing = cx
            .w
            .ability_globals
            .blink
            .tunnels
            .iter()
            .rev()
            .find(|(v, _)| *v == victim)
            .map(|(_, t)| *t)
            .filter(|t| cx.w.objects.alive(*t));
        let tunnel = match existing {
            Some(t) => t,
            None => {
                let t = spawn_tunnel(cx.w, &c, vpos);
                let g = &mut cx.w.ability_globals.blink;
                g.tunnels.retain(|(v, _)| *v != victim);
                g.tunnels.push((victim, t));
                t
            }
        };
        cx.fw.cooldown = c.base_cd;
        let vl = cx.w.objects.get(victim).map_or(0, |o| o.layer as i32);
        if vl == layer::WALL {
            crate::platform::add_shake(cx.w, victim, c.wall_delay, c.wall_shake, 10, None);
            tunnel_delayed_init(cx.w, tunnel, victim, c.wall_duration, c.wall_delay);
        } else if vl == layer::PLAYER {
            cx.fw.cooldown = c.on_player_hit_cd;
            let life = c.min_player_duration.max(F_0_3_D * c.wall_duration);
            tunnel_init(cx, gun, tunnel, victim, life, true);
        } else {
            tunnel_init(cx, gun, tunnel, victim, c.wall_duration, false);
        }
    }
}

/// Global blink state: the most recent tunnel created per victim (the
/// original's never-cleared tunnel list; only "last entry for a victim"
/// is ever read).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlinkGlobals {
    /// (victim, tunnel), at most one entry per victim, creation order.
    pub tunnels: Vec<(ObjId, ObjId)>,
}

impl BlinkGlobals {
    /// Drops entries whose victim no longer exists (it can never match again).
    pub fn prune(w: &mut World) {
        let objs = &w.objects;
        w.ability_globals
            .blink
            .tunnels
            .retain(|(v, _)| objs.exists(*v));
    }
}

// ---------------------------------------------------------------------------
// QuantumTunnel
// ---------------------------------------------------------------------------

/// `QuantumTunnel` (spawned object).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QuantumTunnel {
    pub victim: Option<ObjId>,
    pub life: Fix,
    pub initialized: bool,
    pub age: Fix,
    pub delay: Fix,
    pub delay_passed: Fix,
    pub delayed_life: Fix,
    pub is_player: bool,
    pub played_teleport2: bool,
    /// Last `ScaleAnim` key time (raw).
    pub scale_anim_last: Fix,
}

fn spawn_tunnel(w: &mut World, c: &QuantumCfg, pos: Vec2) -> ObjId {
    let t = QuantumTunnel {
        victim: None,
        life: Fix::ZERO,
        initialized: false,
        age: Fix::ZERO,
        delay: -Fix::ONE,
        delay_passed: Fix::ZERO,
        delayed_life: Fix::ZERO,
        is_player: false,
        played_teleport2: false,
        scale_anim_last: c.scale_anim_last,
    };
    w.open_bracket();
    let id = w
        .objects
        .create(Kind::Behaviour(Box::new(Behaviour::BlinkGun(t))), 0);
    if let Some(o) = w.objects.get_mut(id) {
        o.visual = match c.tunnel_prefab {
            Some(p) => Visual::Prefab {
                prefab: p as u32,
                object: 0,
            },
            None => Visual::None,
        };
        o.add_part(PartKind::Behaviour(0), c.tunnel_hier);
    }
    w.register_all(id);
    w.close_bracket();
    w.consume_hierarchy((c.tunnel_updatables - 1).max(0));
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.position = pos;
        o.tf.reset_rotation();
    }
    id
}

fn with_tunnel<R>(w: &mut World, t: ObjId, f: impl FnOnce(&mut QuantumTunnel) -> R) -> Option<R> {
    match w.objects.get_mut(t).map(|o| &mut o.kind) {
        Some(Kind::Behaviour(b)) => match b.as_mut() {
            Behaviour::BlinkGun(q) => Some(f(q)),
            _ => None,
        },
        _ => None,
    }
}

/// `DelayedInit(victim, life, delay)`.
fn tunnel_delayed_init(w: &mut World, t: ObjId, victim: ObjId, life: Fix, delay: Fix) {
    with_tunnel(w, t, |q| {
        q.delay = delay;
        q.delay_passed = Fix::ZERO;
        q.initialized = false;
        q.victim = Some(victim);
        q.delayed_life = life;
    });
}

/// `Init(victim, life, isPlayer)` called from a gun shot (the shooter's
/// own body may be the victim: its payload is taken out right now).
fn tunnel_init(
    cx: &mut Cx<'_>,
    gun: &mut GunTransform,
    t: ObjId,
    victim: ObjId,
    life: Fix,
    is_player: bool,
) {
    if victim == cx.me {
        // The shooter's own PlayerBody: an enabled rope would be detached.
        gun.b.body.rope = None;
    }
    tunnel_init_world(cx.w, t, victim, life, is_player);
}

/// `QuantumTunnel.Init` (blink-gun.md) on a tunnel object.
pub fn tunnel_init_world(w: &mut World, t: ObjId, victim: ObjId, life: Fix, is_player: bool) {
    if tunnel_init_victim(w, t, victim) {
        with_tunnel(w, t, |q| q.set_init(victim, life, is_player));
        tunnel_hide_victim(w, victim);
    }
}

/// The victim side of `Init` before the state is set: false if the victim
/// is already destroyed (the tunnel destroys itself).
fn tunnel_init_victim(w: &mut World, t: ObjId, victim: ObjId) -> bool {
    let Some(vo) = w.objects.get(victim) else {
        return false;
    };
    if vo.tf.destroyed {
        w.destroy(t);
        return false;
    }
    // A PlayerBody victim: cut an enabled rope, tell a magnet gun.
    match &vo.kind {
        Kind::Bopl(_) => {
            w.with_kind(victim, |w, k| {
                if let Kind::Bopl(b) = k
                    && super::rope_enabled(w, b.body.rope)
                {
                    super::release_rope(w, victim, b);
                }
            });
        }
        Kind::Ability(_) => super::magnet_gun::on_item_was_stolen(w, victim),
        _ => {}
    }
    true
}

/// The end of `Init`: the victim's rotation re-derived, victim hidden.
fn tunnel_hide_victim(w: &mut World, victim: ObjId) {
    if let Some(o) = w.objects.get_mut(victim) {
        // `victimFixTrans.rotation = victimFixTrans.rotation` (re-derives
        // right/up from the stored angle).
        let r = o.tf.rotation;
        o.tf.set_rotation(r);
        o.active = false;
    }
    w.emit(Cue::Sound("teleport"));
}

impl QuantumTunnel {
    fn set_init(&mut self, victim: ObjId, life: Fix, is_player: bool) {
        self.is_player = is_player;
        self.life = life;
        self.victim = Some(victim);
        self.initialized = true;
        self.age = Fix::ZERO;
        self.played_teleport2 = false;
    }
}

impl SpawnedImpl for QuantumTunnel {
    /// The tunnel shows its (hidden) victim's current sprite through the
    /// distortion material, facing like it; `player` = the victim's owner
    /// (material colours).
    fn look(&self, w: &World, _me: ObjId, l: &mut crate::view::Look) {
        let Some(v) = self.victim.and_then(|v| w.objects.get(v)) else {
            l.hidden = true;
            return;
        };
        match &v.kind {
            Kind::Bopl(b) => {
                l.sprite = b.anim.sprite;
                l.flip_x = b.anim.flip_x != !b.ctrl.facing_right;
            }
            Kind::Ability(a) => {
                let mut al = crate::view::Look::default();
                a.look(w, self.victim.unwrap_or(ObjId::NONE), &mut al);
                l.sprite = al.sprite;
                l.flip_x = al.flip_x;
            }
            _ => l.hidden = true,
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId, _part: PartKind, dt: Fix, late: bool) {
        if late {
            return;
        }
        let Some(victim) = self.victim.filter(|v| w.objects.exists(*v)) else {
            w.destroy(me);
            return;
        };
        if !self.initialized && self.delay > Fix::ZERO {
            self.delay_passed += dt * w.time.player_time_scale;
            if self.delay_passed > self.delay && tunnel_init_victim(w, me, victim) {
                self.set_init(victim, self.delayed_life, false);
                tunnel_hide_victim(w, victim);
            }
        }
        if !self.initialized {
            return;
        }
        if self.age > self.life {
            if let Some(o) = w.objects.get_mut(victim) {
                o.active = true;
            }
            w.destroy(me);
        }
        self.age += w.time.player_time_scale * dt;
        let vt = AbilityBody::tf(w, victim);
        if let Some(o) = w.objects.get_mut(me) {
            o.tf.position = vt.position;
            o.tf.set_rotation(vt.rotation);
        }
        if self.age < self.life - self.scale_anim_last {
            return;
        }
        if self.is_player {
            release_position(w, victim);
            self.is_player = false;
        }
        if !self.played_teleport2 {
            w.emit(Cue::Sound("teleport2"));
            self.played_teleport2 = true;
        }
    }
}

/// `determinePlayerReleasePosition()` (blink-gun.md, player release position).
fn release_position(w: &mut World, victim: ObjId) {
    let grounded = victim_physics(w, victim);
    let wh = water_height(w);
    let objs_n = w.physics.rrs.len();
    let vx = AbilityBody::pos(w, victim).x;
    let top = Vec2::new(Fix::ZERO, Fix::from_int(1000));
    if let Some((true, radius)) = grounded {
        let lim = wh + F_HALF;
        let mut list = Vec::new();
        for i in 0..objs_n {
            let par = w.physics.rrs.parents[i];
            let s = w.physics.rrs.shapes[i];
            if collider_active(w, par.obj)
                && in_scene_bounds(s.b.center)
                && s.closest_point(top).y > lim
            {
                if (s.b.center.x - vx).abs() > BLAST_Y_MAX {
                    continue;
                }
                list.push(i);
            }
        }
        if list.is_empty() {
            return;
        }
        let idx = w.rng.random_int(0, list.len() as i32);
        let num = list[idx as usize];
        let par = w.physics.rrs.parents[num];
        let Some(surf) = Surface::of(w, par.obj) else {
            return;
        };
        for _ in 0..20 {
            let u = w.rng.random_fix(Fix::ZERO, Fix::ONE);
            let p = surf.position_from_param(u, Fix::ZERO);
            let q = p + surf.normal(p) * radius;
            let mut rejected = false;
            for k in 0..objs_n {
                if w.physics.rrs.parents[k].hierarchy != par.hierarchy
                    && w.physics.rrs.shapes[k].contains(q)
                {
                    rejected = true;
                    break;
                }
            }
            if p.y > lim && in_scene_bounds(p) && !rejected {
                attach_victim(w, victim, par.obj, p);
                return;
            }
        }
        let p = w.physics.rrs.shapes[num].closest_point(top);
        AbilityBody::set_pos(w, victim, p);
        attach_victim(w, victim, par.obj, p);
        return;
    }
    let mut mean = Vec2::ZERO;
    for i in 0..objs_n {
        mean += w.physics.rrs.shapes[i].b.center;
    }
    mean = mean / Fix::from_int(objs_n as i32);
    let a = mean.y - Fix::from_int(4);
    let mut hi = BLAST_X_MIN;
    let mut lo = BLAST_X_MAX;
    let mut counted = 0;
    for i in 0..objs_n {
        if !collider_active(w, w.physics.rrs.parents[i].obj) {
            continue;
        }
        let s = w.physics.rrs.shapes[i];
        if (s.b.center.x - vx).abs() > BLAST_Y_MAX {
            continue;
        }
        counted += 1;
        let bb = s.aabb(i as i32);
        hi = hi.max(bb.max.x);
        lo = lo.min(bb.min.x);
    }
    if counted == 0 || hi < lo {
        hi = vx + Fix::from_int(30);
        lo = vx - Fix::from_int(30);
    }
    let mut min = Vec2::new(lo, a.max(wh + Fix::ONE));
    let max = Vec2::new(hi, CAMERA_Y_MAX - F_1_7_D);
    if min.x > max.x {
        min.x = max.x;
    }
    if min.y > max.y {
        min.y = max.y;
    }
    let mut p = AbilityBody::pos(w, victim);
    for _ in 0..20 {
        let x = w.rng.random_fix(min.x, max.x);
        let y = w.rng.random_fix(min.y, max.y);
        p = Vec2::new(x, y);
        if !w.physics.point_check_all_rrs(&w.objects, p).hit() {
            break;
        }
    }
    AbilityBody::set_pos(w, victim, p);
    super::with_player_body(w, victim, |_, _, body| {
        body.self_imposed_velocity = Vec2::ZERO;
    });
}

/// (grounded, radius) of a victim's `PlayerPhysics`, if it has one.
fn victim_physics(w: &mut World, victim: ObjId) -> Option<(bool, Fix)> {
    super::with_player_body(w, victim, |_, phys, _| (phys.is_grounded, phys.radius))
}

/// `component.AttachToGround(platform, p)`; victim position := body position.
fn attach_victim(w: &mut World, victim: ObjId, plat: ObjId, p: Vec2) {
    let is_bopl = matches!(w.objects.get(victim).map(|o| &o.kind), Some(Kind::Bopl(_)));
    if is_bopl {
        w.with_kind(victim, |w, k| {
            if let Kind::Bopl(b) = k {
                b.attach_to_ground(w, victim, plat, p);
            }
        });
    } else {
        super::with_player_body(w, victim, |w, phys, body| {
            attach_parts(w, victim, phys, body, plat, p);
        });
    }
}

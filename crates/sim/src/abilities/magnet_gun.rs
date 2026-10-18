//! Magnet gun (`docs/spec/abilities/magnet-gun.md`).
//!
//! The body (aim, gravity, recoil, landing) follows the GunTransform
//! pattern on an [`AbilityBody`]; pulls, pickups, holding, throwing,
//! stealing and parries are this module's. Parries run from
//! [`static_update`] (engine step 8).

use std::sync::Arc;

use super::Framework;
use super::blink_gun::{F_0_03, F_1E_6};
use super::blink_gun::part;
use super::ability_body::{AbilityBody, BodyCfg};
use super::prefab::{ability_prefab, anim_data, anim_named, fields, hier_of};
use super::blink_gun::layer;
use super::prelude::*;
use crate::curve::Curve;
use crate::data::{AnimRef, Content, Prefab};
use crate::physics::shapes::{self, BoxShape, Circle, Manifold, ShapeKind};
use crate::world::Kind;

/// Objects this module spawns.
pub type Spawned = NoSpawn;

/// Animation callbacks.
const CB_PREPULL: u16 = 1;
const CB_PULL: u16 = 2;
const CB_EXIT: u16 = 3;

/// `(Fix)1.5` (double literal): extra fire-point offset per held scale.
const F_1_5: Fix = Fix(6_442_450_944);
/// `(Fix)0.01` (double literal): minimum pull box half sizes.
const F_0_01_D: Fix = Fix(42_949_672);
/// `(Fix)0.0001` (double literal): the thrown victim's probe time.
const F_0_0001_D: Fix = Fix(429_496);
/// `One / (Fix)10`: rope end air time of a thrown roped victim.
const F_TENTH: Fix = Fix(429_496_730);

/// Prefab values (magnet-gun.md, constants).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MagnetCfg {
    pub body: Arc<BodyCfg>,
    pub hier: Option<i32>,
    pub firepoint_offset: Vec2,
    pub knockback: Fix,
    pub aireal_blend: Fix,
    pub max_slow_mo_speed: Fix,
    pub parry_push_str: Fix,
    pub pickup_radius: Fix,
    pub max_lock_dist: Fix,
    pub max_pull_duration: Fix,
    pub pull_width: Fix,
    pub pull_str: Fix,
    pub projectile_pull_str: Fix,
    pub wall_pull_str: Fix,
    pub black_hole_pull_str: Fix,
    pub player_pull_str: Fix,
    pub macho_pull_str: Fix,
    pub close_pull_str_mul: Fix,
    pub fast_pull_sqr_radius: Fix,
    pub player_mass: Fix,
    pub ang_vel_on_locked: Fix,
    pub held_lerp: Fix,
    pub player_rock_extra: Vec2,
    pub boulder_extra: Vec2,
    pub raycast_offset: Vec2,
    pub fire_force: Fix,
    pub fire_rock_force: Fix,
    pub fire_boulder_force: Fix,
    pub fire_force_mines: Fix,
    pub fire_missile_force: Fix,
    pub fire_ragdoll_force: Fix,
    pub fire_black_hole_force: Fix,
    pub small_ragdoll: Fix,
    pub fire_ragdoll: Fix,
    pub only_pull: u32,
    pub only_pickup: u32,
    pub curve: Arc<Curve>,
    pub anim_enter: Option<AnimRef>,
    pub anim_prepull: Option<AnimRef>,
    pub anim_idle: Option<AnimRef>,
    pub anim_exit: Option<AnimRef>,
}

impl MagnetCfg {
    fn from_prefab(c: &Content, p: &Prefab) -> MagnetCfg {
        let f = fields(p, "MagnetGun");
        let data = anim_data(c, f.reference("animData"));
        let fx = |k: &str| f.fix_or(k, Fix::ZERO);
        MagnetCfg {
            body: Arc::new(BodyCfg::from_root(c, p)),
            hier: hier_of(p, "MagnetGun"),
            firepoint_offset: f.vec2("FirepointOffset").unwrap_or_default(),
            knockback: fx("KnockBack"),
            aireal_blend: fx("airealControlBlend"),
            max_slow_mo_speed: fx("maxSlowMoSpeed"),
            parry_push_str: fx("parryPushStr"),
            pickup_radius: fx("pickupRadius"),
            max_lock_dist: fx("maxLockDist"),
            max_pull_duration: f.fix_or("maxPullDuration", Fix::from_int(10)),
            pull_width: fx("pullWidth"),
            pull_str: fx("pullStr"),
            projectile_pull_str: fx("projectilePullStr"),
            wall_pull_str: fx("wallPullStr"),
            black_hole_pull_str: fx("blackHolePullStr"),
            player_pull_str: fx("playerPullStr"),
            macho_pull_str: fx("MachoSlimePullStr"),
            close_pull_str_mul: fx("closePullStrMul"),
            fast_pull_sqr_radius: fx("FastPullSqrRadius"),
            player_mass: f.fix_or("playerMass", Fix::from_int(600)),
            ang_vel_on_locked: fx("angVelOnLockedItem"),
            held_lerp: fx("heldPosLerpFactor01"),
            player_rock_extra: f.vec2("playerRockExtraOffset").unwrap_or_default(),
            boulder_extra: f.vec2("boulderExtraOffset").unwrap_or_default(),
            raycast_offset: f.vec2("raycastOffset").unwrap_or_default(),
            fire_force: fx("FireForce"),
            fire_rock_force: fx("FireRockForce"),
            fire_boulder_force: fx("FireBoulderForce"),
            fire_force_mines: fx("FireForceMines"),
            fire_missile_force: fx("FireMissileForce"),
            fire_ragdoll_force: fx("FireRagdollForce"),
            fire_black_hole_force: fx("FireBlackHoleForce"),
            small_ragdoll: fx("smallRagdollDuration"),
            fire_ragdoll: fx("fireRagdollDuration"),
            only_pull: f.int("onlyPull").unwrap_or(209_727_232) as u32,
            only_pickup: f.int("onlyPickup").unwrap_or(142_618_368) as u32,
            curve: f.curve("pullStrMultiplierCurve").unwrap_or_default(),
            anim_enter: anim_named(c, data, "enter"),
            anim_prepull: anim_named(c, data, "prepull"),
            anim_idle: anim_named(c, data, "idle"),
            anim_exit: anim_named(c, data, "exit"),
        }
    }
}

/// `MagnetState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MagnetState {
    #[default]
    Entering,
    Prepulling,
    Pulling,
    Locked,
    Paused,
    Exiting,
}

/// `HeldItemType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Held {
    #[default]
    Nothing,
    Item,
    Player,
    BlackHole,
    Spell,
}

/// Magnet gun state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MagnetGun {
    pub cfg: Arc<MagnetCfg>,
    pub b: AbilityBody,
    pub anim: SpriteAnimator,
    pub state: MagnetState,
    pub held: Held,
    /// Aim `inputVector`.
    pub d: Vec2,
    pub has_fired: bool,
    pub firepoint_offset: Vec2,
    pub extra: Vec2,
    pub held_pos: Vec2,
    /// `heldItem` (the BoplBody object).
    pub held_item: Option<ObjId>,
    /// `heldItemSlimeController` (the victim bopl).
    pub held_player: Option<ObjId>,
    pub held_black_hole: Option<ObjId>,
    /// `pickedupBounceBall` (a Rock ball held as an item).
    pub bounce_ball: Option<ObjId>,
    /// `heldBoulder` (a Throw boulder held as an item).
    pub held_boulder: Option<ObjId>,
    pub time_spent_pulling: Fix,
    pub multiplier_last_frame: Fix,
    pub battery_ticks: i32,
    pub magnet_ticks: i32,
}

/// Global magnet state: this tick's parry circles.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MagnetGlobals {
    /// (centre, radius, magnet-gun ability object).
    pub parry: Vec<(Vec2, Fix, ObjId)>,
}

/// `BoplBody.Hold(frames, magnet)` on an item (the object's own BoplBody
/// state, `object::BoplBody`; black holes also get `BlackHole.Hold`).
fn hold(w: &mut World, item: ObjId, holder: ObjId, frames: i64) {
    if let Some(o) = w.objects.get_mut(item) {
        o.bb.hold(frames, holder);
    }
}

/// The holder recorded on a body (for `OnItemWasStolen`): its
/// `magnetGunHolder` if that is still alive.
fn previous_holder(w: &World, item: ObjId) -> Option<ObjId> {
    w.objects
        .get(item)
        .and_then(|o| o.bb.holder)
        .filter(|m| w.objects.alive(*m))
}

/// `OnItemWasStolen()` if `obj` is a magnet-gun ability object.
pub fn on_item_was_stolen(w: &mut World, obj: ObjId) {
    with_magnet(w, obj, |w, _fw, m, _me| m.on_item_was_stolen(w));
}

fn with_magnet<R>(
    w: &mut World,
    obj: ObjId,
    f: impl FnOnce(&mut World, &mut Framework, &mut MagnetGun, ObjId) -> R,
) -> Option<R> {
    if !matches!(
        w.objects.get(obj).map(|o| &o.kind),
        Some(Kind::Ability(a)) if matches!(a.state, super::AbilityState::MagnetGun(_))
    ) {
        return None;
    }
    w.with_kind(obj, |w, k| match k {
        Kind::Ability(a) => match &mut a.state {
            super::AbilityState::MagnetGun(m) => Some(f(w, &mut a.fw, m, obj)),
            _ => None,
        },
        _ => None,
    })
}

/// `MagnetGun.StaticUpdate()` (engine step 8): parries.
pub fn static_update(w: &mut World) {
    let list = std::mem::take(&mut w.ability_globals.magnet.parry);
    for i in 0..list.len() {
        for j in i + 1..list.len() {
            let (ca, ra, a) = list[i];
            let (cb, rb, b) = list[j];
            let c1 = Circle {
                center: ca,
                radius: ra,
                layer: 0,
            };
            let c2 = Circle {
                center: cb,
                radius: rb,
                layer: 0,
            };
            let ok = |w: &World, g: ObjId| w.objects.alive(g) && w.objects.active_in_hierarchy(g);
            if shapes::circle_circle_bool(&c1, &c2) && ok(w, a) && ok(w, b) {
                let pa = AbilityBody::pos(w, a);
                let pb = AbilityBody::pos(w, b);
                let v = (pa - pb).normalized_safe();
                with_magnet(w, a, |w, _, m, _| m.parry(w, v));
                with_magnet(w, b, |w, _, m, _| m.parry(w, -v));
            }
        }
    }
}

impl MagnetGun {
    fn fire_point(&self, w: &World, me: ObjId) -> Vec2 {
        let t = AbilityBody::tf(w, me);
        let f = self.firepoint_offset;
        let e = self.extra;
        t.position + (f.y + e.y) * t.up + (f.x + e.x) * t.right
    }

    fn begin_then_do(&mut self, w: &World, a: Option<AnimRef>, cb: u16) {
        if let Some(a) = a {
            let c = w.content.clone();
            self.anim.begin_then_do(&c, a, AnimCallback::Ability(cb));
        }
    }

    fn begin(&mut self, w: &World, a: Option<AnimRef>) {
        if let Some(a) = a {
            let c = w.content.clone();
            self.anim.begin(&c, a);
        }
    }

    fn enter_prepull(&mut self, w: &World) {
        self.state = MagnetState::Prepulling;
        let a = self.cfg.anim_prepull;
        self.begin_then_do(w, a, CB_PULL);
    }

    fn enter_pull(&mut self, w: &World) {
        self.state = MagnetState::Pulling;
        let a = self.cfg.anim_idle;
        self.begin(w, a);
    }

    fn enter_pause(&mut self, w: &mut World) {
        self.state = MagnetState::Paused;
        self.held = Held::Nothing;
        let a = self.cfg.anim_idle;
        self.begin(w, a);
        w.emit(Cue::Sound("magnetPause"));
    }

    fn on_item_was_stolen(&mut self, w: &mut World) {
        if self.state != MagnetState::Exiting {
            self.enter_pause(w);
        }
    }

    fn parry(&mut self, w: &mut World, dir: Vec2) {
        if matches!(self.state, MagnetState::Prepulling | MagnetState::Pulling) {
            self.enter_pause(w);
            self.b.body_add_force(dir * self.cfg.parry_push_str);
        }
    }

    fn clear_ignore(fw: &mut Framework) {
        if let Some(c) = fw.collision.as_mut() {
            c.ignored[1] = None;
        }
    }

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

    /// `MagnetGun.ExitAbility(info)`.
    fn exit(&mut self, cx: &mut Cx<'_>, info: AbilityExitInfo) {
        let me = cx.me;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        self.b.unground(cx.w, me, true, false);
        let r = self.fill(cx.w, me, info);
        Self::clear_ignore(cx.fw);
        cx.exit_body(&mut self.b, r);
    }

    fn enter(&mut self, cx: &mut Cx<'_>) {
        let info = cx.info();
        let me = cx.me;
        self.b.sync_physics_to(cx.w, me, &info);
        cx.w.physics.update_physics_positions(&cx.w.objects, me);
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        self.b.body.external_velocity = Vec2::ZERO;
        self.battery_ticks = 0;
        self.magnet_ticks = 0;
        self.time_spent_pulling = Fix::ZERO;
        self.state = MagnetState::Entering;
        let a = self.cfg.anim_enter;
        self.begin_then_do(cx.w, a, CB_PREPULL);
        if !info.is_grounded {
            self.b.gravity_modifier = F_0_03;
            self.b.body.self_imposed_velocity = info.self_imposed_velocity * F_0_1;
            self.b.body.external_velocity = info.external_velocity * F_0_1;
        }
        self.held_item = None;
        self.held_black_hole = None;
        self.held_player = None;
        self.d = info.forwards;
        self.has_fired = false;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        cx.w.emit(Cue::Sound("magnetEnter"));
    }

    /// `UpdateHeldPosition(fp, raycastWalls)`.
    fn update_held(&mut self, w: &mut World, me: ObjId, fp: Vec2, raycast: bool) {
        let bp = AbilityBody::pos(w, me);
        let v = self.held_pos - bp;
        let a = v.magnitude();
        let v2 = v.normalized_safe();
        let v3 = fp - bp;
        let b = v3.magnitude();
        let u = v3.normalized_safe();
        let mut len = a.lerp(b, self.cfg.held_lerp);
        let dir = v2.slerp(u, self.cfg.held_lerp);
        if raycast {
            if self.held == Held::Item {
                let item = self.held_item;
                set_active(w, item, false);
                len = interpolation_wall_raycast(w, bp, dir, len);
                set_active(w, item, true);
            } else if self.held != Held::Spell {
                len = interpolation_wall_raycast(w, bp, dir, len);
            }
        }
        self.held_pos = bp + dir * len;
    }

    /// `RaycastForWalls(d, fp)`.
    fn raycast_for_walls(&self, w: &mut World, me: ObjId, dir: Vec2, fp: Vec2) -> Vec2 {
        let t = AbilityBody::tf(w, me);
        let off = self.extra.x + self.firepoint_offset.x;
        let mut dist = off;
        let range = off + Fix::from_int(1000);
        let base = fp - t.right * off;
        let s = t.scale * self.cfg.raycast_offset;
        let p2 = base + dir * s.x + dir.mul_by_i() * s.y;
        let p3 = base + dir * s.x + dir.mul_by_neg_i() * s.y;
        let r1 = w.physics.raycast_closest_rr(&w.objects, t.position, dir, range);
        let mut r2 = w.physics.raycast_closest_rr(&w.objects, p2, dir, range);
        let mut r3 = w.physics.raycast_closest_rr(&w.objects, p3, dir, range);
        if r2.hit() {
            r2.near_dist += s.x;
        }
        if r3.hit() {
            r3.near_dist += s.x;
        }
        if r1.hit() {
            dist = r1.near_dist;
        }
        if r2.hit() && r3.hit() {
            dist = r2.near_dist.max(r3.near_dist).min(r1.near_dist);
        }
        if dist < off {
            base + dist * self.d
        } else {
            fp
        }
    }

    /// `nullItemTypeIfDropped()`.
    fn null_if_dropped(&mut self, w: &World) {
        if self.state != MagnetState::Locked {
            return;
        }
        let gone = |o: Option<ObjId>| {
            o.is_none_or(|o| !w.objects.alive(o) || !w.objects.active_in_hierarchy(o))
        };
        match self.held {
            Held::Item | Held::Spell => {
                if gone(self.held_item) {
                    self.held_item = None;
                    self.held = Held::Nothing;
                }
            }
            Held::Player => {
                if gone(self.held_player) {
                    self.held_player = None;
                    self.held = Held::Nothing;
                }
            }
            Held::BlackHole => {
                if gone(self.held_black_hole) {
                    self.held_black_hole = None;
                    self.held = Held::Nothing;
                }
            }
            Held::Nothing => {}
        }
    }

    /// `TryPullItems()`.
    fn try_pull(&mut self, w: &mut World, me: ObjId) {
        let c = self.cfg.clone();
        let k = c.curve.eval(self.time_spent_pulling / c.max_pull_duration);
        let fp = self.fire_point(w, me);
        let u = self.d.normalized_safe();
        let mut a = Fix::from_int(1000);
        let r = w
            .physics
            .raycast_closest_rr(&w.objects, fp, u, Fix::from_int(10_000));
        if r.hit() {
            a = r.near_dist / Fix::TWO;
        }
        let a = a.max(F_0_01_D);
        let a2 = c.pull_width.max(F_0_01_D);
        let right = u * a;
        let bx = BoxShape {
            center: fp + right,
            right,
            up: u.mul_by_i() * a2,
            ie: Vec2::new(Fix::ONE / a, Fix::ONE / a2),
            layer: 0,
        };
        let hits = collide_box(w, &bx, c.only_pull, 100);
        let bp = AbilityBody::pos(w, me);
        let my_ground = self
            .b
            .physics
            .is_grounded
            .then(|| self.b.ground(w))
            .flatten()
            .and_then(|g| sticky_hier(w, g));
        for h in hits {
            let Some(o) = w.objects.get(h) else {
                continue;
            };
            let p = o.tf.position;
            let l = o.layer as i32;
            let q = (p - c.firepoint_offset).sqr_magnitude();
            let mut kk = k;
            if l == layer::PROJECTILE {
                if q < c.fast_pull_sqr_radius {
                    kk = kk * c.close_pull_str_mul;
                }
                if has_bopl_body(w, h) {
                    w.physics.add_force(h, -c.projectile_pull_str * kk * u);
                }
            } else if l == layer::WALL {
                if my_ground.is_some() && my_ground == sticky_hier(w, h) {
                    continue;
                }
                let v = p - bp;
                let pos = bp + v.dot(u) * u;
                add_force_at_position(w, h, -c.wall_pull_str * kk * u, pos);
            } else if l == layer::RIGID_BODY_AFFECTOR {
                let f = -c.black_hole_pull_str * u;
                super::black_hole::add_force(w, h, f);
            } else {
                if q < c.fast_pull_sqr_radius {
                    kk = kk * c.close_pull_str_mul;
                }
                if has_bopl_body(w, h) {
                    w.physics.add_force(h, -c.pull_str * kk * u);
                } else {
                    let macho = matches!(&o.kind, Kind::Ability(a)
                        if w.content.abilities.get(a.def).is_some_and(|d| d.name == "Throw"));
                    let s = if macho {
                        c.macho_pull_str
                    } else {
                        c.player_pull_str
                    };
                    super::with_player_body(w, h, |_, _, body| {
                        body.external_velocity -= s * kk * u;
                    });
                }
            }
        }
    }

    /// `TryPickupNearbyItems()`.
    fn try_pickup(&mut self, cx: &mut Cx<'_>) -> bool {
        let me = cx.me;
        let c = self.cfg.clone();
        let mut fp = self.fire_point(cx.w, me);
        let scale = AbilityBody::tf(cx.w, me).scale;
        let r = c.pickup_radius * scale;
        cx.w.set_collider_enabled(me, false);
        let hits = cx
            .w
            .physics
            .circle_cast_all(&cx.w.objects, fp, r, c.only_pickup, 100);
        cx.w.set_collider_enabled(me, true);
        cx.w.ability_globals.magnet.parry.push((fp, r, me));
        let mut best = Fix::MAX;
        let mut pick = None;
        for h in hits {
            let Some(o) = cx.w.objects.get(h) else {
                continue;
            };
            if o.tf.destroyed
                || !o.active
                || (o.layer as i32 == layer::WALL && super::prefab::object_tag(cx.w, h) != "boulder")
            {
                continue;
            }
            if o.layer as i32 == layer::PLAYER
                && let Kind::Bopl(b) = &o.kind
                && !b.ctrl.has_spawned
            {
                continue;
            }
            let d2 = (o.tf.position - fp).sqr_magnitude();
            if d2 < best {
                best = d2;
                pick = Some(h);
            }
        }
        let Some(h) = pick else {
            self.held = Held::Nothing;
            return false;
        };
        let (hl, hscale) = cx
            .w
            .objects
            .get(h)
            .map_or((0, Fix::ONE), |o| (o.layer as i32, o.tf.scale));
        self.extra = Vec2::RIGHT * F_1_5 * (hscale.max(Fix::ONE) - Fix::ONE);
        self.held_boulder = None;
        if self.state == MagnetState::Prepulling {
            self.enter_pull(cx.w);
        }
        if hl != layer::RIGID_BODY_AFFECTOR
            && let Some(col) = cx.fw.collision.as_mut()
        {
            col.ignored[1] = Some(h);
        }
        if hl == layer::PLAYER {
            let kind = cx.w.objects.get(h).map(|o| match &o.kind {
                Kind::Bopl(_) => 0,
                Kind::Ability(_) => 1,
                _ => 2,
            });
            let bounce_ball = super::is_bounce_ball(cx.w, h);
            if bounce_ball {
                self.bounce_ball = Some(h);
                self.extra += c.player_rock_extra;
                self.held_item = Some(h);
                self.held = Held::Item;
                if !has_bopl_body(cx.w, h) {
                    self.held = Held::Nothing;
                    return false;
                }
                if let Some(b) = cx.w.physics.body_mut(h) {
                    b.angular_velocity = c.ang_vel_on_locked;
                }
                let v = cx.w.physics.body(h).map_or(Vec2::ZERO, |b| b.velocity);
                self.b.body_add_force(v / Fix::from_int(35));
                if let Some(m) = previous_holder(cx.w, h).filter(|m| *m != me) {
                    on_item_was_stolen(cx.w, m);
                }
                fp = self.fire_point(cx.w, me);
                let _ = fp;
                self.held_pos = AbilityBody::pos(cx.w, h);
            } else {
                let victim = match kind {
                    Some(1) => {
                        let ctrl = super::controller_of(cx.w, h);
                        let v = super::with_player_body(cx.w, h, |_, _, b| b.velocity())
                            .unwrap_or(Vec2::ZERO);
                        self.b.body_add_force(v / c.player_mass);
                        let info = AbilityExitInfo {
                            facing_right: true,
                            position: self.fire_point(cx.w, me),
                            ..AbilityExitInfo::default()
                        };
                        Self::clear_ignore(cx.fw);
                        super::exit_component(cx.w, h, info);
                        ctrl
                    }
                    Some(0) => {
                        let v = super::with_player_body(cx.w, h, |_, _, b| b.velocity())
                            .unwrap_or(Vec2::ZERO);
                        self.b.body_add_force(v / c.player_mass);
                        Some(h)
                    }
                    _ => None,
                };
                let Some(v) = victim.filter(|v| cx.w.objects.exists(*v)) else {
                    self.held = Held::Nothing;
                    return false;
                };
                self.held_player = Some(v);
                self.held_pos = AbilityBody::pos(cx.w, v);
                // `MagnetSteal(this)`.
                let prev = match cx.w.objects.get(v).map(|o| &o.kind) {
                    Some(Kind::Bopl(b)) => b.ctrl.magnet_holder,
                    _ => None,
                };
                if let Some(p) = prev.filter(|p| cx.w.objects.alive(*p)) {
                    on_item_was_stolen(cx.w, p);
                }
                let small = c.small_ragdoll;
                cx.w.with_kind(v, |w, k| {
                    if let Kind::Bopl(b) = k {
                        b.ctrl.magnet_holder = Some(me);
                        b.unground(w, v, true, true);
                        b.physics.skip_next_update = true;
                        b.apply_ragdoll(w, v, small, Vec2::ZERO, false, true);
                    }
                });
                self.held = Held::Player;
            }
        } else if hl == layer::RIGID_BODY_AFFECTOR {
            let mass = super::black_hole::mass(cx.w, h);
            if !mass.is_some_and(|m| m > Fix::ZERO) {
                self.held = Held::Nothing;
                return false;
            }
            self.held = Held::BlackHole;
            self.held_black_hole = Some(h);
            // `magnetGunHolder?.OnItemWasStolen(); magnetGunHolder = this;
            // Hold(this, 3)` (Hold clears the holder again: inert).
            if let Some(m) = super::black_hole::take_magnet_holder(cx.w, h, me) {
                on_item_was_stolen(cx.w, m);
            }
            super::black_hole::hold(cx.w, h, 3);
            self.held_pos = AbilityBody::pos(cx.w, h);
            let v = cx.w.physics.body(h).map_or(Vec2::ZERO, |b| b.velocity);
            self.b.body_add_force(v / c.player_mass);
        } else {
            self.held_item = Some(h);
            let v = cx.w.physics.body(h).map_or(Vec2::ZERO, |b| b.velocity);
            if hl == layer::WALL && super::prefab::object_tag(cx.w, h) == "boulder" {
                self.held_boulder = Some(h);
                self.extra = c.boulder_extra;
                self.b.body_add_force(v / Fix::from_int(35));
            } else {
                self.b.body_add_force(v / c.player_mass);
            }
            self.held = if hl == layer::SPELL {
                Held::Spell
            } else {
                Held::Item
            };
            if !has_bopl_body(cx.w, h) {
                self.held = Held::Nothing;
                return false;
            }
            if let Some(m) = previous_holder(cx.w, h).filter(|m| *m != me) {
                on_item_was_stolen(cx.w, m);
            }
            self.held_pos = AbilityBody::pos(cx.w, h);
            if let Some(b) = cx.w.physics.body_mut(h) {
                b.angular_velocity = c.ang_vel_on_locked;
            }
            hold(cx.w, h, me, 3);
            if self.b.physics.is_grounded
                && let Some(g) = self.b.ground(cx.w)
                && let Some(bo) = self.held_boulder.filter(|b| cx.w.objects.alive(*b))
            {
                super::throw::boulder_hold(cx.w, bo);
                if g == bo {
                    self.b.unground(cx.w, me, true, true);
                }
            }
        }
        true
    }

    /// `Fire(d)`.
    fn fire(&mut self, cx: &mut Cx<'_>) {
        let me = cx.me;
        self.has_fired = true;
        let a = self.cfg.anim_exit;
        self.begin_then_do(cx.w, a, CB_EXIT);
        let d = self.d;
        AbilityBody::set_up(cx.w, me, Vec2::new(-d.y, d.x));
        let u = d.normalized_safe();
        self.shoot(cx.w, u);
        self.b.gravity_modifier = Fix::ONE;
        if self.b.physics.is_grounded {
            let n = self.b.current_normal(cx.w, me).unwrap_or(Vec2::UP);
            self.b.body.self_imposed_velocity = -n * self.cfg.knockback;
            self.b.unground(cx.w, me, true, false);
        } else {
            self.b.body.self_imposed_velocity = -u * self.cfg.knockback;
        }
        self.state = MagnetState::Exiting;
    }

    /// `Shoot(u)`.
    fn shoot(&mut self, w: &mut World, u: Vec2) {
        if self.state != MagnetState::Locked {
            return;
        }
        self.state = MagnetState::Exiting;
        let c = self.cfg.clone();
        match self.held {
            Held::Item | Held::Spell => {
                let Some(item) = self.held_item else {
                    return;
                };
                let (l, s) = w
                    .objects
                    .get(item)
                    .map_or((0, Fix::ONE), |o| (o.layer as i32, o.tf.scale));
                let mut f = c.fire_force;
                if l == layer::PROJECTILE {
                    f = f * Fix::TWO;
                } else if l == layer::PLAYER {
                    f = c.fire_rock_force;
                } else if l == layer::SPELL {
                    f = f * s;
                } else {
                    let kind = super::interact(w, item, Interaction::ItemKind);
                    if kind == Some(Fix::from_int(super::ITEM_MISSILE)) {
                        if let Some(o) = w.objects.get_mut(item) {
                            o.tf.set_up(u);
                        }
                        if let Some(b) = w.physics.body_mut(item) {
                            b.velocity = c.fire_missile_force * u;
                        }
                        self.held_item = None;
                        self.bounce_ball = None;
                        return;
                    }
                    if kind == Some(Fix::from_int(super::ITEM_MINE))
                        && let Some(b) = w.physics.body_mut(item)
                    {
                        b.velocity = c.fire_force_mines * u;
                    }
                    if self.held_boulder.is_some() {
                        f = c.fire_boulder_force;
                    }
                }
                w.physics.add_force(item, f * u);
                self.held_item = None;
                self.bounce_ball = None;
                self.held_boulder = None;
            }
            Held::Player => {
                let Some(v) = self.held_player.take() else {
                    return;
                };
                let speed = c.fire_ragdoll_force * u;
                let dur = c.fire_ragdoll;
                w.with_kind(v, |w, k| {
                    if let Kind::Bopl(b) = k {
                        // A roped victim's rope end gets air time 1/10.
                        if let Some(l) = b.body.rope
                            && speed.y > Fix::ZERO
                            && let Some(r) = w.physics.ropes.get_mut(l.rope as usize)
                        {
                            r.set_time_in_air(F_TENTH, l.top);
                        }
                        b.apply_ragdoll(w, v, dur, speed, true, true);
                        b.velocity_based_raycasts(w, v, true, F_0_0001_D);
                    }
                });
            }
            Held::BlackHole => {
                if let Some(h) = self.held_black_hole.take() {
                    let m = super::black_hole::mass(w, h).unwrap_or(Fix::ZERO);
                    let f = m * c.fire_black_hole_force * u;
                    super::black_hole::add_force(w, h, f);
                }
            }
            Held::Nothing => {}
        }
    }

    /// `MagnetGun.UpdateSim`.
    fn update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let owner = cx.fw.owner;
        let c = self.cfg.clone();
        let (aim, horizontal) = cx.w.player(owner).map_or((Vec2::ZERO, Fix::ZERO), |p| {
            (p.inputs.aim_vector(), p.inputs.input_vector().x)
        });
        let m = aim.magnitude();
        if m > DEADZONE {
            self.d = aim / m;
        }
        if !cx.w.objects.active_in_hierarchy(me) {
            return;
        }
        self.time_spent_pulling += cx.w.fixed_delta_time(owner, dt);
        if matches!(
            self.state,
            MagnetState::Pulling | MagnetState::Prepulling | MagnetState::Locked
        ) && self.time_spent_pulling > c.max_pull_duration
        {
            self.enter_pause(cx.w);
        }
        let frozen = cx.w.is_time_stopped_for(owner);
        if !frozen && matches!(self.state, MagnetState::Pulling | MagnetState::Prepulling) {
            self.try_pull(cx.w, me);
            if self.try_pickup(cx) {
                self.state = MagnetState::Locked;
                self.time_spent_pulling = Fix::ZERO;
                cx.w.emit(Cue::Sound("magnetPickup"));
                cx.w.emit(Cue::Sound("beamSparks1"));
            }
        }
        let mut fp = self.fire_point(cx.w, me);
        if self.state == MagnetState::Locked {
            self.null_if_dropped(cx.w);
            match self.held {
                Held::Nothing => self.enter_pause(cx.w),
                Held::Spell => {
                    let item = self.held_item.unwrap_or(ObjId::NONE);
                    if (fp - AbilityBody::pos(cx.w, item)).sqr_magnitude() > c.max_lock_dist {
                        self.held_item = None;
                        self.state = MagnetState::Pulling;
                        return;
                    }
                    self.update_held(cx.w, me, fp, true);
                    self.hold_item(cx.w, me, item);
                }
                Held::Item => {
                    let item = self.held_item.unwrap_or(ObjId::NONE);
                    set_active(cx.w, Some(item), false);
                    fp = self.raycast_for_walls(cx.w, me, self.d, fp);
                    set_active(cx.w, Some(item), true);
                    if (fp - AbilityBody::pos(cx.w, item)).sqr_magnitude() > c.max_lock_dist {
                        self.held_item = None;
                        self.state = MagnetState::Pulling;
                        return;
                    }
                    self.update_held(cx.w, me, fp, true);
                    if let Some(o) = cx.w.objects.get_mut(item) {
                        o.tf.position = self.held_pos;
                    }
                    let v = cx.w.physics.body(item).map_or(Vec2::ZERO, |b| b.velocity);
                    self.b.body.external_velocity += v / Fix::from_int(10);
                    if let Some(b) = cx.w.physics.body_mut(item) {
                        b.velocity = Vec2::ZERO;
                    }
                    if self.bounce_ball.is_some_and(|b| cx.w.objects.alive(b)) {
                        let k = cx.w.fixed_delta_time(owner, dt);
                        super::interact(cx.w, item, Interaction::DelayFuse(k));
                    } else if super::interact(cx.w, item, Interaction::ItemKind)
                        == Some(Fix::from_int(super::ITEM_GRENADE))
                    {
                        let k = dt * cx.w.time.player_time_scale;
                        super::interact(cx.w, item, Interaction::DelayFuse(k));
                    }
                    hold(cx.w, item, me, 2);
                    if let Some(bo) = self.held_boulder {
                        super::throw::boulder_hold(cx.w, bo);
                    }
                    move_object_out_of_walls(cx.w, item);
                }
                Held::Player => {
                    fp = self.raycast_for_walls(cx.w, me, self.d, fp);
                    self.update_held(cx.w, me, fp, true);
                    let v = self.held_player.unwrap_or(ObjId::NONE);
                    let hp = self.held_pos;
                    let small = c.small_ragdoll;
                    cx.w.with_kind(v, |w, k| {
                        if let Kind::Bopl(b) = k {
                            if let Some(o) = w.objects.get_mut(v) {
                                o.tf.position = hp;
                            }
                            b.apply_ragdoll(w, v, small, Vec2::ZERO, false, false);
                            b.physics.skip_next_update = true;
                        }
                    });
                }
                Held::BlackHole => {
                    let h = self.held_black_hole.unwrap_or(ObjId::NONE);
                    fp = self.fire_point(cx.w, me);
                    let mass = super::black_hole::mass(cx.w, h)
                        .unwrap_or(Fix::ZERO);
                    if mass > Fix::ZERO {
                        self.update_held(cx.w, me, fp, false);
                        if let Some(o) = cx.w.objects.get_mut(h) {
                            o.tf.position = self.held_pos;
                        }
                    } else {
                        self.enter_pull(cx.w);
                    }
                    super::black_hole::hold(cx.w, h, 3);
                }
            }
        }
        if self.state != MagnetState::Locked {
            Self::clear_ignore(cx.fw);
        }
        if !frozen && !cx.button_down() && !self.has_fired {
            if self.held == Held::Nothing || self.state != MagnetState::Locked {
                cx.w.emit(Cue::Sound("magnetCancel"));
            } else {
                cx.w.emit(Cue::Sound("magnetShoot"));
                cx.w.emit(Cue::SoundPrefix("beamSparks"));
            }
            self.fire(cx);
        }
        if frozen {
            return;
        }
        // Presentation counters (battery / magnet strips, drill sounds).
        if self.state == MagnetState::Pulling {
            let mut k = c.curve.eval(self.time_spent_pulling / c.max_pull_duration);
            if self.multiplier_last_frame <= Fix::THREE && k > Fix::THREE {
                cx.w.emit(Cue::Sound("drill_start"));
            }
            self.multiplier_last_frame = k;
            if k > Fix::THREE {
                k = k * Fix::TWO;
            }
            let n = k.ceiling().to_int();
            self.magnet_ticks = self.magnet_ticks.wrapping_sub(n);
            self.battery_ticks = self.battery_ticks.wrapping_add(n);
        } else if self.state == MagnetState::Locked {
            self.battery_ticks = self.battery_ticks.wrapping_sub(4);
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
            let boulder = self
                .held_boulder
                .filter(|b| self.state == MagnetState::Locked && cx.w.objects.alive(*b));
            if let Some(bo) = boulder {
                cx.w.set_collider_enabled(bo, false);
            }
            let landed = self.b.velocity_based_raycasts(cx.w, me, true, dt);
            if let Some(bo) = boulder {
                cx.w.set_collider_enabled(bo, true);
            }
            if landed && self.has_fired {
                AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
                let r = self.fill(
                    cx.w,
                    me,
                    AbilityExitInfo {
                        just_landed: true,
                        ..AbilityExitInfo::default()
                    },
                );
                Self::clear_ignore(cx.fw);
                cx.exit_body(&mut self.b, r);
            }
        }
        if self.b.physics.is_grounded
            && let Some(r) = self.b.move_(cx.w, me, Vec2::ZERO, dt, true)
        {
            self.exit(cx, r);
        }
        if !self.has_fired {
            let d = self.d;
            AbilityBody::set_up(cx.w, me, Vec2::new(-d.y, d.x));
            let v = self.b.body.self_imposed_velocity;
            if v.magnitude() > c.max_slow_mo_speed {
                self.b.body.self_imposed_velocity = v.normalized() * c.max_slow_mo_speed;
            }
        } else {
            let v0 = self.b.body.self_imposed_velocity;
            self.b.aireal_movement(horizontal);
            let v1 = self.b.body.self_imposed_velocity;
            self.b.body.self_imposed_velocity = v0.lerp(v1, c.aireal_blend);
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

    /// Spell holding: position, velocity transfer, hold.
    fn hold_item(&mut self, w: &mut World, me: ObjId, item: ObjId) {
        if let Some(o) = w.objects.get_mut(item) {
            o.tf.position = self.held_pos;
        }
        let v = w.physics.body(item).map_or(Vec2::ZERO, |b| b.velocity);
        self.b.body.external_velocity += v / Fix::from_int(10);
        if let Some(b) = w.physics.body_mut(item) {
            b.velocity = Vec2::ZERO;
        }
        hold(w, item, me, 2);
        if let Some(bo) = self.held_boulder {
            super::throw::boulder_hold(w, bo);
        }
    }

    /// `LateUpdateSim`.
    fn late_update(&mut self, w: &mut World, me: ObjId) {
        self.null_if_dropped(w);
        if self.state != MagnetState::Locked || self.held != Held::Player {
            return;
        }
        let Some(v) = self.held_player else {
            return;
        };
        let fp = self.fire_point(w, me);
        let held = self.held_pos;
        w.with_kind(v, |w, k| {
            if let Kind::Bopl(b) = k {
                if b.physics.is_grounded {
                    b.unground(w, v, false, false);
                    if let Some(o) = w.objects.get_mut(v) {
                        o.tf.position = fp;
                    }
                }
                if let Some(l) = b.body.rope
                    && let Some(r) = w.physics.ropes.get_mut(l.rope as usize)
                    && r.enabled
                {
                    let e = r.end(l.top);
                    r.segment[e] = held;
                }
            }
        });
    }
}

impl AbilityImpl for MagnetGun {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let p = ability_prefab(w, def).cloned().unwrap_or_default();
        let cfg = Arc::new(MagnetCfg::from_prefab(&w.content, &p));
        MagnetGun {
            b: AbilityBody::from_cfg(cfg.body.clone()),
            anim: super::ability_body::animator(cfg.body.always_stop_during_time_stop),
            state: MagnetState::Entering,
            held: Held::Nothing,
            d: Vec2::new(Fix::ZERO, Fix::ONE),
            has_fired: false,
            firepoint_offset: cfg.firepoint_offset,
            extra: Vec2::ZERO,
            held_pos: Vec2::ZERO,
            held_item: None,
            held_player: None,
            held_black_hole: None,
            bounce_ball: None,
            held_boulder: None,
            time_spent_pulling: Fix::ZERO,
            multiplier_last_frame: Fix::ZERO,
            battery_ticks: 0,
            magnet_ticks: 0,
            cfg,
        }
    }

    fn build(&mut self, w: &mut World, me: ObjId, fw: &mut Framework, _def: &AbilityDef) {
        self.b.install_parts(w, me, fw);
        if let (Some(h), Some(o)) = (self.cfg.hier, w.objects.get_mut(me)) {
            o.add_part(PartKind::Behaviour(part::COMPONENT), h);
        }
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        self.enter(cx);
    }

    fn update_part(&mut self, cx: &mut Cx<'_>, k: PartKind, dt: Fix, late: bool) {
        match (k, late) {
            (PartKind::Behaviour(part::COMPONENT), false) => self.update(cx, dt),
            (PartKind::Behaviour(part::COMPONENT), true) => self.late_update(cx.w, cx.me),
            (PartKind::Animator, false) => match super::ability_body::advance_anim(cx.w, cx.fw.owner, &mut self.anim) {
                Some(AnimCallback::Ability(CB_PREPULL)) => self.enter_prepull(cx.w),
                Some(AnimCallback::Ability(CB_PULL)) => self.enter_pull(cx.w),
                Some(AnimCallback::Ability(CB_EXIT)) => {
                    Self::clear_ignore(cx.fw);
                    self.exit(cx, AbilityExitInfo::default());
                }
                _ => {}
            },
            _ => {
                if let Some(r) = self.b.update_part(cx, k, dt, late) {
                    self.exit(cx, r);
                }
            }
        }
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        super::ability_body::hurt_contact(self, cx, c);
    }

    fn on_scale_changed(&mut self, _cx: &mut Cx<'_>, m: Fix) {
        self.firepoint_offset = self.firepoint_offset * m;
    }

    fn exit_component(&mut self, cx: &mut Cx<'_>, info: AbilityExitInfo) {
        self.exit(cx, info);
    }

    fn body(&self) -> Option<&AbilityBody> {
        Some(&self.b)
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        Some(&mut self.b)
    }

    fn sprite(&self) -> crate::data::SpriteId {
        self.anim.sprite
    }

    /// The gun body; battery, magnet and magnet top (objects 1–3, their
    /// tick-driven frames of `battery` / `magnet` / `magnetTOP`) while
    /// pulling, locked or paused; the idle spark (object 4) unless
    /// entering, pre-pulling, paused or exiting; the held aura
    /// (`heldParticlePrefab`) around a held item.
    fn look(&self, w: &World, me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        use crate::view::{Attach, PartLook};
        l.frame(&self.anim);
        let shown = matches!(
            self.state,
            MagnetState::Pulling | MagnetState::Locked | MagnetState::Paused
        );
        let data = super::prefab::object_fields(w, me, "MagnetGun")
            .and_then(|f| f.reference("animData"))
            .and_then(|r| anim_data(&w.content, Some(r)));
        let frame = |name: &str, ticks: i32| -> crate::data::SpriteId {
            let Some(a) = data.and_then(|d| w.content.anim_ref(d, name)) else {
                return crate::data::SpriteId::NONE;
            };
            let anim = w.content.anim(a);
            let total = anim.total().max(1);
            let mut t = ticks.rem_euclid(total);
            for f in &anim.frames {
                let d = f.duration.abs();
                if t < d {
                    return f.sprite;
                }
                t -= d;
            }
            anim.frames.last().map_or(crate::data::SpriteId::NONE, |f| f.sprite)
        };
        for (obj, name, ticks) in [
            (1u16, "battery", self.battery_ticks),
            (2, "magnet", self.magnet_ticks),
            (3, "magnetTOP", self.magnet_ticks),
        ] {
            if shown {
                l.parts.push(PartLook::new(obj).frame(frame(name, ticks), false));
            } else {
                l.parts.push(PartLook::hidden(obj));
            }
        }
        l.parts.push(
            if matches!(
                self.state,
                MagnetState::Entering
                    | MagnetState::Prepulling
                    | MagnetState::Paused
                    | MagnetState::Exiting
            ) {
                PartLook::hidden(4)
            } else {
                PartLook::new(4)
            },
        );
        if let Some(h) = self.held_item.filter(|h| w.objects.exists(*h))
            && self.state == MagnetState::Locked
        {
            l.attach.push(Attach {
                field: "heldParticlePrefab",
                position: AbilityBody::pos(w, h),
                rotation: Fix::ZERO,
                scale: Fix::ONE,
                color: None,
                shadow_color: false,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Physics helpers used by the magnet
// ---------------------------------------------------------------------------

fn set_active(w: &mut World, o: Option<ObjId>, a: bool) {
    if let Some(ob) = o.and_then(|o| w.objects.get_mut(o)) {
        ob.active = a;
    }
}

/// The object has a `BoplBody` component.
pub fn has_bopl_body(w: &World, o: ObjId) -> bool {
    w.objects
        .get(o)
        .is_some_and(|ob| ob.part(PartKind::BoplBody).is_some())
}

fn sticky_hier(w: &World, o: ObjId) -> Option<i32> {
    w.objects
        .get(o)
        .and_then(|ob| ob.part(PartKind::Sticky))
        .map(|p| p.hier)
}

/// `interpolationWallRaycast(origin, dir, distance)`.
fn interpolation_wall_raycast(w: &mut World, origin: Vec2, dir: Vec2, dist: Fix) -> Fix {
    let r = w
        .physics
        .raycast_closest_rr(&w.objects, origin, dir, Fix::from_int(1000));
    if r.hit() && r.near_dist < dist {
        r.near_dist
    } else {
        dist
    }
}

/// `BoplBody.AddForceAtPosition(f, pos)` (Force mode): an immediate impulse
/// with lever `normalizeSafe(pos − position)` (physics.md §2.3).
pub fn add_force_at_position(w: &mut World, obj: ObjId, f: Vec2, pos: Vec2) {
    let p = AbilityBody::pos(w, obj);
    let lever = (pos - p).normalized_safe();
    w.physics.apply_impulse(obj, f, lever);
}

/// `DetPhysics.CollideBox(box, mask)` (physics.md §9.3): circles, boxes,
/// rounded rects overlapping the box, list order, at most `max`.
pub fn collide_box(w: &mut World, bx: &BoxShape, mask: u32, max: usize) -> Vec<ObjId> {
    w.physics.remove_destroyed(&w.objects);
    let p = &w.physics;
    let mut out = Vec::new();
    let in_mask = |l: i32| mask >> (l & 31) & 1 != 0;
    for i in 0..p.circles.len() {
        let s = &p.circles.shapes[i];
        let o = p.circles.parents[i].obj;
        if out.len() < max && in_mask(s.layer) && super::ability_body::collider_active(w, o) && shapes::circle_box_bool(s, bx)
        {
            out.push(o);
        }
    }
    for i in 0..p.boxes.len() {
        let s = &p.boxes.shapes[i];
        let o = p.boxes.parents[i].obj;
        if out.len() < max && in_mask(s.layer) && super::ability_body::collider_active(w, o) && shapes::box_box_bool(s, bx) {
            out.push(o);
        }
    }
    for i in 0..p.rrs.len() {
        let s = &p.rrs.shapes[i];
        let o = p.rrs.parents[i].obj;
        if out.len() < max && in_mask(s.layer) && super::ability_body::collider_active(w, o) && shapes::box_rr_bool(bx, s) {
            out.push(o);
        }
    }
    out
}

/// A manifold test of the moved collider against one shape kind.
type ManifoldFn<T> = Box<dyn Fn(&T) -> Option<Manifold>>;

/// `DetPhysics.MoveObjectOutOfWalls(collider)` (physics.md §9.3).
pub fn move_object_out_of_walls(w: &mut World, obj: ObjId) {
    w.physics.update_physics_positions(&w.objects, obj);
    w.physics.remove_destroyed(&w.objects);
    let Some((kind, idx)) = w.physics.find(obj) else {
        return;
    };
    let p = &w.physics;
    let (center, test_rr, test_c, test_b): (
        Vec2,
        ManifoldFn<shapes::RoundedRect>,
        ManifoldFn<Circle>,
        ManifoldFn<BoxShape>,
    ) = match kind {
        ShapeKind::Circle => {
            let s = p.circles.shapes[idx];
            (
                s.center,
                Box::new(move |r| shapes::circle_rr(&s, r)),
                Box::new(move |c| shapes::circle_circle(&s, c)),
                Box::new(move |b| shapes::circle_box(&s, b)),
            )
        }
        ShapeKind::Box => {
            let s = p.boxes.shapes[idx];
            (
                s.center,
                Box::new(move |r| shapes::box_rr(&s, r)),
                Box::new(move |c| shapes::circle_box(c, &s)),
                Box::new(move |b| shapes::box_box(&s, b)),
            )
        }
        _ => {
            let s = p.rrs.shapes[idx];
            (
                s.b.center,
                Box::new(move |r| shapes::rr_rr(&s, r)),
                Box::new(move |c| shapes::circle_rr(c, &s)),
                Box::new(move |b| shapes::box_rr(b, &s)),
            )
        }
    };
    let terrain = |l: i32| {
        l == layer::WALL || l == layer::LETHAL_TERRAIN || l == layer::NON_LETHAL_TERRAIN
    };
    let mut found = None;
    for i in 0..p.rrs.len() {
        let o = p.rrs.parents[i].obj;
        if o != obj
            && super::ability_body::collider_active(w, o)
            && let Some(m) = test_rr(&p.rrs.shapes[i])
        {
            found = Some(m);
            break;
        }
    }
    if found.is_none() {
        for i in 0..p.circles.len() {
            let o = p.circles.parents[i].obj;
            let s = &p.circles.shapes[i];
            if o != obj
                && terrain(s.layer)
                && super::ability_body::collider_active(w, o)
                && let Some(m) = test_c(s)
            {
                found = Some(m);
                break;
            }
        }
    }
    if found.is_none() {
        for i in 0..p.boxes.len() {
            let o = p.boxes.parents[i].obj;
            let s = &p.boxes.shapes[i];
            if o != obj
                && terrain(s.layer)
                && super::ability_body::collider_active(w, o)
                && let Some(m) = test_b(s)
            {
                found = Some(m);
                break;
            }
        }
    }
    if let Some(m) = found {
        let (a, _) = shapes::positional_correction(&m, Fix::ONE, Fix::ZERO, Fix::ONE);
        if let Some(o) = w.objects.get_mut(obj) {
            o.tf.position = center + a;
        }
    }
}

//! Duplicator (`docs/spec/abilities/duplicator.md`).
//!
//! The gun body is [`GunTransform`](super::blink_gun::GunTransform); the
//! shot is [`ShootDuplicator`]. This module also owns object duplication
//! (`DuplicateObject` with the clone rule of duplicator.md), the
//! duplicated-objects list ([`update_duplicates`], engine step 5), the
//! `DuplicatePlatformEffect` and the `Rock_noPlayer` boulder.

use std::sync::Arc;

use super::blink_gun::{GunAbility, GunTransform, self_hit};
use super::ability_body::{AbilityBody, water_height};
use super::prefab::{ability_prefab, fields, prefab_by_ref, updatables, root_position};
use super::blink_gun::layer;
use super::magnet_gun::has_bopl_body;
use super::prelude::*;
use super::{Behaviour, ITEM_ROCKET_ENGINE, ITEM_SPARK_NODE, ITEM_SPIKE};
use crate::bopl::Bopl;
use crate::data::{Content, Prefab};
use crate::physics::shapes::{self, ShapeKind};
use crate::physics::{ColliderDef, RayHit};
use crate::platform::Platform;
use crate::surface::Surface;
use crate::view::{EffectKind, Visual};
use crate::world::Kind;

/// The Duplicator ability.
pub type Duplicator = GunAbility<ShootDuplicator>;

/// Objects this module spawns.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    PlatformEffect(PlatformEffect),
    Rock(RockNoPlayer),
}

/// `maxDistance` (code literal 100).
const MAX_DISTANCE: Fix = Fix(429_496_729_600);
/// `lifespanFrames` (code constant 1800 = 30 s).
pub const LIFESPAN_FRAMES: i32 = 1800;
/// Black-hole clone lifespan (code literal 480).
const BLACK_HOLE_LIFESPAN: i32 = 480;
/// `framesBeforeDuplicatesStartShaking` (60).
const SHAKE_FRAMES: i32 = 60;
/// `(Fix)0.25` (double).
const F_0_25_D: Fix = Fix(1_073_741_824);
/// `(Fix)1.3` (double): the clone's pop-up factor.
const F_1_3_D: Fix = Fix(5_583_457_484);
/// `(Fix)0.1` (double): grenade clone fuse step (applied by the grenade).
const F_0_1_D: Fix = Fix(429_496_729);
/// `(Fix)0.35f`: the platform clone's spawn-sound lead.
const F_0_35: Fix = Fix(1_503_238_528);
/// `(Fix)0.75` (double): spike re-initialisation offset.
pub const F_0_75_D: Fix = Fix(3_221_225_472);
/// `(Fix)2.2` (double): spark node re-initialisation offset.
pub const F_2_2_D: Fix = Fix(9_448_928_051);

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// `ShootDuplicator` prefab values and the prefabs it spawns.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DupCfg {
    pub triple_radius: Fix,
    pub collision_mask: u32,
    pub collision_mask_no_water: u32,
    pub f005: Fix,
    pub f032: Fix,
    pub f001: Fix,
    pub f03: Fix,
    pub use_old_platform_behaviour: bool,
    pub platform_shake_duration: Fix,
    pub platform_shake: Fix,
    pub effect: EffectCfg,
    pub rock: RockCfg,
}

/// `DuplicatePlatformEffect` prefab values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EffectCfg {
    pub prefab: Option<usize>,
    pub hier: i32,
    pub updatables: i32,
    pub overlap_force: Fix,
    pub scale_loss_speed: Fix,
}

/// `Rock_noPlayer` prefab values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RockCfg {
    pub prefab: Option<usize>,
    pub layer: u8,
    pub collider: Option<ColliderDef>,
    pub hier_bounds: i32,
    pub hier_body: i32,
    pub hier_collider: i32,
    pub updatables: i32,
    pub can_hurt_owner: bool,
    pub destroy_x: bool,
    pub destroy_y: bool,
    pub do_not_destroy_up: bool,
    pub down_effect: (i32, Vec2),
    pub up_effect: (i32, Vec2),
    pub x_effect: (i32, Vec2),
}

fn rock_cfg(c: &Content, idx: Option<usize>) -> RockCfg {
    let p = idx.and_then(|i| c.prefabs.get(i));
    let root = p.and_then(|p| p.objects.first());
    let null = Value::Null;
    let f = |class: &str| p.map_or(&null, |p| fields(p, class));
    let bf = f("DestroyIfOutsideSceneBounds");
    let eff = |k: &str| {
        let i = prefab_by_ref(c, bf.reference(k));
        (updatables(c, i), root_position(c, i))
    };
    let hier = |class: &str, d: i32| p.and_then(|p| super::prefab::hier_of(p, class)).unwrap_or(d);
    RockCfg {
        prefab: idx,
        layer: root.map_or(16, |o| o.layer),
        collider: root.and_then(|o| o.collider),
        hier_bounds: hier("DestroyIfOutsideSceneBounds", 1),
        hier_body: hier("BoplBody", 2),
        hier_collider: root.and_then(|o| o.collider_hier).unwrap_or(3),
        updatables: updatables(c, idx).max(3),
        can_hurt_owner: f("Explosion").boolean("canHurtOwner").unwrap_or(true),
        destroy_x: bf.boolean("DestroyIfTooFarX").unwrap_or(true),
        destroy_y: bf.boolean("DestroyIfTooFarY").unwrap_or(true),
        do_not_destroy_up: bf.boolean("DoNotDestroyUp").unwrap_or(false),
        down_effect: eff("DownEffect"),
        up_effect: eff("UpEffect"),
        x_effect: eff("XEffect"),
    }
}

impl DupCfg {
    fn from_prefab(c: &Content, p: &Prefab) -> DupCfg {
        let f = fields(p, "ShootDuplicator");
        let eidx = prefab_by_ref(c, f.reference("platformEffect"));
        let ep = eidx.and_then(|i| c.prefabs.get(i));
        let ef = ep.map_or(&Value::Null, |p| fields(p, "DuplicatePlatformEffect"));
        DupCfg {
            triple_radius: f.fix_or("tripleRaycastRadius", Fix(1_503_238_528)),
            collision_mask: f.int("collisionMask").unwrap_or(82_390_800) as u32,
            collision_mask_no_water: f.int("collisionMaskNoWater").unwrap_or(115_945_216) as u32,
            f005: f.fix_or("f005", Fix(214_748_368)),
            f032: f.fix_or("f032", Fix(1_374_389_504)),
            f001: f.fix_or("f001", Fix(42_949_672)),
            f03: f.fix_or("f03", Fix(1_288_490_240)),
            use_old_platform_behaviour: f.boolean("useOldPlatformBehaviour").unwrap_or(true),
            platform_shake_duration: f.fix_or("PlatformShakeDuration", Fix(5_583_457_280)),
            platform_shake: f.fix_or("platformShake", Fix(1_288_490_240)),
            effect: EffectCfg {
                prefab: eidx,
                hier: ef.int("hierarchyNumber").unwrap_or(1) as i32,
                updatables: updatables(c, eidx).max(1),
                overlap_force: ef.fix_or("platformOverlapForce", Fix(214_748_364_800)),
                scale_loss_speed: ef.fix_or("scaleLossSpeed", Fix(128_849_016)),
            },
            rock: rock_cfg(c, prefab_by_ref(c, f.reference("bounceBallPrefab"))),
        }
    }
}

// ---------------------------------------------------------------------------
// The shot
// ---------------------------------------------------------------------------

/// `ShootDuplicator`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShootDuplicator {
    pub cfg: Arc<DupCfg>,
}

impl super::blink_gun::Gun for ShootDuplicator {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let p = ability_prefab(w, def).cloned().unwrap_or_default();
        ShootDuplicator {
            cfg: Arc::new(DupCfg::from_prefab(&w.content, &p)),
        }
    }

    fn shoot(&mut self, cx: &mut Cx<'_>, _gun: &mut GunTransform, fp: Vec2, dir: Vec2) {
        shoot(cx, &self.cfg, fp, dir, false);
    }
}

fn set_active(w: &mut World, id: ObjId, a: bool) {
    if let Some(o) = w.objects.get_mut(id) {
        o.active = a;
    }
}

/// `ShootDuplicator.Shoot(fp, dir, owner, alreadyHitWater)`.
fn shoot(cx: &mut Cx<'_>, c: &Arc<DupCfg>, fp: Vec2, dir: Vec2, bounce: bool) {
    let me = cx.me;
    cx.w.emit(Cue::Sound("fireRaygun"));
    cx.w.emit(Cue::SoundPrefix("beamSparks"));
    let mask = if bounce {
        c.collision_mask_no_water
    } else {
        c.collision_mask
    };
    let mut hit = cx.w.physics.point_check_all_rrs(&cx.w.objects, fp);
    if !bounce {
        set_active(cx.w, me, false);
    }
    if !hit.hit() {
        hit = cx.w.physics.raycast_closest_triple(
            &cx.w.objects,
            fp,
            dir,
            MAX_DISTANCE,
            mask,
            c.triple_radius,
        );
    }
    if !bounce {
        set_active(cx.w, me, true);
    }
    let mut d = dir;
    let mut origin = fp;
    let o = cx.fw.owner;
    let ring = |cx: &mut Cx<'_>, x: Fix| {
        super::fx(cx.w, me, "WaterRing", Vec2::new(x, Fix::ZERO), Fix::ZERO, Fix::ONE, 0, Vec2::ZERO);
    };
    if (!bounce && fp.y <= water_height(cx.w)) || (bounce && hit.layer == layer::WATER) {
        cx.w.emit(Cue::Sound("waterExplosion"));
        ring(cx, hit.near_pos.x);
        hit = self_hit(cx.w, me, c.f001);
        if !bounce {
            d = Vec2::UP;
            origin = hit.near_pos;
        }
    }
    if !hit.hit() {
        super::ray_fx(
            cx.w,
            me,
            "RaycastParticlePrefab",
            "OnHitParticlePrefab",
            origin,
            d,
            MAX_DISTANCE,
            false,
            None,
            o,
        );
        return;
    }
    super::ray_fx(
        cx.w,
        me,
        "RaycastParticlePrefab",
        "OnHitParticlePrefab",
        origin,
        d,
        hit.near_dist,
        hit.layer != layer::WATER,
        None,
        o,
    );
    if hit.layer == layer::WATER {
        ring(cx, hit.near_pos.x);
        let m = Vec2::new(d.x, -d.y);
        shoot(cx, c, hit.near_pos, m, true);
        cx.w.emit(Cue::Sound("waterExplosion"));
        return;
    }
    let obj_pos = AbilityBody::pos(cx.w, hit.obj);
    apply(cx, c, &hit, obj_pos, hit.near_pos, d, origin);
}

/// `FanOut(n, d, p, offset, a, i)`.
fn fan_out(n: i32, d: Vec2, p: Vec2, offset: Fix, a: Fix, i: i32) -> Vec2 {
    let rot = |t: Fix| d.complex_mul(Vec2::from_angle(t));
    let u = match (n, i) {
        (2, 0) | (3, 1) | (4, 0) => rot(a),
        (2, 1) | (3, 2) | (4, 1) => rot(-a),
        (4, 2) => rot(a * Fix::THREE),
        (4, 3) => rot(-a * Fix::THREE),
        _ => d,
    };
    p + u * offset
}

/// The `Item` component of an object (from its prefab): (count, pattern,
/// spacing, angle).
fn item_data(w: &World, obj: ObjId) -> Option<(i32, i64, Fix, Fix)> {
    let Visual::Prefab { prefab, .. } = w.objects.get(obj)?.visual else {
        return None;
    };
    let p = w.content.prefabs.get(prefab as usize)?;
    let (_, item) = p.find_component("Item")?;
    let f = &item.fields;
    Some((
        f.int("nrOfClonesWhenDuplicated").unwrap_or(1) as i32,
        f.int("duplicatePattern").unwrap_or(0),
        f.fix_or("duplicate_spacing", Fix::ZERO),
        f.fix_or("duplicate_angle", Fix::ZERO),
    ))
}

fn body_velocity(w: &World, o: ObjId) -> Vec2 {
    w.physics.body(o).map_or(Vec2::ZERO, |b| b.velocity)
}

fn obj_scale(w: &World, o: ObjId) -> Fix {
    w.objects.get(o).map_or(Fix::ONE, |ob| ob.tf.scale)
}

/// `ApplyDuplicationEffect(hit, objPos, hitPos, obj, dir, fp, layer)`.
fn apply(
    cx: &mut Cx<'_>,
    c: &Arc<DupCfg>,
    hit: &RayHit,
    obj_pos: Vec2,
    hit_pos: Vec2,
    d: Vec2,
    _fp: Vec2,
) {
    let obj = hit.obj;
    let shooter = cx.fw.owner;
    let l = hit.layer;
    if l == layer::PROJECTILE {
        if !has_bopl_body(cx.w, obj) {
            return;
        }
        let scale = obj_scale(cx.w, obj);
        let offset = Fix::from_int(7) * scale;
        for i in 0..4 {
            let pos = fan_out(4, d, obj_pos, offset, c.f032, i);
            let v0 = body_velocity(cx.w, obj);
            let v = (pos - obj_pos).normalized_safe() * (v0.magnitude() + Fix::from_int(10));
            if let Some(cl) = duplicate_body(cx.w, pos, obj, v, scale) {
                fixups(cx.w, cl, obj, i, shooter);
            }
        }
    } else if l == layer::ITEM || l == layer::SMOKE {
        let Some((count, pattern, spacing, angle)) = item_data(cx.w, obj) else {
            return;
        };
        if !has_bopl_body(cx.w, obj) {
            return;
        }
        let scale = obj_scale(cx.w, obj);
        let v_orig = body_velocity(cx.w, obj);
        let perp = v_orig.normalized_safe().mul_by_i();
        let sp = spacing * scale;
        let mut pos = hit_pos;
        let up = AbilityBody::tf(cx.w, obj).up;
        for j in 0..count {
            let mut v = body_velocity(cx.w, obj);
            match pattern {
                0 => {
                    let s = sp * (Fix::from_int(j) + Fix::ONE);
                    if j % 2 == 0 {
                        pos += perp * s;
                    } else {
                        pos -= perp * s;
                    }
                }
                1 => {
                    let k = Fix::from_int(j + 1) / Fix::from_int(count);
                    pos = obj_pos + Vec2::from_angle(Fix::PI_TIMES_2 * k) * sp;
                }
                2 => {
                    let u = cx.w.rng.random_unit_vector();
                    pos = obj_pos + u * sp;
                }
                _ => {
                    pos = fan_out(count, d, obj_pos, sp, angle, j);
                    v = (pos - obj_pos).normalized_safe() * (v.magnitude() + Fix::from_int(10));
                }
            }
            if let Some(cl) = duplicate_body(cx.w, pos, obj, v, scale) {
                if let Some(o) = cx.w.objects.get_mut(cl) {
                    o.tf.set_up(up);
                }
                fixups(cx.w, cl, obj, j, shooter);
            }
        }
    } else if l == layer::RIGID_BODY_AFFECTOR {
        let Some(mass) = super::black_hole::mass(cx.w, obj) else {
            return;
        };
        cx.w.emit(Cue::Sound("blackholeNoClap"));
        let r = cx.w.physics.circle(obj).map_or(Fix::ZERO, |c| c.radius);
        let pos = obj_pos + d * r * Fix::TWO;
        if let Some(cl) = duplicate_object(cx.w, pos, obj, BLACK_HOLE_LIFESPAN, None) {
            super::black_hole::set_mass(cx.w, cl, mass);
            let f = mass * d * Fix::from_int(6);
            super::black_hole::add_force(cx.w, cl, f);
            fixups(cx.w, cl, obj, 0, shooter);
        }
    } else if l == layer::WEAPON {
        for k in 0..3 {
            let scale = obj_scale(cx.w, obj);
            let pos = fan_out(3, d, obj_pos, Fix::TWO * scale, Fix::ONE, k);
            let v0 = body_velocity(cx.w, obj);
            let v = (pos - obj_pos).normalized_safe() * (v0.magnitude() + Fix::from_int(15));
            let s = obj_scale(cx.w, obj);
            if let Some(cl) = duplicate_body(cx.w, pos, obj, v, s)
                && let Some(Kind::Behaviour(b)) = cx.w.objects.get_mut(cl).map(|o| &mut o.kind)
                && let Behaviour::Pickup(p) = b.as_mut()
            {
                p.init_self = true;
            }
        }
    } else if l == layer::WALL {
        cx.w.emit(Cue::Sound("fireDuplicatorOnPlatform"));
        if !matches!(cx.w.objects.get(obj).map(|o| &o.kind), Some(Kind::Platform(_))) {
            return;
        }
        if let Some(e) = current_effect(cx.w, obj) {
            cx.w.destroy(e);
        }
        if !c.use_old_platform_behaviour {
            // The boulder path of the newer behaviour is not shipped.
            return;
        }
        crate::platform::add_shake(
            cx.w,
            obj,
            c.platform_shake_duration,
            c.platform_shake,
            30,
            None,
        );
        if cx.w.physics.rr(obj).is_none() {
            return;
        }
        let t = AbilityBody::tf(cx.w, obj);
        let pos = platform_spawn_pos(cx.w, obj, d, t.right, t.up, t.position, c.f005);
        let key = cx
            .w
            .objects
            .get(cx.me)
            .and_then(|o| o.part(PartKind::Behaviour(super::blink_gun::part::COMPONENT)))
            .map_or(-1, |p| p.hier);
        let e = spawn_effect(cx.w, &c.effect, pos);
        effect_initialize(cx.w, e, obj, d, c.platform_shake_duration, key, &c.effect);
    } else if l == layer::LETHAL_TERRAIN || l == layer::NON_LETHAL_TERRAIN {
        let s = obj_scale(cx.w, obj);
        let Some(cl) = duplicate_body(cx.w, hit_pos, obj, Vec2::ZERO, s) else {
            return;
        };
        let kind = super::interact(cx.w, cl, Interaction::ItemKind);
        let (step, k) = match kind {
            Some(x) if x == Fix::from_int(ITEM_ROCKET_ENGINE) => (Fix::from_int(7), Fix::ONE),
            Some(x) if x == Fix::from_int(ITEM_SPIKE) => (Fix::from_int(7), -Fix::ONE),
            Some(x) if x == Fix::from_int(ITEM_SPARK_NODE) => (Fix::from_int(5), Fix::ONE),
            _ => {
                if has_bopl_body(cx.w, obj) {
                    let p = obj_pos + d * Fix::from_int(4) * s;
                    if let Some(o) = cx.w.objects.get_mut(cl) {
                        o.tf.position = p;
                    }
                    let v0 = body_velocity(cx.w, obj);
                    let v = (p - obj_pos).normalized_safe() * (v0.magnitude() + Fix::from_int(15));
                    set_start_velocity(cx.w, cl, v);
                    fixups(cx.w, cl, obj, -1, shooter);
                }
                return;
            }
        };
        // A planted engine / spike / spark node moves along its platform.
        let Some(plat) = cx.w.objects.get(obj).and_then(|o| o.parent) else {
            return;
        };
        let Some(surf) = Surface::of(cx.w, plat) else {
            return;
        };
        let ot = AbilityBody::tf(cx.w, obj);
        if let Some(o) = cx.w.objects.get_mut(cl) {
            o.tf.offset = ot.offset;
        }
        let sign = (ot.right * k).dot(d).sign2_fix();
        let local = surf.param_from_position(ot.position, Fix::ZERO);
        let circ = surf.path_len(Fix::ZERO);
        let t = (local + step * sign / circ + Fix::from_int(1000)) % Fix::ONE;
        let p = surf.position_from_param(t, Fix::ZERO);
        let n = surf.normal(p);
        if let Some(o) = cx.w.objects.get_mut(cl) {
            o.tf.set_up(k * n);
        }
        cx.w.objects.set_parent(cl, Some(plat));
        if let Some(o) = cx.w.objects.get_mut(cl) {
            o.tf.position = p;
        }
        // Re-initialise the planted clone on the platform (scale = the hit
        // object's): spike `Initialize(p, 0.75, …)`, engine
        // `RocketEngineInit(…, 0.3, p, …, shooter)`, spark node
        // `Initialize(shooter, none, 2.2, t, …)`.
        match kind {
            Some(x) if x == Fix::from_int(ITEM_SPIKE) => {
                super::spike::init_duplicate(cx.w, cl, p, F_0_75_D, plat, s);
            }
            Some(x) if x == Fix::from_int(ITEM_ROCKET_ENGINE) => {
                super::engine::init_duplicate(cx.w, cl, plat, c.f03, p, s, shooter);
            }
            _ => super::tesla_coil::init_duplicate(cx.w, cl, shooter, F_2_2_D, t, plat, s),
        }
    } else if l == layer::PLAYER {
        duplicate_player(cx, c, obj, obj_pos, d);
    }
    // orb: no object of the game's data is on the orb layer; other layers
    // (EffectorZone, ...) have no effect beyond the ray.
}

/// The `Interaction::Duplicated` fix-ups of a clone.
fn fixups(w: &mut World, clone: ObjId, original: ObjId, index: i32, shooter: i32) {
    super::interact(
        w,
        clone,
        Interaction::Duplicated {
            original,
            index,
            shooter,
        },
    );
}

/// `ShootDuplicator.PlatformSpawnPos(dir, r, u, startPos, srr)`.
fn platform_spawn_pos(
    w: &World,
    plat: ObjId,
    dir: Vec2,
    r: Vec2,
    u: Vec2,
    start: Vec2,
    small: Fix,
) -> Vec2 {
    let Some(rr) = w.physics.rr(plat) else {
        return start;
    };
    let e = Vec2::new(Fix::ONE / rr.b.ie.x, Fix::ONE / rr.b.ie.y);
    let (r, u) = if e.x < small && e.y < small {
        (Vec2::RIGHT, Vec2::UP)
    } else {
        (r, u)
    };
    let s = obj_scale(w, plat);
    let vu = u.dot(dir);
    let vr = r.dot(dir);
    if vu.abs() > vr.abs() {
        start + vu.sign_fix() * Fix::TWO * u * (rr.radius + e.y + s)
    } else {
        start + vr.sign_fix() * Fix::TWO * r * (rr.radius + e.x + s)
    }
}

/// The player branch: a Rock ball becomes a `Rock_noPlayer`; a player gets
/// an ability-less clone.
fn duplicate_player(cx: &mut Cx<'_>, c: &Arc<DupCfg>, obj: ObjId, obj_pos: Vec2, d: Vec2) {
    let is_rock = super::is_bounce_ball(cx.w, obj);
    let (ctrl, is_rock) = match cx.w.objects.get(obj).map(|o| &o.kind) {
        Some(Kind::Ability(a)) => (Some(a.fw.controller), is_rock),
        Some(Kind::Bopl(_)) => (Some(obj), false),
        Some(Kind::Empty) if obj == cx.me => (Some(cx.fw.controller), false),
        _ => (None, false),
    };
    if is_rock {
        if !has_bopl_body(cx.w, obj) {
            return;
        }
        let s = obj_scale(cx.w, obj);
        let rot = AbilityBody::tf(cx.w, obj).rotation;
        let v0 = body_velocity(cx.w, obj);
        let owner = ctrl
            .and_then(|b| cx.w.objects.get(b))
            .map_or(0, |o| o.player_id);
        let pos = obj_pos + d * Fix::from_int(4) * s;
        let v = (pos - obj_pos).normalized_safe() * (v0.magnitude() + Fix::from_int(15));
        let r = spawn_rock(cx.w, &c.rock, obj_pos, rot, owner);
        if let Some(o) = cx.w.objects.get_mut(r) {
            o.tf.position = pos;
        }
        set_start_velocity(cx.w, r, v);
        cx.w.set_collider_scale(r, s);
        add_to_list(cx.w, r, LIFESPAN_FRAMES, None);
        return;
    }
    let Some(ctrl) = ctrl.filter(|b| cx.w.objects.exists(*b)) else {
        return;
    };
    let pid = cx.w.objects.get(obj).map_or(0, |o| o.player_id);
    // The original's current normal (grounded: at its body, else up).
    let normal = match cx.w.objects.get(ctrl).map(|o| (&o.kind, o.tf.position)) {
        Some((Kind::Bopl(b), pos)) if b.physics.is_grounded => b
            .physics
            .attached_ground
            .and_then(|g| Surface::of(cx.w, g))
            .map_or(Vec2::UP, |s| s.normal(pos)),
        _ => Vec2::UP,
    };
    let Some(alive) = cx.w.player(pid).map(|p| p.players_and_clones_still_alive) else {
        return;
    };
    if alive > MAX_CLONES {
        return;
    }
    if let Some(p) = cx.w.player_mut(pid) {
        p.players_and_clones_still_alive += 1;
    }
    let clone = Bopl::spawn(cx.w, obj_pos, pid);
    let dir2 = if d == normal {
        d
    } else {
        d.slerp(normal, c.f03)
    };
    let jump = cx.w.content.player().jump_strength;
    cx.w.with_kind(clone, |w, k| {
        if let Kind::Bopl(b) = k {
            b.ctrl.abilities = Vec::new();
            b.ctrl.indicators = 0;
            b.prepare_to_revive(w, clone, obj_pos);
            b.ctrl.is_abilityless_clone = true;
            b.respawn(w, clone, obj_pos);
            b.spawn_done(w, clone);
            b.body.self_imposed_velocity = dir2 * F_1_3_D * jump;
        }
    });
}

// ---------------------------------------------------------------------------
// DuplicateObject / DuplicateBody and the clone rule
// ---------------------------------------------------------------------------

/// `DuplicateObject(pos, original, lifespan, permanent key)` (duplicator.md,
/// clone rule). Supports spawned objects and platforms; `None` for objects
/// that cannot be cloned.
pub fn duplicate_object(
    w: &mut World,
    pos: Vec2,
    original: ObjId,
    lifespan: i32,
    permanent_key: Option<i32>,
) -> Option<ObjId> {
    let o = w.objects.get(original)?;
    let kind = match &o.kind {
        Kind::Behaviour(b) => Kind::Behaviour(Box::new(b.duplicate(w, original)?)),
        Kind::Platform(p) => Kind::Platform(Box::new(platform_clone(p))),
        _ => return None,
    };
    let (layer, player_id, visual, offset, active, resizable) =
        (o.layer, o.player_id, o.visual, o.tf.offset, o.active, o.bb.resizable);
    let collider = o.collider.map(|c| ColliderDef {
        initialized: false,
        ..c
    });
    // The clone's updatables keep the original's relative order: they are
    // numbered 1..n in the original's order, then offset by the bracket.
    let mut parts: Vec<_> = o.parts[..o.n_parts as usize]
        .iter()
        .filter(|p| !p.destroyed)
        .copied()
        .collect();
    parts.sort_by(|a, b| a.hier.wrapping_sub(b.hier).cmp(&0));
    w.open_bracket();
    let id = w.objects.create(kind, layer);
    if let Some(c) = w.objects.get_mut(id) {
        c.player_id = player_id;
        c.visual = visual;
        c.collider = collider;
        c.tf.offset = offset;
        c.active = active;
        // BoplBody: nothing serialized; `isResizable` from the tag (Awake).
        c.bb.resizable = resizable;
        for (n, p) in parts.iter().enumerate() {
            let i = c.add_part(p.kind, n as i32 + 1);
            c.parts[i].enabled = p.enabled;
        }
    }
    w.register_all(id);
    w.close_bracket();
    if let Some(c) = w.objects.get_mut(id) {
        c.tf.position = pos;
        c.tf.reset_rotation();
    }
    if let Kind::Platform(_) = w.objects.get(id).map(|o| &o.kind)? {
        crate::platform::update_bounds(w, id);
    }
    add_to_list(w, id, lifespan, permanent_key);
    Some(id)
}

/// The clone of a platform's payload (duplicator.md, clone rule: platforms).
fn platform_clone(p: &Platform) -> Platform {
    let mut c = p.clone();
    c.sticky.drop_players_for_frames = 0;
    c.sticky.drop_except = -1;
    c.sticky.drop_force = Fix::ZERO;
    c.sticky.base_scale = Fix::ONE;
    if let Some(s) = c.spring.as_mut() {
        s.initialized = false;
        s.sudden_death_stamp = Fix::ZERO;
        s.random_sudden_death_delay = Fix::ZERO;
        s.float_this_frame = false;
        s.spin = None;
    }
    if let Some(s) = c.shake.as_mut() {
        *s = crate::platform::Shake {
            basic_strength: s.basic_strength,
            ..Default::default()
        };
    }
    if let Some(pa) = c.path.as_mut() {
        pa.target = 0;
    }
    c.time_since_last_scaled_up = Fix::ZERO;
    // Boulder: `isDropped` / `shouldDrop` are runtime (Awake: not dropping).
    if let Some(b) = c.boulder.as_mut() {
        b.dropped = false;
        b.should_drop = false;
    }
    // ResizablePlatform: `RemoveAnimation`, `MassPerArea` carry over; the
    // original extents, removal flag and timer are runtime.
    if let Some(r) = c.resize.as_mut() {
        r.being_removed = false;
        r.time_passed = Fix::ZERO;
        r.o_radius = Fix::ZERO;
        r.o_height = Fix::ZERO;
        r.o_width = Fix::ZERO;
    }
    c
}

/// Sets a not yet initialised body's start velocity (the BoplBody
/// `velocity` setter before `Init`), else the live velocity.
fn set_start_velocity(w: &mut World, o: ObjId, v: Vec2) {
    let init = w
        .objects
        .get(o)
        .and_then(|ob| ob.collider)
        .is_some_and(|c| c.initialized);
    if init {
        if let Some(b) = w.physics.body_mut(o) {
            b.velocity = v;
        }
    } else if let Some(b) = w
        .objects
        .get_mut(o)
        .and_then(|ob| ob.collider.as_mut())
        .and_then(|c| c.body.as_mut())
    {
        b.start_velocity = v;
    }
}

/// `duplicateBoplBody(pos, original, velocity, index, scale)`.
fn duplicate_body(
    w: &mut World,
    pos: Vec2,
    original: ObjId,
    v: Vec2,
    scale: Fix,
) -> Option<ObjId> {
    let cl = duplicate_object(w, pos, original, LIFESPAN_FRAMES, None)?;
    let os = obj_scale(w, original);
    match w.objects.get(original).and_then(|o| o.collider).map(|c| c.kind) {
        Some(ShapeKind::Circle) => {
            let r = match w.physics.circle(original) {
                Some(c) => c.radius,
                None => w
                    .objects
                    .get(original)
                    .and_then(|o| o.collider)
                    .map_or(Fix::ZERO, |c| c.start_radius),
            };
            if let Some(c) = w.objects.get_mut(cl).and_then(|o| o.collider.as_mut()) {
                c.start_radius = r / os;
            }
        }
        Some(ShapeKind::Box) => {
            let e = match w.physics.box_shape(original) {
                Some(b) => Vec2::new(Fix::ONE / b.ie.x, Fix::ONE / b.ie.y),
                None => w
                    .objects
                    .get(original)
                    .and_then(|o| o.collider)
                    .map_or(Vec2::ZERO, |c| c.start_extents),
            };
            if let Some(c) = w.objects.get_mut(cl).and_then(|o| o.collider.as_mut()) {
                let mut e = e / os;
                if e.x.0 == 0 {
                    e.x = c.minimum_extents;
                }
                if e.y.0 == 0 {
                    e.y = c.minimum_extents;
                }
                c.start_extents = e;
            }
        }
        _ => {}
    }
    set_start_velocity(w, cl, v);
    w.set_collider_scale(cl, scale);
    let rot = AbilityBody::tf(w, original).rotation;
    if let Some(o) = w.objects.get_mut(cl) {
        o.tf.set_rotation(rot);
    }
    Some(cl)
}

// ---------------------------------------------------------------------------
// The duplicated-objects list (DetPhysics, engine step 5)
// ---------------------------------------------------------------------------

/// One `DatedMonoUpdatable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Dated {
    pub obj: ObjId,
    pub life: i32,
    pub offset: Vec2,
    /// `HierarchyNumberIfPermanent` (−1 = not permanent).
    pub key: i32,
}

/// Duplicator global state (per scene).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DuplicateList {
    pub list: Vec<Dated>,
    /// `StickyRoundedRectangle.currentDuplicatePlatformEffect`: (platform,
    /// effect).
    pub effects: Vec<(ObjId, ObjId)>,
}

fn add_to_list(w: &mut World, obj: ObjId, life: i32, key: Option<i32>) {
    w.ability_globals.duplicates.list.push(Dated {
        obj,
        life: if key.is_some() { i32::MAX } else { life },
        offset: Vec2::ZERO,
        key: key.unwrap_or(-1),
    });
    if let Some(o) = w.objects.get_mut(obj) {
        o.tf.duplicated = true;
    }
}

/// `DetPhysics.AddObjectToDeleteList(obj, frames)` (e.g. the clouds of a
/// duplicated smoke grenade, smoke.md).
pub fn add_to_delete_list(w: &mut World, obj: ObjId, frames: i32) {
    add_to_list(w, obj, frames, None);
}

/// `DeleteObjectFromDuplicateListWithId(key, extra)`.
fn delete_with_id(w: &mut World, key: i32, extra: i32) {
    for d in w.ability_globals.duplicates.list.iter_mut() {
        if d.key == key && d.life > SHAKE_FRAMES + extra {
            d.life = SHAKE_FRAMES + extra;
            break;
        }
    }
}

/// `DetPhysics.UpdateDuplicatedObjects()` (engine step 5, not while time is
/// stopped).
pub fn update_duplicates(w: &mut World) {
    let mut i = w.ability_globals.duplicates.list.len();
    while i > 0 {
        i -= 1;
        let d = w.ability_globals.duplicates.list[i];
        let gone = w.objects.get(d.obj).is_none_or(|o| o.tf.destroyed);
        if gone {
            w.ability_globals.duplicates.list.remove(i);
            continue;
        }
        let life = d.life.wrapping_sub(1);
        w.ability_globals.duplicates.list[i].life = life;
        if life < 0 {
            w.emit(Cue::Sound("spawnOutDuplicate"));
            // Presentation: the level's `duplicatorParticlePrefab` where
            // the clone vanishes.
            if let Some(o) = w.objects.get(d.obj) {
                let (p, r) = (o.tf.position, o.tf.rotation);
                super::fx_of(w, crate::view::LEVEL_PREFAB, "duplicatorParticlePrefab", p, r, Fix::ONE, 0, Vec2::ZERO);
            }
            w.destroy(d.obj);
        } else if life < SHAKE_FRAMES {
            let k = Fix::ONE - Fix::from_int(life) / Fix::from_int(SHAKE_FRAMES);
            let off = w.rng.random_unit_vector() * k * F_0_25_D;
            if let Some(o) = w.objects.get_mut(d.obj) {
                o.tf.position = o.tf.position - d.offset + off;
            }
            w.ability_globals.duplicates.list[i].offset = off;
        }
    }
}

// ---------------------------------------------------------------------------
// DuplicatePlatformEffect
// ---------------------------------------------------------------------------

/// `DuplicatePlatformEffect` (spawned object).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlatformEffect {
    pub platform: ObjId,
    pub dir: Vec2,
    pub duration: Fix,
    pub key: i32,
    pub initialized: bool,
    pub orig_up: Vec2,
    pub orig_right: Vec2,
    pub orig_start: Vec2,
    pub scale_on_init: Fix,
    pub rotation_on_init: Fix,
    pub effect_scale: Fix,
    pub new_pos: Vec2,
    pub original_extents: Vec2,
    pub original_radius: Fix,
    pub time: Fix,
    pub spawned_effect: bool,
    pub overlap_force: Fix,
    pub scale_loss_speed: Fix,
    /// `One / 20`: PlatformSpawnPos' small-extent threshold.
    pub f005: Fix,
}

fn spawn_effect(w: &mut World, c: &EffectCfg, pos: Vec2) -> ObjId {
    let e = PlatformEffect {
        platform: ObjId::NONE,
        dir: Vec2::ZERO,
        duration: Fix::ZERO,
        key: -1,
        initialized: false,
        orig_up: Vec2::UP,
        orig_right: Vec2::RIGHT,
        orig_start: Vec2::ZERO,
        scale_on_init: Fix::ONE,
        rotation_on_init: Fix::ZERO,
        effect_scale: Fix::ONE,
        new_pos: Vec2::ZERO,
        original_extents: Vec2::ZERO,
        original_radius: Fix::ZERO,
        time: Fix::ZERO,
        spawned_effect: false,
        overlap_force: c.overlap_force,
        scale_loss_speed: c.scale_loss_speed,
        f005: Fix(214_748_368),
    };
    w.open_bracket();
    let id = w.objects.create(
        Kind::Behaviour(Box::new(Behaviour::Duplicator(Spawned::PlatformEffect(e)))),
        0,
    );
    if let Some(o) = w.objects.get_mut(id) {
        o.visual = match c.prefab {
            Some(p) => Visual::Prefab {
                prefab: p as u32,
                object: 0,
            },
            None => Visual::None,
        };
        o.add_part(PartKind::Behaviour(0), c.hier);
    }
    w.register_all(id);
    w.close_bracket();
    w.consume_hierarchy((c.updatables - 1).max(0));
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.position = pos;
        o.tf.reset_rotation();
    }
    id
}

fn current_effect(w: &World, plat: ObjId) -> Option<ObjId> {
    w.ability_globals
        .duplicates
        .effects
        .iter()
        .rev()
        .find(|(p, _)| *p == plat)
        .map(|(_, e)| *e)
        .filter(|e| w.objects.alive(*e))
}

fn rr_extents(w: &World, plat: ObjId) -> (Vec2, Fix) {
    match w.physics.rr(plat) {
        Some(r) => (Vec2::new(Fix::ONE / r.b.ie.x, Fix::ONE / r.b.ie.y), r.radius),
        None => (Vec2::ZERO, Fix::ZERO),
    }
}

/// `DuplicatePlatformEffect.Initialize(platform, dir, duration, key)`.
fn effect_initialize(
    w: &mut World,
    e: ObjId,
    plat: ObjId,
    dir: Vec2,
    duration: Fix,
    key: i32,
    _c: &EffectCfg,
) {
    let Some(rr) = w.physics.rr(plat).copied() else {
        return;
    };
    let t = AbilityBody::tf(w, plat);
    let (ext, rad) = rr_extents(w, plat);
    let scale = obj_scale(w, plat);
    let mut eff = None;
    if let Some(Kind::Behaviour(b)) = w.objects.get(e).map(|o| &o.kind)
        && let Behaviour::Duplicator(Spawned::PlatformEffect(pe)) = b.as_ref()
    {
        eff = Some(pe.clone());
    }
    let Some(mut pe) = eff else {
        return;
    };
    pe.dir = dir;
    pe.orig_up = t.up;
    pe.orig_right = t.right;
    pe.orig_start = t.position;
    pe.f005 = Fix::ONE / Fix::from_int(20);
    pe.new_pos = platform_spawn_pos(w, plat, dir, pe.orig_right, pe.orig_up, pe.orig_start, pe.f005);
    pe.spawned_effect = false;
    pe.platform = plat;
    pe.key = key;
    pe.scale_on_init = Fix::ONE;
    pe.rotation_on_init = t.rotation;
    pe.effect_scale = scale;
    pe.duration = duration;
    pe.initialized = true;
    pe.original_extents = ext;
    pe.original_radius = rad;
    let _ = rr;
    let center = t.position;
    if let Some(o) = w.objects.get_mut(e) {
        o.tf.position = center;
        o.tf.set_rotation(t.rotation);
        o.kind = Kind::Behaviour(Box::new(Behaviour::Duplicator(Spawned::PlatformEffect(pe))));
    }
    w.ability_globals.duplicates.effects.retain(|(p, _)| *p != plat);
    w.ability_globals.duplicates.effects.push((plat, e));
}

impl PlatformEffect {
    /// `ScaleEffect(s)`: the target position with the platform at scale `s`
    /// (the platform's real scale is set and restored).
    fn scale_effect(&mut self, w: &mut World, s: Fix) {
        let plat = self.platform;
        let old = obj_scale(w, plat);
        w.set_collider_scale(plat, s);
        self.scale_on_init = s;
        let (ext, rad) = rr_extents(w, plat);
        self.original_extents = ext;
        self.original_radius = rad;
        self.new_pos = platform_spawn_pos(
            w,
            plat,
            self.dir,
            self.orig_right,
            self.orig_up,
            self.orig_start,
            self.f005,
        );
        w.set_collider_scale(plat, old);
    }

    fn update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        if w.is_time_stopped() || !self.initialized {
            return;
        }
        let plat = self.platform;
        let alive = w.objects.alive(plat);
        if alive {
            self.effect_scale = self
                .effect_scale
                .min(self.effect_scale.lerp(Fix::ONE, self.scale_loss_speed));
            let es = self.effect_scale;
            self.scale_effect(w, es);
            let s = obj_scale(w, plat);
            w.set_collider_scale(plat, s.min(s.lerp(Fix::ONE, self.scale_loss_speed)));
        }
        self.time += dt;
        if alive && let Some(rr) = w.physics.rr(plat).copied() {
            let mut cast = rr;
            cast.radius += Fix::ONE;
            cast.b.center = self.new_pos;
            let ppos = rr.b.center;
            w.physics.remove_destroyed(&w.objects);
            let mut hits = Vec::new();
            for i in 0..w.physics.rrs.len() {
                if hits.len() >= 10 {
                    break;
                }
                let o = w.physics.rrs.parents[i].obj;
                if super::ability_body::collider_active(w, o)
                    && shapes::rr_rr_bool(&cast, &w.physics.rrs.shapes[i])
                {
                    hits.push(o);
                }
            }
            for h in hits {
                let hp = AbilityBody::pos(w, h);
                let v = (hp - ppos).normalized_safe();
                if has_bopl_body(w, h) {
                    let im = w.physics.body(h).map_or(Fix::ZERO, |b| b.inverse_mass);
                    let im = if im == Fix::ZERO { Fix::ONE } else { im };
                    w.physics.add_force(h, v * self.overlap_force / im);
                }
                if let Some(Kind::Platform(p)) = w.objects.get_mut(h).map(|o| &mut o.kind)
                    && let Some(s) = p.spring.as_mut()
                {
                    s.float_this_frame = true;
                }
            }
        }
        if self.time > self.duration {
            if alive {
                delete_with_id(w, self.key, 20);
                if let Some(cl) = duplicate_object(w, self.new_pos, plat, LIFESPAN_FRAMES, Some(self.key))
                {
                    let s = self.scale_on_init;
                    let r = self.original_radius / s;
                    let e = self.original_extents / s;
                    if let Some(c) = w.objects.get_mut(cl).and_then(|o| o.collider.as_mut()) {
                        c.start_radius = r;
                        let mut e = e;
                        if e.x.0 == 0 {
                            e.x = c.minimum_extents;
                        }
                        if e.y.0 == 0 {
                            e.y = c.minimum_extents;
                        }
                        c.start_extents = e;
                    }
                    let hier = w
                        .objects
                        .get(cl)
                        .and_then(|o| o.part(PartKind::Collider))
                        .map_or(0, |p| p.hier);
                    w.physics.register(&mut w.objects, cl, hier);
                    w.set_collider_scale(cl, s);
                    let rot = self.rotation_on_init;
                    if let Some(o) = w.objects.get_mut(cl) {
                        o.tf.set_rotation(rot);
                    }
                }
            }
            w.destroy(me);
        } else if !self.spawned_effect && self.time > self.duration - F_0_35 && alive {
            w.emit(Cue::Sound("duplicatePlatform"));
            self.spawned_effect = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Rock_noPlayer
// ---------------------------------------------------------------------------

/// `Rock_noPlayer`: a lethal rolling boulder (DamagingProjectile +
/// Explosion, LethalTerrain layer).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RockNoPlayer {
    pub owner: i32,
    pub cfg: Arc<RockCfg>,
    /// `BoplBody.prevScale`.
    pub prev_scale: Fix,
}

fn spawn_rock(w: &mut World, c: &RockCfg, pos: Vec2, rot: Fix, owner: i32) -> ObjId {
    let r = RockNoPlayer {
        owner,
        cfg: Arc::new(c.clone()),
        prev_scale: Fix::ONE,
    };
    w.open_bracket();
    let id = w.objects.create(
        Kind::Behaviour(Box::new(Behaviour::Duplicator(Spawned::Rock(r)))),
        c.layer,
    );
    if let Some(o) = w.objects.get_mut(id) {
        o.visual = match c.prefab {
            Some(p) => Visual::Prefab {
                prefab: p as u32,
                object: 0,
            },
            None => Visual::None,
        };
        o.collider = c.collider;
        o.add_part(PartKind::Behaviour(1), c.hier_bounds);
        o.add_part(PartKind::BoplBody, c.hier_body);
        o.add_part(PartKind::Collider, c.hier_collider);
    }
    w.register_all(id);
    w.close_bracket();
    w.consume_hierarchy((c.updatables - 3).max(0));
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.position = pos;
        o.tf.set_rotation(rot);
    }
    id
}

impl RockNoPlayer {
    /// `BoplBody.UpdateSim`: the inverse mass follows scale changes.
    fn body_update(&mut self, w: &mut World, me: ObjId) {
        let Some(o) = w.objects.get(me) else {
            return;
        };
        let s = o.tf.scale;
        let Some(def) = o.collider.filter(|c| c.initialized) else {
            return;
        };
        let Some(bd) = def.body else {
            return;
        };
        if bd.mass == Fix::ZERO || bd.moment_of_inertia == Fix::ZERO || self.prev_scale == s {
            return;
        }
        let k = self.prev_scale / s;
        if let Some(b) = w.physics.body_mut(me) {
            b.inverse_mass = b.inverse_mass * k;
            b.inverse_moi = b.inverse_moi * k;
        }
        self.prev_scale = s;
    }

    /// `DestroyIfOutsideSceneBounds` (no player id: destroyed).
    fn bounds(&self, w: &mut World, me: ObjId) {
        let c = self.cfg.clone();
        let t = AbilityBody::tf(w, me);
        let p = t.position;
        let space = w.is_space();
        let effect = |w: &mut World, kind: EffectKind, n: i32, at: Vec2| {
            w.consume_hierarchy(n);
            w.emit(Cue::Effect {
                kind,
                position: at,
                player_id: 0,
            });
        };
        if !space && c.destroy_y && p.y < WATER_HEIGHT {
            let mut at = p + c.down_effect.1;
            at.y = WATER_HEIGHT - F_1_47 * t.scale;
            effect(w, EffectKind::WaterSplash, c.down_effect.0, at);
        } else if space && c.destroy_y && p.y < CAMERA_Y_MIN {
            let mut at = p + c.up_effect.1;
            at.y = CAMERA_Y_MIN;
            effect(w, EffectKind::BlastZone, c.up_effect.0, at);
        } else if !c.do_not_destroy_up && c.destroy_y && p.y > BLAST_Y_MAX {
            let mut at = p + c.up_effect.1;
            at.y = BLAST_Y_MAX;
            effect(w, EffectKind::BlastZone, c.up_effect.0, at);
        } else if c.destroy_x && (p.x < BLAST_X_MIN || p.x > BLAST_X_MAX) {
            let mut at = p;
            at.x = if p.x < BLAST_X_MIN {
                BLAST_X_MIN
            } else {
                BLAST_X_MAX
            };
            at += c.x_effect.1;
            effect(w, EffectKind::BlastZone, c.x_effect.0, at);
        } else {
            return;
        }
        w.destroy(me);
    }
}

impl SpawnedImpl for Spawned {
    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        if late {
            return;
        }
        match self {
            Spawned::PlatformEffect(e) => e.update(w, me, dt),
            Spawned::Rock(r) if part == PartKind::Behaviour(1) => r.bounds(w, me),
            Spawned::Rock(r) if part == PartKind::BoplBody => r.body_update(w, me),
            _ => {}
        }
    }

    fn lethal(&self, _w: &World, _me: ObjId) -> Option<Lethal> {
        match self {
            Spawned::Rock(r) => Some(Lethal {
                owner: r.owner,
                can_hurt_owner: r.cfg.can_hurt_owner,
                beam: false,
            }),
            Spawned::PlatformEffect(_) => None,
        }
    }

    fn duplicate(&self, _w: &World, _me: ObjId) -> Option<Spawned> {
        match self {
            // Explosion.PlayerOwnerId is runtime state: 0 on a clone.
            Spawned::Rock(r) => Some(Spawned::Rock(RockNoPlayer {
                owner: 0,
                cfg: r.cfg.clone(),
                prev_scale: Fix::ONE,
            })),
            Spawned::PlatformEffect(_) => None,
        }
    }

    fn interact(&mut self, w: &mut World, _me: ObjId, i: Interaction) -> Option<Fix> {
        match (self, i) {
            // The Explosion owner of a terrain clone is the original's.
            (Spawned::Rock(r), Interaction::Duplicated { original, .. }) => {
                if let Some(Kind::Behaviour(b)) = w.objects.get(original).map(|o| &o.kind)
                    && let Behaviour::Duplicator(Spawned::Rock(o)) = b.as_ref()
                {
                    r.owner = o.owner;
                }
                Some(Fix::ZERO)
            }
            _ => None,
        }
    }
}

/// `(Fix)0.1` per grenade clone, exported for the grenade module's
/// `Interaction::Duplicated` handling.
pub const GRENADE_FUSE_STEP: Fix = F_0_1_D;

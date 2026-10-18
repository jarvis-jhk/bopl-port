//! Growth ray (`docs/spec/abilities/growth-ray.md`).
//!
//! The gun body is [`GunTransform`](super::blink_gun::GunTransform); this
//! module is the canonical home of [`ShootScaleChange`] and the
//! [`ScaleChanger`] it spawns (Shrink ray uses both with its own prefab
//! values).

use std::sync::Arc;

use super::Behaviour;
use super::blink_gun::{self, GunAbility, GunTransform, self_hit};
use super::ability_body::{water_height};
use super::prefab::{ability_prefab, fields, prefab_by_ref, updatables};
use super::blink_gun::layer;
use super::prelude::*;
use crate::curve::Curve;
use crate::physics::RayHit;
use crate::view::Visual;
use crate::world::Kind;

/// The Growth ray ability.
pub type GrowthRay = GunAbility<ShootScaleChange>;

/// Objects this module spawns.
pub type Spawned = ScaleChanger;

/// `maxDistance` (code literal 100).
const MAX_DISTANCE: Fix = Fix(429_496_729_600);

/// Static data of a scale ray and its `ScaleChanger` prefab.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScaleRayCfg {
    pub collision_mask: u32,
    pub collision_mask_no_water: u32,
    pub black_hole_growth_inverse01: Fix,
    pub changer_prefab: Option<usize>,
    pub changer_hier: i32,
    pub changer_updatables: i32,
    pub curve: Arc<Curve>,
    pub multiplier: Fix,
    pub player_multiplier: Fix,
    pub small_non_players_multiplier: Fix,
    pub inverse_mass_before_considered_big: Fix,
}

/// `ShootScaleChange` (shared with Shrink ray).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShootScaleChange {
    pub cfg: Arc<ScaleRayCfg>,
}

impl blink_gun::Gun for ShootScaleChange {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let c = &w.content;
        let f = ability_prefab(w, def).map_or(&Value::Null, |p| fields(p, "ShootScaleChange"));
        let changer = prefab_by_ref(c, f.reference("ScaleChangerPrefab"));
        let sf = changer
            .and_then(|i| c.prefabs.get(i))
            .map_or(&Value::Null, |p| fields(p, "ScaleChanger"));
        ShootScaleChange {
            cfg: Arc::new(ScaleRayCfg {
                collision_mask: f.int("collisionMask").unwrap_or(216_608_528) as u32,
                collision_mask_no_water: f.int("collisionMaskNoWater").unwrap_or(216_608_512)
                    as u32,
                black_hole_growth_inverse01: f.fix_or("blackHoleGrowthInverse01", Fix::ZERO),
                changer_prefab: changer,
                changer_hier: sf.int("hierarchyNumber").unwrap_or(1) as i32,
                changer_updatables: updatables(c, changer).max(1),
                curve: sf.curve("scaleAnimation").unwrap_or_default(),
                multiplier: sf.fix_or("multiplier", Fix::ZERO),
                player_multiplier: sf.fix_or("PlayerMultiplier", Fix::ZERO),
                small_non_players_multiplier: sf.fix_or("smallNonPlayersMultiplier", Fix::ZERO),
                inverse_mass_before_considered_big: sf
                    .fix_or("inverseMassBeforeConsideredBig", Fix(214_748_368)),
            }),
        }
    }

    fn shoot(&mut self, cx: &mut Cx<'_>, _gun: &mut GunTransform, fp: Vec2, dir: Vec2) {
        shoot_scale(cx, &self.cfg, fp, dir, false);
    }
}

/// `ShootScaleChange.Shoot(fp, dir, owner, alreadyHitWater)`.
fn shoot_scale(cx: &mut Cx<'_>, c: &Arc<ScaleRayCfg>, fp: Vec2, dir: Vec2, already: bool) {
    let me = cx.me;
    cx.w.emit(Cue::Sound("fireRaygun"));
    let mask = if already {
        c.collision_mask_no_water
    } else {
        c.collision_mask
    };
    let mut hit = cx.w.physics.point_check_all_rrs(&cx.w.objects, fp);
    if !hit.hit() {
        hit = cx
            .w
            .physics
            .raycast_closest(&cx.w.objects, fp, dir, MAX_DISTANCE, mask);
    }
    if (!already && fp.y <= water_height(cx.w)) || (already && hit.layer == layer::WATER) {
        cx.w.emit(Cue::Sound("waterExplosion"));
        let x = hit.near_pos.x;
        super::fx(cx.w, me, "WaterRing", Vec2::new(x, Fix::ZERO), Fix::ZERO, Fix::ONE, 0, Vec2::ZERO);
        hit = self_hit(cx.w, me, blink_gun::SELF_HIT_DIST);
    }
    if hit.hit() {
        apply_scale_change(cx, c, &hit, fp, dir);
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

/// `ApplyScaleChange(hit)`.
fn apply_scale_change(cx: &mut Cx<'_>, c: &Arc<ScaleRayCfg>, hit: &RayHit, fp: Vec2, dir: Vec2) {
    let (me, o) = (cx.me, cx.fw.owner);
    let water = hit.layer == layer::WATER;
    super::ray_fx(
        cx.w,
        me,
        "RaycastParticlePrefab",
        "RaycastParticleHitPrefab",
        fp,
        dir,
        hit.near_dist,
        !water,
        None,
        o,
    );
    if water {
        let x = hit.near_pos.x;
        super::fx(cx.w, me, "WaterRing", Vec2::new(x, Fix::ZERO), Fix::ZERO, Fix::ONE, 0, Vec2::ZERO);
    } else if let Some(v) = cx.w.objects.get(hit.obj) {
        // The flare on the victim (`GrowthFlare.Init(renderer)`).
        let p = v.tf.position;
        super::fx(cx.w, me, "flarePrefab", p, Fix::ZERO, Fix::ONE, o, Vec2::ZERO);
    }
    if water {
        let d = Vec2::new(dir.x, -dir.y);
        shoot_scale(cx, c, hit.near_pos, d, true);
        cx.w.emit(Cue::Sound("waterExplosion"));
        return;
    }
    if hit.layer == layer::RIGID_BODY_AFFECTOR {
        super::black_hole::grow(cx.w, hit.obj, c.black_hole_growth_inverse01, Fix::ZERO);
    } else {
        // (The orb-layer branch never applies: no object of the game's
        // data is on the orb layer.)
        let victim = hit.obj;
        let vl = cx.w.objects.get(victim).map_or(0, |o| o.layer as i32);
        let player = if victim == cx.me {
            Some(cx.fw.owner)
        } else {
            match cx.w.objects.get(victim).map(|o| &o.kind) {
                Some(Kind::Ability(a)) => Some(a.fw.owner),
                Some(Kind::Bopl(b)) => Some(b.ctrl.player_number),
                _ => None,
            }
        };
        spawn_changer(cx.w, c, victim, vl == layer::SPELL, player);
    }
    cx.w.emit(Cue::Sound(if c.black_hole_growth_inverse01 > Fix::ZERO {
        "grow"
    } else {
        "shrink"
    }));
}

fn spawn_changer(
    w: &mut World,
    c: &Arc<ScaleRayCfg>,
    victim: ObjId,
    spell: bool,
    player: Option<i32>,
) -> ObjId {
    let s = ScaleChanger {
        cfg: c.clone(),
        victim,
        spell,
        player,
        time_passed: Fix::ZERO,
    };
    w.open_bracket();
    let id = w
        .objects
        .create(Kind::Behaviour(Box::new(Behaviour::GrowthRay(s))), 0);
    if let Some(o) = w.objects.get_mut(id) {
        o.visual = match c.changer_prefab {
            Some(p) => Visual::Prefab {
                prefab: p as u32,
                object: 0,
            },
            None => Visual::None,
        };
        o.add_part(PartKind::Behaviour(0), c.changer_hier);
    }
    w.register_all(id);
    w.close_bracket();
    w.consume_hierarchy((c.changer_updatables - 1).max(0));
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.position = Vec2::ZERO;
        o.tf.reset_rotation();
    }
    id
}

/// `ScaleChanger` (spawned object, growth-ray.md).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScaleChanger {
    pub cfg: Arc<ScaleRayCfg>,
    /// The hit physics parent's collider (object).
    pub victim: ObjId,
    /// The victim is on the `spell` layer (scaled through `ISpell`).
    pub spell: bool,
    /// The victim is a player (bopl or ability object).
    pub player: Option<i32>,
    pub time_passed: Fix,
}

impl SpawnedImpl for ScaleChanger {
    fn update(&mut self, w: &mut World, me: ObjId, _part: PartKind, dt: Fix, late: bool) {
        if late {
            return;
        }
        let c = self.cfg.clone();
        let gone = w
            .objects
            .get(self.victim)
            .is_none_or(|o| o.part(PartKind::Collider).is_none_or(|p| p.destroyed));
        if self.time_passed > c.curve.last_time() || gone {
            w.destroy(me);
            return;
        }
        match self.player {
            None => {
                if w.is_time_stopped() {
                    return;
                }
                self.time_passed += dt;
                let inv = w.physics.body(self.victim).map_or(Fix::ZERO, |b| b.inverse_mass);
                let scale = w.objects.get(self.victim).map_or(Fix::ONE, |o| o.tf.scale);
                let k = if inv > Fix::ZERO && inv * scale > c.inverse_mass_before_considered_big {
                    c.small_non_players_multiplier
                } else {
                    c.multiplier
                };
                let inc = c.curve.eval(self.time_passed) * k;
                if self.spell {
                    super::interact(w, self.victim, Interaction::SpellAddScale(inc));
                } else {
                    w.set_collider_scale(self.victim, scale + inc);
                }
            }
            Some(id) => {
                let protected = w.player(id).is_some_and(|p| p.is_protected_from_time_stop);
                if !w.is_time_stopped() || protected {
                    let inc = c.curve.eval(self.time_passed) * c.player_multiplier;
                    if let Some(p) = w.player_mut(id) {
                        let s = p.scale() + inc;
                        p.set_scale(s);
                    }
                    self.time_passed += dt;
                }
            }
        }
    }
}

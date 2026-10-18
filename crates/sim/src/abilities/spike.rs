//! Spike (`docs/spec/abilities/spike.md`): the caster slams its platform and
//! a lethal `SpikeAttack` box shoots out of the far side, welded into the
//! platform's composite body. Its Explosion owner is never set (killer 0).

use std::sync::Arc;

use super::Behaviour;
use super::ability_body::{self, AbilityBody, add_force_at_position, advance_anim, play_named_then as play_then, prefab_parts};
use super::prefab::{self, fx, prefab_field, root_fields};
use super::explosion::ExplosionInfo;
use super::prelude::*;
use crate::anim::SpriteAnimator;
use crate::curve::Curve;
use crate::surface::Surface;
use crate::world::Kind;

/// `Spike` (ability) constants.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SpikeCfg {
    pub anim_data: u16,
    pub surface_offset: Fix,
    pub cast_force: Fix,
    pub attack: Option<Arc<AttackCfg>>,
}

/// `SpikeAttack` prefab.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct AttackCfg {
    pub prefab: usize,
    pub thrust: Arc<Curve>,
    pub knock_away: Fix,
    pub knock_away_wall: Fix,
    pub self_push: Fix,
    pub push_time: Fix,
    pub explosion: Option<ExplosionInfo>,
    pub parts: Vec<(PartKind, i32, bool)>,
}

impl AttackCfg {
    pub fn from_prefab(w: &World, prefab: usize) -> Option<AttackCfg> {
        let p = w.content.prefabs.get(prefab)?;
        let f = root_fields(p, "SpikeAttack")?;
        Some(AttackCfg {
            prefab,
            thrust: f.curve("DownThrustAnimation").unwrap_or_default(),
            knock_away: fx(f, "knockAwayStr", Fix::from_int(50)),
            knock_away_wall: fx(f, "knockAwayWallStr", Fix::from_int(1000)),
            self_push: fx(f, "selfPushMultiplier", Fix::HALF),
            push_time: fx(f, "pushTime", Fix::ZERO),
            explosion: root_fields(p, "Explosion").map(ExplosionInfo::from_fields),
            parts: prefab_parts(
                w,
                prefab,
                &[("SpikeAttack", PartKind::Behaviour(0)), ("Explosion", PartKind::Behaviour(1))],
            ),
        })
    }
}

const CB_CAST: u16 = 1;
const CB_EXIT: u16 = 2;

/// The Spike ability object.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Spike {
    pub cfg: Option<Arc<SpikeCfg>>,
    pub body: AbilityBody,
    pub anim: SpriteAnimator,
    pub current: Option<ObjId>,
}

impl Spike {
    fn cast(&mut self, cx: &mut Cx<'_>) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let me = cx.me;
        let g = self.body.physics.attached_ground;
        let ok = g.is_some_and(|g| cx.w.objects.alive(g) && cx.w.objects.active_in_hierarchy(g));
        let Some(g) = g.filter(|_| ok) else {
            self.exit_component(cx, AbilityExitInfo::default());
            return;
        };
        if let Some(old) = self.current.filter(|s| cx.w.objects.exists(*s)) {
            cx.w.destroy(old);
        }
        let pos = AbilityBody::pos(cx.w, me);
        let Some(surf) = Surface::of(cx.w, g) else {
            self.exit_component(cx, AbilityExitInfo::default());
            return;
        };
        let n = surf.normal(pos);
        let hit = cx
            .w
            .physics
            .rr(g)
            .and_then(|rr| crate::physics::shapes::ray_rr(pos, -n, rr));
        let Some((_near, far)) = hit else {
            self.exit_component(cx, AbilityExitInfo::default());
            return;
        };
        let f = pos - n * far;
        cx.w.emit(Cue::Sound("spikeRock"));
        let rot = AbilityBody::rotation(cx.w, me);
        let owner = cx.fw.owner;
        super::fx(cx.w, me, "dustParticlePrefab", pos, rot, Fix::ONE, owner, Vec2::ZERO);
        super::fx(cx.w, me, "dustParticle2Prefab", f, rot, Fix::ONE, owner, Vec2::ZERO);
        cx.w.emit(Cue::Shake(650));
        if let Some(ac) = &cfg.attack {
            let scale = AbilityBody::scale(cx.w, me);
            let s = spawn_attack(cx.w, ac, f, n, g, cfg.surface_offset, scale);
            self.current = Some(s);
        }
        add_force_at_position(cx.w, g, n * cfg.cast_force, pos, false);
        play_then(cx.w, &mut self.anim, cfg.anim_data, "recover", CB_EXIT);
    }
}

impl AbilityImpl for Spike {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let Some(p) = def.prefab else {
            return Spike::default();
        };
        let pf = &w.content.prefabs[p];
        let Some(f) = root_fields(pf, "Spike") else {
            return Spike::default();
        };
        let attack = prefab_field(w, f, "spikePrefab")
            .and_then(|i| AttackCfg::from_prefab(w, i))
            .map(Arc::new);
        let cfg = SpikeCfg {
            anim_data: prefab::anim_data_field(w, f, "animData").unwrap_or(0),
            surface_offset: fx(f, "surfaceOffset", Fix::HALF),
            cast_force: fx(f, "castForce", Fix::from_int(10)),
            attack,
        };
        let anim = ability_body::animator(prefab::root_always_stop(pf));
        Spike {
            cfg: Some(Arc::new(cfg)),
            body: AbilityBody::from_prefab(w, p),
            anim,
            current: None,
        }
    }

    fn build(&mut self, w: &mut World, me: ObjId, fw: &mut Framework, def: &AbilityDef) {
        self.body.install(fw);
        ability_body::add_parts(w, me, fw, def, &[("Spike", PartKind::Behaviour(0))]);
    }

    fn body(&self) -> Option<&AbilityBody> {
        self.cfg.as_ref().map(|_| &self.body)
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        if self.cfg.is_some() {
            Some(&mut self.body)
        } else {
            None
        }
    }

    fn sprite(&self) -> crate::data::SpriteId {
        self.anim.sprite
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        let Some(cfg) = self.cfg.clone() else {
            let i = cx.info();
            cx.exit(AbilityExitInfo::resume(&i));
            return;
        };
        let info = cx.info();
        let me = cx.me;
        self.body.owner = cx.fw.owner;
        AbilityBody::set_rotation(cx.w, me, info.rotation);
        play_then(cx.w, &mut self.anim, cfg.anim_data, "cast", CB_CAST);
        self.body.sync_physics_to(cx.w, me, &info);
        AbilityBody::update_collider(cx.w, me);
        AbilityBody::set_pos(cx.w, me, info.position);
    }

    fn update_part(&mut self, cx: &mut Cx<'_>, k: PartKind, dt: Fix, late: bool) {
        match (k, late) {
            (PartKind::Animator, false) => match advance_anim(cx.w, cx.fw.owner, &mut self.anim) {
                Some(AnimCallback::Ability(CB_CAST)) => self.cast(cx),
                Some(AnimCallback::Ability(CB_EXIT)) => self.exit_component(cx, AbilityExitInfo::default()),
                _ => {}
            },
            (PartKind::Bounds, false) => {
                if self.body.cfg.bounds.check(cx.w, cx.me) {
                    let info = ability_body::dead_record(cx.w, cx.me, ENVIRONMENT_ID, CauseOfDeath::Other);
                    self.exit_component(cx, info);
                }
            }
            (PartKind::Behaviour(0), false) => {
                if self.body.is_grounded()
                    && let Some(info) = self.body.move_(cx.w, cx.me, Vec2::ZERO, dt, true)
                {
                    self.exit_component(cx, info);
                }
            }
            _ => {
                if let Some(r) = self.body.update_part(cx, k, dt, late) {
                    self.exit_component(cx, r);
                }
            }
        }
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        let me = cx.me;
        if let Some((killer, cause)) = self.body.collision_contact(cx.w, me, cx.fw, c) {
            let info = ability_body::dead_record(cx.w, cx.me, killer, cause);
            self.exit_component(cx, info);
        }
    }


    /// `Spike.ExitAbility(info)`.
    fn exit_component(&mut self, cx: &mut Cx<'_>, mut info: AbilityExitInfo) {
        let me = cx.me;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        let fr = cx.fw.info.is_some_and(|i| i.facing_right);
        info.grounded_speed = self.body.body.self_imposed_velocity.magnitude();
        info.is_grounded = self.body.is_grounded();
        info.facing_right = fr;
        info.radius = self.body.physics.radius;
        info.position = AbilityBody::pos(cx.w, me);
        if self.body.is_grounded() {
            info.ground = self.body.physics.attached_ground;
            info.grounded_local_position = self.body.physics.local_platform_pos;
        }
        info.self_imposed_velocity = self.body.body.self_imposed_velocity;
        info.external_velocity = self.body.body.external_velocity;
        cx.exit_body(&mut self.body, info);
    }
}

// -- the spike ----------------------------------------------------------------------------

/// A `SpikeAttack` welded to a platform.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SpikeAttack {
    pub cfg: Arc<AttackCfg>,
    /// The Explosion component (owner 0).
    pub explosion: ExplosionInfo,
    pub ground: ObjId,
    pub local: Fix,
    pub ground_rot_at_cast: Fix,
    pub fixed_offset: Fix,
    pub surface_offset: Fix,
    pub time_passed: Fix,
    pub pushed_this_frame: bool,
}

/// Spawns and initialises a spike at the far point `f` of platform `g`.
pub fn spawn_attack(w: &mut World, cfg: &Arc<AttackCfg>, f: Vec2, n: Vec2, g: ObjId, offset: Fix, scale: Fix) -> ObjId {
    let s = SpikeAttack {
        cfg: cfg.clone(),
        explosion: cfg.explosion.unwrap_or(ExplosionInfo {
            owner: 0,
            can_hurt_owner: true,
            ignite_flammables: true,
            beam: false,
        }),
        ground: g,
        local: Fix::ZERO,
        ground_rot_at_cast: Fix::ZERO,
        fixed_offset: Fix::ZERO,
        surface_offset: Fix::ZERO,
        time_passed: Fix::ZERO,
        pushed_this_frame: false,
    };
    let id = ability_body::spawn_owned(w, cfg.prefab, Kind::Behaviour(Box::new(Behaviour::Spike(s))), &cfg.parts, f, None, 0);
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.set_up(n);
    }
    // Render parent: the platform (moves, deactivates and dies with it).
    w.objects.set_parent(id, Some(g));
    initialize(w, id, f, offset, g, scale, true);
    id
}

fn attack(w: &World, s: ObjId) -> Option<&SpikeAttack> {
    match &w.objects.get(s)?.kind {
        Kind::Behaviour(b) => match b.as_ref() {
            Behaviour::Spike(a) => Some(a),
            _ => None,
        },
        _ => None,
    }
}

/// Runs `f` on the spike state of object `s` (payload taken out).
fn with_attack(w: &mut World, s: ObjId, f: impl FnOnce(&mut World, &mut SpikeAttack)) {
    w.with_kind(s, |w, k| {
        if let Kind::Behaviour(b) = k
            && let Behaviour::Spike(a) = b.as_mut()
        {
            f(w, a);
        }
    });
}

/// `SpikeAttack.Initialize(surfacePos, offset, ground, scale, …, skipInit)`.
pub fn initialize(w: &mut World, s: ObjId, f: Vec2, offset: Fix, g: ObjId, scale: Fix, init: bool) {
    if init {
        ability_body::manual_init(w, s);
    }
    crate::platform::update_bounds(w, g);
    let local = Surface::of(w, g).map_or(Fix::ZERO, |sf| sf.param_from_position(f, Fix::ZERO));
    let grot = w.objects.get(g).map_or(Fix::ZERO, |o| o.tf.rotation);
    w.physics.weld_to_rr(&w.objects, s, g);
    with_attack(w, s, |w, a| {
        a.local = local;
        a.ground_rot_at_cast = grot;
        a.fixed_offset = offset;
        a.surface_offset = offset;
        a.ground = g;
        let rot = w.objects.get(s).map_or(Fix::ZERO, |o| o.tf.rotation);
        ability_body::set_relative_orientation(w, s, rot - grot);
        update_position(w, s, a);
    });
    w.set_collider_scale(s, scale);
}

/// `UpdatePosition()`.
fn update_position(w: &mut World, s: ObjId, a: &mut SpikeAttack) {
    let Some(surf) = Surface::of(w, a.ground) else {
        return;
    };
    let t = w.objects.get(s).map(|o| o.tf).unwrap_or_default();
    let base = surf.position_from_param(a.local, Fix::ZERO);
    a.surface_offset = a.cfg.thrust.eval(a.time_passed) + a.fixed_offset;
    let pivot = base + t.up * a.surface_offset;
    let gt = w.objects.get(a.ground).map(|o| o.tf).unwrap_or_default();
    let inv = Vec2::from_angle(Fix::PI_TIMES_2 - gt.rotation);
    let rel = inv.complex_mul(pivot - gt.position);
    let right = t.right.complex_mul(inv);
    ability_body::set_relative_position(w, s, rel, right);
}

/// Re-`Initialize` a carried spike on boulder `b` (Throw): surface point
/// `position − up · 1000`, `SurfaceOffset / scale`, no collider init.
pub fn reattach(w: &mut World, s: ObjId, b: ObjId, scale: Fix) {
    let Some(so) = attack(w, s).map(|a| a.surface_offset) else {
        return;
    };
    let t = w.objects.get(s).map(|o| o.tf).unwrap_or_default();
    initialize(w, s, t.position - t.up * Fix::from_int(1000), so / scale, b, scale, false);
}

/// A Duplicator clone planted on `g`: `Initialize(pos, offset, g, scale)`
/// (collider initialised now).
pub fn init_duplicate(w: &mut World, s: ObjId, pos: Vec2, offset: Fix, g: ObjId, scale: Fix) {
    initialize(w, s, pos, offset, g, scale, true);
}

/// `RepositionOnParent()` (platform resized / rescaled).
pub fn reposition_on_parent(w: &mut World, s: ObjId) {
    let Some(a) = attack(w, s).cloned() else {
        return;
    };
    if !w.objects.alive(a.ground) {
        return;
    }
    let Some(surf) = Surface::of(w, a.ground) else {
        return;
    };
    let hit_ok = w
        .objects
        .get(s)
        .and_then(|o| o.part(PartKind::Collider))
        .is_some_and(|p| !p.destroyed);
    if !hit_ok {
        return;
    }
    let up = w.physics.box_shape(s).map_or(Vec2::UP, |b| b.up);
    let right = w.physics.box_shape(s).map_or(Vec2::RIGHT, |b| b.right);
    let rr = w.physics.rr(a.ground).copied();
    let (rpos, rrot) = match rr {
        Some(r) => (r.b.center, w.physics.body(a.ground).map_or(Fix::ZERO, |b| b.orientation)),
        None => (surf.position, Fix::ZERO),
    };
    let pivot = surf.position_from_param(a.local, Fix::ZERO) + up * a.surface_offset;
    let inv = Vec2::from_angle(Fix::PI_TIMES_2 - rrot);
    let rel = inv.complex_mul(pivot - rpos);
    ability_body::set_relative_position(w, s, rel, right.complex_mul(inv));
}

impl SpawnedImpl for SpikeAttack {
    fn interact(&mut self, _w: &mut World, _me: ObjId, i: super::Interaction) -> Option<Fix> {
        match i {
            super::Interaction::ItemKind => Some(Fix::from_int(super::ITEM_SPIKE)),
            _ => None,
        }
    }

    /// Clone rule: tuning kept, everything else reset (the duplicator then
    /// re-initialises the clone on the platform); `PlayerOwnerId` 0.
    fn duplicate(&self, _w: &World, _me: ObjId) -> Option<SpikeAttack> {
        Some(SpikeAttack {
            cfg: self.cfg.clone(),
            explosion: ExplosionInfo {
                owner: 0,
                ..self.explosion
            },
            ground: ObjId::NONE,
            local: Fix::ZERO,
            ground_rot_at_cast: Fix::ZERO,
            fixed_offset: Fix::ZERO,
            surface_offset: Fix::ZERO,
            time_passed: Fix::ZERO,
            pushed_this_frame: false,
        })
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        if late {
            return;
        }
        if let PartKind::Behaviour(0) = part {
            self.pushed_this_frame = false;
            self.time_passed += dt * w.time.player_time_scale;
            let ok = w.objects.alive(self.ground) && w.objects.get(self.ground).is_some_and(|o| o.active);
            if self.time_passed <= self.cfg.thrust.last_time() && ok {
                update_position(w, me, self);
            }
        }
    }

    fn on_contact(&mut self, w: &mut World, me: ObjId, c: &Contact) {
        if self.time_passed >= self.cfg.push_time || w.is_time_stopped() {
            return;
        }
        let ok = w
            .objects
            .get(c.other)
            .and_then(|o| o.part(PartKind::Collider))
            .is_some_and(|p| !p.destroyed)
            && ability_body::collider_initialized(w, c.other);
        if !ok || c.other == self.ground {
            return;
        }
        let p = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        let d = (c.contact_point - p).normalized_safe();
        let s = if c.layer == 11 {
            self.cfg.knock_away_wall
        } else {
            self.cfg.knock_away
        };
        add_force_at_position(w, c.other, d * s, c.contact_point, false);
        if !self.pushed_this_frame {
            add_force_at_position(w, me, -d * s * self.cfg.self_push, c.contact_point, false);
            self.pushed_this_frame = true;
        }
    }

    fn lethal(&self, _w: &World, _me: ObjId) -> Option<super::Lethal> {
        Some(self.explosion.lethal())
    }
}

/// Objects of this module.
pub type Spawned = SpikeAttack;

//! The shared `Explosion` and `Shockwave` components (canonical spec:
//! `docs/spec/abilities/mine.md`, "Shared component"), used by Mine,
//! Missile, Grenade, Smoke and by other abilities' explosion prefabs.
//!
//! API:
//! * [`spawn`]`(w, prefab, pos, scale, owner)`: `InstantiateFixed` of an
//!   explosion prefab (any prefab whose root has an `Explosion`, optional
//!   `Shockwave`s, `SpriteAnimator`, `PlaySound` and a collider) with the
//!   collider scale and `PlayerOwnerId` set the way the spawners do.
//! * [`ExplosionInfo`] / [`info`]: the Explosion component data of any
//!   object of this group (explosions, spikes, tesla coils and beams):
//!   owner, `canHurtOwner`, `igniteFlammables`.
//! * [`ShockwaveDef`] / [`Shockwave`]: a shockwave that other modules can
//!   embed in their own spawned objects (gusts call [`Shockwave::activate`]).
//!
//! Spawned explosions live in `mine::Spawned::Explosion`.

use std::sync::Arc;

use super::Behaviour;
use super::ability_body::{self, add_force_at_position};
use super::prefab::{hier_field as hier_of, root_fields};
use crate::anim::{AnimCallback, SpriteAnimator};
use crate::curve::Curve;
use crate::data::{AnimRef, Value};
use crate::fix::Fix;
use crate::object::{ObjId, PartKind};
use crate::surface::Surface;
use crate::vec2::Vec2;
use crate::view::Cue;
use crate::world::{Kind, World};

/// Animator callback: `SelfDestruct()`.
pub const CB_SELF_DESTRUCT: u16 = 1;

/// `Shockwave` serialized values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShockwaveDef {
    pub hier: i32,
    pub platform_force: Fix,
    pub default_force: Fix,
    pub grounded_player_force_multiplier: Fix,
    pub radius: Fix,
    pub mask: u32,
    pub call_every_frame: bool,
    pub kill_after_on_frame: bool,
    pub is_gust_ability: bool,
    pub curve: Arc<Curve>,
}

impl ShockwaveDef {
    pub fn from_fields(f: &Value) -> ShockwaveDef {
        ShockwaveDef {
            hier: hier_of(f),
            platform_force: f.fix_or("platformForce", Fix::ZERO),
            default_force: f.fix_or("defaultForce", Fix::ZERO),
            grounded_player_force_multiplier: f.fix_or("groundedPlayerForceMultiplier", Fix::ONE),
            radius: f.fix_or("radius", Fix::from_int(5)),
            mask: f.int("collisionMask").unwrap_or(0) as u32,
            call_every_frame: f.boolean("CallEveryFrame").unwrap_or(false),
            kill_after_on_frame: f.boolean("killAfterOnFrame").unwrap_or(false),
            is_gust_ability: f.boolean("isGustAbility").unwrap_or(false),
            curve: f.curve("forceCurve01").unwrap_or_default(),
        }
    }
}

/// A `Shockwave` component.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Shockwave {
    pub def: Arc<ShockwaveDef>,
    /// `platformForce` / `defaultForce` (spawners may scale them, Meteor).
    pub platform_force: Fix,
    pub default_force: Fix,
    pub scale: Fix,
    pub time_alive: Fix,
    /// `IPlayerIdHolder` id (`SetPlayerId`; 0 unless set).
    pub owner_id: i32,
    /// The hit set, in hit order.
    pub hits: Vec<ObjId>,
    pub ignore: Vec<ObjId>,
}

impl Shockwave {
    pub fn new(def: Arc<ShockwaveDef>) -> Shockwave {
        Shockwave {
            platform_force: def.platform_force,
            default_force: def.default_force,
            def,
            scale: Fix::ONE,
            time_alive: Fix::ZERO,
            owner_id: 0,
            hits: Vec::new(),
            ignore: Vec::new(),
        }
    }

    /// `Init()`.
    pub fn init(&mut self, w: &mut World, me: ObjId) {
        self.time_alive = Fix::ZERO;
        self.activate(w, me, Fix::ZERO);
    }

    /// `UpdateSim`.
    pub fn update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        if self.def.call_every_frame {
            self.update_hit_objects(w, me, dt);
        }
        if self.def.kill_after_on_frame {
            w.destroy(me);
        }
    }

    /// `ActivateShockWave(dt)`.
    pub fn activate(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        self.hits.clear();
        let center = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        let d = self.def.clone();
        let found =
            w.physics
                .circle_cast_all(&w.objects, center, d.radius * self.scale, d.mask, 50);
        let k = if d.call_every_frame {
            Fix::ONE
        } else {
            Fix::from_int(7)
        };
        let default_force = self.default_force;
        let force_of = |v: Vec2, scale: Fix| -> Fix {
            let s = (scale + Fix::ONE) / Fix::TWO;
            let m = (v.sqr_magnitude() / Fix::from_int(10)).max(Fix::THREE);
            s * default_force / (m * k)
        };
        for h in found {
            if self.hits.contains(&h) {
                continue;
            }
            let Some(o) = w.objects.get(h) else {
                continue;
            };
            if o.tf.destroyed || o.part(PartKind::Collider).is_none() {
                continue;
            }
            let layer = o.layer;
            let tpos = o.tf.position;
            if layer == 8 {
                if super::is_pushable(w, h) == Some(false) {
                    continue;
                }
                let v = tpos - center;
                let f = force_of(v, self.scale);
                push_player_layer(w, h, v, f, d.grounded_player_force_multiplier);
                continue;
            }
            if layer == 16 {
                if !ability_body::is_platform_attachable(w, h) {
                    let v = tpos - center;
                    let f = force_of(v, self.scale);
                    w.physics.add_force(h, v * f * Fix::from_int(9000));
                }
                continue;
            }
            self.hits.push(h);
        }
        self.update_hit_objects(w, me, dt);
    }

    /// `UpdateHitObjects(dt)`.
    pub fn update_hit_objects(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        let c = self.def.curve.eval(self.time_alive);
        self.time_alive += dt;
        if self.time_alive > self.def.curve.last_time() {
            w.destroy(me);
            return;
        }
        let center = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        for i in 0..self.hits.len() {
            let h = self.hits[i];
            if self.ignore.contains(&h) {
                continue;
            }
            let Some(o) = w.objects.get(h) else {
                continue;
            };
            if o.tf.destroyed || o.part(PartKind::Collider).is_none_or(|p| p.destroyed) {
                continue;
            }
            let layer = o.layer;
            let (point, v, mut mag);
            let boulder = super::prefab::object_tag(w, h) == "boulder";
            if let Some(s) = Surface::of(w, h) {
                point = s.closest_point(center);
                v = -s.normal(point);
                let m = ((center - point).sqr_magnitude() / Fix::from_int(10)).max(F_1_5D);
                mag = if boulder {
                    self.default_force / m
                } else {
                    self.platform_force / m
                };
            } else {
                point = o.tf.position;
                v = point - center;
                let m = (v.sqr_magnitude() / Fix::from_int(10)).max(F_1_5D);
                mag = self.default_force / m;
            }
            mag = mag * self.scale;
            let f = v.normalized_safe() * mag * c;
            if layer == 23 {
                super::black_hole::add_force(w, h, f);
            } else {
                add_force_at_position(w, h, f, point, true);
            }
        }
    }
}

/// `InstantiateFixed` of a stand-alone shockwave prefab (a root with one
/// `Shockwave`, e.g. `AirWave_Shockwave`, `meteorShockwave`) at `pos`:
/// `SetPlayerId(owner)`, `SetScale(scale)`, forces multiplied by `force`
/// (Meteor's scale factor; 1 otherwise), `AddObjectToIgnore` for each of
/// `ignore`. It fires on its `Init` (the next tick).
pub fn spawn_shockwave(
    w: &mut World,
    prefab: usize,
    pos: Vec2,
    owner: i32,
    scale: Fix,
    force: Fix,
    ignore: &[ObjId],
) -> ObjId {
    let Some(f) = w
        .content
        .prefabs
        .get(prefab)
        .and_then(|p| root_fields(p, "Shockwave"))
    else {
        return ObjId::NONE;
    };
    let mut s = Shockwave::new(Arc::new(ShockwaveDef::from_fields(f)));
    s.owner_id = owner;
    s.scale = scale;
    if force != Fix::ONE {
        s.platform_force = s.platform_force * force;
        s.default_force = s.default_force * force;
    }
    s.ignore.extend_from_slice(ignore);
    let id = ability_body::spawn_prefab(
        w,
        prefab,
        Kind::Behaviour(Box::new(Behaviour::Mine(super::mine::Spawned::Shockwave(
            s,
        )))),
        &[("Shockwave", PartKind::Behaviour(1))],
        pos,
        None,
    );
    if let Some(o) = w.objects.get_mut(id) {
        o.player_id = owner;
    }
    id
}

/// `(Fix)1.5` (double literal).
const F_1_5D: Fix = Fix(6_442_450_944);

/// A shockwave push on a Player-layer object: PlayerPhysics (grounded ×
/// multiplier), else PlayerBody (×35), else its BoplBody (×9000, force).
pub fn push_player_layer(w: &mut World, h: ObjId, v: Vec2, f: Fix, grounded_mult: Fix) {
    let inv = w.content.player().inverse_mass01;
    let target = match w.objects.get(h).map(|o| &o.kind) {
        Some(Kind::Bopl(_)) => 0,
        Some(Kind::Ability(a)) => match a.body() {
            Some(b) if b.cfg.has_physics => 1,
            Some(_) => 2,
            None => 3,
        },
        _ => 3,
    };
    match (target, w.objects.get_mut(h).map(|o| &mut o.kind)) {
        (0, Some(Kind::Bopl(b))) => {
            let force = if b.physics.is_grounded {
                grounded_mult * v * f
            } else {
                v * f
            };
            b.body.external_velocity += force * inv;
        }
        (1, Some(Kind::Ability(a))) => {
            if let Some(body) = a.body_mut() {
                let force = if body.physics.is_grounded {
                    grounded_mult * v * f
                } else {
                    v * f
                };
                body.add_force(force);
            }
        }
        (2, Some(Kind::Ability(a))) => {
            // PlayerBody without PlayerPhysics.
            if let Some(body) = a.body_mut() {
                body.body_add_force(v * f);
            }
        }
        _ => {
            w.physics.add_force(h, v * f * Fix::from_int(9000));
        }
    }
}

// -- Explosion -----------------------------------------------------------------

/// An explosion prefab, typed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ExplosionDef {
    pub prefab: usize,
    pub hier: i32,
    pub hier_anim: Option<i32>,
    /// `cameraShakeTrauma` (float bits; presentation).
    pub trauma: u32,
    pub can_hurt_owner: bool,
    pub active_only_first_frame: bool,
    pub ignite_flammables: bool,
    pub anim: Option<AnimRef>,
    pub always_stop_during_time_stop: bool,
    pub shockwaves: Vec<Arc<ShockwaveDef>>,
    pub sound: Option<&'static str>,
    /// Tagged `beam` (cause Beamed).
    pub beam: bool,
}

/// Static names of the sounds explosion prefabs play (`PlaySound`).
pub fn static_sound(name: &str) -> Option<&'static str> {
    const NAMES: &[&str] = &[
        "explosion",
        "smokeExplosion",
        "mineExplosion",
        "smokeBombExplosion",
        "lightning",
    ];
    NAMES.iter().find(|n| **n == name).copied()
}

impl ExplosionDef {
    pub fn from_prefab(w: &World, prefab: usize) -> Option<ExplosionDef> {
        let p = w.content.prefabs.get(prefab)?;
        let root = p.objects.first()?;
        let ef = root_fields(p, "Explosion")?;
        let anim = ef.reference("explosionAnimation").and_then(|r| {
            let d = w.content.anim_data.iter().position(|a| a.id == r)?;
            (!w.content.anim_data[d].animations.is_empty()).then_some(AnimRef {
                data: d as u16,
                anim: 0,
            })
        });
        let animator = root.component("SpriteAnimator");
        let shockwaves = root
            .components
            .iter()
            .filter(|c| c.class == "Shockwave")
            .map(|c| Arc::new(ShockwaveDef::from_fields(&c.fields)))
            .collect();
        let sound = root
            .component("PlaySound")
            .and_then(|c| c.fields.str("soundName"))
            .and_then(static_sound);
        let trauma = match ef.get("cameraShakeTrauma") {
            Some(Value::Float(b)) => *b,
            _ => 0,
        };
        Some(ExplosionDef {
            prefab,
            hier: hier_of(ef),
            hier_anim: animator.and_then(|c| c.hierarchy),
            trauma,
            can_hurt_owner: ef.boolean("canHurtOwner").unwrap_or(true),
            active_only_first_frame: ef.boolean("hitBoxActiveOnlyFirstFrame").unwrap_or(true),
            ignite_flammables: ef.boolean("igniteFlammables").unwrap_or(true),
            anim,
            always_stop_during_time_stop: animator
                .and_then(|c| c.fields.boolean("AlwaysStopDuringTimeStop"))
                .unwrap_or(false),
            shockwaves,
            sound,
            beam: root.tag == "beam",
        })
    }
}

/// The Explosion component data other code needs (kill rules, ignition).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExplosionInfo {
    /// `PlayerOwnerId` (0 = never set).
    pub owner: i32,
    pub can_hurt_owner: bool,
    pub ignite_flammables: bool,
    pub beam: bool,
}

impl ExplosionInfo {
    /// From an Explosion component's fields (owner not set).
    pub fn from_fields(f: &Value) -> ExplosionInfo {
        ExplosionInfo {
            owner: 0,
            can_hurt_owner: f.boolean("canHurtOwner").unwrap_or(true),
            ignite_flammables: f.boolean("igniteFlammables").unwrap_or(true),
            beam: false,
        }
    }

    pub fn lethal(&self) -> super::Lethal {
        super::Lethal {
            owner: self.owner,
            can_hurt_owner: self.can_hurt_owner,
            beam: self.beam,
        }
    }
}

/// A spawned explosion object (`Explosion` + `Shockwave`s + animator).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Explosion {
    pub def: Arc<ExplosionDef>,
    pub owner: i32,
    pub first_frame: bool,
    pub anim: SpriteAnimator,
    pub shocks: Vec<Shockwave>,
}

/// Part index of the Explosion component; Shockwave `k` is `1 + k`.
pub const PART_EXPLOSION: u8 = 0;

impl Explosion {
    pub fn info(&self) -> ExplosionInfo {
        ExplosionInfo {
            owner: self.owner,
            can_hurt_owner: self.def.can_hurt_owner,
            ignite_flammables: self.def.ignite_flammables,
            beam: self.def.beam,
        }
    }

    pub fn init(&mut self, w: &mut World, me: ObjId, part: PartKind) {
        match part {
            PartKind::Behaviour(PART_EXPLOSION) => {
                self.first_frame = true;
                if self.def.active_only_first_frame {
                    w.set_collider_enabled(me, true);
                    let t = f32::from_bits(self.def.trauma);
                    if t > 0.0 {
                        w.emit(Cue::Shake((t * 1000.0) as i32));
                    }
                }
                if let Some(a) = self.def.anim {
                    let c = w.content.clone();
                    self.anim
                        .begin_then_do(&c, a, AnimCallback::Ability(CB_SELF_DESTRUCT));
                }
            }
            PartKind::Behaviour(n) => {
                if let Some(s) = self.shocks.get_mut(n as usize - 1) {
                    s.init(w, me);
                }
            }
            _ => {}
        }
    }

    pub fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        if late {
            return;
        }
        match part {
            PartKind::Behaviour(PART_EXPLOSION) => {
                if self.def.active_only_first_frame && !self.first_frame {
                    w.set_collider_enabled(me, false);
                }
                self.first_frame = false;
            }
            PartKind::Behaviour(n) => {
                if let Some(s) = self.shocks.get_mut(n as usize - 1) {
                    s.update(w, me, dt);
                }
            }
            PartKind::Animator => {
                // The animator's owner is the first IPlayerIdHolder on the
                // object: the first Shockwave (id 0 unless set), if any.
                let holder = self.shocks.first().map(|s| s.owner_id);
                let advance = if w.is_time_stopped() {
                    !self.def.always_stop_during_time_stop
                        && holder
                            .and_then(|h| w.player(h))
                            .is_some_and(|p| p.is_protected_from_time_stop)
                } else {
                    true
                };
                if advance {
                    let c = w.content.clone();
                    if let Some(AnimCallback::Ability(CB_SELF_DESTRUCT)) =
                        self.anim.advance(&c, Fix::ZERO)
                    {
                        w.destroy(me);
                    }
                }
            }
            _ => {}
        }
    }
}

/// `InstantiateFixed(explosionPrefab, pos)`, then (as every spawner does)
/// collider scale := `scale` if given and `PlayerOwnerId := owner`. The
/// shockwaves keep scale 1.
pub fn spawn(w: &mut World, prefab: usize, pos: Vec2, scale: Option<Fix>, owner: i32) -> ObjId {
    let Some(def) = ExplosionDef::from_prefab(w, prefab) else {
        return ObjId::NONE;
    };
    let def = Arc::new(def);
    let mut parts = vec![(PartKind::Behaviour(PART_EXPLOSION), def.hier, true)];
    for (k, s) in def.shockwaves.iter().enumerate() {
        parts.push((PartKind::Behaviour(1 + k as u8), s.hier, true));
    }
    if let Some(h) = def.hier_anim {
        parts.push((PartKind::Animator, h, true));
    }
    if let Some(o) = w.content.prefabs[prefab].objects.first()
        && o.collider.is_some()
        && let Some(h) = o.collider_hier
    {
        parts.push((PartKind::Collider, h, o.collider_enabled));
    }
    parts.sort_by_key(|p| p.1);
    let e = Explosion {
        shocks: def
            .shockwaves
            .iter()
            .map(|d| Shockwave::new(d.clone()))
            .collect(),
        def,
        owner,
        first_frame: true,
        anim: SpriteAnimator::default(),
    };
    let sound = e.def.sound;
    let id = ability_body::spawn_owned(
        w,
        prefab,
        Kind::Behaviour(Box::new(Behaviour::Mine(super::mine::Spawned::Explosion(
            e,
        )))),
        &parts,
        pos,
        None,
        0,
    );
    if let Some(s) = scale {
        w.set_collider_scale(id, s);
    }
    if let Some(s) = sound {
        w.emit(Cue::Sound(s));
    }
    id
}

/// The Explosion component of an object, if it has one.
pub fn info(w: &World, obj: ObjId) -> Option<ExplosionInfo> {
    match &w.objects.get(obj)?.kind {
        Kind::Behaviour(b) => match b.as_ref() {
            Behaviour::Mine(super::mine::Spawned::Explosion(e)) => Some(e.info()),
            Behaviour::Spike(s) => Some(s.explosion),
            Behaviour::TeslaCoil(t) => t.explosion(),
            _ => None,
        },
        _ => None,
    }
}

/// "Has a live Explosion component with `igniteFlammables`" (smoke clouds).
pub fn ignites_flammables(w: &World, obj: ObjId) -> bool {
    w.objects.alive(obj) && info(w, obj).is_some_and(|i| i.ignite_flammables)
}

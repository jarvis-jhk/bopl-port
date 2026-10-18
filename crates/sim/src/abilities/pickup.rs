//! `DynamicAbilityPickup` (abilities.md §11.2–§11.3, §13.2).

use super::{SpawnedImpl, instantiate};
use crate::consts::CONST_WATER_HEIGHT;
use crate::fix::Fix;
use crate::object::{ObjId, PartKind};
use crate::physics::shapes::ShapeKind;
use crate::physics::{BodyDef, ColliderDef, Contact};
use crate::vec2::Vec2;
use crate::view::{Cue, Visual};
use crate::world::{Kind, World};

use super::Behaviour;

/// Tuning (prefab values, abilities.md §11.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PickupDef {
    pub up_force: Fix,
    pub wobble_speed: Fix,
    pub wobble_amount: Fix,
    pub launch_force: Fix,
    pub disabled_seconds: Fix,
    pub swap_period: Fix,
    pub vibration_str: Fix,
    pub vibration_duration: Fix,
    pub vibrate_timing_offset: Fix,
    pub gravity: Fix,
    pub hier_pickup: i32,
    pub hier_body: i32,
    pub hier_circle: i32,
    pub collider: ColliderDef,
    pub layer: u8,
    pub updatables: i32,
}

impl Default for PickupDef {
    fn default() -> PickupDef {
        PickupDef {
            up_force: Fix(858_993_459_200),
            wobble_speed: Fix(8_589_934_592),
            wobble_amount: Fix(25_769_803_776),
            launch_force: Fix(42_949_672_960),
            disabled_seconds: Fix(429_496_736),
            swap_period: Fix(30_064_771_072),
            vibration_str: Fix(429_496_736),
            vibration_duration: Fix(2_576_980_480),
            vibrate_timing_offset: Fix::ZERO,
            gravity: Fix(128_849_018_880),
            hier_pickup: 1,
            hier_body: 2,
            hier_circle: 3,
            collider: ColliderDef {
                kind: ShapeKind::Circle,
                start_extents: Vec2::ZERO,
                start_radius: Fix(4_466_765_824),
                minimum_extents: Fix::ZERO,
                min_scale: Fix(429_496_736),
                max_scale: Fix::from_int(5),
                manual_init: true,
                body: Some(BodyDef {
                    mass: Fix::from_int(3),
                    bounciness: Fix(1_288_490_240),
                    friction: Fix(1_288_490_240),
                    dynamic_friction: Fix(1_717_986_944),
                    moment_of_inertia: Fix::ONE,
                    gravity_scale: Fix::ZERO,
                    air_friction01: Fix(4_252_017_664),
                    rotational_drag01: Fix::ONE,
                    start_velocity: Vec2::ZERO,
                    start_angular_velocity: Fix::ZERO,
                }),
                initialized: false,
            },
            layer: 13,
            updatables: 3,
        }
    }
}

/// Pickup state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pickup {
    pub def: PickupDef,
    pub ability: Option<usize>,
    pub time_passed: Fix,
    pub swap_timer: Fix,
    pub hitbox_disabled: bool,
    pub vibrate_offset: Vec2,
    /// `initSelf` (set on Duplicator clones: `InitPickup` runs at `Init`).
    pub init_self: bool,
}

/// Spawns a pickup and runs `InitPickup` (+ a random ability for spawners).
pub fn spawn(w: &mut World, pos: Vec2, ability: Option<usize>, dir: Vec2) -> ObjId {
    let def = w.content.pickup.unwrap_or_default();
    let p = Pickup {
        def,
        ability,
        time_passed: Fix::ZERO,
        swap_timer: Fix::ZERO,
        hitbox_disabled: true,
        vibrate_offset: Vec2::ZERO,
        init_self: false,
    };
    w.open_bracket();
    let id = w
        .objects
        .create(Kind::Behaviour(Box::new(Behaviour::Pickup(p))), def.layer);
    if let Some(o) = w.objects.get_mut(id) {
        o.collider = Some(def.collider);
        o.visual = match w.content.pickup_prefab {
            Some(i) => Visual::Prefab {
                prefab: i as u32,
                object: 0,
            },
            None => Visual::None,
        };
        o.add_part(PartKind::Behaviour(0), def.hier_pickup);
        o.add_part(PartKind::BoplBody, def.hier_body);
        o.add_part(PartKind::Collider, def.hier_circle);
    }
    w.register_all(id);
    w.close_bracket();
    w.consume_hierarchy((def.updatables - 3).max(0));
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.position = pos;
        o.tf.reset_rotation();
    }
    init_pickup(w, id, &def, dir);
    if ability.is_none() {
        let a = random_ability_except(w, None);
        if let Some(Kind::Behaviour(b)) = w.objects.get_mut(id).map(|o| &mut o.kind)
            && let Behaviour::Pickup(p) = b.as_mut()
        {
            p.ability = a;
        }
    }
    id
}

/// `InitPickup(prefab, icon, dir)`: circle initialised now, hitbox
/// disabled, launch impulse (the ability is stored by the caller).
fn init_pickup(w: &mut World, id: ObjId, def: &PickupDef, dir: Vec2) {
    let hier = w
        .objects
        .get(id)
        .and_then(|o| o.part(PartKind::Collider))
        .map_or(0, |p| p.hier);
    w.physics.register(&mut w.objects, id, hier);
    w.set_collider_enabled(id, false);
    w.physics
        .add_force(id, dir * def.launch_force * Fix::from_int(60));
}

/// Random with an excluded entry (abilities.md §13.2).
pub fn random_ability_except(w: &mut World, current: Option<usize>) -> Option<usize> {
    let count = w.content.abilities.len() as i32;
    if count <= 2 {
        return None;
    }
    let n = count - 2;
    let skip = current.map_or(-1, |c| c as i32) - 2;
    let mut r = w.rng.random_int(0, n - 1);
    if skip > 0 && r >= skip {
        r = (r + 1) % n;
    }
    Some((r + 2) as usize)
}

impl SpawnedImpl for Pickup {
    /// The ability's icon on the pickup's root renderer.
    fn look(&self, w: &World, _me: ObjId, l: &mut crate::view::Look) {
        l.sprite = self
            .ability
            .and_then(|a| w.content.abilities.get(a))
            .map_or(crate::data::SpriteId::NONE, |a| a.icon);
    }

    /// `Init`: a duplicated pickup (`initSelf`) re-derives its ability from
    /// its sprite (the copied icon) and runs `InitPickup` upwards.
    fn init(&mut self, w: &mut World, me: ObjId, part: PartKind) {
        if part != PartKind::Behaviour(0) {
            return;
        }
        if self.init_self {
            let def = self.def;
            init_pickup(w, me, &def, Vec2::UP);
            self.hitbox_disabled = true;
        }
        self.swap_timer = Fix::ZERO;
        self.vibrate_offset = Vec2::ZERO;
    }

    /// Clone rule (duplicator.md): the serialized fields (tuning, `initSelf`)
    /// and the sprite (the current icon, hence the ability) carry over; the
    /// timers and the vibration offset are runtime state.
    fn duplicate(&self, _w: &World, _me: ObjId) -> Option<Pickup> {
        Some(Pickup {
            def: self.def,
            ability: self.ability,
            time_passed: Fix::ZERO,
            swap_timer: Fix::ZERO,
            hitbox_disabled: self.hitbox_disabled,
            vibrate_offset: Vec2::ZERO,
            init_self: self.init_self,
        })
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        if late || part != PartKind::Behaviour(0) {
            return;
        }
        let d = self.def;
        if w.is_time_stopped() {
            if let Some(b) = w.physics.body_mut(me) {
                b.velocity = Vec2::ZERO;
            }
        } else {
            self.time_passed += dt;
            self.swap_timer += dt;
        }
        if self.time_passed >= d.disabled_seconds && self.hitbox_disabled {
            self.hitbox_disabled = false;
            w.set_collider_enabled(me, true);
        }
        if self.swap_timer > d.swap_period {
            self.swap_timer = Fix::ZERO;
            self.ability = random_ability_except(w, self.ability);
        }
        let phase = (self.swap_timer + d.swap_period - d.vibrate_timing_offset) % d.swap_period;
        if phase > d.swap_period - d.vibration_duration {
            let off = w.rng.random_inside_unit_circle() * d.vibration_str;
            if let Some(o) = w.objects.get_mut(me) {
                o.tf.position = o.tf.position - self.vibrate_offset + off;
            }
            self.vibrate_offset = off;
        }
        let pos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        let hit =
            w.physics
                .raycast_closest_rr(&w.objects, pos, Vec2::DOWN, Fix::from_int(10_000_000));
        let h = if hit.hit() {
            hit.near_dist
        } else if w.is_space() {
            pos.y - Fix::from_int(-1000)
        } else {
            pos.y - CONST_WATER_HEIGHT
        };
        if let Some(b) = w.physics.body_mut(me) {
            if h < Fix(2_147_483_648) {
                b.velocity = b.velocity * Fix(4_080_218_931);
            }
            b.velocity += Vec2::new(Fix::ZERO, -d.gravity) * dt;
        }
        let t = w.time.sim_time_since_level_loaded;
        let f = Vec2::new(Fix::ZERO, (t * d.wobble_speed).sin() * d.wobble_amount)
            + Vec2::new(Fix::ZERO, d.up_force / h.max(Fix::ONE));
        w.physics.add_force(me, f);
    }

    fn on_contact(&mut self, w: &mut World, me: ObjId, c: &Contact) {
        if c.layer == 13 || !w.objects.alive(me) {
            return;
        }
        let Some(Kind::Bopl(b)) = w.objects.get(c.other).map(|o| &o.kind) else {
            return;
        };
        let _ = b;
        let Some(a) = self.ability else {
            return;
        };
        let pid = w.objects.get(c.other).map_or(0, |o| o.player_id);
        let bopl = c.other;
        let obj = instantiate(w, a, pid, bopl);
        add_additional_ability(w, bopl, obj, a);
        w.emit(Cue::Sound("abilityPickup"));
        w.destroy(me);
    }
}

/// `AddAdditionalAbility` (abilities.md §11.3).
fn add_additional_ability(w: &mut World, bopl: ObjId, obj: ObjId, idx: usize) {
    let n_ab = w.settings.number_of_abilities;
    let pid = w.objects.get(bopl).map_or(0, |o| o.player_id);
    if let Some(Kind::Bopl(b)) = w.objects.get_mut(bopl).map(|o| &mut o.kind) {
        let c = &mut b.ctrl;
        if c.abilities.is_empty() {
            c.abilities = vec![obj];
            c.cooldown_timers = vec![Fix::from_int(100); 3];
            c.indicators = c.indicators.max(1);
        } else if c.abilities.len() >= 3 {
            c.abilities[2] = obj;
            c.cooldown_timers[2] = Fix::from_int(100_000);
        } else {
            c.abilities.push(obj);
            let k = c.abilities.len() - 1;
            if c.cooldown_timers.len() <= k {
                c.cooldown_timers.resize(k + 1, Fix::ZERO);
            }
            c.cooldown_timers[k] = Fix::from_int(100_000);
        }
    }
    let _ = n_ab;
    if let Some(p) = w.player_mut(pid) {
        if p.current_abilities.len() >= 3 {
            p.current_abilities[2] = idx;
        } else {
            p.current_abilities.push(idx);
        }
    }
}

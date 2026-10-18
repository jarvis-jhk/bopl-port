//! Black hole (`docs/spec/abilities/black-hole.md`): `BlackHoleClap` on the
//! ability object and the `BlackHole` it spawns (`BlackHole2`).
//!
//! Update order inside the ability object: SpriteAnimator 1 (pre-update),
//! Ability 2 (framework), PlayerCollision 3, PlayerPhysics 4,
//! DestroyIfOutsideSceneBounds 5, BlackHoleClap 6, DPhysicsBox 7,
//! PlayerBody 8. The hole: BlackHole 1, DPhysicsCircle 2 (manual init).
//!
//! Quirks kept: one hole per slot per round (`isCastable` false after the
//! spawn), platforms are pulled at any distance, plain players are killed
//! ignoring invulnerability, players in ability objects are removed through
//! the framework exit (the module's own cleanup is skipped), the strength
//! is computed (and divides by the inverse mass) for every active collider
//! before any range test.

use std::sync::Arc;

use super::ability_body::{self, AbilityBody, COLLIDER};
use super::prefab;
use super::prelude::*;
use super::{Behaviour, framework_exit, has_player_collision};
use crate::curve::Curve;
use crate::data::AnimRef;
use crate::world::Kind;

const CB_FIRE: u16 = 1;
const CB_FIRE_AIR: u16 = 2;
const CB_FINISH: u16 = 3;

/// `(Fix)0.01f`.
const F_0_01F: Fix = Fix(42_949_672);
/// `(Fix)0.1f` (initial previous target size).
const F_0_1F: Fix = Fix(429_496_736);

/// The `BlackHoleClap` ability.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BlackHole {
    pub b: AbilityBody,
    pub anim: SpriteAnimator,
    pub offset: Vec2,
    pub hole: Option<usize>,
    pub a_enter: AnimRef,
    pub a_exit: AnimRef,
    pub a_enter_air: AnimRef,
    pub a_exit_air: AnimRef,
    /// The object's x scale sign (`transform.localScale.x >= 0`).
    pub facing_right: bool,
}

/// Objects this module spawns.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    Hole(Hole),
}

/// The `BlackHole` component.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hole {
    pub owner: i32,
    pub mass: Fix,
    pub velocity: Vec2,
    pub target_size: Fix,
    pub prev_target_size: Fix,
    pub time_since_last_growth: Fix,
    pub sorting_order: i32,
    pub g: Fix,
    pub platform_multiplier: Fix,
    pub base_growth: Fix,
    pub growth: Fix,
    pub extra_merge_growth: Fix,
    pub size_anim: Arc<Curve>,
    pub min_size: Fix,
    pub drag: Fix,
    pub player_force: Fix,
    pub influence_radius_multiplier: Fix,
    pub start_radius: Fix,
    /// `startingMass` (`mass` at Awake).
    pub starting_mass: Fix,
    /// `magnetGunHolder` (set by a magnet's pickup, cleared by `Hold`).
    pub magnet_holder: Option<ObjId>,
    /// `holdForTicks`.
    pub hold_ticks: i64,
}

impl Hole {
    fn of_prefab(w: &World, prefab: usize) -> Option<Hole> {
        let o = w.content.prefabs.get(prefab)?.objects.first()?;
        let f = &o.component("BlackHole")?.fields;
        Some(Hole {
            owner: ENVIRONMENT_ID,
            mass: f.fix_or("startingMass", Fix::from_int(15)),
            starting_mass: f.fix_or("startingMass", Fix::from_int(15)),
            velocity: Vec2::ZERO,
            target_size: Fix::ZERO,
            prev_target_size: Fix::ZERO,
            time_since_last_growth: Fix::ZERO,
            sorting_order: 0,
            g: f.fix_or("G", Fix::from_int(100)),
            platform_multiplier: f.fix_or("platformMultiplier", F_0_1F),
            base_growth: f.fix_or("baseGrowth", Fix::ONE),
            growth: f.fix_or("growth", Fix::ONE),
            extra_merge_growth: f.fix_or("ExtraBlackHoleMergeGrowth", Fix::from_int(10)),
            size_anim: f.curve("SizeGrowthAnim").unwrap_or_default(),
            min_size: f.fix_or("minSize", Fix(644_245_120)),
            drag: f.fix_or("drag01", Fix::ONE),
            player_force: f.fix_or("playerForce", Fix::ZERO),
            influence_radius_multiplier: f.fix_or("influenceRadiusMultiplier", Fix::from_int(5)),
            start_radius: o.collider.map_or(Fix::ONE, |c| c.start_radius),
            magnet_holder: None,
            hold_ticks: 0,
        })
    }

    fn update_target_size(&mut self) {
        self.prev_target_size = self.target_size;
        self.target_size = Fix::from_int(100) + self.mass.abs() * self.growth;
    }

    /// `UpdateSize()`: the collider radius follows the size animation.
    /// The current size (`s`, the Unity transform scale of the hole).
    fn size(&self) -> Fix {
        let t = self.size_anim.eval(self.time_since_last_growth);
        let s = self.prev_target_size * (Fix::ONE - t) + self.target_size * t;
        s.max(self.min_size)
    }

    fn update_size(&mut self, w: &mut World, me: ObjId) {
        let s = self.size();
        let k = self.start_radius * F_0_01F;
        if s * k < self.min_size {
            return;
        }
        let r = (s * k).max(self.min_size);
        if let Some(c) = w.physics.circle_mut(me) {
            c.radius = r;
        } else if let Some(c) = w.objects.get_mut(me).and_then(|o| o.collider.as_mut()) {
            c.start_radius = r;
        }
    }

    fn rescale_velocity(&mut self, m0: Fix) {
        if self.mass != Fix::ZERO {
            let v = self.velocity;
            self.velocity = v.normalized_safe() * (m0 * v.sqr_magnitude() / self.mass).abs().sqrt();
        }
    }

    /// `Grow(inverseMass, extra)`.
    pub fn grow(&mut self, w: &mut World, inverse_mass: Fix, extra: Fix) {
        let m = Fix::ONE / inverse_mass;
        w.emit(Cue::Sound("blackHoleSuck"));
        let m0 = self.mass;
        self.mass = self.mass + (m + self.base_growth + extra);
        self.rescale_velocity(m0);
        self.update_target_size();
        self.time_since_last_growth = Fix::ZERO;
    }

    /// `GrowIncrementally(g)`.
    pub fn grow_incrementally(&mut self, g: Fix) {
        let m0 = self.mass;
        self.mass += g;
        self.rescale_velocity(m0);
        self.update_target_size();
    }

    /// `SetMass(m)`.
    pub fn set_mass(&mut self, m: Fix) {
        let m0 = self.mass;
        self.mass = m;
        self.rescale_velocity(m0);
        self.update_target_size();
    }

    /// `GravityForce(layer, inverseMass, transform)`.
    fn gravity_force(&self, w: &World, layer: i32, inverse_mass: Fix, obj: ObjId) -> Fix {
        let m = Fix::ONE / inverse_mass;
        let mut r = self.g * m * self.mass;
        if layer == 11 {
            let controlled = match w.objects.get(obj).filter(|o| !o.tf.destroyed).map(|o| &o.kind) {
                Some(Kind::Platform(p)) => p.spring.as_ref().is_some_and(|s| s.is_being_controlled),
                _ => false,
            };
            if controlled {
                r = r * self.platform_multiplier;
            }
        }
        r
    }

    /// The gravity pass over every collider of the world (black-hole.md).
    fn gravity(&self, w: &mut World, me: ObjId) {
        let pos = AbilityBody::pos(w, me);
        let range = w.physics.circle(me).map_or(Fix::ZERO, |c| c.radius)
            * self.influence_radius_multiplier;
        let usable = |w: &World, obj: ObjId| {
            w.objects.get(obj).is_some_and(|o| {
                o.part(PartKind::Collider)
                    .is_some_and(|p| p.enabled && !p.destroyed)
            }) && w.objects.active_in_hierarchy(obj)
        };
        // Circles.
        for i in 0..w.physics.circles.len() {
            let obj = w.physics.circles.parents[i].obj;
            if !usable(w, obj) || obj == me {
                continue;
            }
            let c = w.physics.circles.shapes[i];
            let v = pos - c.center;
            let sq = v.sqr_magnitude();
            let f = self.gravity_force(w, c.layer, w.physics.circles.bodies[i].inverse_mass, obj);
            if sq == Fix::ZERO {
                continue;
            }
            let d = sq.sqrt();
            if d >= range {
                continue;
            }
            let d3 = d * d * d;
            if d3 == Fix::ZERO {
                continue;
            }
            let force = v * f / d3;
            if c.layer == 27 {
                continue;
            }
            w.physics.circles.bodies[i].force += force;
        }
        // Boxes (players and box-shaped ability objects get a weak pull).
        for i in 0..w.physics.boxes.len() {
            let obj = w.physics.boxes.parents[i].obj;
            if !usable(w, obj) {
                continue;
            }
            let b = w.physics.boxes.shapes[i];
            let v = pos - b.center;
            let sq = v.sqr_magnitude();
            let f = self.gravity_force(w, b.layer, w.physics.boxes.bodies[i].inverse_mass, obj);
            if sq == Fix::ZERO {
                continue;
            }
            let d = sq.sqrt();
            if d >= range {
                continue;
            }
            let d3 = d * d * d;
            if d3 == Fix::ZERO {
                continue;
            }
            let force = v * f / d3;
            w.physics.boxes.bodies[i].force += force;
            if b.layer == 8 {
                pull_player(w, obj, (-force) * self.player_force);
            }
        }
        // Rounded rects: no range limit.
        for i in 0..w.physics.rrs.len() {
            let obj = w.physics.rrs.parents[i].obj;
            if !usable(w, obj) {
                continue;
            }
            let r = w.physics.rrs.shapes[i];
            let v = pos - r.b.center;
            let sq = v.sqr_magnitude();
            let f = self.gravity_force(w, r.layer, w.physics.rrs.bodies[i].inverse_mass, obj);
            if sq == Fix::ZERO {
                continue;
            }
            let d = sq.sqrt();
            let d3 = d * d * d;
            if d3 == Fix::ZERO {
                continue;
            }
            let force = v * f / d3;
            w.physics.rrs.bodies[i].force += force;
        }
    }
}

/// `PlayerPhysics.AddForce` or `PlayerBody.AddForce` of a Player-layer box.
fn pull_player(w: &mut World, obj: ObjId, f: Vec2) {
    let im = w.content.player().inverse_mass01;
    match w.objects.get_mut(obj).map(|o| &mut o.kind) {
        Some(Kind::Bopl(b)) => b.body.external_velocity += f * im,
        Some(Kind::Ability(_)) => {
            if let Some(b) = super::ability_body_mut(w, obj) {
                b.external_force(f);
            }
        }
        _ => {}
    }
}

fn with_hole<R>(w: &mut World, obj: ObjId, f: impl FnOnce(&mut World, &mut Hole) -> R) -> Option<R> {
    w.with_kind(obj, |w, k| match k {
        Kind::Behaviour(b) => match b.as_mut() {
            Behaviour::BlackHole(Spawned::Hole(h)) => Some(f(w, h)),
            _ => None,
        },
        _ => None,
    })
}

/// `BlackHole.AddForce(F)`: `velocity += F / mass`.
pub fn add_force(w: &mut World, obj: ObjId, f: Vec2) {
    with_hole(w, obj, |_, h| h.velocity += f / h.mass);
}

/// `BlackHole.AddForceLessMassInfluence(F)` (beams).
pub fn add_force_less_mass_influence(w: &mut World, obj: ObjId, f: Vec2) {
    with_hole(w, obj, |_, h| {
        h.velocity += h.mass.sign2_fix() * f / h.mass.abs().sqrt();
    });
}

/// `BlackHole.Grow(inverseMass, extra)` from other modules (growth rays).
pub fn grow(w: &mut World, obj: ObjId, inverse_mass: Fix, extra: Fix) {
    with_hole(w, obj, |w, h| h.grow(w, inverse_mass, extra));
}

/// `BlackHole.SetMass(m)` from other modules (duplicator).
pub fn set_mass(w: &mut World, obj: ObjId, m: Fix) {
    with_hole(w, obj, |_, h| h.set_mass(m));
}

/// `BlackHole.Hold(magnet, ticks)`: clears `magnetGunHolder` (literal), so
/// the hold countdown never runs.
pub fn hold(w: &mut World, obj: ObjId, ticks: i64) {
    with_hole(w, obj, |_, h| {
        h.magnet_holder = None;
        h.hold_ticks = ticks;
    });
}

/// A magnet gun picks the hole up: returns the previous `magnetGunHolder`
/// (to be told it was stolen) and records `magnet` as the holder.
pub fn take_magnet_holder(w: &mut World, obj: ObjId, magnet: ObjId) -> Option<ObjId> {
    with_hole(w, obj, |_, h| h.magnet_holder.replace(magnet)).flatten()
}

/// The mass of a black hole object.
pub fn mass(w: &World, obj: ObjId) -> Option<Fix> {
    match &w.objects.get(obj)?.kind {
        Kind::Behaviour(b) => match b.as_ref() {
            Behaviour::BlackHole(Spawned::Hole(h)) => Some(h.mass),
            _ => None,
        },
        _ => None,
    }
}

impl AbilityImpl for BlackHole {
    fn exit_component(&mut self, cx: &mut Cx<'_>, r: AbilityExitInfo) {
        self.component_exit(cx, r);
    }

    fn new(w: &World, def: &AbilityDef) -> Self {
        let f = prefab::component_fields(w, def, "BlackHoleClap");
        let data = f
            .reference("animData")
            .and_then(|r| prefab::anim_data_index(w, r))
            .unwrap_or(0);
        let a = |n: &str| prefab::anim(w, data, n);
        BlackHole {
            b: AbilityBody::new(w, def),
            anim: ability_body::animator(prefab::ability_prefab(w, def).is_some_and(prefab::root_always_stop)),
            offset: f.vec2("blackholeOffset").unwrap_or_default(),
            hole: f
                .reference("blackHolePrefab")
                .and_then(|r| prefab::prefab_index(w, r)),
            a_enter: a("enter"),
            a_exit: a("exit"),
            a_enter_air: a("enterAir"),
            a_exit_air: a("exitAir"),
            facing_right: true,
        }
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        let info = cx.info();
        let me = cx.me;
        self.b.sync_physics_to(cx.w, me, &info);
        AbilityBody::update_collider(cx.w, me);
        AbilityBody::set_rotation(cx.w, me, info.rotation);
        self.facing_right = info.facing_right;
        if !info.is_grounded {
            let body = &mut self.b.body;
            body.self_imposed_velocity = body.self_imposed_velocity * F_0_05;
            body.external_velocity = body.external_velocity * F_0_05;
        }
        if self.b.is_grounded() {
            let a = self.a_enter;
            ability_body::play_then(cx.w, &mut self.anim, a, CB_FIRE);
        } else {
            let a = self.a_enter_air;
            ability_body::play_then(cx.w, &mut self.anim, a, CB_FIRE_AIR);
        }
    }

    fn pre_update(&mut self, cx: &mut Cx<'_>, _dt: Fix) {
        let owner = cx.fw.owner;
        match ability_body::advance_code(cx.w, owner, &mut self.anim) {
            Some(CB_FIRE) => {
                self.fire_common(cx);
                let a = self.a_exit;
                ability_body::play_then(cx.w, &mut self.anim, a, CB_FINISH);
            }
            Some(CB_FIRE_AIR) => {
                self.fire_common(cx);
                let a = self.a_exit_air;
                ability_body::play_then(cx.w, &mut self.anim, a, CB_FINISH);
            }
            Some(CB_FINISH) => self.exit_component(cx, AbilityExitInfo::default()),
            _ => {}
        }
    }

    fn update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        // PlayerCollision 3, PlayerPhysics 4, bounds 5.
        AbilityBody::collision_update(cx.fw);
        self.b.physics_update(cx.w, me, dt);
        if let Some(r) = self.b.bounds_update(cx.w, me) {
            self.exit_component(cx, r);
            return;
        }
        // BlackHoleClap 6.
        if self.clap_update(cx, dt) && cx.w.objects.get(me).is_some_and(|o| o.active) {
            // PlayerBody 8.
            self.b.body_update(cx.w, me, dt);
        }
    }

    fn late_update(&mut self, cx: &mut Cx<'_>, _dt: Fix) {
        self.b.physics_late_update(cx.w, cx.me);
        self.b.body_late_update(cx.w, cx.me);
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        ability_body::hurt_contact(self, cx, c);
    }


    fn sprite(&self) -> crate::data::SpriteId {
        self.anim.sprite
    }

    fn look(&self, _w: &World, _me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        l.frame(&self.anim);
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        Some(&mut self.b)
    }
}

impl BlackHole {
    /// `BlackHoleClap.UpdateSim`; false if it exited (the object stops).
    fn clap_update(&mut self, cx: &mut Cx<'_>, dt: Fix) -> bool {
        let me = cx.me;
        if !self.b.is_grounded() {
            if self.b.velocity_based_raycasts(cx.w, me, true, dt) {
                AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
                let r = AbilityExitInfo {
                    just_landed: true,
                    grounded_speed: self.b.physics.grounded_speed,
                    is_grounded: self.b.physics.is_grounded,
                    facing_right: true,
                    position: AbilityBody::pos(cx.w, me),
                    self_imposed_velocity: self.b.body.self_imposed_velocity,
                    external_velocity: self.b.body.external_velocity,
                    ground: self.b.ground(cx.w),
                    grounded_local_position: self.b.physics.local_platform_pos,
                    radius: self.b.physics.radius,
                    ..AbilityExitInfo::default()
                };
                cx.exit_body(&mut self.b, r);
                return false;
            }
        } else {
            let ok = self
                .b
                .ground_alive(cx.w)
                .is_some_and(|g| cx.w.objects.get(g).is_some_and(|o| o.active));
            if !ok {
                AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
                let r = AbilityExitInfo {
                    is_grounded: false,
                    facing_right: true,
                    position: AbilityBody::pos(cx.w, me),
                    self_imposed_velocity: self.b.body.self_imposed_velocity,
                    external_velocity: self.b.body.external_velocity,
                    ground: None,
                    radius: self.b.physics.radius,
                    ..AbilityExitInfo::default()
                };
                cx.exit_body(&mut self.b, r);
                return false;
            }
        }
        if let Some(r) = self.b.move_(cx.w, me, Vec2::ZERO, dt, true) {
            self.exit_component(cx, r);
        }
        true
    }

    fn fire_common(&mut self, cx: &mut Cx<'_>) {
        let me = cx.me;
        let Some(t) = cx.w.objects.get(me).map(|o| o.tf) else {
            return;
        };
        let mut u = self.offset;
        if !self.facing_right {
            u.x = u.x * (-Fix::ONE);
        }
        let u = t.right.complex_mul(u);
        let k = (t.scale / Fix::THREE).clamp(Fix::ONE, Fix::TWO);
        let pos = t.position + k * u;
        if let Some(prefab) = self.hole {
            spawn_hole(cx.w, prefab, pos, cx.fw.owner);
        }
        cx.w.emit(Cue::Sound("spawnBlackHole"));
        cx.fw.is_castable = false;
    }

    /// `BlackHoleClap.ExitAbility(record)`.
    fn component_exit(&mut self, cx: &mut Cx<'_>, mut r: AbilityExitInfo) {
        let me = cx.me;
        r.self_imposed_velocity = self.b.body.self_imposed_velocity;
        r.external_velocity = self.b.body.external_velocity;
        r.facing_right = self.facing_right;
        r.position = AbilityBody::pos(cx.w, me);
        r.grounded_speed = self.b.physics.grounded_speed;
        r.is_grounded = self.b.physics.is_grounded;
        r.ground = self.b.ground(cx.w);
        r.grounded_local_position = self.b.physics.local_platform_pos;
        r.radius = self.b.physics.radius;
        cx.exit_body(&mut self.b, r);
    }
}

/// Spawns a `BlackHole2` (`InstantiateFixed`, `OwnerId`).
pub fn spawn_hole(w: &mut World, prefab: usize, pos: Vec2, owner: i32) -> ObjId {
    let Some(mut h) = Hole::of_prefab(w, prefab) else {
        return ObjId::NONE;
    };
    h.owner = owner;
    let id = ability_body::spawn_prefab(
        w,
        prefab,
        Kind::Behaviour(Box::new(Behaviour::BlackHole(Spawned::Hole(h)))),
        &[
            ("BlackHole", PartKind::Behaviour(0)),
            (COLLIDER, PartKind::Collider),
        ],
        pos,
        None,
    );
    if let Some(o) = w.objects.get_mut(id) {
        o.player_id = owner;
    }
    id
}

impl super::SpawnedImpl for Spawned {
    /// The hole: black (white for negative mass, `blackHoleColor` /
    /// `whiteHoleColor`), distortion −clamp(mass, −1, 1), hole brightness 1
    /// for white holes; the shader's `_Scale` is the renderer's scale.
    fn look(&self, w: &World, me: ObjId, l: &mut crate::view::Look) {
        let Spawned::Hole(h) = self;
        let white = h.mass < Fix::ZERO;
        let f = super::prefab::object_fields(w, me, "BlackHole");
        let col = f
            .and_then(|f| f.get(if white { "whiteHoleColor" } else { "blackHoleColor" }))
            .and_then(super::color_field);
        let mut p = crate::view::PartLook::new(0);
        p.tint = col;
        l.parts.push(p);
        // The visual size follows `s` (the prefab's transform scale is the
        // 100 of a new hole).
        let k = h.size() / Fix::from_int(100);
        l.stretch = crate::vec2::Vec2::new(k, k);
        l.params = [
            Fix::ZERO,
            -h.mass.clamp(-Fix::ONE, Fix::ONE),
            if white { Fix::ONE } else { Fix::ZERO },
            Fix::ZERO,
        ];
    }

    fn init(&mut self, w: &mut World, me: ObjId, part: PartKind) {
        let Spawned::Hole(h) = self;
        if part != PartKind::Behaviour(0) {
            return;
        }
        w.ability_globals.black_hole_index += 1;
        h.sorting_order = -1000 + w.ability_globals.black_hole_index;
        h.prev_target_size = F_0_1F;
        h.target_size = Fix::from_int(100);
        h.update_target_size();
        h.time_since_last_growth = Fix::ZERO;
        h.update_size(w, me);
        let hier = w
            .objects
            .get(me)
            .and_then(|o| o.part(PartKind::Collider))
            .map_or(0, |p| p.hier);
        w.physics.register(&mut w.objects, me, hier);
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        let Spawned::Hole(h) = self;
        if late || part != PartKind::Behaviour(0) {
            return;
        }
        let stopped = w.is_time_stopped();
        if !stopped {
            h.time_since_last_growth += dt;
        }
        h.update_size(w, me);
        if stopped {
            return;
        }
        // Magnet hold countdown (inert: `Hold` clears the holder).
        if h.magnet_holder.is_some_and(|m| w.objects.alive(m)) {
            h.hold_ticks -= 1;
            if h.hold_ticks <= 0 {
                h.magnet_holder = None;
            }
        }
        if let Some(o) = w.objects.get_mut(me) {
            o.tf.position = o.tf.position + h.velocity * dt;
        }
        h.velocity = h.velocity * h.drag;
        h.gravity(w, me);
    }

    /// Clone rule (duplicator.md): the tuning carries over; mass (Awake:
    /// `startingMass`), target size, owner (1000), hold ticks, velocity and
    /// the growth timer are runtime (the duplicator's black-hole branch
    /// then sets the mass).
    fn duplicate(&self, _w: &World, _me: ObjId) -> Option<Spawned> {
        let Spawned::Hole(h) = self;
        Some(Spawned::Hole(Hole {
            owner: ENVIRONMENT_ID,
            mass: h.starting_mass,
            velocity: Vec2::ZERO,
            target_size: Fix::ZERO,
            prev_target_size: Fix::ZERO,
            time_since_last_growth: Fix::ZERO,
            sorting_order: 0,
            magnet_holder: None,
            hold_ticks: 0,
            ..h.clone()
        }))
    }

    fn on_contact(&mut self, w: &mut World, me: ObjId, c: &Contact) {
        let Spawned::Hole(h) = self;
        if h.mass < Fix::ZERO {
            return;
        }
        let other = c.other;
        let Some(o) = w.objects.get(other) else {
            return;
        };
        let inverse_mass = w.physics.body(other).map_or(Fix::ZERO, |b| b.inverse_mass);
        match c.layer {
            23 => {
                let Some(m) = mass(w, other) else {
                    return;
                };
                let order = match &o.kind {
                    Kind::Behaviour(b) => match b.as_ref() {
                        Behaviour::BlackHole(Spawned::Hole(x)) => x.sorting_order,
                        _ => 0,
                    },
                    _ => 0,
                };
                if (m == h.mass && h.sorting_order > order) || h.mass > m {
                    let extra = h.extra_merge_growth;
                    h.grow(w, inverse_mass, extra);
                    w.destroy(other);
                    // Presentation: `OnEatParticle.Stop()` (scale 0).
                    let pos = AbilityBody::pos(w, me);
                    super::fx(w, me, "OnEatParticle", pos, Fix::ZERO, Fix::ZERO, h.owner, Vec2::ZERO);
                }
            }
            8 => {
                let is_ability = matches!(o.kind, Kind::Ability(_));
                let is_bopl = matches!(o.kind, Kind::Bopl(_));
                let collision = is_bopl || has_player_collision(w, other);
                h.grow(w, inverse_mass, Fix::ZERO);
                let owner = h.owner;
                eat_fx(w, me, other, owner);
                if collision && is_ability {
                    let pos = AbilityBody::pos(w, other);
                    framework_exit(
                        w,
                        other,
                        AbilityExitInfo {
                            position: pos,
                            is_dead: true,
                            killer_id: owner,
                            ..AbilityExitInfo::default()
                        },
                    );
                } else if collision {
                    w.with_kind(other, |w, k| {
                        if let Kind::Bopl(b) = k {
                            b.kill_player(w, other, owner, false, true, CauseOfDeath::Other);
                        }
                    });
                } else if is_ability {
                    let pos = AbilityBody::pos(w, other);
                    framework_exit(
                        w,
                        other,
                        AbilityExitInfo {
                            position: pos,
                            is_dead: true,
                            killer_id: owner,
                            is_grounded: false,
                            ..AbilityExitInfo::default()
                        },
                    );
                }
            }
            11 => {
                let pos = AbilityBody::pos(w, me);
                if c.collider.contains(pos) {
                    eat_fx(w, me, other, h.owner);
                    h.grow(w, inverse_mass, Fix::ZERO);
                    w.destroy(other);
                } else {
                    // Presentation: `OnEatParticle.Play()` while touching
                    // ground (stopped by the next eat).
                    super::fx(w, me, "OnEatParticle", pos, Fix::ZERO, Fix::ONE, h.owner, Vec2::ZERO);
                }
            }
            _ => {
                if inverse_mass > Fix::ZERO {
                    eat_fx(w, me, other, h.owner);
                    h.grow(w, inverse_mass, Fix::ZERO);
                    w.destroy(other);
                }
            }
        }
    }
}

/// Presentation: the swallow particles (`OnEatParticlePrefab`) where an
/// eaten object was (`value` = the hole's position, their orbit centre).
fn eat_fx(w: &mut World, hole: ObjId, eaten: ObjId, owner: i32) {
    let (Some(p), Some(h)) = (
        w.objects.get(eaten).map(|o| o.tf.position),
        w.objects.get(hole).map(|o| o.tf.position),
    ) else {
        return;
    };
    super::fx(w, hole, "OnEatParticlePrefab", p, Fix::ZERO, Fix::ONE, owner, h);
}

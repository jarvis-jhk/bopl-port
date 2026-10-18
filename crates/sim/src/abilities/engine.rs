//! Engine (`docs/spec/abilities/engine.md`): `PlantObjectOnPlatform` (the
//! shared "kneel and plant" component, also used by Tesla coil through
//! [`PlantObjectOnPlatform`]) with the `PlaceRocketEngine` placer, and the
//! planted `RocketEngine`.
//!
//! Ability components in update order: Ability 1, SpriteAnimator 2,
//! PlayerPhysics 3, DestroyIfOutsideSceneBounds 4, PlayerCollision 5,
//! PlantObjectOnPlatform 6, PlayerBody 7, DPhysicsBox 8.

use std::sync::Arc;

use super::Behaviour;
use super::ability_body::{self as body, AbilityBody};
use super::prefab::{self, root_component};
use super::prelude::*;
use super::smoke::RingBuffer;
use crate::curve::Curve;
use crate::data::{AnimRef, SpriteId};
use crate::surface::Surface;
use crate::world::Kind;

/// Animator callback codes.
/// `PlantObjectOnPlatform` animator callbacks (shared with Tesla coil).
pub const CB_PLANT: u16 = 1;
pub const CB_EXIT: u16 = 2;
const CB_START: u16 = 3;
const CB_TURN_OFF: u16 = 4;

fn anim_data(w: &World, r: Option<&str>) -> Option<u16> {
    let r = r?;
    w.content
        .anim_data
        .iter()
        .position(|a| a.id == r)
        .map(|i| i as u16)
}

/// `PlantObjectOnPlatform` (shared with Tesla coil): kneel, plant on the
/// attached platform when `cast` ends, recover, exit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PlantObjectOnPlatform {
    pub body: AbilityBody,
    pub anim: SpriteAnimator,
    pub cast: Option<AnimRef>,
    pub recover: Option<AnimRef>,
    /// Snapshot facing (`localScale.x = facingRight ? −1 : 1`).
    pub facing_right: bool,
}

impl PlantObjectOnPlatform {
    pub fn new(w: &World, def: &AbilityDef) -> Self {
        let c = root_component(&w.content, def, "PlantObjectOnPlatform");
        let data = anim_data(w, c.and_then(|c| c.fields.reference("animData")));
        PlantObjectOnPlatform {
            body: AbilityBody::new(w, def),
            anim: body::animator(prefab::ability_prefab(w, def).is_some_and(prefab::root_always_stop)),
            cast: data.and_then(|d| w.content.anim_ref(d, "cast")),
            recover: data.and_then(|d| w.content.anim_ref(d, "recover")),
            facing_right: true,
        }
    }

    /// The planting body: frame, mirrored by facing, and the dummy object
    /// (prefab object 1) in the hand during the cast
    /// (`DummyPositions`/`DummyRotations` from frame `ShowDummyAtFrame`).
    pub fn look(&self, w: &World, me: ObjId, l: &mut crate::view::Look) {
        use crate::view::PartLook;
        l.frame(&self.anim);
        let sx = self.x_scale();
        l.stretch = Vec2::new(sx, Fix::ONE);
        let f = prefab::object_fields(w, me, "PlantObjectOnPlatform");
        let casting = self.anim.current.is_some() && self.anim.current == self.cast;
        let mut shown = None;
        if casting && let Some(f) = f {
            let show = f.int("ShowDummyAtFrame").unwrap_or(1) as i32;
            let num = self.anim.passed_sprites() - 1;
            let pos = f.list("DummyPositions");
            let rot = f.list("DummyRotations");
            let k = num - show;
            if k >= 0 && (num as usize) < pos.len() {
                let off = f.get("DummyOffset").and_then(prefab::float_vec2).unwrap_or_default();
                let p = pos.get(k as usize).and_then(prefab::float_vec2).unwrap_or_default();
                let r = rot
                    .get(k as usize)
                    .and_then(prefab::float_fix)
                    .unwrap_or(Fix::ZERO);
                let sign = if sx < Fix::ZERO { -Fix::ONE } else { Fix::ONE };
                shown = Some(PartLook::new(1).posed(
                    p + off,
                    sign * r * Fix::PI / Fix::from_int(180),
                    Vec2::new(Fix::ONE, Fix::ONE),
                ));
            }
        }
        l.parts.push(shown.unwrap_or(PartLook::hidden(1)));
    }

    /// Presentation: the plant dust (`ParticlePrefab`) at the placed object
    /// (`p`, surface normal `n`) offset by `ParticlePlantOffset.y` along it.
    pub fn plant_fx(w: &mut World, me: ObjId, owner: i32, p: Vec2, n: Vec2) {
        let off = prefab::object_fields(w, me, "PlantObjectOnPlatform")
            .and_then(|f| f.get("ParticlePlantOffset"))
            .and_then(|v| prefab::float_vec2(v).or_else(|| prefab::vec2_field_value(v)))
            .unwrap_or_default();
        super::fx(
            w,
            me,
            "ParticlePrefab",
            p + n * off.y,
            super::rotation_of_up(n),
            Fix::ONE,
            owner,
            Vec2::ZERO,
        );
    }

    /// `x scale` sign of the ability object.
    pub fn x_scale(&self) -> Fix {
        if self.facing_right { -Fix::ONE } else { Fix::ONE }
    }

    /// `OnEnterAbility`.
    pub fn enter(&mut self, cx: &mut Cx<'_>) {
        let info = cx.info();
        let me = cx.me;
        if let Some(o) = cx.w.objects.get_mut(me) {
            o.tf.set_rotation(info.rotation);
        }
        if let Some(a) = self.cast {
            let c = cx.w.content.clone();
            self.anim.begin_then_do(&c, a, AnimCallback::Ability(CB_PLANT));
        }
        self.body.sync_physics_to(cx.w, me, &info);
        cx.w.physics.update_physics_positions(&cx.w.objects, me);
        AbilityBody::set_pos(cx.w, me, info.position);
        self.facing_right = info.facing_right;
    }

    /// The ground to plant on (`None` = exit at once).
    pub fn plant_ground(&self, w: &World) -> Option<ObjId> {
        self.body
            .physics
            .attached_ground
            .filter(|g| w.objects.alive(*g) && w.objects.active_in_hierarchy(*g))
    }

    pub fn begin_recover(&mut self, w: &World) {
        if let Some(a) = self.recover {
            self.anim
                .begin_then_do(&w.content, a, AnimCallback::Ability(CB_EXIT));
        }
    }

    /// `PlantObjectOnPlatform.UpdateSim`.
    pub fn update(&mut self, cx: &mut Cx<'_>, dt: Fix) -> Option<AbilityExitInfo> {
        if self.body.is_grounded() {
            return self.body.move_(cx.w, cx.me, Vec2::ZERO, dt, true);
        }
        None
    }

    /// `ExitAbility(record)` up to the placer's `ModifyExitInfo`.
    pub fn exit_record(&mut self, cx: &mut Cx<'_>, mut rec: AbilityExitInfo) -> AbilityExitInfo {
        let me = cx.me;
        if let Some(o) = cx.w.objects.get_mut(me) {
            o.tf.set_rotation(Fix::ZERO);
        }
        let input = cx
            .w
            .player(cx.owner())
            .map_or(Vec2::ZERO, |p| p.inputs.input_vector())
            .normalized_safe();
        let g = self.body.input_ground_alignment(cx.w, me, input);
        if g.abs() >= GROUND_ALIGNMENT_DEADZONE && g.sign2_fix() * self.x_scale() >= Fix::ZERO {
            rec.enter_turn = true;
        }
        rec.grounded_speed = Fix::ZERO;
        rec.is_grounded = self.body.is_grounded();
        rec.facing_right = self.facing_right;
        rec.radius = self.body.physics.radius;
        rec.position = AbilityBody::pos(cx.w, me);
        if self.body.is_grounded() {
            rec.ground = self.body.physics.attached_ground;
            rec.grounded_local_position = self.body.physics.local_platform_pos;
        }
        rec.self_imposed_velocity = Vec2::ZERO;
        rec.external_velocity = Vec2::ZERO;
        self.body.hand_back_rope(&mut rec);
        rec
    }
}

/// `PlaceRocketEngine` values and the planted-object ring buffer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PlaceRocketEngine {
    pub plant_offset: Vec2,
    pub cast_force: Fix,
    pub prefab: Option<usize>,
    pub engine: EngineDef,
    pub ring: RingBuffer,
}

/// Module state (one per ability object).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Engine {
    pub plant: PlantObjectOnPlatform,
    pub placer: PlaceRocketEngine,
}

/// Objects this module spawns.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    RocketEngine(RocketEngine),
}

impl AbilityImpl for Engine {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let c = &w.content;
        let pc = root_component(c, def, "PlaceRocketEngine");
        let prefab = prefab::prefab_ref(
            c,
            pc.and_then(|p| p.fields.reference("plantablePrefab")),
            "RocketEngine",
        );
        let n = prefab::int_field(pc, "bufferSize", 2) as usize;
        Engine {
            plant: PlantObjectOnPlatform::new(w, def),
            placer: PlaceRocketEngine {
                plant_offset: prefab::vec2_field_or(
                    pc,
                    "plantOffset",
                    Vec2::new(Fix(-3_564_822_784), Fix(1_288_490_240)),
                ),
                cast_force: prefab::fix_field(pc, "castForce", Fix::from_int(10)),
                prefab,
                engine: EngineDef::load(w, prefab),
                ring: RingBuffer::new(n.max(1)),
            },
        }
    }

    fn build(&mut self, _w: &mut World, _me: ObjId, fw: &mut Framework, _def: &AbilityDef) {
        self.plant.body.install(fw);
    }

    fn body(&self) -> Option<&AbilityBody> {
        Some(&self.plant.body)
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        Some(&mut self.plant.body)
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        self.plant.enter(cx);
    }

    fn tick(&mut self, cx: &mut Cx<'_>, dt: Fix, late: bool) {
        let me = cx.me;
        let b = &mut self.plant.body;
        if late {
            b.physics_late_update(cx.w, me);
            b.body_late_update(cx.w, me);
            return;
        }
        cx.ability_update();
        if let Some(AnimCallback::Ability(code)) =
            body::advance_anim(cx.w, cx.owner(), &mut self.plant.anim)
        {
            self.on_anim(cx, code);
        }
        if !cx.running() {
            return;
        }
        let b = &mut self.plant.body;
        b.physics_update(cx.w, me, dt);
        if let Some(rec) = b.bounds_update(cx.w, me) {
            self.exit(cx, rec);
            return;
        }
        AbilityBody::collision_update(cx.fw);
        if let Some(rec) = self.plant.update(cx, dt) {
            self.exit(cx, rec);
        }
        if !cx.running() {
            return;
        }
        self.plant.body.body_update(cx.w, me, dt);
    }

    fn on_anim(&mut self, cx: &mut Cx<'_>, code: u16) {
        match code {
            CB_PLANT => {
                let Some(g) = self.plant.plant_ground(cx.w) else {
                    self.exit(cx, AbilityExitInfo::default());
                    return;
                };
                self.place(cx, g);
                cx.w.emit(Cue::Sound("plantObjectOnPlatform"));
                let w = &*cx.w;
                self.plant.begin_recover(w);
            }
            CB_EXIT => self.exit(cx, AbilityExitInfo::default()),
            _ => {}
        }
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        if let Some((killer, cause)) = self.plant.body.kill_on_collision(cx.w, cx.me, cx.fw, c) {
            self.kill(cx, killer, true, false, cause);
        }
    }

    fn exit_component(&mut self, cx: &mut Cx<'_>, rec: AbilityExitInfo) {
        self.exit(cx, rec);
    }

    fn sprite(&self) -> SpriteId {
        self.plant.anim.sprite
    }

    fn look(&self, w: &World, me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        self.plant.look(w, me, l);
    }
}

impl Engine {
    fn exit(&mut self, cx: &mut Cx<'_>, rec: AbilityExitInfo) {
        let rec = self.plant.exit_record(cx, rec);
        cx.exit(rec);
    }

    /// `PlaceRocketEngine.Place(owner, ground)`.
    fn place(&mut self, cx: &mut Cx<'_>, ground: ObjId) {
        let me = cx.me;
        let w = &mut *cx.w;
        let Some(s) = Surface::of(w, ground) else {
            return;
        };
        let Some(prefab) = self.placer.prefab else {
            return;
        };
        let t = w.objects.get(me).map(|o| o.tf).unwrap_or_default();
        let sgn = if self.plant.x_scale() < Fix::ZERO {
            -Fix::ONE
        } else {
            Fix::ONE
        };
        let off = self.placer.plant_offset;
        let q = s.closest_point(t.position + t.right * sgn * off.x);
        let local = s.param_from_position(q, Fix::ZERO);
        let mut p = s.position_from_param(local, Fix::ZERO);
        let n = s.normal(p);
        p += n * off.y;
        let ed = self.placer.engine.clone();
        let id = body::spawn_prefab(
            w,
            prefab,
            Kind::Empty,
            &[
                ("RocketEngine", PartKind::Behaviour(0)),
                (body::COLLIDER, PartKind::Collider),
                ("BoplBody", PartKind::BoplBody),
                ("SpriteAnimator", PartKind::Behaviour(1)),
            ],
            p,
            None,
        );
        w.objects.set_parent(id, Some(ground));
        if let Some(o) = w.objects.get_mut(id) {
            o.tf.position = p;
            o.tf.set_up(n);
        }
        PlantObjectOnPlatform::plant_fx(w, me, cx.fw.owner, p, n);
        let mut e = RocketEngine {
            def: ed,
            platform: ground,
            local: Fix::ZERO,
            surface_offset: Fix::ZERO,
            prev_scale: Fix::ONE,
            on: false,
            stopping: false,
            timer: Fix::ZERO,
            owner: cx.fw.owner,
            anim: SpriteAnimator::default(),
        };
        e.init(w, id, off.y, p, t.scale, false);
        w.put_kind(
            id,
            Kind::Behaviour(Box::new(Behaviour::Engine(Spawned::RocketEngine(e)))),
        );
        body::add_force_at_position(w, ground, -n * self.placer.cast_force, t.position, false);
        self.placer.ring.add(w, id);
    }
}

/// `RocketEngine` prefab values.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct EngineDef {
    pub force: Arc<Curve>,
    pub offset: Vec2,
    pub radius: Fix,
    pub smoke_mask: u32,
    pub start: Option<AnimRef>,
    pub looped: Option<AnimRef>,
    pub end: Option<AnimRef>,
}

impl EngineDef {
    fn load(w: &World, prefab: Option<usize>) -> EngineDef {
        let rc = prefab.and_then(|p| prefab::component(&w.content, p, "RocketEngine"));
        let data = anim_data(w, rc.and_then(|c| c.fields.reference("FireData")));
        let a = |n: &str| data.and_then(|d| w.content.anim_ref(d, n));
        EngineDef {
            force: prefab::curve_field(rc, "ForceAnim"),
            offset: prefab::vec2_field_or(rc, "offset", Vec2::ZERO),
            radius: prefab::fix_field(rc, "radius", Fix::ZERO),
            smoke_mask: prefab::int_field(rc, "smokeOnlyLayerMask", 0) as u32,
            start: a("start"),
            looped: a("loop"),
            end: a("end"),
        }
    }
}

/// The planted engine.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RocketEngine {
    pub def: EngineDef,
    pub platform: ObjId,
    pub local: Fix,
    pub surface_offset: Fix,
    pub prev_scale: Fix,
    pub on: bool,
    pub stopping: bool,
    pub timer: Fix,
    pub owner: i32,
    pub anim: SpriteAnimator,
}

impl RocketEngine {
    /// `RocketEngineInit(srr, surfaceOffset, position, scale, owner, …,
    /// skipInit)`.
    fn init(
        &mut self,
        w: &mut World,
        me: ObjId,
        surface_offset: Fix,
        pos: Vec2,
        scale: Fix,
        skip_init: bool,
    ) {
        if !skip_init {
            let hier = body::collider_hier(w, me);
            w.physics.register(&mut w.objects, me, hier);
        }
        w.set_collider_scale(me, scale);
        crate::platform::update_bounds(w, self.platform);
        let s = Surface::of(w, self.platform);
        self.local = s.map_or(Fix::ZERO, |s| s.param_from_position(pos, Fix::ZERO));
        self.surface_offset = surface_offset;
        self.on = false;
        self.stopping = false;
        self.timer = Fix::ZERO;
        if let Some(a) = self.def.start {
            let c = w.content.clone();
            self.anim
                .begin_then_do(&c, a, AnimCallback::Ability(CB_START));
        }
        if let Some(o) = w.objects.get_mut(me) {
            o.set_part_enabled(PartKind::Behaviour(1), true);
        }
        w.emit(Cue::Sound("startEngine"));
        let (lo, hi) = w
            .objects
            .get(me)
            .and_then(|o| o.collider)
            .map_or((Fix::ZERO, Fix::MAX), |c| (c.min_scale, c.max_scale));
        let k = scale.clamp(lo, hi);
        self.local = s.map_or(Fix::ZERO, |s| s.param_from_position(pos, Fix::ZERO));
        self.surface_offset = surface_offset * k;
        w.set_collider_scale(me, k);
        self.prev_scale = k;
        w.physics.weld_to_rr(&w.objects, me, self.platform);
        self.reposition(w, me);
    }

    /// `RepositionOnParent()`.
    fn reposition(&self, w: &mut World, me: ObjId) {
        if !w.objects.alive(self.platform) {
            return;
        }
        let Some(s) = Surface::of(w, self.platform) else {
            return;
        };
        let Some(t) = w.objects.get(me).map(|o| o.tf) else {
            return;
        };
        let pt = w.objects.get(self.platform).map(|o| o.tf).unwrap_or_default();
        let v = s.position_from_param(self.local, Fix::ZERO) + t.up * self.surface_offset;
        let u = Vec2::from_angle(Fix::PI_TIMES_2 - pt.rotation);
        let rel = u.complex_mul(v - pt.position);
        let rel_right = t.right.complex_mul(u);
        if let Some(b) = w.physics.own_body_mut(me) {
            b.relative_pos = rel + rel_right.complex_mul(t.offset);
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        let scale = w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale);
        if scale != self.prev_scale {
            let f = scale / self.prev_scale;
            self.surface_offset = self.surface_offset * f;
            self.prev_scale = scale;
        }
        if !self.on {
            return;
        }
        // `BlowUpSmokeTouchingTheFire`: the first Smoke-layer circle at the
        // flame ignites.
        let t = w.objects.get(me).map(|o| o.tf).unwrap_or_default();
        let at = t.position + t.up * (self.def.offset.y * t.scale) + t.right * (self.def.offset.x * t.scale);
        let found = w.physics.circle_cast_circles(&w.objects, at, self.def.radius * t.scale, self.def.smoke_mask, 1);
        if let Some(&c) = found.first()
            && w.objects.get(c).is_some_and(|o| !o.tf.destroyed)
        {
            super::interact(w, c, super::Interaction::Ignite);
        }
        if w.is_time_stopped() {
            return;
        }
        let Some(s) = Surface::of(w, self.platform) else {
            return;
        };
        let p = s.position_from_param(self.local, Fix::ZERO);
        let n = s.normal(p);
        if !self.stopping && (self.timer > self.def.force.last_time() || p.y < body::water_height(w)) {
            self.stopping = true;
            w.emit(Cue::Sound("endEngine"));
            if let Some(a) = self.def.end {
                let c = w.content.clone();
                self.anim
                    .begin_then_do(&c, a, AnimCallback::Ability(CB_TURN_OFF));
            }
            return;
        }
        self.timer += w.fixed_delta_time(ENVIRONMENT_ID, dt);
        let f = self.def.force.eval(self.timer) * (-n) * scale;
        body::add_force_at_position(w, self.platform, f, p, false);
        let pt = w.objects.get(self.platform).map(|o| o.tf).unwrap_or_default();
        if let Some(Kind::Platform(pl)) = w.objects.get_mut(self.platform).map(|o| &mut o.kind)
            && let Some(sp) = pl.spring.as_mut()
            && sp.def.home_is_movable
        {
            sp.set_home(pt.position);
            if sp.spin.is_none() {
                sp.home_rotation = pt.rotation;
            }
        }
    }

    fn anim_update(&mut self, w: &mut World, me: ObjId) {
        match body::advance_anim(w, 0, &mut self.anim) {
            Some(AnimCallback::Ability(CB_START)) => {
                if let Some(a) = self.def.looped {
                    let c = w.content.clone();
                    self.anim.begin(&c, a);
                }
                self.stopping = false;
                self.on = true;
                self.timer = Fix::ZERO;
            }
            Some(AnimCallback::Ability(CB_TURN_OFF)) => {
                self.on = false;
                self.anim = SpriteAnimator::default();
                if let Some(o) = w.objects.get_mut(me) {
                    o.set_part_enabled(PartKind::Behaviour(1), false);
                }
            }
            _ => {}
        }
    }
}

/// A Duplicator clone planted on `platform`: `RocketEngineInit(platform,
/// offset, pos, scale, owner)` (collider initialised now).
pub fn init_duplicate(w: &mut World, e: ObjId, platform: ObjId, offset: Fix, pos: Vec2, scale: Fix, owner: i32) {
    w.with_kind(e, |w, k| {
        if let Kind::Behaviour(b) = k
            && let Behaviour::Engine(Spawned::RocketEngine(r)) = b.as_mut()
        {
            r.platform = platform;
            r.owner = owner;
            r.init(w, e, offset, pos, scale, false);
        }
    });
}

/// A Throw boulder carries a planted engine (throw.md, attach to boulder):
/// `RocketEngineInit(boulder, SurfaceOffset / scale, position + up · 1000,
/// scale, owner, skip init)` — the engine restarts on the boulder.
pub fn reattach(w: &mut World, e: ObjId, boulder: ObjId) {
    let t = body::AbilityBody::tf(w, e);
    w.with_kind(e, |w, k| {
        if let Kind::Behaviour(b) = k
            && let Behaviour::Engine(Spawned::RocketEngine(r)) = b.as_mut()
        {
            r.platform = boulder;
            let off = r.surface_offset / t.scale;
            r.init(w, e, off, t.position + t.up * Fix::from_int(1000), t.scale, true);
        }
    });
}

impl SpawnedImpl for Spawned {
    /// The flame child (object 1) plays the engine's fire animation while
    /// it runs; the smoke (object 2) emits while it burns and time runs;
    /// `loopEngine` plays meanwhile.
    fn look(&self, w: &World, _me: ObjId, l: &mut crate::view::Look) {
        use crate::view::PartLook;
        let Spawned::RocketEngine(e) = self;
        if e.on {
            l.parts.push(PartLook::new(1).frame(e.anim.sprite, e.anim.flip_x));
        } else {
            l.parts.push(PartLook::hidden(1));
        }
        l.parts.push(if !e.on || e.stopping || w.is_time_stopped() {
            PartLook::hidden(2)
        } else {
            PartLook::new(2)
        });
        if e.on && !w.is_time_stopped() {
            l.loop_sound = Some("loopEngine");
        }
    }

    fn interact(&mut self, _w: &mut World, _me: ObjId, i: super::Interaction) -> Option<Fix> {
        match i {
            super::Interaction::ItemKind => Some(Fix::from_int(super::ITEM_ROCKET_ENGINE)),
            _ => None,
        }
    }

    /// Clone rule: tuning kept, everything else reset (the duplicator then
    /// re-initialises the clone on the platform).
    fn duplicate(&self, _w: &World, _me: ObjId) -> Option<Spawned> {
        let Spawned::RocketEngine(e) = self;
        Some(Spawned::RocketEngine(RocketEngine {
            def: e.def.clone(),
            platform: ObjId::NONE,
            local: Fix::ZERO,
            surface_offset: Fix::ZERO,
            prev_scale: Fix::ONE,
            on: false,
            stopping: false,
            timer: Fix::ZERO,
            owner: 0,
            anim: SpriteAnimator::default(),
        }))
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        let Spawned::RocketEngine(e) = self;
        if late {
            return;
        }
        match part {
            PartKind::Behaviour(0) => e.update(w, me, dt),
            PartKind::Behaviour(1) => e.anim_update(w, me),
            _ => {}
        }
    }
}

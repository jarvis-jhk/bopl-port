//! Meteor (`docs/spec/abilities/meteor.md`): `MeteorSmash`, the lethal smear
//! column (`MeteorSmear` + `Explosion`) and the impact `Shockwave`.
//!
//! Components in update order: Ability 1, PlayerPhysics 2, MeteorSmash 3,
//! PlayerCollision 4, DestroyIfOutsideSceneBounds 5, SpriteAnimator 6,
//! DPhysicsBox 7, PlayerBody 8.
//!
//! The impact shockwave is the shared Shockwave component of abilities/mine.md
//! (`explosion::spawn_shockwave`, forces × the meteor scale, the smashed
//! platform ignored).

use std::sync::Arc;

use super::Behaviour;
use super::ability_body::{self as body, AbilityBody};
use super::prefab::{self, root_component};
use super::prelude::*;
use crate::curve::Curve;
use crate::data::{AnimRef, SpriteId};
use crate::physics::shapes::Circle;
use crate::physics::{RayHit, ShapeSnap};
use crate::surface::Surface;
use crate::world::Kind;

const CB_JUMP: u16 = 1;
const CB_VIBRATE: u16 = 2;

/// `(Fix)1.75` (double).
const F_1_75D: Fix = Fix(7_516_192_768);
/// `(Fix)0.8` (double).
const F_0_8D: Fix = Fix(3_435_973_836);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum State {
    #[default]
    Anticipation,
    Jump,
    Vibrate,
    Dash,
}

/// Prefab values of `MeteorSmash`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MeteorDef {
    pub time_before_drop: Fix,
    pub jump_speed: Fix,
    pub hold_speed: Fix,
    pub max_hold_time: Fix,
    pub min_hold_time: Fix,
    pub speed_curve: Arc<Curve>,
    pub vibration_str: Fix,
    pub fixed_updates_in_dash: i32,
    pub ground_force: Fix,
    pub player_push_force: Fix,
    pub raycast_mask: u32,
    pub shake_str: Fix,
    pub shake_duration: Fix,
    pub spin_curve: Arc<Curve>,
    pub smear: Option<usize>,
    pub shockwave: Option<usize>,
    pub smear_lifespan: i32,
    pub anim_g_enter: Option<AnimRef>,
    pub anim_air: Option<AnimRef>,
    pub anim_hold: Option<AnimRef>,
    pub anim_release: Option<AnimRef>,
    pub anim_a_start_hold: Option<AnimRef>,
}

/// Module state (one per ability object).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Meteor {
    pub def: MeteorDef,
    pub body: AbilityBody,
    pub anim: SpriteAnimator,
    pub state: State,
    pub t: Fix,
    pub true_min_hold: Fix,
    pub prev_offset: Vec2,
    pub updates_in_dash: i32,
    pub has_entered: bool,
    pub sprite_shown: bool,
}

/// Spawned objects.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    Smear(Smear),
}

fn anim_data(w: &World, r: Option<&str>) -> Option<u16> {
    let r = r?;
    w.content
        .anim_data
        .iter()
        .position(|a| a.id == r)
        .map(|i| i as u16)
}

impl AbilityImpl for Meteor {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let c = &w.content;
        let m = root_component(c, def, "MeteorSmash");
        let f = |k: &str, d: Fix| prefab::fix_field(m, k, d);
        let data = anim_data(w, m.and_then(|c| c.fields.reference("animData")));
        let a = |n: &str| data.and_then(|d| c.anim_ref(d, n));
        let smear = prefab::prefab_ref(
            c,
            m.and_then(|m| m.fields.reference("SmearFramePrefab")),
            "MeteorSmear_explo",
        );
        let shockwave = prefab::prefab_ref(
            c,
            m.and_then(|m| m.fields.reference("ShockwavePrefab")),
            "meteorShockwave",
        );
        let ms = smear.and_then(|p| prefab::component(c, p, "MeteorSmear"));
        Meteor {
            def: MeteorDef {
                time_before_drop: f("timeBeforeDrop", Fix(2_576_980_480)),
                jump_speed: f("jumpSpeed", Fix::from_int(12)),
                hold_speed: f("holdSpeed", F_0_5),
                max_hold_time: f("maxHoldTime", Fix::TWO),
                min_hold_time: f("minHoldTime", Fix(214_748_368)),
                speed_curve: prefab::curve_field(m, "speedCurve"),
                vibration_str: f("vibrationStr", F_0_1),
                fixed_updates_in_dash: prefab::int_field(m, "FixedUpdatesInDash", 2) as i32,
                ground_force: f("groundForce", Fix::from_int(250)),
                player_push_force: f("playerPushForce", Fix::from_int(50)),
                raycast_mask: prefab::int_field(m, "raycastMask", 67_440_400) as u32,
                shake_str: f("shakeStr", Fix(3_435_973_888)),
                shake_duration: f("shakeDuration", Fix::ONE),
                spin_curve: prefab::curve_field(m, "spinCurve"),
                smear,
                shockwave,
                smear_lifespan: prefab::int_field(ms, "LifespanTicks", 2) as i32,
                anim_g_enter: a("gEnter"),
                anim_air: a("air"),
                anim_hold: a("hold"),
                anim_release: a("release"),
                anim_a_start_hold: a("aStartHold"),
            },
            body: AbilityBody::new(w, def),
            sprite_shown: true,
            ..Meteor::default()
        }
    }

    fn build(&mut self, _w: &mut World, _me: ObjId, fw: &mut Framework, _def: &AbilityDef) {
        self.body.install(fw);
    }

    fn body(&self) -> Option<&AbilityBody> {
        Some(&self.body)
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        Some(&mut self.body)
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        let info = cx.info();
        let me = cx.me;
        cx.fw.kill_players_on_contact = false;
        self.has_entered = true;
        self.sprite_shown = true;
        self.body.sync_physics_to(cx.w, me, &info);
        cx.w.physics.update_physics_positions(&cx.w.objects, me);
        if let Some(o) = cx.w.objects.get_mut(me) {
            o.tf.set_rotation(Fix::ZERO);
        }
        self.t = Fix::ZERO;
        self.state = State::Anticipation;
        self.prev_offset = Vec2::ZERO;
        self.body.body.self_imposed_velocity = Vec2::ZERO;
        self.body.body.external_velocity = Vec2::ZERO;
        if info.is_grounded {
            if let Some(a) = self.def.anim_g_enter {
                let c = cx.w.content.clone();
                self.anim
                    .begin_then_do(&c, a, AnimCallback::Ability(CB_JUMP));
            }
        } else {
            self.jump(cx);
        }
    }

    fn tick(&mut self, cx: &mut Cx<'_>, dt: Fix, late: bool) {
        let me = cx.me;
        if late {
            self.body.physics_late_update(cx.w, me);
            self.body.body_late_update(cx.w, me);
            return;
        }
        cx.ability_update();
        self.body.physics_update(cx.w, me, dt);
        self.smash_update(cx, dt);
        if !cx.running() {
            return;
        }
        AbilityBody::collision_update(cx.fw);
        if let Some(rec) = self.body.bounds_update(cx.w, me) {
            self.exit(cx, rec);
            return;
        }
        if let Some(AnimCallback::Ability(code)) =
            body::advance_anim(cx.w, cx.owner(), &mut self.anim)
        {
            self.on_anim(cx, code);
        }
        if !cx.running() {
            return;
        }
        self.body.body_update(cx.w, me, dt);
    }

    fn on_anim(&mut self, cx: &mut Cx<'_>, code: u16) {
        match code {
            CB_JUMP => self.jump(cx),
            CB_VIBRATE => self.vibrate(cx),
            _ => {}
        }
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        if let Some((killer, cause)) = self.body.kill_on_collision(cx.w, cx.me, cx.fw, c) {
            self.kill(cx, killer, true, false, cause);
        }
    }

    fn exit_component(&mut self, cx: &mut Cx<'_>, rec: AbilityExitInfo) {
        self.exit(cx, rec);
    }

    fn sprite(&self) -> SpriteId {
        if self.sprite_shown {
            self.anim.sprite
        } else {
            SpriteId::NONE
        }
    }

    fn look(&self, _w: &World, _me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        l.frame(&self.anim);
        l.hidden = !self.sprite_shown;
        if self.state == State::Vibrate {
            l.loop_sound = Some("meteor_vibrate");
        }
    }
}

impl Meteor {
    fn begin(&mut self, w: &World, a: Option<AnimRef>, cb: Option<u16>) {
        if let Some(a) = a {
            match cb {
                Some(c) => self
                    .anim
                    .begin_then_do(&w.content, a, AnimCallback::Ability(c)),
                None => self.anim.begin(&w.content, a),
            }
        }
    }

    /// `Jump()` (after `gEnter`, or at once when cast in the air).
    fn jump(&mut self, cx: &mut Cx<'_>) {
        let info = cx.info();
        let me = cx.me;
        if !info.is_grounded {
            self.true_min_hold = self.def.min_hold_time;
            // Vibrate_fromAirStart
            self.t = Fix::ZERO;
            if let (Some(a), Some(h)) = (self.def.anim_a_start_hold, self.def.anim_hold) {
                let c = cx.w.content.clone();
                self.anim.begin_then_play(&c, a, h, None);
            }
            self.state = State::Vibrate;
            return;
        }
        self.true_min_hold = Fix::ZERO;
        cx.w.emit(Cue::Sound("meteor_jump"));
        let g = self.body.is_grounded();
        self.body.unground(cx.w, me, true, false);
        let lift = if g {
            self.body.cfg.extra_jump_teleport_multiplier
        } else {
            F_1_75D
        };
        let p = AbilityBody::pos(cx.w, me) + Vec2::UP * lift;
        AbilityBody::set_pos(cx.w, me, p);
        if self
            .body
            .velocity_based_raycasts(cx.w, me, true, STANDARD_FIXED_DT)
        {
            self.body.body.self_imposed_velocity = Vec2::ZERO;
            self.exit_grounded(cx);
            return;
        }
        let p = AbilityBody::pos(cx.w, me) + Vec2::UP * self.body.cfg.extra_jump_teleport_multiplier;
        AbilityBody::set_pos(cx.w, me, p);
        self.body.body.self_imposed_velocity =
            Vec2::UP * self.def.speed_curve.eval(self.t) * self.def.jump_speed;
        let w = &*cx.w;
        self.begin(w, self.def.anim_air, Some(CB_VIBRATE));
        self.state = State::Jump;
    }

    fn vibrate(&mut self, cx: &mut Cx<'_>) {
        self.t = Fix::ZERO;
        let w = &*cx.w;
        self.begin(w, self.def.anim_hold, None);
        self.state = State::Vibrate;
    }

    /// `MeteorSmash.UpdateSim`.
    fn smash_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        self.t += cx.w.fixed_delta_time(cx.owner(), dt);
        match self.state {
            State::Anticipation => {
                if let Some(rec) = self.body.move_(cx.w, me, Vec2::ZERO, dt, true) {
                    self.exit(cx, rec);
                }
            }
            State::Jump => {
                self.body.body.self_imposed_velocity =
                    Vec2::UP * self.def.speed_curve.eval(self.t) * self.def.jump_speed;
            }
            State::Vibrate => {
                if self.t > self.def.max_hold_time
                    || (self.t > self.true_min_hold && !cx.button_down())
                {
                    self.smash(cx);
                    let w = &*cx.w;
                    self.begin(w, self.def.anim_release, None);
                    return;
                }
                let o = cx.w.rng.random_inside_unit_circle().normalized_safe()
                    * self.def.vibration_str
                    * self.t.max(F_0_5);
                let p = AbilityBody::pos(cx.w, me) - self.prev_offset + o;
                AbilityBody::set_pos(cx.w, me, p);
                self.prev_offset = o;
                self.body.body.self_imposed_velocity = Vec2::UP * self.def.hold_speed;
                self.body.body.external_velocity = self.body.body.external_velocity * F_0_8D;
            }
            State::Dash => {
                self.updates_in_dash += 1;
                if self.updates_in_dash > self.def.fixed_updates_in_dash {
                    let p = AbilityBody::pos(cx.w, me);
                    let y = body::water_height(cx.w) - Fix::from_int(20);
                    AbilityBody::set_pos(cx.w, me, Vec2::new(p.x, y));
                }
                return;
            }
        }
        if self.body.velocity_based_raycasts(cx.w, me, true, dt) {
            self.exit_grounded(cx);
        }
    }

    /// `Dash()`: the smash.
    fn smash(&mut self, cx: &mut Cx<'_>) {
        let me = cx.me;
        let owner = cx.owner();
        cx.fw.kill_players_on_contact = true;
        let f = (self.t / self.def.max_hold_time).min(Fix::ONE);
        self.state = State::Dash;
        self.updates_in_dash = 0;
        self.body.body.self_imposed_velocity = Vec2::ZERO;
        self.body.body.external_velocity = Vec2::ZERO;
        self.sprite_shown = false;
        let r = self.body.physics.radius;
        let start = AbilityBody::pos(cx.w, me);
        let mut hits: Vec<RayHit> = Vec::new();
        let any = cx.w.physics.raycast_all(
            &cx.w.objects,
            start + Vec2::DOWN * r,
            Vec2::DOWN,
            Fix::from_int(1_000_000),
            self.def.raycast_mask,
            &mut hits,
            Some(me),
        );
        let wh = body::water_height(cx.w);
        if !any {
            AbilityBody::set_pos(cx.w, me, Vec2::new(start.x, wh - Fix::from_int(10)));
            let to = Vec2::new(start.x, wh);
            self.spawn_smear(cx, start, to);
            self.body.body.self_imposed_velocity = Vec2::ZERO;
            self.body.body.external_velocity = Vec2::ZERO;
            cx.w.emit(Cue::Sound("dash"));
            self.exit_airborne(cx);
            return;
        }
        let my_hier = body::collider_hier(cx.w, me);
        for h in hits {
            let near = h.near_pos;
            if h.layer == 11 {
                let Some(s) = Surface::of(cx.w, h.obj) else {
                    continue;
                };
                let c = s.closest_point(near);
                // Presentation: the impact smoke (`SmokePrefab`, flipped).
                let fs = (Fix::ONE + f * Fix::TWO / Fix::from_int(5))
                    * cx.w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale).min(Fix::THREE / Fix::TWO);
                super::fx(cx.w, me, "SmokePrefab", c, Fix::PI, fs, owner, Vec2::ZERO);
                AbilityBody::set_pos(cx.w, me, near);
                let local = s.param_from_position(c, r);
                let k = (f + Fix::ONE) * F_0_5;
                body::add_force_at_position(
                    cx.w,
                    h.obj,
                    Vec2::DOWN * self.def.ground_force * k,
                    near,
                    false,
                );
                if self.t > self.def.time_before_drop {
                    crate::platform::drop_all_attached_players(
                        cx.w,
                        h.obj,
                        owner,
                        self.def.player_push_force * self.t,
                    );
                    crate::platform::add_shake(
                        cx.w,
                        h.obj,
                        self.def.shake_duration,
                        self.def.shake_str,
                        100,
                        Some(self.def.spin_curve.clone()),
                    );
                } else {
                    crate::platform::add_shake(
                        cx.w,
                        h.obj,
                        self.def.shake_duration,
                        self.def.shake_str / Fix::THREE,
                        0,
                        Some(self.def.spin_curve.clone()),
                    );
                }
                if let Some(p) = self.def.shockwave {
                    super::explosion::spawn_shockwave(cx.w, p, near, owner, Fix::ONE, k, &[h.obj]);
                }
                self.spawn_smear(cx, start, near);
                if let Some(s) = Surface::of(cx.w, h.obj) {
                    let p = s.position_from_param(local, r);
                    AbilityBody::set_pos(cx.w, me, p);
                }
                let p = AbilityBody::pos(cx.w, me);
                self.body.attach_to_ground(cx.w, me, h.obj, p);
                cx.w.emit(Cue::Sound("meteor_dash"));
                self.exit_grounded(cx);
                return;
            }
            if h.layer == 4 {
                AbilityBody::set_pos(cx.w, me, Vec2::new(start.x, wh - Fix::from_int(20)));
                self.spawn_smear(cx, start, near);
                cx.w.emit(Cue::Sound("dash"));
                self.exit_airborne(cx);
                return;
            }
            AbilityBody::set_pos(cx.w, me, near);
            let mut boxs = cx.w.physics.box_shape(me).copied();
            if let Some(b) = boxs.as_mut() {
                b.center = near;
            }
            if let Some(b) = boxs {
                body::touch(
                    cx.w,
                    Contact {
                        me: h.obj,
                        other: me,
                        other_hier: my_hier,
                        my_hier: h.hierarchy,
                        collider: ShapeSnap::Box(b),
                        layer: 8,
                        impact_velocity: Vec2::DOWN * Fix::from_int(50),
                        contact_point: near,
                        normal: Vec2::UP,
                        penetration: F_0_1,
                    },
                );
            }
            let c = Contact {
                me,
                other: h.obj,
                other_hier: h.hierarchy,
                my_hier,
                collider: ShapeSnap::Circle(Circle {
                    center: near,
                    radius: Fix::ONE,
                    layer: h.layer,
                }),
                layer: h.layer,
                impact_velocity: Vec2::ZERO,
                contact_point: near,
                normal: Vec2::UP,
                penetration: F_0_1,
            };
            self.on_contact(cx, &c);
            if !self.has_entered {
                cx.w.emit(Cue::Sound("dash"));
                self.spawn_smear(cx, start, near);
                return;
            }
        }
    }

    /// `SpawnSmearEffect(from, to)`.
    fn spawn_smear(&mut self, cx: &mut Cx<'_>, from: Vec2, to: Vec2) {
        let Some(p) = self.def.smear else {
            return;
        };
        let owner = cx.owner();
        let scale = cx.w.objects.get(cx.me).map_or(Fix::ONE, |o| o.tf.scale);
        let id = body::spawn_prefab(
            cx.w,
            p,
            Kind::Behaviour(Box::new(Behaviour::Meteor(Spawned::Smear(Smear {
                ticks: 0,
                lifespan: self.def.smear_lifespan,
                owner,
            })))),
            &[
                ("MeteorSmear", PartKind::Behaviour(0)),
                (body::COLLIDER, PartKind::Collider),
                ("Explosion", PartKind::Behaviour(1)),
            ],
            to,
            None,
        );
        let hier = body::collider_hier(cx.w, id);
        cx.w.physics.register(&mut cx.w.objects, id, hier);
        // Both the player's and the meteor's PlayerCollision ignore it.
        let ctrl = cx.fw.controller;
        cx.w.with_kind(ctrl, |_, k| {
            if let Kind::Bopl(b) = k {
                b.collision.ignored[0] = Some(id);
            }
        });
        if let Some(c) = cx.fw.collision.as_mut() {
            c.ignored[0] = Some(id);
        }
        let Some(b) = cx.w.physics.box_shape(id).copied() else {
            return;
        };
        let mut ext = Vec2::new(Fix::ONE / b.ie.x, Fix::ONE / b.ie.y);
        let y = ext.y;
        ext.y = (to - from).magnitude() * F_0_5;
        ext.x = ext.x * scale;
        if let Some(bm) = cx.w.physics.box_mut(id) {
            bm.update_extents(ext);
        }
        if let Some(o) = cx.w.objects.get_mut(id) {
            o.tf.offset.y = o.tf.offset.y * (ext.y / y);
        }
    }

    fn exit(&mut self, cx: &mut Cx<'_>, rec: AbilityExitInfo) {
        cx.fw.kill_players_on_contact = false;
        self.has_entered = false;
        cx.exit_body(&mut self.body, rec);
    }

    fn exit_grounded(&mut self, cx: &mut Cx<'_>) {
        let me = cx.me;
        let rec = AbilityExitInfo {
            grounded_speed: Fix::ZERO,
            facing_right: true,
            self_imposed_velocity: self.body.body.self_imposed_velocity,
            external_velocity: self.body.body.external_velocity,
            is_grounded: true,
            just_landed: true,
            ground: self.body.physics.attached_ground,
            position: AbilityBody::pos(cx.w, me),
            radius: self.body.physics.radius,
            grounded_local_position: self.body.physics.local_platform_pos,
            ..AbilityExitInfo::default()
        };
        self.exit(cx, rec);
    }

    fn exit_airborne(&mut self, cx: &mut Cx<'_>) {
        let me = cx.me;
        cx.fw.kill_players_on_contact = false;
        let rec = AbilityExitInfo {
            grounded_speed: Fix::ZERO,
            facing_right: true,
            self_imposed_velocity: self.body.body.self_imposed_velocity,
            external_velocity: self.body.body.external_velocity,
            is_grounded: false,
            just_landed: false,
            ground: None,
            position: AbilityBody::pos(cx.w, me),
            radius: self.body.physics.radius,
            ..AbilityExitInfo::default()
        };
        self.exit(cx, rec);
    }
}

/// The smear column (`MeteorSmear` + `Explosion`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Smear {
    pub ticks: i32,
    pub lifespan: i32,
    /// `Explosion.PlayerOwnerId`.
    pub owner: i32,
}

impl SpawnedImpl for Spawned {
    /// The smear: the owner's colours, stretched along the dash (y scale =
    /// length / the box's authored half height; `params[1]` = the smear
    /// shader's `_Scale.y` = y scale · `smearScaling` 0.5).
    fn look(&self, w: &World, me: ObjId, l: &mut crate::view::Look) {
        let Spawned::Smear(s) = self;
        l.player = s.owner;
        let (Some(b), Some(c)) = (
            w.physics.box_shape(me),
            w.objects.get(me).and_then(|o| o.collider),
        ) else {
            return;
        };
        let half = Fix::ONE / b.ie.y.max(Fix(1));
        let y2 = half * Fix::TWO / c.start_extents.y.max(Fix(1));
        let scale = w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale).max(Fix(1));
        l.stretch = Vec2::new(Fix::ONE, y2 / scale);
        l.params[1] = y2 / Fix::TWO;
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, _dt: Fix, late: bool) {
        if late {
            return;
        }
        match self {
            Spawned::Smear(s) if part == PartKind::Behaviour(0) => {
                s.ticks += 1;
                if s.ticks >= s.lifespan {
                    w.destroy(me);
                }
            }
            _ => {}
        }
    }

    fn lethal(&self, _w: &World, _me: ObjId) -> Option<Lethal> {
        match self {
            Spawned::Smear(s) => Some(Lethal {
                owner: s.owner,
                can_hurt_owner: true,
                beam: false,
            }),
        }
    }
}

//! Beam (`docs/spec/abilities/beam.md`): the Beam ability (staff on an
//! [`AbilityBody`], the lethal orb at the tip) and the DetPhysics beam
//! system ([`update_beams`]: ray casts, water reflection, crossing and
//! merging, the pooled lethal `BeamObject` boxes and the impact pushes).

use std::sync::Arc;

use super::blink_gun::part;
use super::ability_body::{AbilityBody, BodyCfg};
use super::prefab::{ability_prefab, anim_data, anim_named, fields, hier_of, prefab_by_ref, updatables};
use super::blink_gun::layer;
use super::magnet_gun::has_bopl_body;
use super::prelude::*;
use super::{Behaviour, Framework};
use crate::curve::Curve;
use crate::data::{AnimRef, Content, Prefab};
use crate::physics::ColliderDef;
use crate::physics::shapes::ShapeKind;
use crate::surface::Surface;
use crate::view::Visual;
use crate::world::Kind;

// ---------------------------------------------------------------------------
// Code literals
// ---------------------------------------------------------------------------

/// `(Fix)2.8` (double): staff tip distance.
const F_2_8: Fix = Fix(12_025_908_428);
/// `(Fix)0.5` (double).
const F_HALF: Fix = Fix(2_147_483_648);
/// `(Fix)0.997` (double): "straight down" threshold.
const F_0_997: Fix = Fix(4_282_082_394);
/// `(Fix)0.001` (double): zero-y replacement of the water crossing.
const F_0_001_D: Fix = Fix(4_294_967);
/// `-(Fix)0.5`: head-on crossing threshold.
const F_NEG_HALF: Fix = Fix(-2_147_483_648);
// Orb positions during the enter animation (double literals).
const F_0_1: Fix = Fix(429_496_729);
const F_2_3: Fix = Fix(9_878_424_780);
const F_3_1: Fix = Fix(13_314_398_617);
const F_0_9: Fix = Fix(3_865_470_566);
const F_0_6: Fix = Fix(2_576_980_377);
const F_1_28: Fix = Fix(5_497_558_138);
const F_0_4: Fix = Fix(1_717_986_918);
const F_0_05: Fix = Fix(214_748_364);
const F_0_26: Fix = Fix(1_116_691_496);

/// Animation callbacks of the beam body.
const CB_LEFT_FIRST_FRAME: u16 = 1;
const CB_START_BEAM: u16 = 2;
const CB_EXIT: u16 = 3;
/// Orb animator callbacks.
const ORB_CONTINUE: u16 = 1;
const ORB_DESTROY: u16 = 2;

// ---------------------------------------------------------------------------
// The ability
// ---------------------------------------------------------------------------

/// Prefab values of the Beam (beam.md, constants).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BeamCfg {
    pub body: Arc<BodyCfg>,
    pub hier: Option<i32>,
    pub orig_offset: Vec2,
    pub orig_extents: Vec2,
    pub beam_offset: Fix,
    pub max_time: Fix,
    pub max_time_air: Fix,
    pub exit_scale_time: Fix,
    pub exit_scale_anim: Arc<Curve>,
    pub angular_aim_speed: Fix,
    pub angular_aim_speed_anim: Arc<Curve>,
    pub min_grounded_angle: Fix,
    pub up_alignment: Fix,
    pub on_exit_up_speed: Fix,
    pub hurtbox_extents_air: Vec2,
    pub hurtbox_offset_air: Vec2,
    pub shake_duration: Fix,
    pub shake_str: Fix,
    pub anim_enter_frame1: Option<AnimRef>,
    pub anim_enter: Option<AnimRef>,
    pub anim_enter_up: Option<AnimRef>,
    pub anim_enter_air: Option<AnimRef>,
    pub anim_loop: Option<AnimRef>,
    pub anim_loop_up: Option<AnimRef>,
    pub anim_loop_air: Option<AnimRef>,
    pub anim_exit: Option<AnimRef>,
    pub anim_exit_up: Option<AnimRef>,
    pub anim_exit_air: Option<AnimRef>,
    pub orb: OrbCfg,
}

/// The `orbAnim` prefab and its animations.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OrbCfg {
    pub prefab: Option<usize>,
    pub collider: Option<ColliderDef>,
    pub layer: u8,
    pub hier_animator: i32,
    pub hier_collider: i32,
    pub hier_holder: i32,
    pub updatables: i32,
    pub can_hurt_owner: bool,
    pub white_enter: Option<AnimRef>,
    pub white_enter_air: Option<AnimRef>,
    pub white: Option<AnimRef>,
    pub white_exit: Option<AnimRef>,
}

impl BeamCfg {
    fn from_prefab(c: &Content, p: &Prefab) -> BeamCfg {
        let f = fields(p, "Beam");
        let data = anim_data(c, f.reference("animData"));
        let orb_data = anim_data(c, f.reference("orbAnimData"));
        let orb_air = anim_data(c, f.reference("orbAnimDataAir"));
        let a = |n: &str| anim_named(c, data, n);
        let root = p.objects.first();
        let orb_prefab = prefab_by_ref(c, f.reference("OrbEffectPrefab"));
        let op = orb_prefab.and_then(|i| c.prefabs.get(i));
        let orb_obj = op.and_then(|p| p.objects.first());
        let ef = op.map_or(&Value::Null, |p| fields(p, "Explosion"));
        BeamCfg {
            body: Arc::new(BodyCfg::from_root(c, p)),
            hier: hier_of(p, "Beam"),
            orig_offset: root.and_then(|o| o.fix).map_or(Vec2::ZERO, |t| t.offset),
            orig_extents: root
                .and_then(|o| o.collider)
                .map_or(Vec2::ZERO, |c| c.start_extents),
            beam_offset: f.fix_or("beamOffset", Fix::ONE),
            max_time: f.fix_or("maxTime", Fix::from_int(5)),
            max_time_air: f.fix_or("maxTimeAir", Fix::from_int(5)),
            exit_scale_time: f.fix_or("exitScaleTime", Fix(2_576_980_480)),
            exit_scale_anim: f.curve("exitScaleAnim").unwrap_or_default(),
            angular_aim_speed: f.fix_or("angularAimSpeed", Fix(10_737_418)),
            angular_aim_speed_anim: f.curve("angularAimSpeedAnim01").unwrap_or_default(),
            min_grounded_angle: f.fix_or("minGroundedAngle", Fix(3_006_477_056)),
            up_alignment: f.fix_or("upAlignmentUntilUpAnimationIsUsed", Fix(2_576_980_480)),
            on_exit_up_speed: f.fix_or("onExitUpSpeed", Fix::from_int(10)),
            hurtbox_extents_air: f.vec2("hurtBoxExtentsAir").unwrap_or_default(),
            hurtbox_offset_air: f.vec2("hurtBoxOffsetAir").unwrap_or_default(),
            shake_duration: f.fix_or("shakeDuration", Fix::ZERO),
            shake_str: f.fix_or("shakeStr", Fix::ZERO),
            anim_enter_frame1: a("enterFrame1"),
            anim_enter: a("enter"),
            anim_enter_up: a("enterUp"),
            anim_enter_air: a("enterAir"),
            anim_loop: a("loop"),
            anim_loop_up: a("loopUp"),
            anim_loop_air: a("loopAir"),
            anim_exit: a("exit"),
            anim_exit_up: a("exitUp"),
            anim_exit_air: a("exitAir"),
            orb: OrbCfg {
                prefab: orb_prefab,
                collider: orb_obj.and_then(|o| o.collider),
                layer: orb_obj.map_or(18, |o| o.layer),
                hier_animator: op.and_then(|p| hier_of(p, "SpriteAnimator")).unwrap_or(1),
                hier_collider: orb_obj.and_then(|o| o.collider_hier).unwrap_or(2),
                hier_holder: op
                    .and_then(|p| hier_of(p, "DisableIfNotHeld_deter"))
                    .unwrap_or(3),
                updatables: updatables(c, orb_prefab).max(3),
                can_hurt_owner: ef.boolean("canHurtOwner").unwrap_or(false),
                white_enter: anim_named(c, orb_data, "whiteEnter"),
                white_enter_air: anim_named(c, orb_air, "whiteEnter"),
                white: anim_named(c, orb_data, "white"),
                white_exit: anim_named(c, orb_data, "whiteExit"),
            },
        }
    }
}

/// `BeamState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BeamState {
    #[default]
    Enter,
    Loop,
    Exit,
}

/// The Beam ability state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Beam {
    pub cfg: Arc<BeamCfg>,
    pub b: AbilityBody,
    pub anim: SpriteAnimator,
    pub state: BeamState,
    pub staff_dir: Vec2,
    pub relative_staff_dir: Vec2,
    pub in_air: bool,
    pub beam_aimed_up: bool,
    pub on_first_frame: bool,
    pub time_since_enter: Fix,
    pub time_since_beam_start: Fix,
    pub beam_index: i32,
    /// Unity `localScale.x < 0` (the staff points left).
    pub flipped: bool,
    pub current_ground: Option<ObjId>,
    /// The `orbAnim` object (created on a use when none is alive).
    pub orb: Option<ObjId>,
}

/// Objects this module spawns: the orb and the pooled beam boxes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    Orb(Orb),
    BeamObject(BeamObject),
}

impl Beam {
    fn info_facing(&self, cx: &Cx<'_>) -> bool {
        cx.fw.info.is_some_and(|i| i.facing_right)
    }

    fn right(w: &World, me: ObjId) -> Vec2 {
        AbilityBody::tf(w, me).right
    }

    /// `FirePos()`.
    fn fire_pos(&self, w: &World, me: ObjId) -> Vec2 {
        let t = AbilityBody::tf(w, me);
        t.position + self.staff_dir * F_2_8 * t.scale
    }

    /// `SetFacingDirection(a)`.
    fn set_facing(&mut self, cx: &Cx<'_>, a: Vec2) {
        let r = Self::right(cx.w, cx.me);
        let a = if a == Vec2::ZERO {
            if self.info_facing(cx) { r } else { -r }
        } else {
            a
        };
        self.flipped = a.dot(r) <= Fix::ZERO;
    }

    /// `InitStartDir(a)`.
    fn init_start_dir(&mut self, cx: &Cx<'_>, a: Vec2) {
        let r = Self::right(cx.w, cx.me);
        let a = if a == Vec2::ZERO {
            if self.info_facing(cx) { r } else { -r }
        } else {
            a
        };
        self.set_start_dir(cx, a);
    }

    /// `SetStartDir(a)`.
    fn set_start_dir(&mut self, cx: &Cx<'_>, a: Vec2) {
        let r = Self::right(cx.w, cx.me);
        let conj = Vec2::new(r.x, -r.y);
        let m = self.cfg.min_grounded_angle;
        let th = a.complex_mul(conj).normalized_vector_angle();
        let s = if th < Fix::PI + m || th > Fix::PI_TIMES_2 - m {
            Vec2::from_angle(th)
        } else if !self.flipped {
            Vec2::from_angle(Fix::PI_TIMES_2 - m)
        } else {
            Vec2::from_angle(Fix::PI + m)
        };
        self.staff_dir = s.complex_mul(r);
        self.relative_staff_dir = self.staff_dir.complex_mul(conj);
        self.beam_aimed_up = self.relative_staff_dir.dot(Vec2::UP) > self.cfg.up_alignment;
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

    fn component_hier(w: &World, me: ObjId) -> i32 {
        w.objects
            .get(me)
            .and_then(|o| o.part(PartKind::Behaviour(part::COMPONENT)))
            .map_or(0, |p| p.hier)
    }

    fn set_hurtbox(w: &mut World, me: ObjId, offset: Vec2, extents: Vec2) {
        if let Some(o) = w.objects.get_mut(me) {
            o.tf.offset = offset;
        }
        set_box_extents(w, me, extents);
    }

    /// `OnEnterAbility()`.
    fn enter(&mut self, cx: &mut Cx<'_>) {
        let info = cx.info();
        let me = cx.me;
        let c = self.cfg.clone();
        let aim = cx
            .w
            .player(info.player_id)
            .map_or(Vec2::ZERO, |p| p.inputs.aim_vector());
        let sel = info.self_imposed_velocity;
        self.b.sync_physics_to(cx.w, me, &info);
        cx.w.physics.update_physics_positions(&cx.w.objects, me);
        self.beam_index = -1;
        self.time_since_beam_start = Fix::ZERO;
        self.time_since_enter = Fix::ZERO;
        self.on_first_frame = true;
        self.set_facing(cx, aim);
        let h = Self::component_hier(cx.w, me);
        if let Some(col) = cx.fw.collision.as_mut() {
            col.ignored_beam_id = h;
        }
        self.current_ground = None;
        self.state = BeamState::Enter;
        if info.is_grounded && info.ground.is_some_and(|g| cx.w.objects.alive(g)) {
            self.b.body.self_imposed_velocity = sel;
            Self::set_hurtbox(cx.w, me, c.orig_offset, c.orig_extents);
            self.in_air = false;
            self.current_ground = self.b.physics.attached_ground;
            self.init_start_dir(cx, aim);
            self.begin_then_do(cx.w, c.anim_enter_frame1, CB_LEFT_FIRST_FRAME);
        } else {
            self.b.body.self_imposed_velocity = Vec2::ZERO;
            Self::set_hurtbox(cx.w, me, c.hurtbox_offset_air, c.hurtbox_extents_air);
            self.in_air = true;
            self.begin_then_do(cx.w, c.anim_enter_air, CB_START_BEAM);
            if aim == Vec2::ZERO {
                self.staff_dir = Vec2::DOWN;
                self.relative_staff_dir = Vec2::DOWN;
            } else {
                self.staff_dir = aim;
                self.relative_staff_dir = aim;
            }
            self.beam_aimed_up = false;
        }
        cx.w.emit(Cue::SoundPrefix("beamSparks"));
        cx.w.emit(Cue::PlayerSound("beamCharge", info.player_id));
        // The orb at the staff tip.
        let fp = self.fire_pos(cx.w, me);
        let orb = match self.orb.filter(|o| cx.w.objects.exists(*o)) {
            Some(o) => o,
            None => spawn_orb(cx.w, &c.orb, fp),
        };
        self.orb = Some(orb);
        set_collider_enabled(cx.w, orb, false);
        orb_hold(cx.w, orb, me);
        let (owner, in_air) = (info.player_id, self.in_air);
        let oc = c.orb.clone();
        with_orb(cx.w, orb, |w, o| {
            o.owner = owner;
            let a = if in_air {
                oc.white_enter_air
            } else {
                oc.white_enter
            };
            if let Some(a) = a {
                let ct = w.content.clone();
                o.anim
                    .begin_then_do(&ct, a, AnimCallback::Ability(ORB_CONTINUE));
            }
        });
        if let Some(o) = cx.w.objects.get_mut(orb) {
            o.active = true;
        }
    }

    /// `StartBeam()`.
    fn start_beam(&mut self, cx: &mut Cx<'_>) {
        let me = cx.me;
        let c = self.cfg.clone();
        cx.w.emit(Cue::Sound("beamFire"));
        if let Some(o) = self.orb {
            set_collider_enabled(cx.w, o, true);
        }
        if let Some(o) = cx.w.objects.get_mut(me) {
            o.tf.offset = c.hurtbox_offset_air;
        }
        self.time_since_beam_start = Fix::ZERO;
        self.state = BeamState::Loop;
        let r = Self::right(cx.w, me);
        if !self.in_air {
            self.flipped = self.staff_dir.dot(r) <= Fix::ZERO;
        }
        let sign = if self.info_facing(cx) {
            Fix::ONE
        } else {
            -Fix::ONE
        };
        if self.in_air {
            self.begin(cx.w, c.anim_loop_air);
        } else if self.beam_aimed_up {
            self.staff_dir = r * sign;
            self.begin(cx.w, c.anim_loop_up);
        } else {
            self.staff_dir = r * sign;
            self.begin(cx.w, c.anim_loop);
        }
    }

    /// `OnLeftFirstFrame()` (inside the animator's update).
    fn left_first_frame(&mut self, cx: &mut Cx<'_>) {
        let c = self.cfg.clone();
        let a = if self.beam_aimed_up {
            c.anim_enter_up
        } else {
            c.anim_enter
        };
        self.begin_then_do(cx.w, a, CB_START_BEAM);
        let aim = cx
            .w
            .player(cx.fw.owner)
            .map_or(Vec2::ZERO, |p| p.inputs.aim_vector());
        self.set_facing(cx, aim);
        self.update(cx, Fix::ZERO);
        self.on_first_frame = false;
    }

    /// `StopOrbAnim()`.
    fn stop_orb(&mut self, w: &mut World) {
        let Some(o) = self.orb else {
            return;
        };
        let alive = w
            .objects
            .get(o)
            .and_then(|ob| ob.part(PartKind::Collider))
            .is_some_and(|p| !p.destroyed);
        if !alive {
            return;
        }
        set_collider_enabled(w, o, false);
        let a = self.cfg.orb.white_exit;
        with_orb(w, o, |w, orb| {
            if let Some(a) = a {
                let ct = w.content.clone();
                orb.anim
                    .begin_then_do(&ct, a, AnimCallback::Ability(ORB_DESTROY));
            }
        });
    }

    /// `Stop()`.
    fn stop(&mut self, cx: &mut Cx<'_>) {
        self.stop_orb(cx.w);
        self.state = BeamState::Exit;
        let c = self.cfg.clone();
        let a = if self.in_air {
            c.anim_exit_air
        } else if self.beam_aimed_up {
            c.anim_exit_up
        } else {
            c.anim_exit
        };
        self.begin_then_do(cx.w, a, CB_EXIT);
    }

    /// `Beam.ExitAbility(info)`.
    fn exit(&mut self, cx: &mut Cx<'_>, mut r: AbilityExitInfo) {
        let me = cx.me;
        self.b.body.self_imposed_velocity += Vec2::UP * self.cfg.on_exit_up_speed;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        r.grounded_speed = self.b.physics.grounded_speed;
        r.is_grounded = self.b.physics.is_grounded;
        r.position = AbilityBody::pos(cx.w, me);
        r.facing_right = self.info_facing(cx);
        r.self_imposed_velocity = self.b.body.self_imposed_velocity;
        r.external_velocity = self.b.body.external_velocity;
        r.ground = self.b.physics.attached_ground;
        if r.is_grounded {
            r.grounded_local_position = self.b.physics.local_platform_pos;
        }
        r.radius = self.b.physics.radius;
        cx.exit_body(&mut self.b, r);
    }

    /// The PlayerPhysics ground events (`OnAttachedToGround`,
    /// `OnNoLongerGrounded`) raised by the last physics call.
    fn ground_events(&mut self, cx: &mut Cx<'_>) {
        if std::mem::take(&mut self.b.ungrounded_event) && !self.in_air {
            self.retarget(cx);
        }
        if std::mem::take(&mut self.b.attached_event) {
            if self.in_air {
                self.exit(
                    cx,
                    AbilityExitInfo {
                        just_landed: true,
                        ..AbilityExitInfo::default()
                    },
                );
            } else {
                self.retarget(cx);
            }
        }
    }

    fn retarget(&mut self, cx: &Cx<'_>) {
        let r = Self::right(cx.w, cx.me);
        self.relative_staff_dir = self.staff_dir.complex_mul(Vec2::new(r.x, -r.y));
        let s = self.staff_dir;
        self.set_facing(cx, s);
    }

    /// `MovePlayer(dt)`.
    fn move_player(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let owner = cx.fw.owner;
        let input = if !self.in_air && self.state == BeamState::Exit {
            cx.w.player(owner)
                .map_or(Vec2::ZERO, |p| p.inputs.input_vector())
        } else {
            Vec2::ZERO
        };
        if !self.b.physics.is_grounded && self.in_air {
            self.b.aireal_movement(Fix::ZERO);
            let mut landed = self.b.velocity_based_raycasts(cx.w, me, true, dt);
            self.ground_events(cx);
            let pos = AbilityBody::pos(cx.w, me);
            let hits = cx
                .w
                .physics
                .circle_cast_rrs(&cx.w.objects, pos, self.b.physics.radius, 1);
            if let Some(&plat) = hits.first() {
                landed = true;
                if Surface::of(cx.w, plat).is_some() {
                    self.b.attach_to_ground(cx.w, me, plat, pos);
                }
            }
            if landed {
                AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
                let r = AbilityExitInfo {
                    just_landed: true,
                    grounded_speed: self.b.physics.grounded_speed,
                    is_grounded: self.b.physics.is_grounded,
                    position: AbilityBody::pos(cx.w, me),
                    external_velocity: self.b.body.external_velocity,
                    self_imposed_velocity: self.b.body.self_imposed_velocity,
                    ground: self.b.physics.attached_ground,
                    grounded_local_position: self.b.physics.local_platform_pos,
                    radius: self.b.physics.radius,
                    ..AbilityExitInfo::default()
                };
                self.exit(cx, r);
            }
        } else if !self.in_air && !self.b.physics.is_grounded {
            self.b.add_gravity_factor(cx.w);
            let drop = self.b.move_(cx.w, me, input, dt, true);
            self.after_move(cx, drop);
        } else {
            let drop = self.b.move_(cx.w, me, input, dt, true);
            self.after_move(cx, drop);
        }
    }

    fn after_move(&mut self, cx: &mut Cx<'_>, drop: Option<AbilityExitInfo>) {
        // Events raised inside Move come before a drop-through exit.
        self.ground_events(cx);
        if let Some(r) = drop {
            self.exit(cx, r);
        }
    }

    /// `UpdateBeam(dt)` (EmitBeam).
    fn emit_beam(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let c = self.cfg.clone();
        let fp = self.fire_pos(cx.w, me);
        let max = if self.in_air {
            c.max_time_air
        } else {
            c.max_time
        };
        let mut k = Fix::ONE;
        if self.time_since_beam_start + c.exit_scale_time > max && self.state == BeamState::Loop {
            let t = (c.exit_scale_time - (max - self.time_since_beam_start)) / c.exit_scale_time;
            k = c.exit_scale_anim.eval(t);
        }
        if fp.y >= CONST_WATER_HEIGHT || cx.w.is_space() {
            let scale = AbilityBody::tf(cx.w, me).scale;
            let body = BeamBody {
                id: Self::component_hier(cx.w, me),
                owner: cx.fw.owner,
                position: fp + self.staff_dir * c.beam_offset,
                direction: self.staff_dir,
                time_passed: self.time_since_beam_start,
                scale: scale * k,
                distance: Fix::ZERO,
                ground: self.current_ground,
            };
            self.beam_index = add_beam_body(cx.w, body) as i32;
        } else {
            self.beam_index = -1;
        }
        self.time_since_beam_start += dt;
    }

    /// `Beam.UpdateSim(dt)`.
    fn update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let owner = cx.fw.owner;
        let c = self.cfg.clone();
        if let Some(o) = self.orb.filter(|o| cx.w.objects.alive(*o)) {
            orb_hold(cx.w, o, me);
        }
        let protected = cx
            .w
            .player(owner)
            .is_some_and(|p| p.is_protected_from_time_stop);
        if cx.w.is_time_stopped() && !protected {
            return;
        }
        let inc = c.angular_aim_speed * c.angular_aim_speed_anim.eval(self.time_since_enter);
        self.time_since_enter += dt;
        let aim = cx
            .w
            .player(owner)
            .map_or(Vec2::ZERO, |p| p.inputs.aim_vector());
        if aim != Vec2::ZERO {
            if self.in_air {
                let v = self.staff_dir.approach_normalized(aim, inc);
                self.relative_staff_dir = v;
                self.staff_dir = v;
            } else {
                if self.on_first_frame {
                    self.init_start_dir(cx, aim);
                }
                let r = Self::right(cx.w, me);
                let target = aim.complex_mul(Vec2::new(r.x, -r.y));
                let mut v = self.relative_staff_dir.approach_normalized(target, inc);
                let th = v.normalized_vector_angle();
                let m = c.min_grounded_angle;
                if !(th < Fix::PI + m) && !(th > Fix::PI_TIMES_2 - m) {
                    v = self.relative_staff_dir;
                }
                self.relative_staff_dir = v;
            }
        }
        if !self.in_air {
            let r = Self::right(cx.w, me);
            self.staff_dir = r.complex_mul(self.relative_staff_dir);
        } else {
            let s = self.staff_dir;
            let up = if s.y > Fix::ONE || s.y < -Fix::ONE {
                Vec2::UP
            } else {
                -s
            };
            AbilityBody::set_up(cx.w, me, up);
        }
        if self.state == BeamState::Enter
            && !self.in_air
            && let Some(g) = self.current_ground
        {
            crate::platform::add_shake(cx.w, g, c.shake_duration, c.shake_str, 1, None);
        }
        if matches!(self.state, BeamState::Enter | BeamState::Loop) && !cx.button_down() {
            self.stop(cx);
        }
        self.move_player(cx, dt);
        let jump = cx
            .w
            .player(owner)
            .is_some_and(|p| p.inputs.jump_down_this_frame);
        match self.state {
            BeamState::Loop => {
                self.emit_beam(cx, dt);
                if jump {
                    self.stop_orb(cx.w);
                    self.exit(
                        cx,
                        AbilityExitInfo {
                            enter_jump: true,
                            ..AbilityExitInfo::default()
                        },
                    );
                    return;
                }
            }
            BeamState::Exit => {
                if jump {
                    self.stop_orb(cx.w);
                    self.exit(
                        cx,
                        AbilityExitInfo {
                            enter_jump: true,
                            ..AbilityExitInfo::default()
                        },
                    );
                    return;
                }
            }
            BeamState::Enter => {}
        }
        let max = if self.in_air {
            c.max_time_air
        } else {
            c.max_time
        };
        if self.time_since_beam_start + c.exit_scale_time > max
            && self.state == BeamState::Loop
            && self.time_since_beam_start > max
        {
            self.stop(cx);
        }
    }

    /// `LateUpdateSim`: places the orb (its circle is lethal in the loop).
    fn late_update(&mut self, cx: &mut Cx<'_>) {
        let me = cx.me;
        let Some(orb) = self.orb.filter(|o| cx.w.objects.alive(*o)) else {
            return;
        };
        let t = AbilityBody::tf(cx.w, me);
        let passed = self.anim.passed_sprites();
        let sx = if self.flipped { -Fix::ONE } else { Fix::ONE };
        let local = |v: Vec2| t.position + t.right.complex_mul(v) * t.scale;
        let fp = self.fire_pos(cx.w, me);
        let p = if self.in_air {
            match self.state {
                BeamState::Loop => Some(fp),
                BeamState::Enter if passed >= 10 => {
                    Some(t.position + self.staff_dir * F_3_1 * t.scale)
                }
                BeamState::Enter if passed >= 9 => {
                    Some(local(Vec2::new(F_0_1 * sx, -Fix::TWO)))
                }
                BeamState::Enter if passed >= 8 => Some(local(Vec2::new(F_0_1 * sx, -F_2_3))),
                BeamState::Enter => Some(fp),
                BeamState::Exit => None,
            }
        } else if self.beam_aimed_up {
            match self.state {
                BeamState::Loop => Some(fp),
                BeamState::Enter if passed >= 8 => Some(local(Vec2::new(F_0_9 * sx, F_0_6))),
                BeamState::Enter if passed >= 7 => Some(local(Vec2::new(F_1_28 * sx, F_0_4))),
                BeamState::Enter => Some(local(Vec2::new(Fix::TWO * sx, -F_0_05))),
                BeamState::Exit => None,
            }
        } else {
            match self.state {
                BeamState::Loop => Some(fp),
                BeamState::Enter if passed >= 7 => Some(local(Vec2::new(F_1_28 * sx, F_0_26))),
                BeamState::Enter => Some(local(Vec2::new(Fix::TWO * sx, -F_0_05))),
                BeamState::Exit => None,
            }
        };
        if let Some(p) = p
            && let Some(o) = cx.w.objects.get_mut(orb)
        {
            o.tf.position = p;
        }
    }
}

impl AbilityImpl for Beam {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let p = ability_prefab(w, def).cloned().unwrap_or_default();
        let cfg = Arc::new(BeamCfg::from_prefab(&w.content, &p));
        Beam {
            b: AbilityBody::from_cfg(cfg.body.clone()),
            anim: super::ability_body::animator(cfg.body.always_stop_during_time_stop),
            state: BeamState::Enter,
            staff_dir: Vec2::RIGHT,
            relative_staff_dir: Vec2::RIGHT,
            in_air: false,
            beam_aimed_up: false,
            on_first_frame: false,
            time_since_enter: Fix::ZERO,
            time_since_beam_start: Fix::ZERO,
            beam_index: -1,
            flipped: false,
            current_ground: None,
            orb: None,
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
            (PartKind::Behaviour(part::COMPONENT), true) => self.late_update(cx),
            (PartKind::Animator, false) => match super::ability_body::advance_anim(cx.w, cx.fw.owner, &mut self.anim) {
                Some(AnimCallback::Ability(CB_LEFT_FIRST_FRAME)) => self.left_first_frame(cx),
                Some(AnimCallback::Ability(CB_START_BEAM)) => self.start_beam(cx),
                Some(AnimCallback::Ability(CB_EXIT)) => self.exit(cx, AbilityExitInfo::default()),
                _ => {}
            },
            _ => {
                let r = self.b.update_part(cx, k, dt, late);
                self.ground_events(cx);
                if let Some(r) = r {
                    self.exit(cx, r);
                }
            }
        }
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        super::ability_body::hurt_contact(self, cx, c);
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

    /// The staff body; the cast sparks (object 1) only play briefly at the
    /// beam start in the original (not shown here).
    fn look(&self, _w: &World, _me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        l.frame(&self.anim);
        l.parts.push(crate::view::PartLook::hidden(1));
        // `PlayOrContinueLoop("beamLoop")` while firing.
        if self.state == BeamState::Loop {
            l.loop_sound = Some("beamLoop");
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers on colliders
// ---------------------------------------------------------------------------

fn set_collider_enabled(w: &mut World, o: ObjId, e: bool) {
    w.set_collider_enabled(o, e);
}

/// `DPhysicsBox.SetExtents(e)` (zero components → minimum extents).
fn set_box_extents(w: &mut World, me: ObjId, e: Vec2) {
    let Some(def) = w.objects.get(me).and_then(|o| o.collider) else {
        return;
    };
    let mut e = e;
    if e.x.0 == 0 {
        e.x = def.minimum_extents;
    }
    if e.y.0 == 0 {
        e.y = def.minimum_extents;
    }
    if def.initialized {
        if let Some(b) = w.physics.box_mut(me) {
            b.update_extents(e);
        }
    } else if let Some(c) = w.objects.get_mut(me).and_then(|o| o.collider.as_mut()) {
        c.start_extents = e;
    }
}

// ---------------------------------------------------------------------------
// The orb (`orbAnim`)
// ---------------------------------------------------------------------------

/// The lethal orb at the staff tip (`OrbAnimation` + `DisableIfNotHeld_deter`
/// + Explosion).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Orb {
    pub owner: i32,
    pub can_hurt_owner: bool,
    pub anim: SpriteAnimator,
    pub white: Option<AnimRef>,
    pub holder: Option<ObjId>,
    pub held: bool,
    pub inited: bool,
}

fn spawn_orb(w: &mut World, c: &OrbCfg, pos: Vec2) -> ObjId {
    let orb = Orb {
        owner: 0,
        can_hurt_owner: c.can_hurt_owner,
        anim: SpriteAnimator::default(),
        white: c.white,
        holder: None,
        held: false,
        inited: false,
    };
    w.open_bracket();
    let id = w.objects.create(
        Kind::Behaviour(Box::new(Behaviour::Beam(Spawned::Orb(orb)))),
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
        o.add_part(PartKind::Behaviour(1), c.hier_animator);
        o.add_part(PartKind::Collider, c.hier_collider);
        o.add_part(PartKind::Behaviour(3), c.hier_holder);
    }
    w.register_all(id);
    w.close_bracket();
    w.consume_hierarchy((c.updatables - 3).max(0));
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.position = pos;
        o.tf.reset_rotation();
    }
    // `orbAnim.hitbox.ManualInit()` right away.
    let hier = w
        .objects
        .get(id)
        .and_then(|o| o.part(PartKind::Collider))
        .map_or(0, |p| p.hier);
    w.physics.register(&mut w.objects, id, hier);
    id
}

fn with_orb<R>(w: &mut World, o: ObjId, f: impl FnOnce(&mut World, &mut Orb) -> R) -> Option<R> {
    if !matches!(w.objects.get(o).map(|ob| &ob.kind), Some(Kind::Behaviour(_))) {
        return None;
    }
    w.with_kind(o, |w, k| match k {
        Kind::Behaviour(b) => match b.as_mut() {
            Behaviour::Beam(Spawned::Orb(orb)) => Some(f(w, orb)),
            _ => None,
        },
        _ => None,
    })
}

/// `DisableIfNotHeld_deter.HoldThisFrame(holder)`.
fn orb_hold(w: &mut World, o: ObjId, holder: ObjId) {
    with_orb(w, o, |_, orb| {
        orb.holder = Some(holder);
        orb.held = true;
        orb.inited = true;
    });
    if let Some(ob) = w.objects.get_mut(o) {
        ob.active = true;
    }
}

impl Orb {
    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind) {
        match part {
            PartKind::Behaviour(1) => {
                // No player-id holder: frozen during any time stop.
                if w.is_time_stopped() {
                    return;
                }
                let c = w.content.clone();
                match self.anim.advance(&c, Fix::ZERO) {
                    Some(AnimCallback::Ability(ORB_CONTINUE)) => {
                        if let Some(a) = self.white {
                            self.anim.begin(&c, a);
                        }
                    }
                    Some(AnimCallback::Ability(ORB_DESTROY)) => {
                        let alive = w
                            .objects
                            .get(me)
                            .and_then(|o| o.part(PartKind::Collider))
                            .is_some_and(|p| !p.destroyed);
                        if alive {
                            w.destroy(me);
                        }
                    }
                    _ => {}
                }
            }
            PartKind::Behaviour(3) if self.inited => {
                if self.holder.is_none_or(|h| !w.objects.exists(h)) {
                    w.destroy(me);
                }
                if !self.held
                    && let Some(o) = w.objects.get_mut(me)
                {
                    o.active = false;
                }
                self.held = false;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// BeamObject (pooled lethal box)
// ---------------------------------------------------------------------------

/// A pooled beam box (`BeamObject` + Explosion, tagged `beam`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BeamObject {
    pub owner: i32,
    pub beam_id: i32,
    pub can_hurt_owner: bool,
}

/// The beam id of a beam box (`BeamObject.beamId`), if `obj` is one.
pub fn beam_box_id(w: &World, obj: ObjId) -> Option<i32> {
    match &w.objects.get(obj)?.kind {
        Kind::Behaviour(b) => match b.as_ref() {
            Behaviour::Beam(Spawned::BeamObject(o)) => Some(o.beam_id),
            _ => None,
        },
        _ => None,
    }
}

impl SpawnedImpl for Spawned {
    /// Orbs: their frame. Beam boxes: `params` = (beam length, width) from
    /// the box (`SetDimensions`), `player` = the beam's owner (colours).
    fn look(&self, w: &World, me: ObjId, l: &mut crate::view::Look) {
        match self {
            Spawned::Orb(o) => {
                l.frame(&o.anim);
                l.player = o.owner;
            }
            Spawned::BeamObject(b) => {
                l.player = b.owner;
                let Some(bx) = w.physics.box_shape(me) else {
                    l.hidden = true;
                    return;
                };
                let extra = w
                    .ability_globals
                    .beams
                    .cfg
                    .as_ref()
                    .map_or(Fix(1_717_986_944), |c| c.extra_length);
                let half = Fix::ONE / bx.ie.x.max(Fix(1));
                let width = Fix::ONE / bx.ie.y.max(Fix(1));
                l.params[0] = ((half - extra) * Fix::TWO).max(Fix::ZERO);
                l.params[1] = width;
            }
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, _dt: Fix, late: bool) {
        if late {
            return;
        }
        if let Spawned::Orb(o) = self {
            o.update(w, me, part);
        }
    }

    fn lethal(&self, _w: &World, _me: ObjId) -> Option<Lethal> {
        match self {
            Spawned::Orb(o) => Some(Lethal {
                owner: o.owner,
                can_hurt_owner: o.can_hurt_owner,
                beam: false,
            }),
            Spawned::BeamObject(b) => Some(Lethal {
                owner: b.owner,
                can_hurt_owner: b.can_hurt_owner,
                beam: true,
            }),
        }
    }
}

/// Static data of the `BeamObject` prefab (from content, looked up once).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BeamObjectCfg {
    prefab: Option<usize>,
    collider: Option<ColliderDef>,
    layer: u8,
    hier: i32,
    hier_collider: i32,
    updatables: i32,
    base_width01: Fix,
    extra_length: Fix,
    can_hurt_owner: bool,
}

fn beam_object_cfg(c: &Content) -> BeamObjectCfg {
    let idx = c
        .prefabs
        .iter()
        .position(|p| p.objects.first().is_some_and(|o| o.component("BeamObject").is_some()));
    let p = idx.and_then(|i| c.prefabs.get(i));
    let root = p.and_then(|p| p.objects.first());
    let f = p.map_or(&Value::Null, |p| fields(p, "BeamObject"));
    let ef = p.map_or(&Value::Null, |p| fields(p, "Explosion"));
    BeamObjectCfg {
        prefab: idx,
        collider: root.and_then(|o| o.collider),
        layer: root.map_or(18, |o| o.layer),
        hier: f.int("hierarchyNumber").unwrap_or(1) as i32,
        hier_collider: root.and_then(|o| o.collider_hier).unwrap_or(2),
        updatables: updatables(c, idx).max(2),
        base_width01: f.fix_or("baseWidth01", Fix(1_932_735_232)),
        extra_length: f.fix_or("extraLength", Fix(1_717_986_944)),
        can_hurt_owner: ef.boolean("canHurtOwner").unwrap_or(true),
    }
}

fn spawn_beam_object(w: &mut World, c: &BeamObjectCfg, pos: Vec2) -> ObjId {
    let b = BeamObject {
        owner: 0,
        beam_id: 0,
        can_hurt_owner: c.can_hurt_owner,
    };
    w.open_bracket();
    let id = w.objects.create(
        Kind::Behaviour(Box::new(Behaviour::Beam(Spawned::BeamObject(b)))),
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
        o.add_part(PartKind::Behaviour(0), c.hier);
        o.add_part(PartKind::Collider, c.hier_collider);
    }
    w.register_all(id);
    w.close_bracket();
    w.consume_hierarchy((c.updatables - 2).max(0));
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.position = pos;
        o.tf.reset_rotation();
    }
    id
}

/// `BeamObject.SetDimensions(base, width, dir, distance, age, anim, owner, id)`.
#[allow(clippy::too_many_arguments)]
fn set_dimensions(
    w: &mut World,
    c: &BeamObjectCfg,
    o: ObjId,
    base: Vec2,
    width: Fix,
    dir: Vec2,
    distance: Fix,
    age: Fix,
    owner: i32,
    id: i32,
) {
    if let Some(Kind::Behaviour(b)) = w.objects.get_mut(o).map(|ob| &mut ob.kind)
        && let Behaviour::Beam(Spawned::BeamObject(bo)) = b.as_mut()
    {
        bo.owner = owner;
        bo.beam_id = id;
    }
    let anim = w
        .level
        .as_ref()
        .map(|l| l.beam_width_anim.clone())
        .unwrap_or_default();
    let width = width * (c.base_width01 * anim.eval(age));
    let half = distance * F_HALF + c.extra_length;
    if let Some(ob) = w.objects.get_mut(o) {
        ob.tf.position = base;
        ob.tf.offset = Vec2::new(half, Fix::ZERO);
    }
    set_box_extents(w, o, Vec2::new(half, width));
    if let Some(ob) = w.objects.get_mut(o) {
        ob.tf.set_right(dir.normalized_safe());
    }
    w.physics.update_physics_positions(&w.objects, o);
}

// ---------------------------------------------------------------------------
// The beam system (DetPhysics.UpdateBeams)
// ---------------------------------------------------------------------------

/// `DetPhysics.BeamBody`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BeamBody {
    pub id: i32,
    pub owner: i32,
    pub position: Vec2,
    pub direction: Vec2,
    pub time_passed: Fix,
    pub scale: Fix,
    pub distance: Fix,
    pub ground: Option<ObjId>,
}

/// `DetPhysics.BeamRole`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Role {
    #[default]
    Single,
    MaybeChild,
    Child,
    Parent,
    ParentAndChild,
    ChildWithEffect,
}

/// Array capacity of the original's per-index beam arrays.
const MAX_BEAMS: usize = 128;

/// The DetPhysics beam state (per scene).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BeamSystem {
    /// `beamBodies` and `beamsThisFrame` (bodies submitted this tick).
    pub bodies: Vec<BeamBody>,
    pub count: usize,
    pub initial: usize,
    pub roles: Vec<Role>,
    pub parent: Vec<i16>,
    pub is_reflected: Vec<bool>,
    pub was_reflected: Vec<bool>,
    pub refl_dist: Vec<Fix>,
    pub hit: Vec<Option<ObjId>>,
    /// `lineIdtoInd` (id → index), entry order.
    pub map: Vec<(i32, i16)>,
    pub roles_last: Vec<Role>,
    pub total_last: usize,
    /// (idA, idB, age) of last tick's crossings.
    pub crossings_last: Vec<(i32, i32, i32)>,
    /// (indA, indB, age) of this tick's crossings.
    pub crossings_this: Vec<(i16, i16, i32)>,
    /// `beamObjectPool`.
    pub pool: Vec<ObjId>,
    /// The `BeamObject` prefab (looked up on first use).
    cfg: Option<BeamObjectCfg>,
}

impl Default for BeamSystem {
    fn default() -> BeamSystem {
        BeamSystem {
            bodies: vec![BeamBody::default(); MAX_BEAMS],
            count: 0,
            initial: 0,
            roles: vec![Role::Single; MAX_BEAMS],
            parent: vec![-1; MAX_BEAMS],
            is_reflected: vec![false; MAX_BEAMS],
            was_reflected: vec![false; MAX_BEAMS],
            refl_dist: vec![Fix::ZERO; MAX_BEAMS],
            hit: vec![None; MAX_BEAMS],
            map: Vec::new(),
            roles_last: vec![Role::Single; MAX_BEAMS],
            total_last: 0,
            crossings_last: Vec::new(),
            crossings_this: Vec::new(),
            pool: Vec::new(),
            cfg: None,
        }
    }
}

/// `AddBeamBody(body)`: the index.
pub fn add_beam_body(w: &mut World, b: BeamBody) -> usize {
    let s = &mut w.ability_globals.beams;
    let i = s.count;
    if i < MAX_BEAMS {
        s.bodies[i] = b;
    }
    s.count += 1;
    i
}

fn map_get(s: &BeamSystem, id: i32) -> Option<usize> {
    s.map.iter().find(|e| e.0 == id).map(|e| e.1 as usize)
}

/// `PhysTools.LineToLineIntersection`: whether it hits and the `out` value
/// (along A, along B), which is written even on a miss ((0, 0) if parallel).
fn line_line(pa: Vec2, da: Vec2, la: Fix, pb: Vec2, db: Vec2, lb: Fix) -> (bool, Vec2) {
    let p = pb - pa;
    let nb = db.mul_by_i();
    let w = p.dot(nb) * nb;
    let k = w.dot(da);
    if k == Fix::ZERO {
        return (false, Vec2::ZERO);
    }
    let ta = w.dot(w) / k;
    let tb = (da * ta - p).dot(db);
    (
        Fix::ZERO < ta && ta < la && Fix::ZERO < tb && tb < lb,
        Vec2::new(ta, tb),
    )
}

fn mirror(v: Vec2) -> Vec2 {
    Vec2::new(v.x, -v.y)
}

/// Segments of a beam: (position, direction, direct length).
fn direct(s: &BeamSystem, i: usize) -> (Vec2, Vec2, Fix) {
    let b = &s.bodies[i];
    let l = b.distance
        - if s.is_reflected[i] {
            s.refl_dist[i]
        } else {
            Fix::ZERO
        };
    (b.position, b.direction, l)
}

/// `IntersectBeams(a, b)` (minimised along B when `along_b`).
fn intersect(s: &BeamSystem, a: usize, b: usize, along_b: bool) -> Option<Vec2> {
    let (pa, da, la) = direct(s, a);
    let (pb, db, lb) = direct(s, b);
    let fa = s.was_reflected[a] && s.is_reflected[a];
    let fb = !along_b && ((s.was_reflected[b] && s.is_reflected[b]) || !fa);
    let mut n = 0;
    let mut best = Fix::MAX;
    let mut out = Vec2::ZERO;
    let (h1, d1) = line_line(pa, da, la, pb, db, lb);
    if h1 {
        n += 1;
        best = if fb { d1.x } else { d1.y };
        out = d1;
    }
    if s.is_reflected[a] {
        let (h, mut d) = line_line(pa + da * la, mirror(da), s.refl_dist[a], pb, db, lb);
        if h {
            d.x += la;
            n += 1;
            if fb && d.x < best {
                best = d.x;
                out = d;
            } else if !fb && d.y < best {
                best = d.y;
                out = d;
            }
        }
    }
    if s.is_reflected[b] {
        let (h, mut d) = line_line(pa, da, la, pb + db * lb, mirror(db), s.refl_dist[b]);
        if h {
            d.y += lb;
            n += 1;
            // Literal quirk: the "along B" comparison reads the first test's
            // `out` value (written even when that test missed).
            if fb && d.x < best {
                best = d.x;
                out = d;
            } else if !fb && d1.y < best {
                best = d.y;
                out = d;
            }
        }
    }
    if s.is_reflected[a] && s.is_reflected[b] {
        let (h, mut d) = line_line(
            pa + da * la,
            mirror(da),
            s.refl_dist[a],
            pb + db * lb,
            mirror(db),
            s.refl_dist[b],
        );
        if h {
            d.x += la;
            d.y += lb;
            n += 1;
            if (fb && d.x < best) || (!fb && d.y < best) {
                out = d;
            }
        }
    }
    (n > 0).then_some(out)
}

/// `RaycastBeam(ind)`.
fn raycast_beam(w: &mut World, i: usize) {
    let p = w.physics.params;
    let space = w.is_space();
    let b = w.ability_globals.beams.bodies[i];
    let hit = w.physics.raycast_closest(
        &w.objects,
        b.position,
        b.direction,
        p.max_beam_distance,
        p.beam_hit_mask,
    );
    let s = &mut w.ability_globals.beams;
    let mut near = hit.near_dist.min(p.max_beam_distance);
    s.is_reflected[i] = false;
    s.refl_dist[i] = Fix::ZERO;
    s.hit[i] = hit.hit().then_some(hit.obj);
    if !hit.hit() || hit.near_pos.y < CONST_WATER_HEIGHT {
        let end = b.position + b.direction * p.max_beam_distance;
        if !space && end.y < CONST_WATER_HEIGHT {
            let h = CONST_WATER_HEIGHT - b.position.y;
            let mut v = b.direction * p.max_beam_distance;
            if v.y == Fix::ZERO {
                v.y = F_0_001_D;
            }
            let wp = Vec2::new(v.x * h / v.y, h) + b.position;
            let dir2 = Vec2::new(b.direction.x, b.direction.y * (-Fix::ONE));
            if b.direction.dot(Vec2::DOWN) >= F_0_997 {
                s.bodies[i].distance = wp.distance(b.position) + s.refl_dist[i];
                return;
            }
            let h2 = w.physics.raycast_closest(
                &w.objects,
                wp,
                dir2,
                p.max_beam_distance,
                p.beam_hit_mask,
            );
            let s = &mut w.ability_globals.beams;
            s.is_reflected[i] = true;
            s.refl_dist[i] = h2.near_dist.min(p.max_beam_distance);
            s.hit[i] = h2.hit().then_some(h2.obj);
            near = wp.distance(b.position) + s.refl_dist[i];
        }
    }
    w.ability_globals.beams.bodies[i].distance = near;
}

/// Sets beam `j` cut at `d` along it by parent `par` (crossBeams steps 6/7).
fn cut(s: &mut BeamSystem, j: usize, d: Fix, par: usize, role: Role) {
    let lj = s.bodies[j].distance - s.refl_dist[j];
    let r = d - lj;
    s.bodies[j].distance = d;
    s.roles[j] = role;
    s.parent[j] = par as i16;
    if r <= Fix::ZERO {
        s.is_reflected[j] = false;
        s.refl_dist[j] = Fix::ZERO;
    } else {
        s.is_reflected[j] = true;
        s.refl_dist[j] = r;
    }
}

/// `crossBeams(a, b, age, (x, y))`.
fn cross_beams(w: &mut World, a: usize, b: usize, age: i32, xy: Vec2) {
    let p = w.physics.params;
    let s = &mut w.ability_globals.beams;
    s.roles[a] = Role::Child;
    s.roles[b] = Role::Child;
    let ba = s.bodies[a];
    let bb = s.bodies[b];
    let la = ba.distance - s.refl_dist[a];
    let lb = bb.distance - s.refl_dist[b];
    let ra = xy.x - la;
    let rb = xy.y - lb;
    let (x, y) = (xy.x, xy.y);
    let mut v = ba.direction;
    let mut u = bb.direction;
    let (pt, m);
    if x > la && y > lb {
        let mb = mirror(bb.direction);
        let ma = mirror(ba.direction);
        pt = ba.position + ba.direction * la + ma * ra;
        m = mb.nlerp(ma, F_HALF);
        s.refl_dist[a] = ra;
        s.refl_dist[b] = rb;
        s.is_reflected[a] = true;
        s.is_reflected[b] = true;
        v = ma;
        u = mb;
    } else if x > la {
        pt = bb.position + bb.direction * y;
        let ma = mirror(ba.direction);
        m = bb.direction.nlerp(ma, F_HALF);
        s.refl_dist[a] = ra;
        s.refl_dist[b] = Fix::ZERO;
        s.is_reflected[a] = true;
        s.is_reflected[b] = false;
        v = ma;
    } else if y > lb {
        pt = ba.position + ba.direction * x;
        let mb = mirror(bb.direction);
        m = ba.direction.nlerp(mb, F_HALF);
        s.refl_dist[a] = Fix::ZERO;
        s.refl_dist[b] = rb;
        s.is_reflected[a] = false;
        s.is_reflected[b] = true;
        v = mb;
    } else {
        pt = bb.position + bb.direction * y;
        m = bb.direction.nlerp(ba.direction, F_HALF);
        s.is_reflected[a] = false;
        s.is_reflected[b] = false;
        s.refl_dist[a] = Fix::ZERO;
        s.refl_dist[b] = Fix::ZERO;
    }
    s.bodies[a].distance = x;
    s.bodies[b].distance = y;
    let num;
    if v.dot(u) < F_NEG_HALF {
        s.crossings_this.push((a as i16, b as i16, age));
        num = a;
        s.roles[a] = Role::ChildWithEffect;
        s.roles[b] = Role::ChildWithEffect;
    } else {
        let id = i32::MAX - s.crossings_this.len() as i32 - 1;
        let body = BeamBody {
            scale: Fix::ZERO,
            position: pt + m * p.cross_beam_offset,
            time_passed: Fix::from_int(age) * FIXED_DT,
            distance: p.max_beam_distance,
            direction: m,
            id,
            owner: ENVIRONMENT_ID,
            ground: None,
        };
        let i = s.count;
        if i < MAX_BEAMS {
            s.bodies[i] = body;
        }
        s.count += 1;
        num = i;
        s.map.push((id, num as i16));
        s.crossings_this.push((a as i16, b as i16, age));
        s.roles[num] = Role::Parent;
        raycast_beam(w, num);
        w.ability_globals.beams.was_reflected[num] = false;
    }
    let s = &mut w.ability_globals.beams;
    s.parent[a] = num as i16;
    s.parent[b] = num as i16;
    for i in 0..num {
        if i == a || i == b || matches!(s.roles[i], Role::Single | Role::MaybeChild) {
            continue;
        }
        let Some(d) = intersect(s, num, i, true) else {
            continue;
        };
        if d.x < s.bodies[num].distance {
            cut(s, num, d.x, i, Role::ParentAndChild);
        }
    }
    for j in 0..s.initial {
        if !matches!(s.roles[j], Role::Single | Role::MaybeChild) || j == a || j == b {
            continue;
        }
        for par in [num, a, b] {
            if let Some(d) = intersect(s, par, j, true)
                && d.y < s.bodies[j].distance
            {
                cut(s, j, d.y, par, Role::MaybeChild);
            }
        }
    }
}

/// `DetPhysics.UpdateBeams()` (Simulate step 8).
pub fn update_beams(w: &mut World) {
    let s = &mut w.ability_globals.beams;
    if s.count == 0 && s.total_last == 0 && s.pool.is_empty() && s.crossings_last.is_empty() {
        s.map.clear();
        return;
    }
    let count = s.count.min(MAX_BEAMS);
    s.count = count;
    for i in 0..s.total_last {
        s.roles_last[i] = s.roles[i];
    }
    let map_last = std::mem::take(&mut s.map);
    s.was_reflected.iter_mut().for_each(|x| *x = false);
    let reflected_last: Vec<i32> = map_last
        .iter()
        .filter(|(_, v)| s.is_reflected[*v as usize])
        .map(|(k, _)| *k)
        .collect();
    s.roles.iter_mut().for_each(|r| *r = Role::Single);
    for k in 0..s.total_last.max(count) {
        s.parent[k] = -1;
    }
    for l in 0..count {
        let id = s.bodies[l].id;
        s.map.push((id, l as i16));
    }
    s.initial = count;
    for m in 0..count {
        raycast_beam(w, m);
    }
    let s = &mut w.ability_globals.beams;
    for id in &reflected_last {
        if let Some(v) = map_get(s, *id) {
            s.was_reflected[v] = true;
        }
    }
    // Persistent crossings.
    let last = std::mem::take(&mut w.ability_globals.beams.crossings_last);
    for (ida, idb, age) in &last {
        let s = &w.ability_globals.beams;
        if let (Some(a), Some(b)) = (map_get(s, *ida), map_get(s, *idb))
            && let Some(d) = intersect(s, a, b, false)
        {
            cross_beams(w, a, b, *age, d);
        }
    }
    // New crossings.
    let initial = w.ability_globals.beams.initial;
    for i in 0..initial {
        let s = &w.ability_globals.beams;
        if !matches!(s.roles[i], Role::Single | Role::MaybeChild) {
            continue;
        }
        let mut best = None;
        let mut bd = Vec2::new(Fix::MAX, Fix::MAX);
        for j in i + 1..initial {
            if matches!(s.roles[j], Role::Single | Role::MaybeChild)
                && let Some(d) = intersect(s, i, j, false)
                && d.x < bd.x
            {
                bd = d;
                best = Some(j);
            }
        }
        if let Some(j) = best {
            cross_beams(w, i, j, 0, bd);
            w.emit(Cue::Sound("beamsCrossed"));
        }
    }
    let s = &mut w.ability_globals.beams;
    s.crossings_last = s
        .crossings_this
        .iter()
        .map(|(a, b, age)| (s.bodies[*a as usize].id, s.bodies[*b as usize].id, age + 1))
        .collect();
    s.crossings_this.clear();
    let count = s.count.min(MAX_BEAMS);
    // Merge strength.
    for n in 0..count {
        if !matches!(s.roles[n], Role::MaybeChild | Role::Child) {
            continue;
        }
        let mut k = s.parent[n];
        while k >= 0 {
            let ku = k as usize;
            let r = s.roles[ku];
            let sc = s.bodies[n].scale;
            s.bodies[ku].scale += sc;
            if !matches!(r, Role::MaybeChild | Role::Child | Role::ParentAndChild) {
                break;
            }
            k = s.parent[ku];
        }
    }
    // Split reflections.
    let mut extra = 0;
    let mut sparks = false;
    for n in 0..count {
        if !s.is_reflected[n] || count + extra >= MAX_BEAMS {
            continue;
        }
        if !s.was_reflected[n] {
            sparks = true;
        }
        let mut b = s.bodies[n];
        b.position += b.direction * (b.distance - s.refl_dist[n]);
        b.distance = s.refl_dist[n];
        b.direction = Vec2::new(b.direction.x, -b.direction.y);
        b.id = -b.id;
        let k = count + extra;
        s.bodies[k] = b;
        s.bodies[n].distance -= s.refl_dist[n];
        s.roles[k] = s.roles[n];
        s.roles[n] = match s.roles[n] {
            Role::Child | Role::ChildWithEffect | Role::MaybeChild => Role::Single,
            Role::ParentAndChild => Role::Parent,
            r => r,
        };
        s.is_reflected[k] = false;
        s.hit[k] = s.hit[n];
        s.map.push((b.id, k as i16));
        extra += 1;
    }
    if sparks {
        w.emit(Cue::SoundPrefix("beamSparks"));
    }
    render_beams(w, count, extra);
    let s = &mut w.ability_globals.beams;
    s.total_last = count;
    s.count = 0;
}

/// `renderBeams(extra)`: the lethal boxes and the impact pushes.
fn render_beams(w: &mut World, count: usize, extra: usize) {
    let num = count + extra;
    let c = match &w.ability_globals.beams.cfg {
        Some(c) => c.clone(),
        None => {
            let c = beam_object_cfg(&w.content);
            w.ability_globals.beams.cfg = Some(c.clone());
            c
        }
    };
    let p = w.physics.params;
    // Pool: create missing boxes, activate, deactivate the rest.
    for j in 0..num {
        let b = w.ability_globals.beams.bodies[j];
        let pool = &w.ability_globals.beams.pool;
        let existing = pool.get(j).copied().filter(|o| w.objects.alive(*o));
        match existing {
            None => {
                let o = spawn_beam_object(w, &c, b.position);
                set_dimensions(
                    w,
                    &c,
                    o,
                    b.position,
                    p.beam_width * b.scale,
                    b.direction,
                    b.distance,
                    b.time_passed,
                    b.owner,
                    b.id,
                );
                let hier = w
                    .objects
                    .get(o)
                    .and_then(|ob| ob.part(PartKind::Collider))
                    .map_or(0, |pp| pp.hier);
                w.physics.register(&mut w.objects, o, hier);
                let pool = &mut w.ability_globals.beams.pool;
                if pool.len() <= j {
                    pool.resize(j + 1, ObjId::NONE);
                }
                pool[j] = o;
            }
            Some(o) => {
                if let Some(ob) = w.objects.get_mut(o)
                    && !ob.active
                {
                    ob.active = true;
                }
            }
        }
    }
    let mut k = num;
    while let Some(o) = w.ability_globals.beams.pool.get(k).copied() {
        match w.objects.get_mut(o) {
            Some(ob) if !ob.destroyed && ob.active => ob.active = false,
            _ => break,
        }
        k += 1;
    }
    for l in 0..num {
        let s = &w.ability_globals.beams;
        let mut b = s.bodies[l];
        if matches!(s.roles[l], Role::Parent | Role::ParentAndChild) {
            b.position += b.direction * p.beam_parent_offset;
            b.distance -= p.beam_parent_offset;
        }
        let o = s.pool[l];
        set_dimensions(
            w,
            &c,
            o,
            b.position,
            p.beam_width * b.scale,
            b.direction,
            b.distance,
            b.time_passed,
            b.owner,
            b.id,
        );
    }
    // Impacts.
    for n in 0..num {
        let s = &w.ability_globals.beams;
        let role = s.roles[n];
        if matches!(role, Role::Child | Role::ParentAndChild | Role::MaybeChild) {
            continue;
        }
        let b = s.bodies[n];
        if s.is_reflected[n] && n < count {
            continue;
        }
        let Some(h) = s.hit[n].filter(|h| w.objects.exists(*h)) else {
            continue;
        };
        if role == Role::ChildWithEffect {
            continue;
        }
        let e = b.position + b.direction * b.distance;
        let k = b.scale.min(Fix::from_int(40));
        let l = w.objects.get(h).map_or(0, |o| o.layer as i32);
        if l == layer::WALL || l == layer::NON_LETHAL_TERRAIN || l == layer::LETHAL_TERRAIN {
            let live = w
                .objects
                .get(h)
                .and_then(|o| o.part(PartKind::Collider))
                .is_some_and(|pp| !pp.destroyed);
            if live && e.y > CONST_WATER_HEIGHT + Fix::ONE && !w.is_time_stopped_for(b.owner) {
                w.physics
                    .add_force(h, b.direction * p.beam_platform_push_force * k);
            }
        } else if l == layer::RIGID_BODY_AFFECTOR {
            if !w.is_time_stopped_for(b.owner) {
                let f = b.direction * p.beam_black_hole_push_force * k;
                super::black_hole::add_force_less_mass_influence(w, h, f);
            }
        } else if l == layer::PLAYER && has_bopl_body(w, h) {
            w.physics
                .add_force(h, b.direction * p.beam_rigid_body_push_force * k);
        }
    }
    let _ = ShapeKind::Box;
}

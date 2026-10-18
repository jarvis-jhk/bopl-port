//! Rock (`docs/spec/abilities/rock.md`): the player becomes a heavy
//! bouncing rigid body (`BounceBall`) that kills on contact
//! (`KillPlayersOnContact`) and stays coupled to a grappling rope it held
//! (the BounceBall drives its rope end itself; it has no PlayerBody).

use std::sync::Arc;

use super::ability_body::{self, Bounds};
use super::prefab::{fx, root_fields};
use super::prelude::*;
use super::{ITEM_BOUNCE_BALL, Interaction};
use crate::bopl::RopeLink;
use crate::curve::Curve;

/// `BounceBall` constants.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RockCfg {
    pub max_duration: Fix,
    pub cancellable: bool,
    pub exit_time: Fix,
    pub player_collision_bounce: Fix,
    pub scale_on_enter: Arc<Curve>,
    pub scale_on_exit: Arc<Curve>,
    pub bounds: Bounds,
    pub min_rope_length: Fix,
    pub rope_pull_str: Fix,
    pub sqrd_max_rope_separation: Fix,
    pub rope_player_mass01: Fix,
}

/// `(Fix)0.6667f`, `(Fix)0.2f`, `(Fix)0.1f`.
const F_0_6667F: Fix = Fix(2_863_454_720);
const F_0_2F: Fix = Fix(858_993_472);
const F_0_1F: Fix = Fix(429_496_736);

/// The Rock ability object (`BounceBall`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rock {
    pub cfg: Option<Arc<RockCfg>>,
    pub is_exiting: bool,
    pub time_since_exit_started: Fix,
    pub time_since_activation: Fix,
    pub scale_sum: Fix,
    pub exit_scale: Fix,
    pub scale_last_frame: Fix,
    pub prev_vel: Vec2,
    /// `ropeBody` / `topAttachment`.
    pub rope: Option<RopeLink>,
    /// `ropeNumberOnEnter`.
    pub rope_number_on_enter: u32,
}

impl Rock {
    fn scale(w: &World, me: ObjId) -> Fix {
        w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale)
    }

    fn velocity(w: &World, me: ObjId) -> Vec2 {
        w.physics.body(me).map_or(Vec2::ZERO, |b| b.velocity)
    }

    fn set_velocity(w: &mut World, me: ObjId, v: Vec2) {
        if let Some(b) = w.physics.body_mut(me) {
            b.velocity = v;
        }
    }

    fn rope_mass(&self) -> Fix {
        let m = self.cfg.as_ref().map_or(Fix::ZERO, |c| c.rope_player_mass01);
        if m == Fix::ZERO { F_0_5 } else { m }
    }

    /// `BounceBall.AttachRope(rope, top)`: the old rope's `top`-argument end
    /// gets air time 100, rope gravity and the rope end mass (literal).
    pub fn attach_rope(&mut self, w: &mut World, new: Option<u16>, top: bool) {
        if let Some(old) = self.rope {
            let g = w.physics.params.rope_gravity;
            let m = self.rope_mass();
            if let Some(r) = w.physics.ropes.get_mut(old.rope as usize) {
                r.set_time_in_air(Fix::from_int(100), top);
                r.set_force(Vec2::DOWN * g, top);
                r.set_mass01(m, top, Fix::ONE);
            }
        }
        self.rope = new.map(|rope| RopeLink { rope, top });
    }

    /// The rope end index `e` (0 for a top attachment, else the last).
    fn rope_end(w: &World, l: RopeLink) -> usize {
        w.physics.ropes.get(l.rope as usize).map_or(0, |r| r.end(l.top))
    }

    /// `UpdateRopeAttachment` (BounceBall.UpdateSim).
    fn update_rope(&mut self, cx: &mut Cx<'_>) {
        let me = cx.me;
        if let Some(l) = self.rope
            && cx.w.physics.ropes.get(l.rope as usize).is_some_and(|r| r.number != self.rope_number_on_enter)
        {
            self.rope = None;
        }
        let Some(l) = self.rope else {
            return;
        };
        let e = Self::rope_end(cx.w, l);
        let owner = cx.fw.owner;
        if let Some(r) = cx.w.physics.ropes.enabled(l.rope as usize) {
            let jump = cx.w.player(owner).is_some_and(|p| p.id == r.owner as i32 && p.inputs.jump_down_this_frame);
            if jump {
                let v = (r.segment[e] - r.prev[e]) / FIXED_DT;
                Self::set_velocity(cx.w, me, v);
                self.attach_rope(cx.w, None, l.top);
                cx.w.emit(Cue::Sound("hookshotLetGo"));
            }
        }
        let Some(l) = self.rope.filter(|l| cx.w.physics.ropes.enabled(l.rope as usize).is_some()) else {
            return;
        };
        let scale = Self::scale(cx.w, me);
        let m = self.rope_mass();
        let v = Self::velocity(cx.w, me);
        let dv = v - self.prev_vel;
        self.prev_vel = v;
        let pos = cx.w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        let stopped = cx.w.is_time_stopped();
        let c = self.cfg.clone().unwrap_or_default();
        let Some(r) = cx.w.physics.ropes.get_mut(l.rope as usize) else {
            return;
        };
        r.set_time_in_air(Fix::from_int(100), l.top);
        r.set_mass01(m, l.top, scale);
        r.set_force(dv, l.top);
        let e = r.end(l.top);
        let other = r.segment[r.count - 1 - e];
        let detached = r.detached;
        let pull = r.hook_arrived() && !stopped && !detached;
        let seg = r.segment[e];
        if !detached && let Some(o) = cx.w.objects.get_mut(me) {
            o.tf.set_up(-(pos - other).normalized_safe());
        }
        if pull {
            let mut d = pos - seg;
            if d.sqr_magnitude() > c.sqrd_max_rope_separation {
                d = d.normalized_safe() * c.sqrd_max_rope_separation;
            }
            cx.w.physics.add_force(me, d * c.rope_pull_str * scale * Fix::from_int(60));
        }
        if let Some(r) = cx.w.physics.ropes.get_mut(l.rope as usize) {
            r.segment[e] = pos;
        }
    }

    /// `BounceBall.LateUpdateSim`.
    fn late(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let (me, owner) = (cx.me, cx.fw.owner);
        let cancel = !cx.button_down() && cfg.cancellable;
        if (cancel || self.time_since_activation > cfg.max_duration) && !self.is_exiting {
            self.is_exiting = true;
            self.exit_scale = Self::scale(cx.w, me) - self.scale_sum;
        }
        // Rope over-stretch: let go with the end segment's velocity.
        if let Some(l) = self.rope
            && let Some(r) = cx.w.physics.ropes.enabled(l.rope as usize)
        {
            let e = r.end(l.top);
            let lim = cx.w.physics.params.max_stretch_length;
            let over = (0..r.count.saturating_sub(2))
                .any(|i| (r.segment[i] - r.segment[i + 1]).sqr_magnitude() > lim);
            if over {
                let v = (r.segment[e] - r.prev[e]) / FIXED_DT;
                Self::set_velocity(cx.w, me, v);
                self.attach_rope(cx.w, None, l.top);
            }
        }
        let pdt = cx.w.fixed_delta_time(owner, dt);
        self.time_since_activation += pdt;
        if self.is_exiting {
            self.time_since_exit_started += pdt;
            if self.time_since_exit_started > cfg.exit_time {
                self.exit_component(cx, AbilityExitInfo::default());
                return;
            }
        }
        if self.is_exiting {
            let d = Self::scale(cx.w, me) - self.scale_last_frame;
            self.exit_scale += d;
            let s = cfg.scale_on_exit.eval(self.time_since_exit_started) * self.exit_scale;
            cx.w.set_collider_scale(me, s);
        } else {
            let c = cfg.scale_on_enter.eval(self.time_since_activation);
            let s = Self::scale(cx.w, me) + c;
            cx.w.set_collider_scale(me, s);
            self.scale_sum += c;
        }
        self.scale_last_frame = Self::scale(cx.w, me);
    }
}

impl AbilityImpl for Rock {
    /// The ball and its slime trail (object 1) in the owner's material.
    fn look(&self, _w: &World, _me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        let mut t = crate::view::PartLook::new(1);
        t.player_material = true;
        l.parts.push(t);
    }

    fn new(w: &World, def: &AbilityDef) -> Self {
        let Some(p) = def.prefab else {
            return Rock::default();
        };
        let pf = &w.content.prefabs[p];
        let Some(f) = root_fields(pf, "BounceBall") else {
            return Rock::default();
        };
        let cfg = RockCfg {
            max_duration: fx(f, "maxDuration", Fix::ONE),
            cancellable: f.boolean("IsCancellable").unwrap_or(false),
            exit_time: fx(f, "exitTime", Fix::ZERO),
            player_collision_bounce: fx(f, "playerCollisionBounce", Fix::ZERO),
            scale_on_enter: f.curve("scaleOnEnter").unwrap_or_default(),
            scale_on_exit: f.curve("ScaleOnExit").unwrap_or_default(),
            bounds: Bounds::of_prefab(w, p),
            min_rope_length: fx(f, "minRopeLength", Fix::ZERO),
            rope_pull_str: fx(f, "ropePullStr", Fix::ZERO),
            sqrd_max_rope_separation: fx(f, "sqrdMaxRopeSeparation", Fix::ZERO),
            rope_player_mass01: fx(f, "RopePlayerMass01", Fix::ZERO),
        };
        Rock {
            cfg: Some(Arc::new(cfg)),
            scale_last_frame: Fix::ZERO,
            ..Rock::default()
        }
    }

    fn build(&mut self, w: &mut World, me: ObjId, fw: &mut Framework, def: &AbilityDef) {
        ability_body::add_parts(w, me, fw, def, &[("BounceBall", PartKind::Behaviour(0))]);
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        if self.cfg.is_none() {
            let i = cx.info();
            cx.exit(AbilityExitInfo::resume(&i));
            return;
        }
        let info = cx.info();
        let me = cx.me;
        if !ability_body::collider_initialized(cx.w, me) {
            ability_body::manual_init(cx.w, me);
        }
        cx.w.emit(Cue::Sound("rockBounce"));
        self.is_exiting = false;
        self.time_since_exit_started = Fix::ZERO;
        self.time_since_activation = Fix::ZERO;
        if let Some(o) = cx.w.objects.get_mut(me) {
            o.tf.set_rotation(Fix::ZERO);
        }
        self.scale_sum = Fix::ZERO;
        self.rope = info.rope;
        self.rope_number_on_enter = info
            .rope
            .and_then(|l| cx.w.physics.ropes.get(l.rope as usize))
            .map_or(0, |r| r.number);
        let s = if Vec2::RIGHT.dot(info.self_imposed_velocity) <= Fix::ZERO {
            Fix::ONE
        } else {
            -Fix::ONE
        };
        let v = info.external_velocity + info.self_imposed_velocity;
        self.prev_vel = v;
        let r = cx
            .w
            .objects
            .get(me)
            .and_then(|o| o.collider)
            .map_or(Fix::ZERO, |c| c.start_radius);
        let c0 = self.cfg.as_ref().map_or(Fix::ZERO, |c| c.scale_on_enter.eval(Fix::ZERO));
        let (pos, vel, ang) = match (info.is_grounded, info.ground.and_then(|g| crate::surface::Surface::of(cx.w, g))) {
            (true, Some(surf)) => {
                let p = surf.position_from_param(info.grounded_local_position, info.radius) + r * info.up * c0;
                let a = (info.external_velocity + info.self_imposed_velocity).magnitude() * s * r * F_0_2F;
                (p, v * F_0_6667F, a)
            }
            _ => (info.position, v, v.magnitude() * s * r * F_0_2F),
        };
        if let Some(o) = cx.w.objects.get_mut(me) {
            o.tf.position = pos;
        }
        let ps = cx.w.player(info.player_id).map_or(Fix::ONE, |p| p.scale());
        if let Some(b) = cx.w.physics.body_mut(me) {
            b.velocity = vel;
            b.angular_velocity = ang / ps;
        }
        if let Some(l) = self.rope {
            if let Some(b) = cx.w.physics.body_mut(me) {
                b.angular_velocity = Fix::ZERO;
            }
            let scale = Self::scale(cx.w, me);
            let min = self.cfg.as_ref().map_or(Fix::ZERO, |c| c.min_rope_length);
            if let Some(r) = cx.w.physics.ropes.get_mut(l.rope as usize) {
                let n = Fix::from_int(r.count as i32);
                if r.separation * n < min * scale {
                    r.separation = scale * min / n;
                }
            }
        }
    }

    fn update_part(&mut self, cx: &mut Cx<'_>, k: PartKind, dt: Fix, late: bool) {
        match (k, late) {
            (PartKind::Behaviour(0), false) => self.update_rope(cx),
            (PartKind::Behaviour(0), true) => self.late(cx, dt),
            (PartKind::Bounds, false) => {
                let b = self.cfg.as_ref().map(|c| c.bounds).unwrap_or_default();
                if b.check(cx.w, cx.me) {
                    let info = ability_body::dead_record(cx.w, cx.me, ENVIRONMENT_ID, CauseOfDeath::Other);
                    self.exit_component(cx, info);
                }
            }
            _ => {}
        }
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        let Some(cfg) = &self.cfg else {
            return;
        };
        if self.rope.is_some() {
            return;
        }
        let f = match c.layer {
            8 => c.normal * cfg.player_collision_bounce,
            9 => c.normal * cfg.player_collision_bounce * F_0_1F,
            _ => return,
        };
        cx.w.physics.add_force(cx.me, f * Fix::from_int(60));
    }

    /// `BounceBall` interfaces: rope attach (grappling hook), magnet holds
    /// (activation timer paused), item kind.
    fn interact(&mut self, cx: &mut Cx<'_>, i: Interaction) -> Option<Fix> {
        match i {
            Interaction::AttachRope { rope, top } => {
                self.attach_rope(cx.w, rope, top);
                Some(Fix::ZERO)
            }
            Interaction::DelayFuse(k) => {
                self.time_since_activation -= k;
                Some(Fix::ZERO)
            }
            Interaction::ItemKind => Some(Fix::from_int(ITEM_BOUNCE_BALL)),
            _ => None,
        }
    }

    /// Nothing kills the rock by contact (no PlayerCollision).
    fn on_killed(&mut self, _cx: &mut Cx<'_>, _killer: i32, _cause: CauseOfDeath) {}

    /// `BounceBall.ExitAbility(info)`.
    fn exit_component(&mut self, cx: &mut Cx<'_>, mut info: AbilityExitInfo) {
        let me = cx.me;
        cx.w.set_collider_scale(me, self.exit_scale);
        if let Some(o) = cx.w.objects.get_mut(me) {
            o.tf.set_rotation(Fix::ZERO);
        }
        let v = Self::velocity(cx.w, me);
        info.grounded_speed = v.magnitude();
        info.is_grounded = false;
        info.facing_right = v.x >= Fix::ZERO;
        info.position = cx.w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        info.self_imposed_velocity = v;
        info.external_velocity = Vec2::ZERO;
        info.ground = None;
        // `EnterAbilityClearRope`: the rope goes into the record.
        info.rope = self.rope.take();
        self.prev_vel = v;
        // Projectiles stuck in the rock go with it.
        let children = cx.w.objects.get(me).map(|o| o.children.clone()).unwrap_or_default();
        for c in children {
            if cx.w.objects.get(c).is_some_and(|o| o.layer == 9) {
                cx.w.destroy(c);
            }
        }
        cx.exit(info);
    }
}

/// Objects this module spawns.
pub type Spawned = NoSpawn;

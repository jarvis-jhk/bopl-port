//! Grappling hook (`docs/spec/abilities/grappling-hook.md`): the
//! `HookshotInstant` instant ability and the rope system's game objects
//! (`Rope`, `RopeHook`, `RopeAttachment`). Rope bodies and their simulation
//! are in `physics/ropes.rs`, the body coupling in `abilities/rope.rs`.

use super::ability_body::{self as body};
use super::prefab::{self, root_component};
use super::prelude::*;
use super::rope::{self, RopeCfg};
use super::Behaviour;
use crate::bopl::RopeLink;
use crate::data::{Prefab, PrefabComponent};
use crate::physics::ShapeSnap;
use crate::surface::Surface;
use crate::world::Kind;

/// `HookshotInstant` constants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HookDef {
    pub hook_spawn_offset: Fix,
    pub hook_speed: Fix,
    pub frames_between_reel_ins: i32,
    pub starting_separation: Fix,
    pub reeled_in_separation: Fix,
    pub reel_in_speed: Fix,
    pub separation_before_deletion: Fix,
    pub cooldown: Fix,
    pub rope: RopeDef,
    pub hook: RopeHookDef,
}

/// `Rope` prefab values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RopeDef {
    pub prefab: Option<usize>,
    pub hier: i32,
    pub updatables: i32,
    pub layer: u8,
    pub nr_of_segments: usize,
    pub max_ropes_per_player: i32,
    pub reel_out_speed: Fix,
    pub segments_affected_by_reel_out: usize,
    pub segment_wave_length: i32,
    pub reel_out_segment_speed: Fix,
    pub sprinkler_speed: Fix,
    pub max_travel_time: Fix,
}

/// `RopeHook` + `RopeAttachment` prefab values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RopeHookDef {
    pub prefab: Option<usize>,
    pub layer: u8,
    pub updatables: i32,
    pub hier_attachment: i32,
    pub hier_hook: i32,
    pub hier_collider: i32,
    pub collider: Option<crate::physics::ColliderDef>,
    pub segments_affected_by_speed: usize,
    pub segment_speed_falloff01: Fix,
    pub time_until_fall: Fix,
    pub pull_str: Fix,
    pub platform_pull_str: Fix,
    pub boulder_pull_str: Fix,
    pub platform_heaviness_scaling: Fix,
    pub stretch_before_break: Fix,
    pub stretch_before_break_platform: Fix,
    pub platform_mass01: Fix,
    pub non_platform_mass: Fix,
}

fn prefab_of<'a>(w: &'a World, r: Option<&str>, name: &str) -> Option<(usize, &'a Prefab)> {
    let i = r
        .and_then(|r| w.content.prefab_index.get(r).copied())
        .or_else(|| w.content.prefabs.iter().position(|p| p.name == name))?;
    Some((i, &w.content.prefabs[i]))
}

fn comp<'a>(p: &'a Prefab, class: &str) -> Option<&'a PrefabComponent> {
    p.find_component(class).map(|(_, c)| c)
}

impl HookDef {
    fn load(w: &World, def: &AbilityDef) -> HookDef {
        let c = root_component(&w.content, def, "HookshotInstant");
        let f = |k: &str, d: Fix| prefab::fix_field(c, k, d);
        let mut h = HookDef {
            hook_spawn_offset: f("hookSpawnOffset", F_0_5),
            hook_speed: f("hookSpeed", Fix::from_int(100)),
            frames_between_reel_ins: prefab::int_field(c, "framesBetweenReelIns", 1) as i32,
            starting_separation: f("startingSegmentSeparation", Fix::ONE),
            reeled_in_separation: f("reeledInSegmentSeparation", F_0_1),
            reel_in_speed: f("ReelInSpeed", Fix(1_717_986_944)),
            separation_before_deletion: f("separationBeforeReelinDeletion", F_0_3),
            cooldown: f("cooldown", Fix::from_int(3)),
            ..HookDef::default()
        };
        let rr = c.and_then(|c| c.fields.reference("ropePrefab"));
        if let Some((i, p)) = prefab_of(w, rr, "Rope") {
            let rc = comp(p, "Rope");
            let g = |k: &str, d: Fix| prefab::fix_field(rc, k, d);
            h.rope = RopeDef {
                prefab: Some(i),
                hier: rc.and_then(|c| c.hierarchy).unwrap_or(1),
                updatables: p.updatable_count(),
                layer: p.objects.first().map_or(24, |o| o.layer),
                nr_of_segments: prefab::int_field(rc, "nrOfSegments", 12) as usize,
                max_ropes_per_player: prefab::int_field(rc, "maxRopesPerPlayer", 16) as i32,
                reel_out_speed: g("ReelOutSpeed", Fix(21_474_836)),
                segments_affected_by_reel_out: prefab::int_field(rc, "segmentsAffectedByReelOut", 10)
                    as usize,
                segment_wave_length: prefab::int_field(rc, "segmentWaveLength", 5) as i32,
                reel_out_segment_speed: g("reelOutSegmentSpeed", Fix::ZERO),
                sprinkler_speed: g("sprinklerSpeed", Fix::from_int(75)),
                max_travel_time: g("maxTravelTime", Fix(3_435_973_888)),
            };
        }
        let hr = c.and_then(|c| c.fields.reference("hookPrefab"));
        if let Some((i, p)) = prefab_of(w, hr, "RopeHook") {
            let hc = comp(p, "RopeHook");
            let ac = comp(p, "RopeAttachment");
            let a = |k: &str, d: Fix| prefab::fix_field(ac, k, d);
            let root = p.objects.first();
            h.hook = RopeHookDef {
                prefab: Some(i),
                layer: root.map_or(24, |o| o.layer),
                updatables: p.updatable_count(),
                hier_attachment: ac.and_then(|c| c.hierarchy).unwrap_or(1),
                hier_hook: hc.and_then(|c| c.hierarchy).unwrap_or(3),
                hier_collider: root.and_then(|o| o.collider_hier).unwrap_or(2),
                collider: root.and_then(|o| o.collider),
                segments_affected_by_speed: prefab::int_field(hc, "segmentsAffectedBySpeed", 5)
                    as usize,
                segment_speed_falloff01: prefab::fix_field(hc, "segmentSpeedFalloff01", Fix(4_080_218_880)),
                time_until_fall: prefab::fix_field(hc, "timeUntilFall", Fix(3_435_973_888)),
                pull_str: a("pullStr", Fix::from_int(-3)),
                platform_pull_str: a("platformPullStr", Fix::from_int(-10)),
                boulder_pull_str: a("boulderPullStr", Fix::from_int(-5)),
                platform_heaviness_scaling: a("platformHeavinessScaling", F_0_01),
                stretch_before_break: a("stretchBeforeBreak", Fix::from_int(100)),
                stretch_before_break_platform: a("stretchBeforeBreak_platform", Fix::from_int(100)),
                platform_mass01: a("platformMass01", Fix(3_865_470_464)),
                non_platform_mass: a("nonPlatformMass", F_0_5),
            };
        }
        h
    }
}

/// Module state (one per ability object): `HookshotInstant`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct GrapplingHook {
    pub def: HookDef,
    /// `ropeBody`: the rope this slot fired last (normal shots only).
    pub rope: Option<u16>,
    pub fired_number: u32,
    pub frames_since_reel_in: i32,
    pub cooldown_set: bool,
    pub tick_used_last: i64,
}

/// Rope-system objects.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    /// `Rope` (part `Behaviour(0)`).
    Rope(RopeObj),
    /// `RopeHook` object: `RopeAttachment` (part `Behaviour(1)`), its circle,
    /// `RopeHook` (part `Behaviour(2)`).
    Hook(HookObj),
}

impl AbilityImpl for GrapplingHook {
    fn new(w: &World, def: &AbilityDef) -> Self {
        GrapplingHook {
            def: HookDef::load(w, def),
            ..GrapplingHook::default()
        }
    }

    /// `HookshotInstant.UseAbility` (the instant cast).
    fn update(&mut self, cx: &mut Cx<'_>, _dt: Fix) {
        let info = cx.info();
        let owner = cx.owner();
        let w = &mut *cx.w;
        let mine = self
            .rope
            .and_then(|r| w.physics.ropes.enabled(r as usize).map(|b| (r, b.owner)));
        let current = info
            .rope
            .and_then(|l| w.physics.ropes.enabled(l.rope as usize).map(|b| b.number));
        if let (Some((r, r_owner)), Some(num)) = (mine, current)
            && num == self.fired_number
            && r_owner as i32 == info.player_id
        {
            self.reel_in(w, r, &info);
        } else {
            self.fire(w, owner, &info, cx.me, cx.fw);
        }
    }
}

impl GrapplingHook {
    fn reel_in(&mut self, w: &mut World, r: u16, info: &PlayerInfo) {
        let now = w.time.simulation_ticks;
        if self.tick_used_last + 60 < now {
            w.emit(Cue::Sound("reelInFire"));
        }
        self.tick_used_last = now;
        let top = info.rope.is_some_and(|l| l.top);
        self.frames_since_reel_in += 1;
        if self.frames_since_reel_in < self.def.frames_between_reel_ins {
            return;
        }
        let d = self.def;
        let Some(rb) = w.physics.ropes.get_mut(r as usize) else {
            return;
        };
        let far = if top { rb.count - 1 } else { 0 };
        rb.separation = (rb.separation - d.reel_in_speed / Fix::from_int(rb.count as i32))
            .max(d.reeled_in_separation);
        let mut more = true;
        if rb.separation < d.separation_before_deletion {
            more = rb.reel_in_segment(8);
        }
        if !more {
            let seg = rb.segment[far];
            let caster = info.controller;
            let bp = w.objects.get(caster).map_or(Vec2::ZERO, |o| o.tf.position);
            let v = (seg - bp).normalized_safe() * F_0_1D;
            if !info.is_grounded
                && let Some(o) = w.objects.get_mut(caster)
            {
                o.tf.position = seg + v;
            }
            w.physics.ropes.remove(r as usize);
            self.rope = None;
            w.emit(Cue::Sound("hookshotLetGo"));
        }
        self.frames_since_reel_in = 0;
    }

    fn fire(&mut self, w: &mut World, owner: i32, info: &PlayerInfo, me: ObjId, fw: &mut Framework) {
        self.cooldown_set = false;
        w.emit(Cue::Sound("hookshotFire"));
        let a = w.player(owner).map_or(Vec2::ZERO, |p| p.inputs.aim_vector());
        let vel = if a == Vec2::ZERO { Vec2::RIGHT } else { a } * self.def.hook_speed;
        let caster = info.controller;
        let target = info.rope_target.filter(|t| w.objects.exists(*t));
        if info.use_rope_on_other_player
            && let Some(t) = target
        {
            let tpos = w.objects.get(t).map_or(Vec2::ZERO, |o| o.tf.position);
            let base = tpos + a * self.def.hook_spawn_offset;
            let tid = w.objects.get(t).map_or(0, |o| o.player_id);
            let Some((rope_obj, rb)) = spawn_rope(w, &self.def, base, tpos, info, owner) else {
                return;
            };
            self.fired_number = w.physics.ropes.get(rb as usize).map_or(0, |r| r.number);
            spawn_hook(w, &self.def, base, rb, true, vel, rope_obj, tid);
            defer_body_op(w, caster, BodyOp::ZeroVelocity);
            let had_body = with_rope_body(w, t, |w, body, cfg| {
                body.self_imposed_velocity = Vec2::ZERO;
                rope::attach_rope(w, body, cfg, None, false);
                rope::attach_rope(w, body, cfg, Some(rb), false);
            });
            if !had_body {
                // A Rock ball (`BounceBall`): the same detach / attach.
                let a = super::Interaction::AttachRope { rope: None, top: false };
                if super::interact(w, t, a).is_some() {
                    let a = super::Interaction::AttachRope { rope: Some(rb), top: false };
                    super::interact(w, t, a);
                }
            }
            self.cooldown_set = true;
            fw.cooldown = self.def.cooldown;
        } else {
            let p = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
            let base = p + a * self.def.hook_spawn_offset;
            let bp = w.objects.get(caster).map_or(Vec2::ZERO, |o| o.tf.position);
            let Some((rope_obj, rb)) = spawn_rope(w, &self.def, base, bp, info, owner) else {
                return;
            };
            self.fired_number = w.physics.ropes.get(rb as usize).map_or(0, |r| r.number);
            spawn_hook(w, &self.def, base, rb, true, vel, rope_obj, owner);
            defer_body_op(w, caster, BodyOp::ZeroVelocity);
            defer_body_op(w, caster, BodyOp::AttachRope(None, false));
            defer_body_op(w, caster, BodyOp::AttachRope(Some(rb), false));
            self.rope = Some(rb);
        }
    }
}

/// `UpdateCooldowns(body)` for a rope slot (`rope` = the caster body's rope).
pub fn update_cooldowns(w: &mut World, a: ObjId, rope: Option<RopeLink>) {
    let body_rope = rope.and_then(|l| {
        w.physics
            .ropes
            .enabled(l.rope as usize)
            .map(|r| (r.owner, r.number))
    });
    w.with_kind(a, |w, k| {
        let Kind::Ability(x) = k else {
            return;
        };
        let super::AbilityState::GrapplingHook(g) = &mut x.state else {
            return;
        };
        let owner = x.fw.owner;
        let mine = g
            .rope
            .and_then(|r| w.physics.ropes.get(r as usize))
            .map(|r| r.number);
        let lost = match body_rope {
            None => true,
            Some((o, n)) => o as i32 != owner || mine.is_none_or(|m| m != n),
        };
        if !g.cooldown_set && lost {
            g.cooldown_set = true;
            x.fw.cooldown = g.def.cooldown;
        } else if !g.cooldown_set {
            x.fw.cooldown = Fix::ZERO;
        }
    });
}

// -- deferred caster-body operations ----------------------------------------------

/// Changes to the caster's `PlayerBody` requested during an instant cast
/// (the caster's payload is taken out then); applied by the caster right
/// after the cast, before anything else reads the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BodyOp {
    ZeroVelocity,
    AttachRope(Option<u16>, bool),
}

fn defer_body_op(w: &mut World, obj: ObjId, op: BodyOp) {
    match w.objects.get(obj).map(|o| &o.kind) {
        Some(Kind::Empty) => w.ability_globals.pending_body_ops.push((obj, op)),
        Some(_) => {
            with_rope_body(w, obj, |w, body, cfg| apply_op(w, body, cfg, op));
        }
        None => {}
    }
}

fn apply_op(w: &mut World, body: &mut crate::bopl::PlayerBody, cfg: RopeCfg, op: BodyOp) {
    match op {
        BodyOp::ZeroVelocity => body.self_imposed_velocity = Vec2::ZERO,
        BodyOp::AttachRope(r, top) => rope::attach_rope(w, body, cfg, r, top),
    }
}

/// Applies the deferred operations for the bopl `me` (called with its body).
pub fn apply_body_ops(w: &mut World, me: ObjId, body: &mut crate::bopl::PlayerBody) {
    let mut i = 0;
    while i < w.ability_globals.pending_body_ops.len() {
        if w.ability_globals.pending_body_ops[i].0 == me {
            let (_, op) = w.ability_globals.pending_body_ops.remove(i);
            let cfg = RopeCfg::player(w);
            apply_op(w, body, cfg, op);
        } else {
            i += 1;
        }
    }
}

/// Runs `f` on the `PlayerBody` (and its rope values) of a bopl or of an
/// ability object whose prefab has a PlayerBody; false if it has none.
pub fn with_rope_body(
    w: &mut World,
    obj: ObjId,
    f: impl FnOnce(&mut World, &mut crate::bopl::PlayerBody, RopeCfg),
) -> bool {
    let has_body = match w.objects.get(obj).map(|o| &o.kind) {
        Some(Kind::Bopl(_)) => true,
        Some(Kind::Ability(x)) => x.body().is_some_and(|b| b.cfg.has_body),
        _ => false,
    };
    if !has_body {
        return false;
    }
    let cfg = match super::ability_body(w, obj) {
        Some(b) => b.cfg.rope_cfg(),
        None => RopeCfg::player(w),
    };
    super::with_player_body(w, obj, |w, _, body| f(w, body, cfg)).is_some()
}

// -- Rope object -------------------------------------------------------------------

/// The `Rope` component.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RopeObj {
    pub def: RopeDef,
    pub rope: Option<u16>,
    pub time_since_last_reel_out: Fix,
    pub reel_in: bool,
    pub time_passed: Fix,
    pub hook_arrived: bool,
}

/// `InstantiateFixed(ropePrefab, base)` + `Rope.Initialize(...)`.
fn spawn_rope(
    w: &mut World,
    d: &HookDef,
    base: Vec2,
    player_pos: Vec2,
    info: &PlayerInfo,
    owner: i32,
) -> Option<(ObjId, u16)> {
    let rd = d.rope;
    w.open_bracket();
    let id = w.objects.create(
        Kind::Behaviour(Box::new(Behaviour::GrapplingHook(Spawned::Rope(RopeObj {
            def: rd,
            rope: None,
            time_since_last_reel_out: Fix::ZERO,
            reel_in: false,
            time_passed: Fix::ZERO,
            hook_arrived: false,
        })))),
        rd.layer,
    );
    if let Some(o) = w.objects.get_mut(id) {
        o.visual = match rd.prefab {
            Some(p) => crate::view::Visual::Prefab {
                prefab: p as u32,
                object: 0,
            },
            None => crate::view::Visual::None,
        };
        o.add_part(PartKind::Behaviour(0), rd.hier);
    }
    w.register_all(id);
    w.close_bracket();
    w.consume_hierarchy((rd.updatables - 1).max(0));
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.position = base;
        o.tf.reset_rotation();
    }
    // Rope.Initialize
    w.physics.ropes.space = w.is_space();
    let Some(i) = w.physics.ropes.allocate(rd.nr_of_segments) else {
        // Pool exhausted (the original logs an error): no rope.
        w.destroy(id);
        return None;
    };
    w.physics.ropes.counter = w.physics.ropes.counter.wrapping_add(1);
    let number = w.physics.ropes.counter;
    {
        let r = w.physics.ropes.get_mut(i).unwrap();
        r.time_since_hook_arrived = -Fix::ONE;
        r.owner = owner as u8;
        r.number = number;
        r.mass_start = Fix::ZERO;
        r.mass_end = Fix::ZERO;
        r.detached = false;
        r.separation = d.starting_separation;
    }
    // Too many ropes: the oldest (literal search, see the spec) is disabled.
    let mut count = 0;
    let mut oldest = 0usize;
    let mut min = 10_000u32;
    for (k, r) in w.physics.ropes.bodies.iter().enumerate() {
        if r.enabled && r.owner as i32 == owner {
            count += 1;
            if r.number < min {
                oldest = k;
                min = r.number;
            }
        }
    }
    if count > rd.max_ropes_per_player
        && let Some(r) = w.physics.ropes.get_mut(oldest)
    {
        r.disabled_this_frame = true;
        r.enabled = false;
        r.number = 0;
        r.owner = 0;
    }
    let r = w.physics.ropes.get_mut(i).unwrap();
    if !info.is_grounded {
        r.time_in_air_end = Fix::from_int(100);
    }
    let n = Fix::from_int(r.count as i32);
    for j in 0..r.count {
        r.segment[j] = base + (player_pos - base) * Fix::from_int(j as i32) / n;
        r.prev[j] = r.segment[j];
    }
    if let Some(Kind::Behaviour(b)) = w.objects.get_mut(id).map(|o| &mut o.kind)
        && let Behaviour::GrapplingHook(Spawned::Rope(ro)) = b.as_mut()
    {
        ro.rope = Some(i as u16);
    }
    Some((id, i as u16))
}

fn off_screen(w: &World, p: Vec2, k: Fix) -> bool {
    p.x > BLAST_X_MAX * k
        || p.x < BLAST_X_MIN * k
        || p.y > BLAST_Y_MAX * k
        || p.y < body::water_height(w) * k
}

impl RopeObj {
    fn update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        if let Some(i) = self.rope
            && w.physics.ropes.enabled(i as usize).is_some()
        {
            self.time_since_last_reel_out += dt;
            self.time_passed += dt;
            if self.hook_arrived {
                self.reel_in = true;
            } else if self.time_passed > self.def.max_travel_time {
                if let Some(r) = w.physics.ropes.get_mut(i as usize) {
                    r.enabled = false;
                    r.disabled_this_frame = true;
                    r.detached = true;
                }
                self.rope = None;
                self.check_destroyed(w, me);
                return;
            }
            let d = self.def;
            let reel_in = self.reel_in;
            let mut reel_out = false;
            if self.time_since_last_reel_out > d.reel_out_speed && !reel_in {
                self.time_since_last_reel_out = Fix::ZERO;
                reel_out = true;
            }
            let r = w.physics.ropes.get_mut(i as usize).unwrap();
            if r.time_since_hook_arrived >= Fix::ZERO {
                r.time_since_hook_arrived += dt;
            }
            let mut stop = false;
            if !reel_in {
                let last = r.count - 1;
                let dir = (r.segment[last] - r.segment[0]).normalized_safe();
                let wob = dir.mul_by_i() * d.sprinkler_speed * dt;
                let mut num = 0;
                let mut flag = true;
                for k in 0..r.count - 1 {
                    if num >= d.segment_wave_length {
                        flag = !flag;
                        num = 0;
                    }
                    num += 1;
                    if k + d.segments_affected_by_reel_out >= r.count {
                        let s = if flag { Fix::ONE } else { -Fix::ONE } * wob;
                        let mut v = dir * d.reel_out_segment_speed * dt;
                        v += s * d.sprinkler_speed * dt;
                        r.segment[k] += v;
                    }
                }
                if reel_out {
                    let last = r.count - 1;
                    let v = r.segment[last - 1] - r.segment[last];
                    let p = r.segment[last] + v * F_0_5D;
                    if !r.spawn_additional_segment(p) {
                        stop = true;
                    }
                }
            }
            if stop {
                self.reel_in = true;
            }
            let r = w.physics.ropes.get(i as usize).unwrap();
            let (s0, sl) = (r.segment[0], r.segment[r.count - 1]);
            let one = Fix::ONE;
            let two = Fix::TWO;
            if (off_screen(w, s0, one) && off_screen(w, sl, one))
                || off_screen(w, s0, two)
                || off_screen(w, sl, two)
            {
                w.physics.ropes.remove(i as usize);
            }
        }
        self.check_destroyed(w, me);
    }

    /// `CheckIfDestroyed()`.
    fn check_destroyed(&mut self, w: &mut World, me: ObjId) {
        if !w.objects.alive(me) {
            return;
        }
        let gone = self
            .rope
            .is_none_or(|i| w.physics.ropes.enabled(i as usize).is_none());
        if gone {
            if let Some(i) = self.rope
                && let Some(r) = w.physics.ropes.get_mut(i as usize)
            {
                r.enabled = false;
                r.disabled_this_frame = true;
                r.detached = true;
            }
            w.destroy(me);
        }
    }
}

/// `(Fix)0.5` (double).
const F_0_5D: Fix = Fix(2_147_483_648);

// -- Hook and anchor ------------------------------------------------------------------

/// Where a `RopeAttachment` is anchored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Anchor {
    Platform {
        platform: ObjId,
        local: Fix,
    },
    Object {
        parent: ObjId,
        offset: Vec2,
    },
}

/// The `RopeHook` object: its `RopeAttachment` and (until it attaches) the
/// flying `RopeHook`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HookObj {
    pub def: RopeHookDef,
    pub rope: u16,
    pub rope_obj: ObjId,
    pub velocity: Vec2,
    pub end_player: i32,
    pub top: bool,
    pub time_passed: Fix,
    // RopeAttachment
    pub anchor: Anchor,
    /// The anchor's rigid body owner (platform ground body or BoplBody).
    pub body: Option<ObjId>,
    pub pull: Fix,
    pub reel_in_frame: i32,
    /// A black hole remembered by the anchor (black-hole module).
    pub black_hole: Option<ObjId>,
}

#[allow(clippy::too_many_arguments)]
fn spawn_hook(
    w: &mut World,
    d: &HookDef,
    base: Vec2,
    rope: u16,
    top: bool,
    velocity: Vec2,
    rope_obj: ObjId,
    end_player: i32,
) {
    let hd = d.hook;
    w.open_bracket();
    let id = w.objects.create(Kind::Empty, hd.layer);
    if let Some(o) = w.objects.get_mut(id) {
        o.visual = match hd.prefab {
            Some(p) => crate::view::Visual::Prefab {
                prefab: p as u32,
                object: 0,
            },
            None => crate::view::Visual::None,
        };
        o.collider = hd.collider;
        o.add_part(PartKind::Behaviour(1), hd.hier_attachment);
        o.add_part(PartKind::Collider, hd.hier_collider);
        o.add_part(PartKind::Behaviour(2), hd.hier_hook);
    }
    w.register_all(id);
    w.close_bracket();
    w.consume_hierarchy((hd.updatables - 3).max(0));
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.position = base;
        o.tf.reset_rotation();
    }
    // RopeHook.Initialize: manual init of the circle, anchor on itself.
    let hier = body::collider_hier(w, id);
    w.physics.register(&mut w.objects, id, hier);
    let mut h = HookObj {
        def: hd,
        rope,
        rope_obj,
        velocity,
        end_player,
        top,
        time_passed: Fix::ZERO,
        anchor: Anchor::Object {
            parent: id,
            offset: Vec2::ZERO,
        },
        body: None,
        pull: hd.platform_pull_str,
        reel_in_frame: 0,
        black_hole: None,
    };
    h.attach_to_object(w, id, Vec2::ZERO);
    w.put_kind(
        id,
        Kind::Behaviour(Box::new(Behaviour::GrapplingHook(Spawned::Hook(h)))),
    );
}

/// `InstantiateFixed(ropeAttachmentPrefab, pos)` + `AttachToPlatform(pos,
/// platform, rope, top)`: a stand-alone rope anchor (Platform ability).
pub fn spawn_platform_anchor(
    w: &mut World,
    prefab: usize,
    pos: Vec2,
    platform: ObjId,
    rope: u16,
    top: bool,
) -> ObjId {
    let Some(p) = w.content.prefabs.get(prefab) else {
        return ObjId::NONE;
    };
    let ac = comp(p, "RopeAttachment");
    let a = |k: &str, d: Fix| prefab::fix_field(ac, k, d);
    let def = RopeHookDef {
        prefab: Some(prefab),
        layer: p.objects.first().map_or(24, |o| o.layer),
        updatables: p.updatable_count(),
        hier_attachment: ac.and_then(|c| c.hierarchy).unwrap_or(1),
        pull_str: a("pullStr", Fix::from_int(-3)),
        platform_pull_str: a("platformPullStr", Fix::from_int(-10)),
        boulder_pull_str: a("boulderPullStr", Fix::from_int(-5)),
        platform_heaviness_scaling: a("platformHeavinessScaling", F_0_01),
        stretch_before_break: a("stretchBeforeBreak", Fix::from_int(100)),
        stretch_before_break_platform: a("stretchBeforeBreak_platform", Fix::from_int(100)),
        platform_mass01: a("platformMass01", Fix(3_865_470_464)),
        non_platform_mass: a("nonPlatformMass", F_0_5),
        ..RopeHookDef::default()
    };
    let h = HookObj {
        def,
        rope,
        rope_obj: ObjId::NONE,
        velocity: Vec2::ZERO,
        end_player: 0,
        top,
        time_passed: Fix::ZERO,
        anchor: Anchor::Object {
            parent: ObjId::NONE,
            offset: Vec2::ZERO,
        },
        body: None,
        pull: def.platform_pull_str,
        reel_in_frame: 0,
        black_hole: None,
    };
    let id = body::spawn_prefab(
        w,
        prefab,
        Kind::Behaviour(Box::new(Behaviour::GrapplingHook(Spawned::Hook(h)))),
        &[("RopeAttachment", PartKind::Behaviour(1))],
        pos,
        None,
    );
    w.with_kind(id, |w, k| {
        if let Kind::Behaviour(b) = k
            && let Behaviour::GrapplingHook(Spawned::Hook(h)) = b.as_mut()
        {
            h.attach_to_platform(w, id, pos, platform);
        }
    });
    id
}

/// `(platform, LocalPlatformPosition)` of a rope anchor attached to a
/// platform (`RopeAttachment.IsPlatformAttachment()`).
pub fn platform_anchor(w: &World, a: ObjId) -> Option<(ObjId, Fix)> {
    match &w.objects.get(a)?.kind {
        Kind::Behaviour(b) => match b.as_ref() {
            Behaviour::GrapplingHook(Spawned::Hook(h)) => match h.anchor {
                Anchor::Platform { platform, local } => Some((platform, local)),
                Anchor::Object { .. } => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// A Throw boulder takes a rope anchor along (throw.md, attach to boulder):
/// re-anchor at the boulder's closest point to `anchor − up · 100`, local
/// position 1/2, shift the first half of the rope by the displacement,
/// previous positions := current, one rope simulation step.
pub fn carry_anchor_to(w: &mut World, a: ObjId, boulder: ObjId, up: Vec2) {
    let old = w.objects.get(a).map_or(Vec2::ZERO, |o| o.tf.position);
    let pos = old - up * Fix::from_int(100);
    crate::platform::update_bounds(w, boulder);
    let rope = w.with_kind(a, |w, k| {
        if let Kind::Behaviour(b) = k
            && let Behaviour::GrapplingHook(Spawned::Hook(h)) = b.as_mut()
        {
            h.attach_to_platform(w, a, pos, boulder);
            if let Anchor::Platform { local, .. } = &mut h.anchor {
                *local = Fix::ONE / Fix::TWO;
            }
            h.reposition(w, a);
            return Some(h.rope);
        }
        None
    });
    let Some(rope) = rope else {
        return;
    };
    let new = w.objects.get(a).map_or(Vec2::ZERO, |o| o.tf.position);
    let v = new - old;
    if let Some(r) = w.physics.ropes.get_mut(rope as usize) {
        for i in 0..r.count / 2 {
            r.segment[i] += v;
        }
        for j in 0..r.count {
            r.prev[j] = r.segment[j];
        }
    }
    w.physics.simulate_single_rope(rope as usize);
}

/// The object is a boulder (tagged `boulder` in its prefab).
pub fn is_boulder(w: &World, obj: ObjId) -> bool {
    match w.objects.get(obj).map(|o| o.visual) {
        Some(crate::view::Visual::Prefab { prefab, object }) => w
            .content
            .prefabs
            .get(prefab as usize)
            .and_then(|p| p.objects.get(object as usize))
            .is_some_and(|o| o.tag == "boulder"),
        _ => false,
    }
}

/// Whether an object has a `BoplBody` (a non-trigger physics body).
fn has_bopl_body(w: &World, obj: ObjId) -> bool {
    w.objects
        .get(obj)
        .and_then(|o| o.collider)
        .is_some_and(|c| c.body.is_some())
}

fn shape_center(s: &ShapeSnap) -> Vec2 {
    match s {
        ShapeSnap::Circle(c) => c.center,
        ShapeSnap::Box(b) => b.center,
        ShapeSnap::RoundedRect(r) => r.b.center,
    }
}

impl HookObj {
    fn rope_ok(&self, w: &World) -> bool {
        w.physics.ropes.enabled(self.rope as usize).is_some()
    }

    /// `AttachToNonPlatform(parent, offset, rope, top)`.
    fn attach_to_object(&mut self, w: &mut World, parent: ObjId, offset: Vec2) {
        self.pull = self.def.platform_pull_str;
        self.anchor = Anchor::Object { parent, offset };
        self.body = has_bopl_body(w, parent).then_some(parent);
        let scale = w.objects.get(parent).map_or(Fix::ONE, |o| o.tf.scale);
        let (top, m) = (self.top, self.def.non_platform_mass);
        if let Some(r) = w.physics.ropes.get_mut(self.rope as usize) {
            r.set_time_in_air(Fix::ONE, top);
            r.set_mass01(m, top, scale);
            r.set_force(Vec2::ZERO, top);
        }
    }

    /// `AttachToPlatform(pos, platform, rope, top)`.
    fn attach_to_platform(&mut self, w: &mut World, me: ObjId, pos: Vec2, plat: ObjId) {
        self.pull = if is_boulder(w, plat) {
            self.def.boulder_pull_str
        } else {
            self.def.platform_pull_str
        };
        let local = Surface::of(w, plat).map_or(Fix::ZERO, |s| s.param_from_position(pos, Fix::ZERO));
        self.anchor = Anchor::Platform {
            platform: plat,
            local,
        };
        self.body = Some(plat);
        self.reposition(w, me);
    }

    /// `RepositionOnParent()` (platform anchors).
    fn reposition(&self, w: &mut World, me: ObjId) {
        if let Anchor::Platform { platform, local } = self.anchor
            && w.objects.exists(platform)
            && !w.objects.get(platform).is_some_and(|o| o.destroyed)
            && w
                .objects
                .get(platform)
                .and_then(|o| o.collider)
                .is_some_and(|c| c.initialized)
            && let Some(s) = Surface::of(w, platform)
        {
            let p = s.position_from_param(local, Fix::ZERO);
            if let Some(o) = w.objects.get_mut(me) {
                o.tf.position = p;
            }
        }
    }

    fn place_on_parent(&self, w: &mut World, me: ObjId) {
        if let Anchor::Object { parent, offset } = self.anchor
            && let Some(t) = w.objects.get(parent).map(|o| o.tf)
        {
            let p = t.position + t.up * offset.y * t.scale + t.right * offset.x * t.scale;
            if let Some(o) = w.objects.get_mut(me) {
                o.tf.position = p;
            }
        }
    }

    /// `Deattach()`.
    fn deattach(&mut self, w: &mut World, me: ObjId) {
        let top = self.top;
        if let Some(r) = w.physics.ropes.get_mut(self.rope as usize) {
            r.set_mass01(F_0_5D, top, Fix::ONE);
            r.set_time_in_air(Fix::from_int(100), top);
            r.set_force(Vec2::ZERO, top);
            r.dettach(top);
        }
        w.destroy(me);
    }

    /// `RopeAttachment.UpdateSim`.
    fn attachment_update(&mut self, w: &mut World, me: ObjId) {
        if !self.rope_ok(w) {
            w.destroy(me);
            return;
        }
        let live = |w: &World, o: ObjId| {
            w.objects.get(o).is_some_and(|ob| !ob.tf.destroyed) && w.objects.active_in_hierarchy(o)
        };
        match self.anchor {
            Anchor::Platform { platform, .. } => {
                if !(w.objects.alive(platform) && w.objects.active_in_hierarchy(platform)) {
                    self.deattach(w, me);
                    return;
                }
            }
            Anchor::Object { parent, .. } => {
                if !live(w, parent) {
                    self.deattach(w, me);
                    return;
                }
            }
        }
        let is_platform = matches!(self.anchor, Anchor::Platform { .. });
        let top = self.top;
        let r = w.physics.ropes.get(self.rope as usize).unwrap();
        let (own, next) = if top { (0, 1) } else { (r.count - 1, r.count - 2) };
        if r.enabled && r.hook_arrived() && !r.detached && !w.is_time_stopped() {
            if let Some(b) = self.body.filter(|b| w.objects.exists(*b)) {
                let k = if is_platform { self.pull } else { self.def.pull_str };
                let f = (r.segment[own] - r.segment[next]) * k;
                let at = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
                body::add_force_at_position(w, b, f, at, false);
            }
            // A remembered black hole reels the rope in and finally
            // swallows it (grappling-hook.md, anchor update step 4).
            if let Some(bh) = self.black_hole.filter(|b| w.objects.exists(*b)) {
                if super::black_hole::mass(w, bh).is_some_and(|m| m > Fix::ZERO) {
                    self.reel_in_frame += 1;
                    let f = self.reel_in_frame;
                    let count = w.physics.ropes.get(self.rope as usize).map_or(0, |r| r.count);
                    if (f > 60 && f % 3 == 0) || (f <= 60 && f % 8 == 0) || count < 8 {
                        let reeled = w
                            .physics
                            .ropes
                            .get_mut(self.rope as usize)
                            .is_some_and(|r| r.reel_in_segment(3));
                        if !reeled {
                            let c = w.objects.get(bh).map_or(Vec2::ZERO, |o| o.tf.position);
                            if let Some(o) = w.objects.get_mut(me) {
                                o.tf.position = c;
                            }
                            if let Some(r) = w.physics.ropes.get_mut(self.rope as usize) {
                                let opposite = if top { r.count - 1 } else { 0 };
                                r.segment[opposite] = c;
                            }
                        }
                    }
                } else {
                    // Mass ≤ 0: forget it and Deattach; the update goes on
                    // (literal: steps 5–6 run on the destroyed anchor).
                    self.black_hole = None;
                    self.deattach(w, me);
                }
            } else {
                self.black_hole = None;
            }
        }
        if is_platform {
            let inv = self
                .body
                .and_then(|b| w.physics.body(b))
                .map_or(Fix::ZERO, |b| b.inverse_mass);
            let s = self.def.platform_heaviness_scaling / inv;
            let m = self.def.platform_mass01;
            if let Some(r) = w.physics.ropes.get_mut(self.rope as usize) {
                r.set_time_in_air(Fix::ZERO, top);
                r.set_mass01(m, top, s);
                r.set_force(Vec2::ZERO, top);
            }
            self.reposition(w, me);
            let p = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
            if let Some(r) = w.physics.ropes.get_mut(self.rope as usize) {
                let own = r.end(top);
                r.segment[own] = p;
            }
        } else {
            self.place_on_parent(w, me);
        }
        let lim = if is_platform {
            self.def.stretch_before_break_platform
        } else {
            self.def.stretch_before_break
        };
        if let Some(r) = w.physics.ropes.get(self.rope as usize) {
            let mut i = 0;
            while i + 2 < r.count {
                if (r.segment[i] - r.segment[i + 1]).sqr_magnitude() > lim {
                    self.deattach(w, me);
                    break;
                }
                i += 1;
            }
        }
    }

    /// `RopeAttachment.LateUpdateSim`.
    fn attachment_late_update(&mut self, w: &mut World, me: ObjId) {
        match self.anchor {
            Anchor::Platform { .. } => self.reposition(w, me),
            Anchor::Object { .. } => self.place_on_parent(w, me),
        }
        let p = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        if let Some(r) = w.physics.ropes.get_mut(self.rope as usize) {
            let own = r.end(self.top);
            r.segment[own] = p;
        }
    }

    /// `RopeHook.UpdateSim`.
    fn hook_update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        let top = self.top;
        let d = self.def;
        if let Some(r) = w.physics.ropes.get_mut(self.rope as usize) {
            r.set_time_in_air(Fix::ZERO, top);
            let mut f = d.segment_speed_falloff01;
            let mut i = 2;
            while i < r.count && i < d.segments_affected_by_speed {
                let k = r.count - i;
                r.segment[k] += self.velocity * dt * f;
                f = f * d.segment_speed_falloff01;
                i += 1;
            }
        }
        self.time_passed += dt;
        if self.time_passed > d.time_until_fall {
            w.destroy(self.rope_obj);
            w.destroy(me);
            w.physics.ropes.remove(self.rope as usize);
        } else if let Some(o) = w.objects.get_mut(me) {
            // `hitbox.position += v · dt` (the circle's transform).
            o.tf.position += self.velocity * dt;
        }
    }

    /// `RopeHook.OnCollide`.
    fn on_collide(&mut self, w: &mut World, me: ObjId, c: &Contact) {
        if !w.objects.alive(me)
            || !w
                .objects
                .get(me)
                .and_then(|o| o.part(PartKind::Behaviour(2)))
                .is_some_and(|p| !p.destroyed)
            || !self.rope_ok(w)
        {
            return;
        }
        let mut p = shape_center(&c.collider);
        let hook_pos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        if matches!(c.layer, 16 | 21 | 11) {
            if !c.collider.contains(hook_pos) {
                return;
            }
            p = hook_pos;
        }
        if c.layer == 8 {
            let pid = w.objects.get(c.other).map_or(0, |o| o.player_id);
            let owner = w
                .physics
                .ropes
                .get(self.rope as usize)
                .map_or(0, |r| r.owner as i32);
            if pid == 0 || pid == self.end_player || pid == owner {
                return;
            }
        }
        w.emit(Cue::Effect {
            kind: crate::view::EffectKind::Ability(HIT_PARTICLE),
            position: c.contact_point,
            player_id: 0,
        });
        if let Some(Kind::Behaviour(b)) = w.objects.get_mut(self.rope_obj).map(|o| &mut o.kind)
            && let Behaviour::GrapplingHook(Spawned::Rope(r)) = b.as_mut()
        {
            r.hook_arrived = true;
        }
        let rope = self.rope;
        let arrived = |w: &mut World| {
            if let Some(r) = w.physics.ropes.get_mut(rope as usize)
                && r.time_since_hook_arrived < Fix::ZERO
            {
                r.time_since_hook_arrived = Fix::ZERO;
            }
        };
        let start = |w: &mut World| {
            if let Some(r) = w.physics.ropes.get_mut(rope as usize) {
                r.mass_start = Fix::ONE;
                r.force_start = Vec2::ZERO;
            }
        };
        let rel = |w: &World, p: Vec2| {
            let t = w.objects.get(c.other).map(|o| o.tf).unwrap_or_default();
            (p - t.position).complex_mul(Vec2::from_angle(-t.rotation)) / t.scale
        };
        match c.layer {
            11 => {
                start(w);
                arrived(w);
                w.emit(Cue::Sound("hookshotOnHit"));
                if Surface::of(w, c.other).is_some() {
                    self.attach_to_platform(w, me, hook_pos, c.other);
                }
            }
            8 => {
                w.emit(Cue::Sound("hookshotHitPlayer"));
                let number = w.physics.ropes.get(rope as usize).map_or(0, |r| r.number);
                let had_body = with_rope_body(w, c.other, |w, body, cfg| {
                    if let Some(l) = body.rope
                        && w
                            .physics
                            .ropes
                            .get(l.rope as usize)
                            .is_some_and(|r| r.number != number)
                    {
                        if let Some(r) = w.physics.ropes.get_mut(l.rope as usize) {
                            r.dettach(true);
                            r.dettach(false);
                        }
                        body.rope = None;
                    }
                    rope::attach_rope(w, body, cfg, Some(rope), true);
                });
                if had_body {
                    arrived(w);
                    w.destroy(me);
                    return;
                }
                start(w);
                let off = rel(w, p);
                self.attach_to_object(w, c.other, off);
                arrived(w);
            }
            16 | 21 | 9 | 10 | 23 | 26 => {
                start(w);
                w.emit(Cue::Sound("hookshotOnHit"));
                let off = rel(w, p);
                self.attach_to_object(w, c.other, off);
                if c.layer == 23 {
                    self.black_hole = Some(c.other);
                }
                arrived(w);
            }
            _ => return,
        }
        // The hitbox and the RopeHook behaviour are removed; the object stays
        // as the anchor.
        w.destroy_part(me, PartKind::Collider);
        w.destroy_part(me, PartKind::Behaviour(2));
    }
}

/// `EffectKind::Ability` code of the hook's hit particle.
pub const HIT_PARTICLE: u16 = 0x2701;

impl SpawnedImpl for Spawned {
    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        match self {
            Spawned::Rope(r) => {
                if !late && part == PartKind::Behaviour(0) {
                    r.update(w, me, dt);
                }
            }
            Spawned::Hook(h) => match (part, late) {
                (PartKind::Behaviour(1), false) => h.attachment_update(w, me),
                (PartKind::Behaviour(1), true) => h.attachment_late_update(w, me),
                (PartKind::Behaviour(2), false) => h.hook_update(w, me, dt),
                _ => {}
            },
        }
    }

    fn on_contact(&mut self, w: &mut World, me: ObjId, c: &Contact) {
        if let Spawned::Hook(h) = self {
            h.on_collide(w, me, c);
        }
    }
}

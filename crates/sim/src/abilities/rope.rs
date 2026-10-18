//! Rope coupling on bodies (abilities/grappling-hook.md "Rope coupling on
//! bodies", player.md §6.8–§6.9): the `PlayerBody` / `PlayerPhysics` parts
//! of the rope system, shared by the bopl and ability bodies. The rope pool
//! and simulation are in `physics/ropes.rs`; rope objects, hooks and anchors
//! in `grappling_hook.rs`.

use crate::bopl::{PlayerBody, PlayerPhysics, RopeLink};
use crate::consts::{F_0_5, F_0_07D, F_0_9D, FIXED_DT};
use crate::fix::Fix;
use crate::object::ObjId;
use crate::vec2::Vec2;
use crate::view::Cue;
use crate::world::World;

/// Rope-relevant prefab values of a body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RopeCfg {
    pub rope_player_mass01: Fix,
    pub ignore_gravity_when_roped: bool,
    pub cannot_be_ungrounded_by_ropes: bool,
}

impl RopeCfg {
    /// The bopl's values.
    pub fn player(w: &World) -> RopeCfg {
        let p = w.content.player();
        RopeCfg {
            rope_player_mass01: p.rope_player_mass01,
            ignore_gravity_when_roped: p.ignore_gravity_when_roped,
            cannot_be_ungrounded_by_ropes: p.cannot_be_ungrounded_by_ropes,
        }
    }

    /// `RopePlayerMass01`, 0.5 if zero.
    fn mass(&self) -> Fix {
        if self.rope_player_mass01 == Fix::ZERO {
            F_0_5
        } else {
            self.rope_player_mass01
        }
    }
}

/// The body holds an enabled rope.
pub fn enabled(w: &World, link: Option<RopeLink>) -> bool {
    link.is_some_and(|l| w.physics.ropes.enabled(l.rope as usize).is_some())
}

/// `segment[end] = p` for a body's rope end.
pub fn pin_end(w: &mut World, l: RopeLink, p: Vec2) {
    if let Some(r) = w.physics.ropes.get_mut(l.rope as usize) {
        let k = r.end(l.top);
        r.segment[k] = p;
    }
}

fn remember(body: &mut PlayerBody) {
    body.prev_player_vel = body.velocity();
    body.prev_imposed_vel = body.self_imposed_velocity;
    body.prev_external_vel = body.external_velocity;
}

/// `PlayerBody.AttachRope(rope, top)`: the **old** rope is modified at the
/// `top` argument's end (literal).
pub fn attach_rope(
    w: &mut World,
    body: &mut PlayerBody,
    cfg: RopeCfg,
    new: Option<u16>,
    top: bool,
) {
    if let Some(old) = body.rope {
        let g = w.physics.params.rope_gravity;
        if let Some(r) = w.physics.ropes.get_mut(old.rope as usize) {
            let f = if cfg.ignore_gravity_when_roped {
                Vec2::ZERO
            } else {
                Vec2::DOWN * g
            };
            r.set_force(f, top);
            r.set_time_in_air(Fix::from_int(100), top);
            if new.is_none() {
                r.dettach(top);
            } else {
                r.set_mass01(cfg.mass(), top, Fix::ONE);
            }
        }
    }
    remember(body);
    body.rope = new.map(|rope| RopeLink { rope, top });
}

/// `PlayerBody.UpdateSim` up to the external-velocity decay. Returns true
/// if the body was integrated here (roped); otherwise the caller integrates.
pub fn body_update(
    w: &mut World,
    me: ObjId,
    owner: i32,
    body: &mut PlayerBody,
    phys: Option<&PlayerPhysics>,
    cfg: RopeCfg,
    dt: Fix,
) -> bool {
    if let Some(l) = body.rope
        && let Some(r) = w.physics.ropes.enabled(l.rope as usize)
    {
        let airborne = match phys {
            Some(p) => !p.is_grounded && !p.jumped_this_frame,
            None => true,
        };
        let jump = w
            .player(owner)
            .is_some_and(|p| p.inputs.jump_down_this_frame);
        if w.player(owner).is_some_and(|p| p.id == r.owner as i32) && airborne && jump {
            let k = r.end(l.top);
            body.self_imposed_velocity = (r.segment[k] - r.prev[k]) / FIXED_DT;
            attach_rope(w, body, cfg, None, l.top);
            w.emit(Cue::Sound("hookshotLetGo"));
        }
    }
    if let Some(l) = body.rope
        && w.physics.ropes.enabled(l.rope as usize).is_none()
    {
        body.rope = None;
    }
    let Some(l) = body.rope else {
        return false;
    };
    let fdt = w.fixed_delta_time(owner, dt);
    let mut pos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position) + body.velocity() * fdt;
    let grounded = phys.is_some_and(|p| p.is_grounded);
    let (detached, arrived) = match w.physics.ropes.get(l.rope as usize) {
        Some(r) => (r.detached, r.hook_arrived()),
        None => (false, false),
    };
    if detached {
        pin_end(w, l, pos);
        attach_rope(w, body, cfg, None, l.top);
    } else if let Some(r) = w.physics.ropes.get_mut(l.rope as usize) {
        let k = r.end(l.top);
        if !arrived {
            r.segment[k] = pos;
            r.set_time_in_air(Fix::ZERO, l.top);
            r.set_mass01(Fix::ONE, l.top, Fix::ONE);
            r.set_force(Vec2::ZERO, l.top);
        } else if grounded {
            let m = Fix::ONE.lerp(F_0_9D, r.time_since_hook_arrived.clamp01());
            r.set_time_in_air(Fix::ZERO, l.top);
            r.set_mass01(m, l.top, Fix::ONE);
            r.set_force(Vec2::ZERO, l.top);
        } else {
            let air = if l.top {
                r.time_in_air_start += dt;
                r.time_in_air_start
            } else {
                r.time_in_air_end += dt;
                r.time_in_air_end
            };
            r.set_mass01(cfg.mass(), l.top, Fix::ONE);
            let mut wgt = (air * Fix::TWO).clamp01();
            if cfg.ignore_gravity_when_roped {
                wgt = Fix::ONE - cfg.rope_player_mass01;
            }
            let v = r.segment[k] - pos;
            let len = v.magnitude();
            if len != Fix::ZERO {
                let c = r.separation - len;
                let u = v / len;
                r.segment[k] += c * u * (Fix::ONE - wgt);
                pos -= c * u * wgt;
            }
        }
    }
    if let Some(o) = w.objects.get_mut(me) {
        o.tf.position = pos;
    }
    remember(body);
    true
}

/// `PlayerBody.LateUpdateSim` rope part (stretch break, end force).
pub fn body_late_update(w: &mut World, body: &mut PlayerBody, cfg: RopeCfg) {
    let Some(l) = body.rope else {
        return;
    };
    let Some(r) = w.physics.ropes.enabled(l.rope as usize) else {
        body.rope = None;
        return;
    };
    let max = w.physics.params.max_stretch_length;
    let mut i = 0;
    while i + 2 < r.count {
        if (r.segment[i] - r.segment[i + 1]).sqr_magnitude() > max {
            attach_rope(w, body, cfg, None, l.top);
            break;
        }
        i += 1;
    }
    let Some(l) = body.rope else {
        return;
    };
    let pg = w.physics.params.player_gravity;
    let mut a = body.self_imposed_velocity - body.prev_imposed_vel;
    a += Vec2::DOWN * pg;
    let b = body.external_velocity - body.prev_external_vel;
    if cfg.ignore_gravity_when_roped {
        a.y = Fix::ZERO;
    }
    let f = a + b + body.extra_swing_force;
    if let Some(r) = w.physics.ropes.get_mut(l.rope as usize)
        && r.enabled
    {
        r.set_force(f, l.top);
    }
    body.extra_swing_force = Vec2::ZERO;
}

/// `PlayerPhysics.LateUpdateSim` grounded rope part, before the end is
/// pinned: `None` without an arrived rope, else whether the body must
/// unground (`UnGround(true, false)`); the caller then pins the end.
pub fn grounded_late_update(
    w: &mut World,
    body: &mut PlayerBody,
    pos: Vec2,
    normal: Vec2,
    cfg: RopeCfg,
) -> Option<bool> {
    let l = body.rope?;
    let r = w.physics.ropes.enabled(l.rope as usize)?;
    if !r.hook_arrived() {
        return None;
    }
    let k = r.end(l.top);
    let v = r.segment[k] - pos;
    if v.sqr_magnitude() >= F_0_07D {
        body.external_velocity += v * w.physics.params.grounded_player_rope_str;
    }
    Some(
        normal.dot(body.external_velocity) > Fix::from_int(30)
            && !cfg.cannot_be_ungrounded_by_ropes,
    )
}

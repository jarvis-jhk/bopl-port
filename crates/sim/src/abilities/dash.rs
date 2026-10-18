//! Dash (`docs/spec/abilities/dash.md`).
//!
//! Components in update order: Dash 1, Ability 2, PlayerPhysics 3,
//! PlayerCollision 4, DestroyIfOutsideSceneBounds 5, PlayerBody 6,
//! DPhysicsBox 7.

use super::ability_body::{self as body, AbilityBody};
use super::prefab::{self, root_component};
use super::prelude::*;
use crate::data::SpriteId;
use crate::physics::RayHit;
use crate::surface::Surface;

/// Prefab constants of the `Dash` component.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DashDef {
    pub pause_length: Fix,
    pub dash_length: Fix,
    pub fixed_updates_in_dash: i32,
    pub leave_speed: Fix,
    pub colliders: u32,
    pub anticipation_sprite: SpriteId,
}

/// Module state (one per ability object).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Dash {
    pub def: DashDef,
    pub body: AbilityBody,
    pub in_dash: bool,
    pub time_passed: Fix,
    pub updates_passed_in_dash: i32,
    /// `inputVector`: the dash direction.
    pub dir: Vec2,
    pub hit_ground: Option<ObjId>,
    pub hit_ground_local: Fix,
    /// Body sprite shown (else the smear).
    pub sprite_shown: bool,
}

/// Objects this module spawns.
pub type Spawned = NoSpawn;

impl AbilityImpl for Dash {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let c = root_component(&w.content, def, "Dash");
        let f = |k: &str, d: Fix| prefab::fix_field(c, k, d);
        let sprite = c
            .and_then(|c| c.fields.reference("anticipationSprite"))
            .map_or(SpriteId::NONE, |s| w.content.sprite(s));
        Dash {
            def: DashDef {
                pause_length: f("pauseLength", Fix(42_949_672)),
                dash_length: f("dashLength", Fix::from_int(13)),
                fixed_updates_in_dash: prefab::int_field(c, "FixedUpdatesInDash", 2) as i32,
                leave_speed: f("leaveSpeed", Fix::from_int(60)),
                colliders: prefab::int_field(c, "colliders", 75_827_968) as u32,
                anticipation_sprite: sprite,
            },
            body: AbilityBody::new(w, def),
            sprite_shown: true,
            ..Dash::default()
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
        self.sprite_shown = true;
        self.updates_passed_in_dash = 0;
        self.body.sync_physics_to(cx.w, cx.me, &info);
        cx.w.physics.update_physics_positions(&cx.w.objects, cx.me);
        if let Some(o) = cx.w.objects.get_mut(cx.me) {
            o.tf.set_rotation(Fix::ZERO);
        }
        self.in_dash = false;
        self.time_passed = Fix::ZERO;
        self.hit_ground = None;
        // readyDash
        self.body.unground(cx.w, cx.me, true, false);
        self.dir = aim(cx.w, cx.owner());
        self.body.body.self_imposed_velocity = Vec2::ZERO;
        self.body.body.external_velocity = Vec2::ZERO;
        cx.w.emit(Cue::Sound("dash"));
    }

    fn tick(&mut self, cx: &mut Cx<'_>, dt: Fix, late: bool) {
        let me = cx.me;
        if late {
            self.body.physics_late_update(cx.w, me);
            self.body.body_late_update(cx.w, me);
            return;
        }
        self.dash_update(cx, dt);
        if !cx.running() {
            return;
        }
        cx.ability_update();
        self.body.physics_update(cx.w, me, dt);
        AbilityBody::collision_update(cx.fw);
        if let Some(rec) = self.body.bounds_update(cx.w, me) {
            self.exit(cx, rec);
            return;
        }
        self.body.body_update(cx.w, me, dt);
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
            self.def.anticipation_sprite
        } else {
            SpriteId::NONE
        }
    }

    /// The anticipation sprite, then the smear child (prefab object 1)
    /// stretched along the dash: local y scale = distance / (4.1 · scale),
    /// `params[1]` = the smear shader's `_Scale.y` (y scale · 0.2).
    fn look(&self, w: &World, me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        use crate::view::PartLook;
        l.sprite = self.def.anticipation_sprite;
        if self.sprite_shown {
            l.parts.push(PartLook::hidden(1));
            return;
        }
        l.hidden = true;
        let from = AbilityBody::pos(w, me);
        let dist = match self.hit_ground.filter(|g| w.objects.exists(*g)) {
            Some(g) => Surface::of(w, g)
                .map(|s| {
                    (s.position_from_param(self.hit_ground_local, self.body.physics.radius) - from)
                        .magnitude()
                })
                .unwrap_or(self.def.dash_length),
            None => {
                let mut to = from + self.dir * self.def.dash_length;
                let wh = body::water_height(w);
                if to.y <= wh {
                    to.y = wh;
                }
                (to - from).magnitude()
            }
        };
        let scale = AbilityBody::scale(w, me);
        let y = dist * Fix::from_int(10) / (Fix::from_int(41) * scale);
        let mut p = PartLook::new(1).posed(Vec2::ZERO, Fix::ZERO, Vec2::new(Fix::ONE, y));
        p.params[1] = y / Fix::from_int(5);
        l.parts.push(p);
    }
}

fn aim(w: &World, owner: i32) -> Vec2 {
    w.player(owner)
        .map_or(Vec2::ZERO, |p| p.inputs.aim_vector())
}

impl Dash {
    /// `Dash.UpdateSim`.
    fn dash_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let owner = cx.owner();
        self.time_passed += cx.w.fixed_delta_time(owner, dt);
        if self.time_passed < self.def.pause_length {
            let v = aim(cx.w, owner);
            if v.magnitude() > DEADZONE {
                self.dir = v;
            }
        } else if !self.in_dash {
            self.in_dash = true;
            self.perform_dash(cx);
        } else {
            if self.updates_passed_in_dash > self.def.fixed_updates_in_dash {
                self.sprite_shown = true;
                let me = cx.me;
                match self.hit_ground.filter(|g| cx.w.objects.exists(*g)) {
                    Some(g) => {
                        let r = self.body.physics.radius;
                        if let Some(s) = Surface::of(cx.w, g) {
                            let p = s.position_from_param(self.hit_ground_local, r);
                            AbilityBody::set_pos(cx.w, me, p);
                        }
                        let p = AbilityBody::pos(cx.w, me);
                        self.body.attach_to_ground(cx.w, me, g, p);
                        self.exit(cx, AbilityExitInfo::default());
                    }
                    None => {
                        let from = AbilityBody::pos(cx.w, me);
                        let mut to = from + self.dir * self.def.dash_length;
                        let wh = body::water_height(cx.w);
                        if to.y < wh {
                            let t = (wh - from.y) / self.dir.y;
                            to.y = wh;
                            to.x = from.x + self.dir.x * t;
                        }
                        AbilityBody::set_pos(cx.w, me, to);
                        self.exit(cx, AbilityExitInfo::default());
                    }
                }
            }
            self.updates_passed_in_dash += 1;
        }
    }

    /// `performDash()`.
    fn perform_dash(&mut self, cx: &mut Cx<'_>) {
        let me = cx.me;
        if self.dir.magnitude() < DEADZONE {
            self.dir = Vec2::UP;
        }
        self.dir = self.dir.normalized();
        let rot = rotation_from_direction(self.dir, Vec2::UP);
        if let Some(o) = cx.w.objects.get_mut(me) {
            o.tf.set_rotation(rot);
        }
        self.updates_passed_in_dash = 0;
        self.body.body.self_imposed_velocity = Vec2::ZERO;
        self.body.body.external_velocity = Vec2::ZERO;
        self.sprite_shown = false;
        let origin = AbilityBody::pos(cx.w, me);
        let mut hits: Vec<RayHit> = Vec::new();
        cx.w.physics.raycast_all(
            &cx.w.objects,
            origin,
            self.dir,
            self.def.dash_length,
            self.def.colliders,
            &mut hits,
            Some(me),
        );
        let my_hier = body::collider_hier(cx.w, me);
        for h in hits {
            if h.layer == 11 {
                self.hit_ground = Some(h.obj);
                if let Some(s) = Surface::of(cx.w, h.obj) {
                    let q = s.closest_point(h.near_pos);
                    self.hit_ground_local = s.param_from_position(q, self.body.physics.radius);
                }
                return;
            }
            let v = h.near_pos - origin;
            let normal = v.normalized_safe();
            // The hit object is told it was touched by the dash box.
            if let Some(snap) = body::shape_snap(cx.w, me) {
                body::touch(
                    cx.w,
                    Contact {
                        me: h.obj,
                        other: me,
                        other_hier: my_hier,
                        my_hier: h.hierarchy,
                        collider: snap,
                        layer: 8,
                        impact_velocity: v,
                        contact_point: h.near_pos,
                        normal,
                        penetration: Fix::ONE,
                    },
                );
            }
            // Does the dasher die from it? (body temporarily at the hit.)
            let saved = AbilityBody::pos(cx.w, me);
            AbilityBody::set_pos(cx.w, me, h.near_pos);
            let c = Contact {
                me,
                other: h.obj,
                other_hier: h.hierarchy,
                my_hier,
                collider: body::shape_snap(cx.w, h.obj).unwrap_or(crate::physics::ShapeSnap::Circle(
                    crate::physics::shapes::Circle {
                        center: h.near_pos,
                        radius: Fix::ZERO,
                        layer: h.layer,
                    },
                )),
                layer: h.layer,
                impact_velocity: Vec2::ZERO,
                contact_point: h.near_pos,
                normal,
                penetration: Fix::ZERO,
            };
            if let Some((killer, cause)) = self.body.kill_player_on_collision(cx.w, me, cx.fw, &c)
                && self.kill(cx, killer, true, false, cause)
            {
                return;
            }
            AbilityBody::set_pos(cx.w, me, saved);
        }
        self.hit_ground = None;
    }

    /// `Dash.ExitAbility(record)`.
    fn exit(&mut self, cx: &mut Cx<'_>, mut rec: AbilityExitInfo) {
        let me = cx.me;
        if let Some(l) = self.body.rope() {
            let p = AbilityBody::pos(cx.w, me);
            let f = self.dir * self.def.leave_speed * Fix::from_int(10);
            if let Some(r) = cx.w.physics.ropes.get_mut(l.rope as usize) {
                r.set_force(f, l.top);
                if l.top {
                    r.segment[0] = p;
                } else {
                    let n = r.count;
                    let step = (p - r.segment[0]) / Fix::from_int(n as i32);
                    for i in 1..n {
                        r.segment[i] = r.segment[0] + step * Fix::from_int(i as i32);
                    }
                    for j in 0..n {
                        r.prev[j] = r.segment[j];
                    }
                }
            }
        }
        self.sprite_shown = true;
        rec.grounded_speed = Fix::ZERO;
        rec.facing_right = self.dir.dot(Vec2::RIGHT) <= Fix::ZERO;
        rec.self_imposed_velocity = self.dir * self.def.leave_speed;
        if cx.w.is_space() {
            rec.self_imposed_velocity = rec.self_imposed_velocity * F_0_5;
        }
        rec.external_velocity = Vec2::ZERO;
        rec.is_grounded = false;
        rec.ground = self.body.physics.attached_ground;
        rec.position = AbilityBody::pos(cx.w, me);
        rec.radius = self.body.physics.radius;
        rec.grounded_local_position = self.body.physics.local_platform_pos;
        cx.exit_body(&mut self.body, rec);
    }
}

/// `Tools.GetRotationFromDirection(dir, up)`.
pub fn rotation_from_direction(dir: Vec2, up: Vec2) -> Fix {
    let r = dir.normalized().dot(up.normalized()).acos();
    if dir.x > Fix::ZERO {
        Fix::PI_TIMES_2 - r
    } else {
        r
    }
}

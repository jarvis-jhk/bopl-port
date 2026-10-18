//! Roll (`docs/spec/abilities/roll.md`).
//!
//! Components in update order: Ability 1, SpriteAnimator 2, PlayerPhysics 3,
//! Roll 4, DestroyIfOutsideSceneBounds 5, PlayerCollision 6, DPhysicsCircle
//! 7, PlayerBody 8 (child RollWave animator 9: presentation).

use std::sync::Arc;

use super::ability_body::{self as body, AbilityBody};
use super::prefab::{self, root_component};
use super::prelude::*;
use crate::curve::Curve;
use crate::data::{AnimRef, SpriteId};
use crate::physics::ShapeSnap;
use crate::physics::shapes::Circle;

/// Prefab values of the `Roll` component.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RollDef {
    pub charge_time: Fix,
    pub dash_time: Fix,
    pub max_speed: Fix,
    pub move_collision_density: Fix,
    pub collides_with: u32,
    pub short_jump_str: Fix,
    pub speed_curve: Arc<Curve>,
    pub anim_startup: Option<AnimRef>,
    pub anim_loop: Option<AnimRef>,
    pub anim_fast: Option<AnimRef>,
    pub anim_ball: Option<AnimRef>,
}

/// `(Fix)0.2` (double).
const F_0_2D: Fix = Fix(858_993_459);
/// `(Fix)0.75f`.
const F_0_75: Fix = Fix(3_221_225_472);
/// Sweep slots.
const SWEEP: usize = 30;
/// Hits per sweep position.
const HITS: usize = 10;

/// Module state (one per ability object).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Roll {
    pub def: RollDef,
    pub body: AbilityBody,
    pub anim: SpriteAnimator,
    pub is_charging: bool,
    pub waiting_to_release: bool,
    pub has_charged: bool,
    pub is_short_jumping: bool,
    pub jump_button_has_been_up: bool,
    pub seconds_charged: Fix,
    pub time_since_release: Fix,
    pub top_speed: Fix,
    /// The component's own grounded speed.
    pub roll_speed: Fix,
    pub moving_right: bool,
}

/// Objects this module spawns.
pub type Spawned = NoSpawn;

fn anim_data(w: &World, r: Option<&str>) -> Option<u16> {
    let r = r?;
    w.content
        .anim_data
        .iter()
        .position(|a| a.id == r)
        .map(|i| i as u16)
}

impl AbilityImpl for Roll {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let c = root_component(&w.content, def, "Roll");
        let f = |k: &str, d: Fix| prefab::fix_field(c, k, d);
        let data = anim_data(w, c.and_then(|c| c.fields.reference("animData")));
        let a = |n: &str| data.and_then(|d| w.content.anim_ref(d, n));
        Roll {
            def: RollDef {
                charge_time: f("chargeTime", Fix::ONE),
                dash_time: f("dashTime", Fix::TWO),
                max_speed: f("MaxSpeed", Fix::from_int(500)),
                move_collision_density: f("MoveCollisionDensity", Fix::TWO),
                collides_with: prefab::int_field(c, "collidesWith", 331_520) as u32,
                short_jump_str: f("shortJumpStr", Fix::from_int(10)),
                speed_curve: prefab::curve_field(c, "speedCurve"),
                anim_startup: a("startup"),
                anim_loop: a("loop"),
                anim_fast: a("rollFast"),
                anim_ball: a("ball"),
            },
            body: AbilityBody::new(w, def),
            ..Roll::default()
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
        self.body.squishable = true;
        self.seconds_charged = Fix::ZERO;
        self.waiting_to_release = false;
        self.moving_right = info.facing_right;
        if let Some(o) = cx.w.objects.get_mut(me) {
            o.tf.set_rotation(info.rotation);
        }
        self.is_charging = true;
        self.is_short_jumping = false;
        self.has_charged = false;
        self.jump_button_has_been_up = false;
        self.top_speed = Fix::ZERO;
        self.roll_speed = Fix::ZERO;
        self.time_since_release = Fix::ZERO;
        let c = cx.w.content.clone();
        if let (Some(s), Some(l)) = (self.def.anim_startup, self.def.anim_loop) {
            self.anim.begin_then_play(&c, s, l, None);
        }
        cx.w.emit(Cue::Sound("rollCharge"));
        self.body.sync_physics_to(cx.w, me, &info);
        cx.w.physics.update_physics_positions(&cx.w.objects, me);
        self.body.body.external_velocity = info.external_velocity * F_0_2D;
        if info.is_grounded {
            self.body.body.self_imposed_velocity = Vec2::ZERO;
        } else {
            let y = self.body.body.self_imposed_velocity.y;
            self.body.body.self_imposed_velocity = Vec2::new(Fix::ZERO, y);
        }
        cx.fw.kill_players_on_contact = false;
    }

    fn tick(&mut self, cx: &mut Cx<'_>, dt: Fix, late: bool) {
        let me = cx.me;
        if late {
            self.body.physics_late_update(cx.w, me);
            self.body.body_late_update(cx.w, me);
            return;
        }
        cx.ability_update();
        let _ = body::advance_anim(cx.w, cx.owner(), &mut self.anim);
        self.body.physics_update(cx.w, me, dt);
        self.roll_update(cx, dt);
        if !cx.running() {
            return;
        }
        if let Some(rec) = self.body.bounds_update(cx.w, me) {
            self.exit(cx, rec);
            return;
        }
        AbilityBody::collision_update(cx.fw);
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

    /// The roll frame; the wave under the ball (object 1) while grounded
    /// and not dashing, the player-coloured slime trail (object 2) and,
    /// from the release until the exit, the dash trail
    /// (`DashTrailPrefab`, the owner's colour) at the ball.
    fn look(&self, w: &World, me: ObjId, fw: &Framework, l: &mut crate::view::Look) {
        use crate::view::{Attach, PartLook};
        l.frame(&self.anim);
        let grounded = self.body.is_grounded();
        if !(grounded && !fw.kill_players_on_contact) {
            l.parts.push(PartLook::hidden(1));
        }
        let mut t = PartLook::new(2);
        t.player_material = true;
        if !grounded {
            t.visible = false;
        }
        l.parts.push(t);
        if fw.kill_players_on_contact && !self.is_charging && !self.waiting_to_release {
            l.attach.push(Attach {
                field: "DashTrailPrefab",
                position: AbilityBody::pos(w, me),
                rotation: Fix::ZERO,
                scale: AbilityBody::scale(w, me),
                color: None,
                shadow_color: true,
            });
        }
    }

    fn sprite(&self) -> SpriteId {
        self.anim.sprite
    }
}

impl Roll {
    fn sign(&self) -> Fix {
        if self.moving_right { Fix::ONE } else { -Fix::ONE }
    }

    /// `Roll.UpdateSim`.
    fn roll_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let owner = cx.owner();
        let jump_down = cx.w.player(owner).is_some_and(|p| p.inputs.input.jump());
        if !jump_down {
            self.jump_button_has_been_up = true;
        }
        self.old_update(cx, dt, jump_down);
        if !cx.running() {
            return;
        }
        if self.body.is_grounded() {
            if self.is_charging {
                self.roll_speed = Fix::ZERO;
            } else {
                let t = self.time_since_release / self.def.dash_time;
                self.roll_speed = self.def.speed_curve.eval(t).clamp01() * self.top_speed;
            }
            self.roll_speed = self.roll_speed * self.sign();
            return;
        }
        if !self.is_charging && !self.is_short_jumping {
            self.body.aireal_movement(self.sign());
        }
        self.body.add_gravity_factor(cx.w);
        let hit = self.body.velocity_based_raycasts(cx.w, me, true, dt);
        if hit && self.is_short_jumping {
            self.exit(
                cx,
                AbilityExitInfo {
                    just_landed: true,
                    ..AbilityExitInfo::default()
                },
            );
        } else if self.is_short_jumping && self.body.body.self_imposed_velocity.y < Fix::ZERO {
            self.exit(cx, AbilityExitInfo::default());
        }
    }

    /// `OldUpdate(dt)` (the logic step).
    fn old_update(&mut self, cx: &mut Cx<'_>, dt: Fix, jump_down: bool) {
        let me = cx.me;
        let owner = cx.owner();
        if self.body.is_grounded()
            && self
                .body
                .physics
                .attached_ground
                .is_none_or(|g| !(cx.w.objects.exists(g) && cx.w.objects.active_in_hierarchy(g)))
        {
            self.body.unground(cx.w, me, true, true);
            self.exit(cx, AbilityExitInfo::default());
            return;
        }
        if self.is_charging {
            self.seconds_charged += cx.w.fixed_delta_time(owner, dt);
            if self.jump_button_has_been_up && jump_down {
                self.body.jump(cx.w, me);
                cx.w.emit(Cue::PlayerSound("slimeJump", owner));
                self.exit(cx, AbilityExitInfo::default());
                return;
            }
        } else {
            self.time_since_release += cx.w.fixed_delta_time(owner, dt);
            if self.jump_button_has_been_up && jump_down {
                let right = cx.w.objects.get(me).map_or(Vec2::RIGHT, |o| o.tf.right);
                let v = self.roll_speed * right;
                self.body.unground(cx.w, me, true, true);
                self.body.body.self_imposed_velocity = v * F_0_75;
                self.exit(cx, AbilityExitInfo::default());
                return;
            }
        }
        if self.is_charging && !cx.button_down() {
            if self.seconds_charged >= self.def.charge_time {
                self.waiting_to_release = true;
                let top = self.body.rope().is_some_and(|l| l.top);
                self.body.attach_rope(cx.w, None, top);
            } else {
                if self.body.is_grounded() {
                    self.exit(
                        cx,
                        AbilityExitInfo {
                            just_landed: true,
                            ..AbilityExitInfo::default()
                        },
                    );
                    return;
                }
                self.short_jump(cx);
            }
        }
        if self.time_since_release > self.def.dash_time {
            self.exit(cx, AbilityExitInfo::default());
            return;
        }
        if !self.has_charged && self.seconds_charged >= self.def.charge_time && !self.is_short_jumping {
            self.has_charged = true;
            if let Some(a) = self.def.anim_fast {
                let c = cx.w.content.clone();
                self.anim.begin(&c, a);
            }
            cx.w.emit(Cue::Sound("rollReady"));
        }
        if self.waiting_to_release && self.body.is_grounded() {
            self.release_dash(cx);
        }
        if self.body.is_grounded() {
            let s = self.roll_speed;
            self.move_(cx, s);
        }
    }

    fn short_jump(&mut self, cx: &mut Cx<'_>) {
        self.is_charging = false;
        self.is_short_jumping = true;
        let js = self.body.cfg.jump_strength;
        std::sync::Arc::make_mut(&mut self.body.cfg).jump_strength = self.def.short_jump_str;
        self.body.jump(cx.w, cx.me);
        std::sync::Arc::make_mut(&mut self.body.cfg).jump_strength = js;
    }

    fn release_dash(&mut self, cx: &mut Cx<'_>) {
        if let Some(c) = cx.fw.collision.as_mut() {
            c.ignored[2] = Some(cx.me);
        }
        self.body.squishable = false;
        self.waiting_to_release = false;
        cx.fw.kill_players_on_contact = true;
        self.is_charging = false;
        self.top_speed = self.def.max_speed;
        if let Some(a) = self.def.anim_ball {
            let c = cx.w.content.clone();
            self.anim.begin(&c, a);
        }
        cx.w.emit(Cue::Sound("rollRelease"));
    }

    /// `Move(speed)` (grounded).
    fn move_(&mut self, cx: &mut Cx<'_>, speed: Fix) {
        let me = cx.me;
        let owner = cx.owner();
        let mut hop: [Option<ObjId>; 5] = [None; 5];
        let mut sweep: Vec<Vec2> = Vec::new();
        if self.is_charging {
            if let Some(rec) = self.body.move_(cx.w, me, Vec2::ZERO, FIXED_DT, true) {
                self.exit(cx, rec);
            }
            return;
        }
        let h = cx.w.fixed_delta_time(owner, FIXED_DT);
        let radius = cx.w.physics.circle(me).map_or(Fix::ONE, |c| c.radius);
        let n = (speed.abs() * h * self.def.move_collision_density / radius).to_long() as i32 + 1;
        let k = Fix::from_int(n) + Fix::ONE;
        let step = Fix::ONE / k * speed;
        if self.body.ground(cx.w).is_none_or(|g| !cx.w.objects.alive(g)) {
            if let Some(rec) = self.body.drop_platform_test(cx.w, me) {
                self.exit(cx, rec);
            }
            return;
        }
        let r = self.body.physics.radius;
        for i in 1..n {
            self.body
                .move_along_swap_if_blocked(cx.w, me, r, step, h, &mut hop);
            if let Some(s) = self.body.surface(cx.w) {
                let q = s.position_from_param(self.body.physics.local_platform_pos, r);
                if (i as usize) <= SWEEP {
                    sweep.push(q);
                }
            }
        }
        let r = self.body.physics.radius;
        self.body
            .move_along_swap_if_blocked(cx.w, me, r, step, h, &mut hop);
        self.body.align_to_ground(cx.w, me);
        self.sweep(cx, &sweep);
        if let Some(rec) = self.body.drop_platform_test(cx.w, me) {
            self.exit(cx, rec);
        }
    }

    /// `CollideExtraHurtBoxes()`.
    fn sweep(&mut self, cx: &mut Cx<'_>, slots: &[Vec2]) {
        let me = cx.me;
        let Some(radius) = cx.w.physics.circle(me).map(|c| c.radius) else {
            return;
        };
        let my_hier = body::collider_hier(cx.w, me);
        for &q in slots {
            let hits = cx
                .w
                .physics
                .circle_cast_all(&cx.w.objects, q, radius, self.def.collides_with, HITS);
            let v = self.body.velocity();
            for &o in &hits {
                let oh = body::collider_hier(cx.w, o);
                body::touch(
                    cx.w,
                    Contact {
                        me: o,
                        other: me,
                        other_hier: my_hier,
                        my_hier: oh,
                        collider: ShapeSnap::Circle(Circle {
                            center: q,
                            radius,
                            layer: 8,
                        }),
                        layer: 8,
                        impact_velocity: v,
                        contact_point: q,
                        normal: Vec2::ZERO,
                        penetration: Fix::ZERO,
                    },
                );
            }
            if !cx.w.objects.alive(me) {
                break;
            }
            let saved = AbilityBody::pos(cx.w, me);
            for &o in &hits {
                let Some(t) = cx.w.objects.get(o).map(|ob| (ob.tf.position, ob.layer)) else {
                    continue;
                };
                let c = Contact {
                    me,
                    other: o,
                    other_hier: body::collider_hier(cx.w, o),
                    my_hier,
                    collider: ShapeSnap::Circle(Circle {
                        center: t.0,
                        radius,
                        layer: t.1 as i32,
                    }),
                    layer: t.1 as i32,
                    impact_velocity: -self.body.velocity(),
                    contact_point: q,
                    normal: Vec2::ZERO,
                    penetration: Fix::ZERO,
                };
                AbilityBody::set_pos(cx.w, me, q);
                self.on_contact(cx, &c);
            }
            AbilityBody::set_pos(cx.w, me, saved);
        }
    }

    /// `Roll.ExitAbility(record)`.
    fn exit(&mut self, cx: &mut Cx<'_>, mut rec: AbilityExitInfo) {
        let me = cx.me;
        let info = cx.info();
        self.seconds_charged = Fix::ZERO;
        self.waiting_to_release = true;
        if let Some(o) = cx.w.objects.get_mut(me) {
            o.tf.set_rotation(Fix::ZERO);
        }
        rec.grounded_speed = self.body.velocity().magnitude();
        rec.is_grounded = self.body.is_grounded();
        rec.facing_right = info.facing_right;
        rec.radius = self.body.physics.radius;
        let p = AbilityBody::pos(cx.w, me);
        if self.body.is_grounded() {
            let n = self.body.current_normal(cx.w, me).unwrap_or(Vec2::UP);
            rec.position = p - n * (self.body.physics.radius - info.radius);
            rec.ground = self.body.physics.attached_ground;
            rec.grounded_local_position = self.body.physics.local_platform_pos;
        } else {
            rec.position = p;
        }
        rec.self_imposed_velocity = self.body.body.self_imposed_velocity;
        rec.external_velocity = self.body.body.external_velocity;
        cx.exit_body(&mut self.body, rec);
    }
}

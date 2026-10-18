//! Drill (`docs/spec/abilities/drill.md`): `Drill` + `HitboxCombo` and the
//! three `invisibleHitbox` tip boxes.
//!
//! Components in update order: Ability 1, SpriteAnimator 2,
//! DestroyIfOutsideSceneBounds 3, PlayerCollision 4, DPhysicsBox 5,
//! PlayerBody 6, Drill 7 (no PlayerPhysics).

use std::sync::Arc;

use super::Behaviour;
use super::ability_body::{self as body, AbilityBody};
use super::prefab::{self, root_component};
use super::prelude::*;
use crate::curve::Curve;
use crate::data::{AnimRef, SpriteId};
use crate::physics::shapes::{BoxShape, circle_box_bool, box_box_bool, box_rr_bool};
use crate::world::Kind;

const CB_START: u16 = 1;
const CB_EXIT: u16 = 2;

/// Prefab values of the `Drill` component and the hitbox combo.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DrillDef {
    pub drill_offset: Vec2,
    pub drill_hitbox: Vec2,
    pub drillables: u32,
    pub gravity_str: Fix,
    pub strong_gravity: Fix,
    pub keep_strong_gravity_seconds: Fix,
    pub max_early_fall_speed: Fix,
    pub drill_str: Fix,
    pub rotation_speed_air: Fix,
    pub speed_to_rotation: Arc<Curve>,
    pub min_underground_speed: Fix,
    pub enter_ground_speed_multiplier: Fix,
    pub leave_ground_speed_multiplier: Fix,
    pub min_speed_dust_entry: Fix,
    pub min_speed_dust_exit: Fix,
    pub on_enter_offset: Vec2,
    pub anim_enter: Option<AnimRef>,
    pub anim_loop: Option<AnimRef>,
    pub anim_exit: Option<AnimRef>,
    /// `(offset, extents)` per hitbox.
    pub boxes: Vec<(Vec2, Vec2)>,
    pub hitbox_prefab: Option<usize>,
}

/// Module state (one per ability object).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Drill {
    pub def: DrillDef,
    pub body: AbilityBody,
    pub anim: SpriteAnimator,
    pub is_drilling: bool,
    pub is_underground: bool,
    pub time_passed: Fix,
    pub current_platform: Option<ObjId>,
    /// The spawned tip boxes (once per ability instance).
    pub hitboxes: Option<Vec<ObjId>>,
}

/// Spawned objects.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    /// An `invisibleHitbox` (`Hitbox`, part `Behaviour(0)`).
    Hitbox(Hitbox),
}

/// A tip box: an Explosion-layer box owned by the driller.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hitbox {
    /// The combo's object (the drill ability object).
    pub combo: ObjId,
    pub owner: i32,
}

fn anim_data(w: &World, r: Option<&str>) -> Option<u16> {
    let r = r?;
    w.content
        .anim_data
        .iter()
        .position(|a| a.id == r)
        .map(|i| i as u16)
}

impl AbilityImpl for Drill {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let c = root_component(&w.content, def, "Drill");
        let f = |k: &str, d: Fix| prefab::fix_field(c, k, d);
        let v = |k: &str, d: Vec2| prefab::vec2_field_or(c, k, d);
        let data = anim_data(w, c.and_then(|c| c.fields.reference("animData")));
        let a = |n: &str| data.and_then(|d| w.content.anim_ref(d, n));
        let hc = root_component(&w.content, def, "HitboxCombo");
        let boxes = prefab::field(hc, "hitBoxDefinitions")
            .map(|l| match l {
                Value::List(l) => l
                    .iter()
                    .map(|b| {
                        (
                            b.vec2("offset").unwrap_or_default(),
                            b.vec2("extents").unwrap_or_default(),
                        )
                    })
                    .collect(),
                _ => Vec::new(),
            })
            .unwrap_or_default();
        let hitbox_prefab = prefab::prefab_ref(
            &w.content,
            hc.and_then(|h| h.fields.reference("hitboxPrefab")),
            "invisibleHitbox",
        );
        Drill {
            def: DrillDef {
                drill_offset: v("drillOffset", Vec2::new(Fix::ZERO, Fix::from_int(-2))),
                drill_hitbox: v("drillHitbox", Vec2::raw(1_159_641_216, 5_540_507_648)),
                drillables: prefab::int_field(c, "drillables", 2048) as u32,
                gravity_str: f("gravityStr", Fix::from_int(45)),
                strong_gravity: f("strongGravity", Fix::from_int(102)),
                keep_strong_gravity_seconds: f("keepStrongGravitySeconds", Fix(1_717_986_944)),
                max_early_fall_speed: f("maxEarlyFallSpeed", Fix::from_int(45)),
                drill_str: f("drillstr", Fix::from_int(6)),
                rotation_speed_air: f("rotationSpeedAir", Fix::from_int(7)),
                speed_to_rotation: prefab::curve_field(c, "SpeedToRotationSpeed01"),
                min_underground_speed: f("minUndergroundSpeed", Fix::from_int(10)),
                enter_ground_speed_multiplier: f("EnterGroundSpeedMultiplier", Fix(2_834_678_528)),
                leave_ground_speed_multiplier: f("leaveGroundSpeedMultiplier", Fix(6_442_450_944)),
                min_speed_dust_entry: f("minSpeedBeforeDustCloudOnEntry", Fix::from_int(100)),
                min_speed_dust_exit: f("minSpeedBeforeDustCloud", Fix::from_int(250)),
                on_enter_offset: v("onEnterOffset", Vec2::raw(0, 8_632_884_224)),
                anim_enter: a("enter"),
                anim_loop: a("loop"),
                anim_exit: a("exit"),
                boxes,
                hitbox_prefab,
            },
            body: AbilityBody::new(w, def),
            ..Drill::default()
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
        if let Some(o) = cx.w.objects.get_mut(me) {
            o.tf.set_rotation(info.rotation);
        }
        cx.w.emit(Cue::Sound("drill_start"));
        if let Some(a) = self.def.anim_enter {
            let c = cx.w.content.clone();
            self.anim
                .begin_then_do(&c, a, AnimCallback::Ability(CB_START));
        }
        self.is_drilling = false;
        self.time_passed = Fix::ZERO;
        self.is_underground = false;
        self.body.body.external_velocity = info.external_velocity;
        self.body.body.self_imposed_velocity = info.self_imposed_velocity;
        let scale = cx.w.player(cx.owner()).map_or(Fix::ONE, |p| p.scale());
        let p = info.position + (self.def.on_enter_offset * scale).complex_mul(info.up.mul_by_neg_i());
        AbilityBody::set_pos(cx.w, me, p);
        cx.w.physics.update_physics_positions(&cx.w.objects, me);
        self.spawn_hitboxes(cx);
        self.set_hitboxes_active(cx.w, false);
        self.update_hitboxes(cx.w, me);
        if let Some(hb) = &self.hitboxes {
            for &h in hb {
                cx.w.physics.update_physics_positions(&cx.w.objects, h);
            }
        }
    }

    fn tick(&mut self, cx: &mut Cx<'_>, dt: Fix, late: bool) {
        let me = cx.me;
        if late {
            self.body.body_late_update(cx.w, me);
            return;
        }
        cx.ability_update();
        if let Some(AnimCallback::Ability(code)) =
            body::advance_anim(cx.w, cx.owner(), &mut self.anim)
        {
            self.on_anim(cx, code);
        }
        if !cx.running() {
            return;
        }
        if let Some(rec) = self.body.bounds_update(cx.w, me) {
            self.exit(cx, rec);
            return;
        }
        AbilityBody::collision_update(cx.fw);
        self.body.body_update(cx.w, me, dt);
        self.drill_update(cx, dt);
    }

    fn on_anim(&mut self, cx: &mut Cx<'_>, code: u16) {
        match code {
            CB_START => {
                if let Some(a) = self.def.anim_loop {
                    let c = cx.w.content.clone();
                    self.anim.begin(&c, a);
                }
                self.is_drilling = true;
                self.set_hitboxes_active(cx.w, true);
            }
            CB_EXIT => self.exit_with_offset(cx, true),
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
        self.anim.sprite
    }

    /// The drill frame; underground the dirt trail sprites (objects 1–3:
    /// light, medium, dark in the platform type's colours) and the burrow
    /// particles (`particleSystemPrefab`) follow it.
    fn look(&self, w: &World, me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        use crate::view::{Attach, PartLook};
        l.frame(&self.anim);
        let cols = self.colors(w, me);
        for (i, k) in [(1u16, 2usize), (2, 1), (3, 0)] {
            if self.is_underground {
                let mut p = PartLook::new(i);
                p.tint = cols.map(|c| c[k]);
                l.parts.push(p);
            } else {
                l.parts.push(PartLook::hidden(i));
            }
        }
        // `PlayOrContinueLoop` of the drilling sounds.
        if self.is_underground {
            l.loop_sound = Some("drill_underground_loop");
        } else if self.is_drilling {
            l.loop_sound = Some("drill_loop");
        }
        if self.is_underground && self.is_drilling {
            l.attach.push(Attach {
                field: "particleSystemPrefab",
                position: AbilityBody::pos(w, me),
                rotation: AbilityBody::rotation(w, me),
                scale: AbilityBody::scale(w, me),
                color: cols.map(|c| c[2]),
                shadow_color: false,
            });
        }
    }
}

impl Drill {
    /// `platformDependentColors` of the current platform's type: dark,
    /// medium, light (presentation data).
    fn colors(&self, w: &World, me: ObjId) -> Option<[[Fix; 4]; 3]> {
        let t = match self.current_platform.and_then(|p| w.objects.get(p)).map(|o| &o.kind) {
            Some(crate::world::Kind::Platform(p)) => p.sticky.platform_type,
            _ => 0,
        };
        let f = prefab::object_fields(w, me, "Drill")?;
        let e = f
            .list("platformDependentColors")
            .iter()
            .find(|e| e.int("type") == Some(t as i64))?;
        Some([
            super::color_field(e.get("dark")?)?,
            super::color_field(e.get("medium")?)?,
            super::color_field(e.get("light")?)?,
        ])
    }

    /// Presentation: `spawnDustCloud(pos, normal)` in the platform's light
    /// colour.
    fn dust(&self, cx: &mut Cx<'_>, pos: Vec2, up: Vec2) {
        let c = self.colors(cx.w, cx.me);
        let rot = super::rotation_of_up(up);
        cx.w.emit(Cue::Fx(crate::view::FxCue {
            prefab: match cx.w.objects.get(cx.me).map(|o| o.visual) {
                Some(crate::view::Visual::Prefab { prefab, .. }) => prefab,
                _ => return,
            },
            field: "dustCloudParticlePrefab",
            position: pos,
            rotation: rot,
            scale: AbilityBody::scale(cx.w, cx.me),
            player_id: cx.fw.owner,
            value: Vec2::ZERO,
            color: c.map(|c| c[2]),
            source: cx.me,
        }));
    }
    fn spawn_hitboxes(&mut self, cx: &mut Cx<'_>) {
        let me = cx.me;
        if self.hitboxes.is_none() {
            let Some(p) = self.def.hitbox_prefab else {
                return;
            };
            let pos = AbilityBody::pos(cx.w, me);
            let mut v = Vec::new();
            for &(off, ext) in &self.def.boxes {
                let id = body::spawn_prefab(
                    cx.w,
                    p,
                    Kind::Empty,
                    &[
                        (body::COLLIDER, PartKind::Collider),
                        ("Hitbox", PartKind::Behaviour(0)),
                    ],
                    pos + off,
                    Some(Fix::ZERO),
                );
                if let Some(o) = cx.w.objects.get_mut(id)
                    && let Some(c) = o.collider.as_mut()
                {
                    // SetExtents before init: the start extents.
                    c.start_extents = ext;
                }
                let hier = body::collider_hier(cx.w, id);
                cx.w.physics.register(&mut cx.w.objects, id, hier);
                v.push(id);
            }
            for &id in &v {
                if let Some(o) = cx.w.objects.get_mut(id) {
                    o.player_id = cx.fw.owner;
                }
                cx.w.put_kind(
                    id,
                    Kind::Behaviour(Box::new(Behaviour::Drill(Spawned::Hitbox(Hitbox {
                        combo: me,
                        owner: cx.fw.owner,
                    })))),
                );
            }
            self.hitboxes = Some(v);
        }
        if let (Some(c), Some(hb)) = (cx.fw.collision.as_mut(), &self.hitboxes) {
            for (k, &h) in hb.iter().enumerate().take(3) {
                c.ignored[3 + k] = Some(h);
            }
        }
    }

    fn set_hitboxes_active(&self, w: &mut World, active: bool) {
        if let Some(hb) = &self.hitboxes {
            for &h in hb {
                if let Some(o) = w.objects.get_mut(h) {
                    o.active = active;
                }
            }
        }
    }

    fn hitboxes_active(&self, w: &World) -> bool {
        self.hitboxes
            .as_ref()
            .and_then(|h| h.first())
            .and_then(|h| w.objects.get(*h))
            .is_some_and(|o| o.active)
    }

    /// `UpdateHitBoxPositionsAndScale()`.
    fn update_hitboxes(&self, w: &mut World, me: ObjId) {
        let Some(t) = w.objects.get(me).map(|o| o.tf) else {
            return;
        };
        let Some(hb) = &self.hitboxes else {
            return;
        };
        for (i, &h) in hb.iter().enumerate() {
            let off = self.def.boxes.get(i).map_or(Vec2::ZERO, |b| b.0);
            let Some(ht) = w.objects.get(h).map(|o| o.tf) else {
                continue;
            };
            if let Some(o) = w.objects.get_mut(h) {
                o.tf.position = t.position + (off * t.scale).complex_mul(t.right);
                o.tf.set_rotation(t.rotation);
            }
            if ht.scale != t.scale {
                w.set_collider_scale(h, t.scale);
            }
        }
    }

    /// `CollideBox(box, mask)`: the first collider overlapping the box.
    fn collide_box(w: &World, b: &BoxShape, mask: u32) -> Option<ObjId> {
        let p = &w.physics;
        let on = |l: i32| mask >> (l & 31) & 1 != 0;
        for i in 0..p.circles.len() {
            let s = &p.circles.shapes[i];
            let o = p.circles.parents[i].obj;
            if on(s.layer) && body::collider_active(w, o) && circle_box_bool(s, b) {
                return Some(o);
            }
        }
        for i in 0..p.boxes.len() {
            let s = &p.boxes.shapes[i];
            let o = p.boxes.parents[i].obj;
            if on(s.layer) && body::collider_active(w, o) && box_box_bool(s, b) {
                return Some(o);
            }
        }
        for i in 0..p.rrs.len() {
            let s = &p.rrs.shapes[i];
            let o = p.rrs.parents[i].obj;
            if on(s.layer) && body::collider_active(w, o) && box_rr_bool(b, s) {
                return Some(o);
            }
        }
        None
    }

    /// `Drill.UpdateSim`.
    fn drill_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let owner = cx.owner();
        let dt = cx.w.fixed_delta_time(owner, dt);
        if !cx.w.objects.get(me).is_some_and(|o| o.active) {
            return;
        }
        if !self.hitboxes_active(cx.w) && self.is_drilling {
            self.set_hitboxes_active(cx.w, true);
        }
        if dt == Fix::ZERO {
            self.update_hitboxes(cx.w, me);
            return;
        }
        self.time_passed += dt;
        let scale = cx.w.player(owner).map_or(Fix::ONE, |p| p.scale());
        let t = cx.w.objects.get(me).map(|o| o.tf).unwrap_or_default();
        let d = &self.def;
        let hx = d.drill_hitbox.x * scale;
        let hy = d.drill_hitbox.y * scale;
        let probe = BoxShape {
            center: t.position + (d.drill_offset * scale).complex_mul(t.right),
            right: t.right * hx,
            up: t.up * hy,
            ie: Vec2::new(Fix::ONE / hx, Fix::ONE / hy),
            layer: 8,
        };
        let hit = Self::collide_box(cx.w, &probe, d.drillables);
        let stuck = hit.is_some() && !self.is_drilling;
        if let Some(h) = hit
            && matches!(cx.w.objects.get(h).map(|o| &o.kind), Some(Kind::Platform(_)))
        {
            self.current_platform = Some(h);
        }
        if let Some(p) = cx.w.player_mut(owner) {
            p.custom_aim_ability_position = t.position;
        }
        let aim = cx.w.player(owner).map_or(Vec2::ZERO, |p| p.inputs.aim_vector());
        if aim != Vec2::ZERO {
            let k = d.speed_to_rotation.eval(self.body.body.self_imposed_velocity.magnitude());
            let inc = if self.is_underground {
                d.rotation_speed_air * k * dt
            } else {
                d.rotation_speed_air * dt
            };
            let tip = (-t.up).approach_normalized(aim, inc);
            if let Some(o) = cx.w.objects.get_mut(me) {
                o.tf.set_up(-tip);
            }
        }
        if stuck {
            self.body.body.self_imposed_velocity = Vec2::ZERO;
        }
        let mut surfaced = false;
        let sv = &mut self.body.body.self_imposed_velocity;
        if hit.is_some() && !stuck {
            let was_under = self.is_underground;
            self.is_underground = true;
            if !was_under {
                let up = cx.w.objects.get(me).map_or(Vec2::UP, |o| o.tf.up);
                let pos = AbilityBody::pos(cx.w, me);
                if let Some(cp) = self.current_platform
                    && let Some(s) = crate::surface::Surface::of(cx.w, cp)
                    && s.normal(pos).dot(up) <= Fix::ZERO
                {
                    self.exit_with_offset(cx, false);
                    return;
                }
                let sv = &mut self.body.body.self_imposed_velocity;
                let m = self.def.min_underground_speed;
                if sv.sqr_magnitude() < m * m {
                    *sv = sv.normalized_safe() * m;
                }
                *sv = *sv * self.def.enter_ground_speed_multiplier;
                self.body.body.external_velocity = Vec2::ZERO;
                if self.body.body.self_imposed_velocity.sqr_magnitude() > self.def.min_speed_dust_entry {
                    self.dust(cx, pos - up * Fix::THREE, -up);
                    cx.w.emit(Cue::Sound("drill_underground_enter"));
                    cx.w.emit(Cue::Sound("debris"));
                }
            }
        } else {
            if self.is_underground {
                *sv = *sv * d.leave_ground_speed_multiplier;
                if sv.sqr_magnitude() > d.min_speed_dust_exit {
                    let (pos, up) = (
                        AbilityBody::pos(cx.w, me),
                        cx.w.objects.get(me).map_or(Vec2::UP, |o| o.tf.up),
                    );
                    self.dust(cx, pos, up);
                    cx.w.emit(Cue::Sound("drill_underground_exit"));
                    cx.w.emit(Cue::Sound("debris"));
                }
                surfaced = true;
            }
            self.is_underground = false;
        }
        let up = cx.w.objects.get(me).map_or(Vec2::UP, |o| o.tf.up);
        let d = &self.def;
        let sv = &mut self.body.body.self_imposed_velocity;
        if self.is_underground {
            *sv = (-up) * sv.magnitude();
            *sv += (-up) * d.drill_str * dt;
        } else if !super::rope::enabled(cx.w, self.body.body.rope) {
            if self.time_passed < d.keep_strong_gravity_seconds && sv.y < d.max_early_fall_speed {
                *sv += Vec2::DOWN * d.strong_gravity * dt;
            } else {
                *sv += Vec2::DOWN * d.gravity_str * dt;
            }
        }
        if hit.is_some()
            && let Some(cp) = self.current_platform.filter(|c| cx.w.objects.exists(*c))
        {
            let pos = AbilityBody::pos(cx.w, me);
            let v = velocity_at_pos(cx.w, cp, pos);
            AbilityBody::set_pos(cx.w, me, pos + v * dt);
        }
        if super::rope::enabled(cx.w, self.body.body.rope) && self.is_underground {
            let top = self.body.rope().is_some_and(|l| l.top);
            self.body.attach_rope(cx.w, None, top);
        }
        if !cx.button_down() && !self.is_underground && self.is_drilling {
            if surfaced {
                self.exit_with_offset(cx, false);
                return;
            }
            self.is_drilling = false;
            cx.w.emit(Cue::Sound("drill_end"));
            if let Some(a) = self.def.anim_exit {
                let c = cx.w.content.clone();
                self.anim
                    .begin_then_do(&c, a, AnimCallback::Ability(CB_EXIT));
            }
        }
        self.update_hitboxes(cx.w, me);
    }

    /// `exitAbility(useOffset)`.
    fn exit_with_offset(&mut self, cx: &mut Cx<'_>, offset: bool) {
        let me = cx.me;
        let mut p = AbilityBody::pos(cx.w, me);
        if offset {
            let scale = cx.w.player(cx.owner()).map_or(Fix::ONE, |p| p.scale());
            let up = cx.w.objects.get(me).map_or(Vec2::UP, |o| o.tf.up);
            p -= (self.def.on_enter_offset * scale).complex_mul(up.mul_by_neg_i());
        }
        self.exit(
            cx,
            AbilityExitInfo {
                position: p,
                ..AbilityExitInfo::default()
            },
        );
    }

    /// `Drill.ExitAbility(record)`.
    fn exit(&mut self, cx: &mut Cx<'_>, mut rec: AbilityExitInfo) {
        self.set_hitboxes_active(cx.w, false);
        rec.self_imposed_velocity = self.body.body.self_imposed_velocity;
        rec.external_velocity = self.body.body.external_velocity;
        rec.facing_right = self.body.velocity().dot(Vec2::RIGHT) >= Fix::ZERO;
        cx.exit_body(&mut self.body, rec);
    }
}

/// `StickyRoundedRectangle.VelocityAtPos(p)` (body values, composite aware).
pub fn velocity_at_pos(w: &World, plat: ObjId, p: Vec2) -> Vec2 {
    let pos = w.objects.get(plat).map_or(Vec2::ZERO, |o| o.tf.position);
    let Some(b) = w.physics.body(plat) else {
        return Vec2::ZERO;
    };
    let v = p - pos;
    (v.magnitude() * b.angular_velocity) * v.normalized_safe().mul_by_i() + b.velocity
}

impl SpawnedImpl for Spawned {
    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, _dt: Fix, late: bool) {
        let Spawned::Hitbox(h) = self;
        if late || part != PartKind::Behaviour(0) {
            return;
        }
        if !w.objects.exists(h.combo) {
            w.destroy(me);
            return;
        }
        let combo = w.objects.active_in_hierarchy(h.combo);
        let mine = w.objects.active_in_hierarchy(me);
        if combo != mine
            && let Some(o) = w.objects.get_mut(me)
        {
            o.active = combo;
        }
    }

    fn lethal(&self, _w: &World, _me: ObjId) -> Option<Lethal> {
        let Spawned::Hitbox(h) = self;
        Some(Lethal {
            owner: h.owner,
            can_hurt_owner: true,
            beam: false,
        })
    }
}

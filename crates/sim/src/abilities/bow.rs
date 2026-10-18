//! Bow (`docs/spec/abilities/bow.md`): `BowTransform` on the ability object
//! and the `Arrow` projectile (shared `Projectile` component).
//!
//! Update order inside the Bow object: SpriteAnimator 1 (pre-update),
//! Ability 2 (framework), PlayerPhysics 3, PlayerCollision 4,
//! DestroyIfOutsideSceneBounds 5, BowTransform 6, DPhysicsBox 7,
//! PlayerBody 8.

use super::ability_body::{self, AbilityBody, Bounds, COLLIDER};
use super::prefab;
use super::prelude::*;
use super::smoke::RingBuffer;
use super::Behaviour;
use crate::data::AnimRef;
use crate::world::Kind;

const CB_LOADING: u16 = 1;
const CB_FINISHED_LOADING: u16 = 2;
const CB_EXIT: u16 = 3;

/// `(Fix)0.03f`: airborne gravity modifier.
const F_0_03: Fix = Fix(128_849_016);
/// `(Fix)1E-06f`: "moving" threshold on the squared speed.
const F_1E_6: Fix = Fix(4_294);

/// Arrow prefab data.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ArrowDef {
    pub prefab: Option<usize>,
    pub stick_to: u32,
    pub bounds: Bounds,
}

/// The Bow's `BowTransform` and body.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Bow {
    pub b: AbilityBody,
    pub anim: SpriteAnimator,
    pub arrow: ArrowDef,
    pub arrow_speed: Fix,
    pub knock_back: Fix,
    pub firepoint_offset: Vec2,
    pub time_before_arrows_hurt_owner: Fix,
    pub aireal_control_blend: Fix,
    pub max_slow_mo_speed: Fix,
    pub a_transform: AnimRef,
    pub a_load: AnimRef,
    pub a_loop: AnimRef,
    pub a_fire_air: AnimRef,
    pub a_fire_ground: AnimRef,
    pub arrows: RingBuffer,
    // state
    pub is_loading: bool,
    pub loading_frame: i32,
    pub d: Vec2,
    pub released: bool,
    pub has_fired: bool,
}

/// Objects the Bow spawns.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    Arrow(Arrow),
}

/// Parts of an arrow object.
const P_BOUNDS: PartKind = PartKind::Behaviour(0);
const P_ARROW: PartKind = PartKind::Behaviour(1);
const P_PROJECTILE: PartKind = PartKind::Behaviour(2);

/// The `Arrow` + `Projectile` + `BoplBody` of a flying arrow.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Arrow {
    pub owner: i32,
    pub stick_to: u32,
    pub bounds: Bounds,
    pub landed: bool,
    /// `Projectile.timePassed`, `EnableHurtOwnerSeconds`, `canHurtOwner`.
    pub time_passed: Fix,
    pub hurt_owner_after: Fix,
    pub can_hurt_owner: bool,
}

impl AbilityImpl for Bow {
    fn exit_component(&mut self, cx: &mut Cx<'_>, r: AbilityExitInfo) {
        self.component_exit(cx, r);
    }

    fn new(w: &World, def: &AbilityDef) -> Self {
        let f = prefab::component_fields(w, def, "BowTransform");
        let data = f
            .reference("animData")
            .and_then(|r| prefab::anim_data_index(w, r))
            .unwrap_or(0);
        let a = |n: &str| prefab::anim(w, data, n);
        let arrow_prefab = f
            .reference("Arrow")
            .and_then(|r| prefab::prefab_index(w, r));
        let arrow_root = arrow_prefab
            .and_then(|p| w.content.prefabs.get(p))
            .and_then(|p| p.objects.first());
        let arrow = ArrowDef {
            prefab: arrow_prefab,
            stick_to: arrow_root
                .and_then(|o| o.component("Arrow"))
                .and_then(|c| c.fields.int("StickTo"))
                .unwrap_or(2_166_016) as u32,
            bounds: arrow_root.map_or_else(Bounds::default, |o| Bounds::of_object(w, o)),
        };
        Bow {
            b: AbilityBody::new(w, def),
            anim: ability_body::animator(prefab::ability_prefab(w, def).is_some_and(prefab::root_always_stop)),
            arrow,
            arrow_speed: f.fix_or("ArrowSpeed", Fix::ONE),
            knock_back: Fix::from_int(f.int("KnockBack").unwrap_or(20) as i32),
            firepoint_offset: f.vec2("FirepointOffset").unwrap_or_default(),
            time_before_arrows_hurt_owner: f.fix_or("TimeBeforeArrowsHurtOwner", Fix(1_288_490_240)),
            aireal_control_blend: f.fix_or("airealControlBlend", F_0_5),
            max_slow_mo_speed: f.fix_or("maxSlowMoSpeed", Fix::from_int(9)),
            a_transform: a("transform"),
            a_load: a("load"),
            a_loop: a("loop"),
            a_fire_air: a("fireAir"),
            a_fire_ground: a("fireGround"),
            arrows: RingBuffer::new((f.int("maxNumberOfArrows").unwrap_or(20) as usize).max(1)),
            is_loading: false,
            loading_frame: 0,
            d: Vec2::UP,
            released: false,
            has_fired: false,
        }
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        let info = cx.info();
        let me = cx.me;
        self.b.sync_physics_to(cx.w, me, &info);
        AbilityBody::update_collider(cx.w, me);
        self.is_loading = false;
        self.loading_frame = 0;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        self.b.body.external_velocity = Vec2::ZERO;
        let t = self.a_transform;
        ability_body::play_then(cx.w, &mut self.anim, t, CB_LOADING);
        if !info.is_grounded {
            self.b.gravity_modifier = F_0_03;
            self.b.body.self_imposed_velocity = info.self_imposed_velocity * F_0_1;
            self.b.body.external_velocity = info.external_velocity * F_0_1;
        }
        self.d = info.up;
        self.released = false;
        self.has_fired = false;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
    }

    fn pre_update(&mut self, cx: &mut Cx<'_>, _dt: Fix) {
        // SpriteAnimator (hierarchy 1).
        let owner = cx.fw.owner;
        match ability_body::advance_code(cx.w, owner, &mut self.anim) {
            Some(CB_LOADING) => {
                self.is_loading = true;
                cx.w.emit(Cue::Sound("arrowLoad"));
                let (l, lp) = (self.a_load, self.a_loop);
                ability_body::play_then_loop(cx.w, &mut self.anim, l, lp, Some(CB_FINISHED_LOADING));
            }
            Some(CB_FINISHED_LOADING) => self.loading_frame = 4,
            Some(CB_EXIT) => self.exit_component(cx, AbilityExitInfo::default()),
            _ => {}
        }
    }

    fn update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        // PlayerPhysics 3, PlayerCollision 4, bounds 5.
        self.b.physics_update(cx.w, me, dt);
        AbilityBody::collision_update(cx.fw);
        if let Some(r) = self.b.bounds_update(cx.w, me) {
            self.exit_component(cx, r);
            return;
        }
        // BowTransform 6.
        self.bow_update(cx, dt);
        // PlayerBody 8 (skipped if the object was deactivated meanwhile).
        if cx.w.objects.get(me).is_some_and(|o| o.active) {
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

impl Bow {
    fn input(cx: &Cx<'_>) -> (Vec2, Fix) {
        cx.w.player(cx.fw.owner).map_or((Vec2::ZERO, Fix::ZERO), |p| {
            (p.inputs.aim_vector(), p.inputs.input_vector().x)
        })
    }

    fn bow_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let (aim, horizontal) = Self::input(cx);
        if aim.magnitude() > DEADZONE && !self.released {
            self.d = aim.normalized();
        }
        if !cx.button_down() && !self.has_fired {
            self.released = true;
        }
        if self.released
            && !self.has_fired
            && (self.loading_frame > 0 || self.anim.passed_sprites() > 2)
        {
            self.fire(cx, self.d);
        }
        if self.is_loading && self.loading_frame < 4 {
            self.loading_frame = self.anim.passed_sprites();
        }
        if self.b.is_grounded() {
            let ground_ok = self.b.ground(cx.w).is_some_and(|g| {
                cx.w.objects.active_in_hierarchy(g)
                    && cx
                        .w
                        .objects
                        .get(g)
                        .is_some_and(|o| o.part_enabled(PartKind::Sticky))
            });
            if self.b.body.self_imposed_velocity.sqr_magnitude() > F_1E_6 || !ground_ok {
                self.b.gravity_modifier = F_0_03;
                self.b.unground(cx.w, me, false, true);
            }
        }
        if !self.b.is_grounded() {
            self.b.add_gravity_factor(cx.w);
            if self.b.velocity_based_raycasts(cx.w, me, true, dt) && self.has_fired {
                AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
                let rec = AbilityExitInfo {
                    just_landed: true,
                    grounded_speed: self.b.physics.grounded_speed,
                    is_grounded: self.b.physics.is_grounded,
                    facing_right: self.d.x >= Fix::ZERO,
                    position: AbilityBody::pos(cx.w, me),
                    self_imposed_velocity: self.b.body.self_imposed_velocity,
                    external_velocity: self.b.body.external_velocity,
                    ground: self.b.ground(cx.w),
                    grounded_local_position: self.b.physics.local_platform_pos,
                    radius: self.b.physics.radius,
                    ..AbilityExitInfo::default()
                };
                cx.exit_body(&mut self.b, rec);
            }
        }
        if self.b.is_grounded()
            && let Some(r) = self.b.move_(cx.w, me, Vec2::ZERO, dt, true)
        {
            self.exit_component(cx, r);
        }
        if !self.has_fired {
            if let Some(o) = cx.w.objects.get_mut(me) {
                o.tf.set_up(self.d);
            }
            let v = self.b.body.self_imposed_velocity;
            if v.magnitude() > self.max_slow_mo_speed {
                self.b.body.self_imposed_velocity = v.normalized() * self.max_slow_mo_speed;
            }
        } else {
            let v0 = self.b.body.self_imposed_velocity;
            self.b.aireal_movement(horizontal);
            let v1 = self.b.body.self_imposed_velocity;
            self.b.body.self_imposed_velocity = v0.lerp(v1, self.aireal_control_blend);
        }
        let v = self.b.body.self_imposed_velocity;
        if v.magnitude() > self.b.cfg.max_speed {
            self.b.body.self_imposed_velocity = v.normalized() * self.b.cfg.max_speed;
        }
        if self.b.is_grounded()
            && let Some(r) = self.b.drop_platform_test(cx.w, me)
        {
            self.exit_component(cx, r);
        }
    }

    fn fire(&mut self, cx: &mut Cx<'_>, d: Vec2) {
        let me = cx.me;
        self.is_loading = false;
        self.has_fired = true;
        let d = d.normalized();
        self.shoot(cx, d);
        cx.w.emit(Cue::Sound("arrowShoot"));
        self.b.gravity_modifier = Fix::ONE;
        if self.b.is_grounded() {
            let a = self.a_fire_ground;
            ability_body::play_then(cx.w, &mut self.anim, a, CB_EXIT);
            let n = self.b.current_normal(cx.w, me).unwrap_or(Vec2::UP);
            self.b.body.self_imposed_velocity = (-n) * self.knock_back;
            self.b.unground(cx.w, me, false, true);
        } else {
            let a = self.a_fire_air;
            ability_body::play_then(cx.w, &mut self.anim, a, CB_EXIT);
            self.b.body.self_imposed_velocity = (-d) * self.knock_back;
        }
    }

    fn shoot(&mut self, cx: &mut Cx<'_>, dir: Vec2) {
        let Some(prefab) = self.arrow.prefab else {
            return;
        };
        let me = cx.me;
        let owner = cx.fw.owner;
        let Some(t) = cx.w.objects.get(me).map(|o| o.tf) else {
            return;
        };
        let pos = t.position + self.firepoint_offset.x * t.right + self.firepoint_offset.y * t.up;
        let arrow = Arrow {
            owner,
            stick_to: self.arrow.stick_to,
            bounds: self.arrow.bounds,
            landed: false,
            time_passed: Fix::ZERO,
            hurt_owner_after: Fix::ZERO,
            can_hurt_owner: false,
        };
        let id = ability_body::spawn_prefab(
            cx.w,
            prefab,
            Kind::Behaviour(Box::new(Behaviour::Bow(Spawned::Arrow(arrow)))),
            &[
                ("DestroyIfOutsideSceneBounds", P_BOUNDS),
                ("Arrow", P_ARROW),
                ("Projectile", P_PROJECTILE),
                ("BoplBody", PartKind::BoplBody),
                (COLLIDER, PartKind::Collider),
            ],
            pos,
            Some(t.rotation),
        );
        cx.w.set_collider_scale(id, t.scale);
        let k = Fix::ONE + (t.scale - Fix::ONE) / Fix::TWO;
        self.arrows.add_by(cx.w, id, arrow_flying);
        let v0 = dir * (Fix::from_int(self.loading_frame) + Fix::ONE) * self.arrow_speed * k
            + self.b.body.self_imposed_velocity;
        let delay = self.time_before_arrows_hurt_owner * k / v0.magnitude();
        if let Some(o) = cx.w.objects.get_mut(id) {
            o.player_id = owner;
            if let Some(b) = o.collider.as_mut().and_then(|c| c.body.as_mut()) {
                b.start_velocity = v0;
            }
            o.tf.set_rotation(t.rotation);
            if let Kind::Behaviour(bh) = &mut o.kind
                && let Behaviour::Bow(Spawned::Arrow(a)) = bh.as_mut()
            {
                a.hurt_owner_after = delay;
            }
        }
    }

    /// `BowTransform.ExitAbility(record)` (the first ability component's
    /// exit: fire-animation end, death, out of bounds, drop-through).
    fn component_exit(&mut self, cx: &mut Cx<'_>, mut r: AbilityExitInfo) {
        let me = cx.me;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        self.b.unground(cx.w, me, true, false);
        r.grounded_speed = self.b.physics.grounded_speed;
        r.is_grounded = self.b.physics.is_grounded;
        r.facing_right = self.d.x >= Fix::ZERO;
        r.position = AbilityBody::pos(cx.w, me);
        r.external_velocity = self.b.body.external_velocity;
        r.self_imposed_velocity = self.b.body.self_imposed_velocity;
        r.ground = self.b.ground(cx.w);
        r.radius = self.b.physics.radius;
        cx.exit_body(&mut self.b, r);
    }
}

/// An arrow still counts in the ring buffer while its body exists.
fn arrow_flying(w: &World, id: ObjId) -> bool {
    w.objects
        .get(id)
        .and_then(|o| o.part(PartKind::BoplBody))
        .is_some_and(|p| !p.destroyed)
}

impl super::SpawnedImpl for Spawned {
    /// Arrows use the owner's material (`BowTransform`).
    fn look(&self, _w: &World, _me: ObjId, l: &mut crate::view::Look) {
        l.player_material = true;
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        let Spawned::Arrow(a) = self;
        if late {
            return;
        }
        match part {
            P_BOUNDS => {
                if a.bounds.check(w, me) {
                    // Projectile layer: destroyed.
                    w.destroy(me);
                }
            }
            P_ARROW => {
                if a.landed || ability_body::is_held(w, me) {
                    return;
                }
                let v = w.physics.body(me).map_or(Vec2::ZERO, |b| b.velocity);
                let up = if v.magnitude() == Fix::ZERO {
                    Vec2::UP
                } else {
                    v.normalized()
                };
                if let Some(o) = w.objects.get_mut(me) {
                    o.tf.set_up(up);
                }
            }
            P_PROJECTILE => {
                a.time_passed += dt * w.time.player_time_scale;
                if a.hurt_owner_after > Fix::ZERO && a.time_passed > a.hurt_owner_after {
                    a.can_hurt_owner = true;
                }
            }
            _ => {}
        }
    }

    fn on_contact(&mut self, w: &mut World, me: ObjId, c: &Contact) {
        let Spawned::Arrow(a) = self;
        if a.landed || !(0..32).contains(&c.layer) || a.stick_to >> c.layer & 1 == 0 {
            return;
        }
        // Released from a magnet gun if held (`BoplBody.Drop`).
        if let Some(o) = w.objects.get_mut(me)
            && o.bb.is_held()
        {
            o.bb.drop_hold();
        }
        let Some(other) = w.objects.get(c.other) else {
            return;
        };
        // Players and ability objects (no BoplBody): fly on.
        if other.layer == 8 && other.collider.is_none_or(|k| k.body.is_none()) {
            return;
        }
        a.landed = true;
        w.emit(Cue::Sound("arrowHit"));
        for k in [P_BOUNDS, P_PROJECTILE, PartKind::BoplBody, PartKind::Collider] {
            w.destroy_part(me, k);
        }
        let target = c.other;
        if let Some(o) = w.objects.get_mut(me) {
            o.tf.destroyed = true;
            o.tf.position = c.contact_point;
        }
        // Presentation cap: more than 80 children drop the first stuck arrow.
        let kids = w.objects.get(target).map(|o| o.children.clone()).unwrap_or_default();
        if kids.len() > 80
            && let Some(first) = kids.iter().copied().find(|k| {
                matches!(w.objects.get(*k).map(|o| &o.kind),
                    Some(Kind::Behaviour(b)) if matches!(b.as_ref(), Behaviour::Bow(_)))
            })
        {
            w.destroy(first);
        }
        w.objects.set_parent(me, Some(target));
    }

    /// Clone rule: `StickTo`, `IgnitesExplosives`, `canHurtOwner` and
    /// `timePassed` carry over; `hasLanded`, the hurt-owner delay (−1) and
    /// `playerOwnerId` (0) are runtime.
    fn duplicate(&self, _w: &World, _me: ObjId) -> Option<Spawned> {
        let Spawned::Arrow(a) = self;
        Some(Spawned::Arrow(Arrow {
            owner: 0,
            landed: false,
            hurt_owner_after: -Fix::ONE,
            ..a.clone()
        }))
    }

    fn lethal(&self, _w: &World, _me: ObjId) -> Option<super::Lethal> {
        let Spawned::Arrow(a) = self;
        Some(super::Lethal {
            owner: a.owner,
            can_hurt_owner: a.can_hurt_owner,
            beam: false,
        })
    }
}


//! Push (`docs/spec/abilities/push.md`): `ControlPlatform`, the player
//! grabs the platform it stands on and pushes it with the stick.
//!
//! Update order inside the object: Ability 1 (framework), SpriteAnimator 2,
//! PlayerPhysics 3, DestroyIfOutsideSceneBounds 4, PlayerCollision 5,
//! ControlPlatform 6, DPhysicsBox 7, PlayerBody 8.
//!
//! No level uses `AnimateGround` (scripted path platforms), so its
//! destruction on grab is a no-op here.

use super::ability_body::{self, AbilityBody};
use super::prefab;
use super::prelude::*;
use crate::data::AnimRef;
use crate::world::Kind;

const CB_BEGIN_CHANNELING: u16 = 1;

/// `(Fix)0.8` (double): external velocity decay per tick.
const F_0_8D: Fix = Fix(3_435_973_836);

/// The Push ability.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Push {
    pub b: AbilityBody,
    pub anim: SpriteAnimator,
    pub platform_force: Fix,
    pub boulder_force_mul: Fix,
    pub home_lerp_speed: Fix,
    pub bound_offset: Fix,
    pub updates_before_animation_update: i32,
    pub a_grab: AnimRef,
    // state
    pub is_channeling: bool,
    pub animator_enabled: bool,
    pub time_since_activation: Fix,
    pub update_count: i32,
}

/// Objects this module spawns.
pub type Spawned = NoSpawn;

impl AbilityImpl for Push {
    fn exit_component(&mut self, cx: &mut Cx<'_>, r: AbilityExitInfo) {
        self.component_exit(cx, r);
    }

    fn new(w: &World, def: &AbilityDef) -> Self {
        let f = prefab::component_fields(w, def, "ControlPlatform");
        let data = f
            .reference("animData")
            .and_then(|r| prefab::anim_data_index(w, r))
            .unwrap_or(0);
        Push {
            b: AbilityBody::new(w, def),
            anim: ability_body::animator(prefab::ability_prefab(w, def).is_some_and(prefab::root_always_stop)),
            platform_force: f.fix_or("platformForce", Fix::from_int(10)),
            boulder_force_mul: f.fix_or("boulderForceMul01", F_0_5),
            home_lerp_speed: f.fix_or("homePositionLerpSpeed", Fix::TWO),
            bound_offset: f.fix_or("SceneBoundLimitOffset", Fix::from_int(5)),
            updates_before_animation_update: f.int("updatesBeforeAnimationUpdate").unwrap_or(5)
                as i32,
            a_grab: prefab::anim(w, data, "grabGround"),
            is_channeling: false,
            animator_enabled: true,
            time_since_activation: Fix::ZERO,
            update_count: 0,
        }
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        let info = cx.info();
        let me = cx.me;
        self.is_channeling = false;
        self.b.sync_physics_to(cx.w, me, &info);
        AbilityBody::update_collider(cx.w, me);
        self.animator_enabled = true;
        self.b.body.external_velocity = Vec2::ZERO;
        self.b.body.self_imposed_velocity = Vec2::ZERO;
        self.time_since_activation = Fix::ZERO;
        if info.is_grounded {
            let a = self.a_grab;
            ability_body::play_then(cx.w, &mut self.anim, a, CB_BEGIN_CHANNELING);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let owner = cx.fw.owner;
        // SpriteAnimator 2.
        if self.animator_enabled && ability_body::advance_code(cx.w, owner, &mut self.anim) == Some(CB_BEGIN_CHANNELING)
        {
            self.is_channeling = true;
            self.animator_enabled = false;
        }
        // PlayerPhysics 3, bounds 4, PlayerCollision 5.
        self.b.physics_update(cx.w, me, dt);
        if let Some(r) = self.b.bounds_update(cx.w, me) {
            self.exit_component(cx, r);
            return;
        }
        AbilityBody::collision_update(cx.fw);
        // ControlPlatform 6.
        self.control_update(cx, dt);
        // PlayerBody 8.
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


    /// The push frame; the sweat (object 1) while channeling with the stick
    /// outside the dead zone.
    fn look(&self, w: &World, _me: ObjId, fw: &Framework, l: &mut crate::view::Look) {
        l.frame(&self.anim);
        let pushing = self.is_channeling
            && w.player(fw.owner)
                .is_some_and(|p| p.inputs.input_vector().magnitude() > DEADZONE);
        l.parts.push(if pushing {
            crate::view::PartLook::new(1)
        } else {
            crate::view::PartLook::hidden(1)
        });
    }

    fn sprite(&self) -> crate::data::SpriteId {
        self.anim.sprite
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        Some(&mut self.b)
    }
}

impl Push {
    fn control_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let owner = cx.fw.owner;
        self.b.body.external_velocity = self.b.body.external_velocity * F_0_8D;
        if let Some(r) = self.b.move_(cx.w, me, Vec2::ZERO, dt, true) {
            self.exit_component(cx, r);
        }
        if cx.w.is_time_stopped_for(owner) {
            return;
        }
        if !self.b.is_grounded() {
            self.release(cx);
            return;
        }
        let v = cx
            .w
            .player(owner)
            .map_or(Vec2::ZERO, |p| p.inputs.input_vector());
        let f = if v.magnitude() < DEADZONE {
            Vec2::ZERO
        } else {
            v * self.platform_force
        };
        let Some(g) = self.b.ground(cx.w) else {
            return;
        };
        let has_spring = match cx.w.objects.get_mut(g).map(|o| &mut o.kind) {
            Some(Kind::Platform(p)) => match &mut p.spring {
                Some(s) => {
                    s.is_being_controlled = true;
                    true
                }
                None => false,
            },
            _ => false,
        };
        let body_pos = AbilityBody::pos(cx.w, me);
        if has_spring && !cx.w.round.sudden_death {
            let gpos = AbilityBody::pos(cx.w, g);
            let pdt = cx.w.fixed_delta_time(owner, dt);
            let t = pdt * self.home_lerp_speed;
            if let Some(Kind::Platform(p)) = cx.w.objects.get_mut(g).map(|o| &mut o.kind)
                && let Some(s) = &mut p.spring
            {
                let h = gpos.lerp(s.home, t);
                let h = Vec2::new(
                    h.x.clamp(CAMERA_X_MIN + self.bound_offset, CAMERA_X_MAX - self.bound_offset),
                    h.y.clamp(Fix::from_int(-10_000), CAMERA_Y_MAX - self.bound_offset),
                );
                s.set_home(h);
            }
            ability_body::add_force_at_position(cx.w, g, f, body_pos, false);
        } else {
            ability_body::add_force_at_position(cx.w, g, f * self.boulder_force_mul, body_pos, false);
        }
        // OldUpdate.
        self.time_since_activation += cx.w.fixed_delta_time(owner, dt);
        if self.is_channeling && self.b.ground(cx.w).is_some() {
            self.update_count += 1;
            if self.update_count > self.updates_before_animation_update {
                self.update_count = 0;
            }
        }
        if !cx.button_down() {
            self.release(cx);
        }
    }

    /// `ExitAbility()`: release the platform, then the component exit.
    fn release(&mut self, cx: &mut Cx<'_>) {
        if let Some(g) = self.b.ground(cx.w)
            && let Some(Kind::Platform(p)) = cx.w.objects.get_mut(g).map(|o| &mut o.kind)
            && let Some(s) = &mut p.spring
        {
            s.is_being_controlled = false;
        }
        self.exit_component(cx, AbilityExitInfo::default());
    }

    /// `ControlPlatform.ExitAbility(record)`.
    fn component_exit(&mut self, cx: &mut Cx<'_>, mut r: AbilityExitInfo) {
        let me = cx.me;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        r.grounded_speed = self.b.physics.grounded_speed;
        r.is_grounded = self.b.physics.is_grounded;
        r.position = AbilityBody::pos(cx.w, me);
        r.self_imposed_velocity = self.b.body.self_imposed_velocity;
        r.external_velocity = self.b.body.external_velocity;
        r.ground = self.b.ground(cx.w);
        r.grounded_local_position = self.b.physics.local_platform_pos;
        r.radius = self.b.physics.radius;
        r.facing_right = cx.fw.info.is_some_and(|i| i.facing_right);
        cx.exit_body(&mut self.b, r);
    }
}

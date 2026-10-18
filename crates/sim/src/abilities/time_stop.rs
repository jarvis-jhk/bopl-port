//! Time stop (`docs/spec/abilities/time-stop.md`): the `CastSpell` channel
//! on the ability object and the `TimeStopSphere` (the global time stop,
//! whose lifetime is counted in IEEE single precision exactly like the
//! original).
//!
//! Update order inside the Time stop object: CastSpell 1 (pre-update),
//! Ability 2 (framework), SpriteAnimator 3, PlayerPhysics 4,
//! DestroyIfOutsideSceneBounds 5, PlayerCollision 6, DPhysicsCircle 7,
//! PlayerBody 8.
//!
//! The global semantics (`PlayerTimeScale`, `isProtectedFromTimeStop`,
//! frozen inputs, animators, physics, cooldowns) are read by the engine
//! through `World::is_time_stopped` / `fixed_delta_time`; this module only
//! writes them.

use super::Behaviour;
use super::ability_body::{self, AbilityBody};
use super::prefab;
use super::prelude::*;
use crate::data::AnimRef;
use crate::physics::shapes::{ShapeKind, circle_rr};
use crate::world::Kind;

const CB_BEGIN_CHANNELING: u16 = 1;
const CB_CAST: u16 = 2;
const CB_EXIT: u16 = 3;

/// Squared speed above which the damping is applied twice (integer 35).
const FAST_SQR_SPEED: i32 = 35;

/// The Time stop ability (`CastSpell`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TimeStop {
    pub b: AbilityBody,
    pub anim: SpriteAnimator,
    pub cast_time: Fix,
    pub center_offset: Vec2,
    pub exit_offset: Vec2,
    pub push_mask: u32,
    pub squish_mask: u32,
    pub inverse_mass: Fix,
    pub air_resistance: Fix,
    pub only_castable_once: bool,
    pub spell: Option<usize>,
    pub a_idle_to_fall: AnimRef,
    pub a_fall_to_loop: AnimRef,
    pub a_loop: AnimRef,
    pub a_cast_spell: AnimRef,
    pub a_ease_out: AnimRef,
    // state
    pub time_since_activation: Fix,
    pub has_fired: bool,
    pub is_casting_spell: bool,
    pub has_started_unstable: bool,
}

/// Objects this module spawns.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    Sphere(TimeStopSphere),
}

/// The `TimeStop` component of `TimeStopSphere` (floats kept as bits).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TimeStopSphere {
    pub caster: i32,
    /// `secondsElapsed` (f32 bits).
    pub seconds_elapsed: u32,
    /// `duration` (f32 bits).
    pub duration: u32,
    /// Time of the last `ExitAlpha` key (f32 bits).
    pub exit_alpha_end: u32,
}

impl TimeStopSphere {
    fn of_prefab(w: &World, prefab: usize) -> TimeStopSphere {
        let f = w
            .content
            .prefabs
            .get(prefab)
            .and_then(|p| p.objects.first())
            .and_then(|o| o.component("TimeStop"))
            .map(|c| c.fields.clone())
            .unwrap_or(Value::Null);
        let float = |v: Option<&Value>| match v {
            Some(Value::Float(b)) => Some(*b),
            Some(Value::Int(i)) => Some((*i as f32).to_bits()),
            _ => None,
        };
        let end = f
            .get("ExitAlpha")
            .map(|c| c.list("m_Curve"))
            .and_then(|keys| keys.last())
            .and_then(|k| float(k.get("time")))
            .unwrap_or(1.501_036_2_f32.to_bits());
        TimeStopSphere {
            caster: 0,
            seconds_elapsed: 0f32.to_bits(),
            duration: float(f.get("duration")).unwrap_or(5f32.to_bits()),
            exit_alpha_end: end,
        }
    }

    /// `UpdateTimeScaleProtection()`: the caster's team is protected.
    fn update_protection(&self, w: &mut World) {
        let team = w.player(self.caster).map(|p| p.team);
        for p in &mut w.players {
            p.is_protected_from_time_stop = Some(p.team) == team;
        }
    }
}

/// `(float)fix`: the raw value converted to f32, divided by 2³² in f32.
pub fn fix_to_f32(f: Fix) -> f32 {
    (f.0 as f32) / 4_294_967_296f32
}

impl AbilityImpl for TimeStop {
    fn exit_component(&mut self, cx: &mut Cx<'_>, r: AbilityExitInfo) {
        self.component_exit(cx, r);
    }

    fn new(w: &World, def: &AbilityDef) -> Self {
        let f = prefab::component_fields(w, def, "CastSpell");
        let data = f
            .reference("animData")
            .and_then(|r| prefab::anim_data_index(w, r))
            .unwrap_or(0);
        let a = |n: &str| prefab::anim(w, data, n);
        TimeStop {
            b: AbilityBody::new(w, def),
            anim: ability_body::animator(prefab::ability_prefab(w, def).is_some_and(prefab::root_always_stop)),
            cast_time: f.fix_or("castTime", Fix::from_int(9)),
            center_offset: f.vec2("centerOffset").unwrap_or_default(),
            exit_offset: f.vec2("ExitAbilityOffset").unwrap_or_default(),
            push_mask: f.int("platformPushLayerMask").unwrap_or(0) as u32,
            squish_mask: f.int("squishLayerMask").unwrap_or(0) as u32,
            inverse_mass: f.fix_or("inverseMass", Fix::ZERO),
            air_resistance: f.fix_or("airResistance01", Fix::ONE),
            only_castable_once: f.boolean("onlyCastableOnce").unwrap_or(false),
            spell: f
                .reference("spell")
                .and_then(|r| prefab::prefab_index(w, r)),
            a_idle_to_fall: a("idleToFall"),
            a_fall_to_loop: a("fallToLoop"),
            a_loop: a("loop"),
            a_cast_spell: a("castSpell"),
            a_ease_out: a("easeOutSpellCast"),
            time_since_activation: Fix::ZERO,
            has_fired: false,
            is_casting_spell: false,
            has_started_unstable: false,
        }
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        let info = cx.info();
        let me = cx.me;
        self.b.sync_physics_to(cx.w, me, &info);
        AbilityBody::update_collider(cx.w, me);
        self.b.body.self_imposed_velocity = Vec2::ZERO;
        self.b.body.external_velocity = Vec2::ZERO;
        self.time_since_activation = Fix::ZERO;
        if info.is_grounded {
            let a = self.a_idle_to_fall;
            ability_body::play_then(cx.w, &mut self.anim, a, CB_BEGIN_CHANNELING);
        } else {
            self.begin_channeling(cx);
        }
        self.has_fired = false;
        self.is_casting_spell = false;
        self.has_started_unstable = false;
    }

    fn pre_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        // CastSpell (hierarchy 1).
        self.cast_spell_update(cx, dt);
    }

    fn update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let owner = cx.fw.owner;
        // SpriteAnimator 3.
        match ability_body::advance_code(cx.w, owner, &mut self.anim) {
            Some(CB_BEGIN_CHANNELING) => self.begin_channeling(cx),
            Some(CB_CAST) => self.cast(cx),
            Some(CB_EXIT) => self.exit_component(cx, AbilityExitInfo::default()),
            _ => {}
        }
        if !cx.w.objects.get(me).is_some_and(|o| o.active) {
            return;
        }
        // PlayerPhysics 4, bounds 5, PlayerCollision 6, PlayerBody 8.
        self.b.physics_update(cx.w, me, dt);
        if let Some(r) = self.b.bounds_update(cx.w, me) {
            self.exit_component(cx, r);
            return;
        }
        AbilityBody::collision_update(cx.fw);
        self.b.body_update(cx.w, me, dt);
    }

    fn late_update(&mut self, cx: &mut Cx<'_>, _dt: Fix) {
        self.b.physics_late_update(cx.w, cx.me);
        self.b.body_late_update(cx.w, cx.me);
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        ability_body::hurt_contact(self, cx, c);
    }


    /// The channel: the aura (object 1) while channeling with `params` =
    /// (seconds channeled, seconds since unstable or −1);
    /// `CreepyAmbientElectrical` meanwhile.
    fn look(&self, w: &World, _me: ObjId, fw: &Framework, l: &mut crate::view::Look) {
        l.frame(&self.anim);
        let channeling = fw.in_use && !self.is_casting_spell && !self.has_fired;
        if channeling {
            l.parts.push(crate::view::PartLook::new(1));
            l.params[0] = self.time_since_activation;
            let unstable_at = self.cast_time - Fix::TWO;
            l.params[1] = if self.has_started_unstable {
                self.time_since_activation - unstable_at
            } else {
                -Fix::ONE
            };
            l.params[3] = Fix::ONE;
            if !w.is_time_stopped_for(fw.owner) {
                l.loop_sound = Some("CreepyAmbientElectrical");
            }
        } else {
            l.parts.push(crate::view::PartLook::hidden(1));
        }
    }

    fn sprite(&self) -> crate::data::SpriteId {
        self.anim.sprite
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        Some(&mut self.b)
    }
}

impl TimeStop {
    fn begin_channeling(&mut self, cx: &mut Cx<'_>) {
        let me = cx.me;
        self.b.unground(cx.w, me, true, false);
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        self.b.gravity_modifier = Fix::ZERO;
        let (a, l) = (self.a_fall_to_loop, self.a_loop);
        ability_body::play_then_loop(cx.w, &mut self.anim, a, l, None);
    }

    fn cast_spell_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let owner = cx.fw.owner;
        if self.b.is_grounded() && self.b.ground_alive(cx.w).is_some() {
            self.b.move_along_current_platform(cx.w, me, dt);
        } else {
            if cx.w.is_time_stopped_for(owner) {
                return;
            }
            let pos = AbilityBody::pos(cx.w, me);
            let radius = cx.w.physics.circle(me).map_or(Fix::ZERO, |c| c.radius);
            // Push out of platforms.
            let hits = cx
                .w
                .physics
                .circle_cast_all(&cx.w.objects, pos, radius, self.push_mask, 5);
            for h in hits {
                if cx.w.physics.find(h).map(|(k, _)| k) != Some(ShapeKind::RoundedRect) {
                    continue;
                }
                let (Some(c), Some(r)) = (cx.w.physics.circle(me), cx.w.physics.rr(h)) else {
                    continue;
                };
                if let Some(m) = circle_rr(c, r) {
                    self.b.body_add_force((-m.normal) * m.penetration * self.inverse_mass);
                }
            }
            // Squished between platforms.
            let hits = cx
                .w
                .physics
                .circle_cast_all(&cx.w.objects, pos, radius, self.squish_mask, 5);
            if hits.len() >= 2 {
                for h in hits {
                    let inside = match cx.w.physics.find(h) {
                        Some((ShapeKind::Circle, _)) => {
                            cx.w.physics.circle(h).is_some_and(|c| c.contains(pos))
                        }
                        Some((ShapeKind::Box, _)) => {
                            cx.w.physics.box_shape(h).is_some_and(|b| b.contains(pos))
                        }
                        Some(_) => cx.w.physics.rr(h).is_some_and(|r| r.contains(pos)),
                        None => false,
                    };
                    if inside {
                        AbilityImpl::kill(self, cx, ENVIRONMENT_ID, true, true, CauseOfDeath::Squished);
                        return;
                    }
                }
            }
            let k = self.air_resistance;
            let body = &mut self.b.body;
            body.self_imposed_velocity = body.self_imposed_velocity * k;
            body.external_velocity = body.external_velocity * k;
            if body.velocity().sqr_magnitude() > Fix::from_int(FAST_SQR_SPEED) {
                body.self_imposed_velocity = body.self_imposed_velocity * k;
                body.external_velocity = body.external_velocity * k;
            }
        }
        self.time_since_activation += cx.w.fixed_delta_time(owner, dt);
        if !self.is_casting_spell
            && (!cx.button_down() || self.time_since_activation > self.cast_time)
        {
            self.has_fired = self.time_since_activation > self.cast_time;
            if !self.has_fired {
                // Presentation: `DarknessAura.CancelEffect`.
                aura_fx(cx, "ExitEffectPrefab");
                self.exit_component(cx, AbilityExitInfo::default());
                return;
            }
            let a = self.a_cast_spell;
            ability_body::play_then(cx.w, &mut self.anim, a, CB_CAST);
            self.is_casting_spell = true;
        }
        if self.time_since_activation >= self.cast_time - Fix::TWO && !self.has_started_unstable {
            self.has_started_unstable = true;
        }
    }

    fn cast(&mut self, cx: &mut Cx<'_>) {
        if self.only_castable_once {
            cx.fw.is_castable = false;
        }
        cx.w.emit(Cue::Sound("timeStop"));
        // Presentation: `DarknessAura.ExitEffect`.
        aura_fx(cx, "BlackBorderPrefab");
        aura_fx(cx, "BlackSpherePrefab");
        if let Some(prefab) = self.spell {
            let Some(t) = cx.w.objects.get(cx.me).map(|o| o.tf) else {
                return;
            };
            let mut s = TimeStopSphere::of_prefab(cx.w, prefab);
            s.caster = cx.fw.owner;
            let id = ability_body::spawn_prefab(
                cx.w,
                prefab,
                Kind::Behaviour(Box::new(Behaviour::TimeStop(Spawned::Sphere(s)))),
                &[("TimeStop", PartKind::Behaviour(0))],
                t.position + self.center_offset * t.scale,
                None,
            );
            if let Some(o) = cx.w.objects.get_mut(id) {
                o.player_id = cx.fw.owner;
            }
        }
        let a = self.a_ease_out;
        ability_body::play_then(cx.w, &mut self.anim, a, CB_EXIT);
    }

    /// `CastSpell.ExitAbility(record)`.
    fn component_exit(&mut self, cx: &mut Cx<'_>, mut r: AbilityExitInfo) {
        let me = cx.me;
        let owner = cx.fw.owner;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        self.b.unground(cx.w, me, true, false);
        let scale = cx.w.player(owner).map_or(Fix::ONE, |p| p.scale());
        r.grounded_speed = self.b.physics.grounded_speed;
        r.is_grounded = self.b.physics.is_grounded;
        r.position = AbilityBody::pos(cx.w, me) + self.exit_offset * scale;
        r.self_imposed_velocity = self.b.body.self_imposed_velocity;
        r.external_velocity = Vec2::ZERO;
        r.ground = self.b.ground(cx.w);
        r.grounded_local_position = self.b.physics.local_platform_pos;
        r.radius = self.b.physics.radius;
        cx.exit_body(&mut self.b, r);
    }
}

impl super::SpawnedImpl for Spawned {
    /// The inversion sphere: `params` = (seconds elapsed, duration).
    fn look(&self, _w: &World, _me: ObjId, l: &mut crate::view::Look) {
        let Spawned::Sphere(s) = self;
        l.params[0] = Fix::from_f32(f32::from_bits(s.seconds_elapsed));
        l.params[1] = Fix::from_f32(f32::from_bits(s.duration));
    }

    fn init(&mut self, w: &mut World, _me: ObjId, _part: PartKind) {
        let Spawned::Sphere(s) = self;
        s.seconds_elapsed = 0f32.to_bits();
        s.update_protection(w);
    }

    fn update(&mut self, w: &mut World, me: ObjId, _part: PartKind, dt: Fix, late: bool) {
        let Spawned::Sphere(s) = self;
        if late {
            return;
        }
        let e = f32::from_bits(s.seconds_elapsed) + fix_to_f32(dt);
        s.seconds_elapsed = e.to_bits();
        w.time.player_time_scale = Fix::ZERO;
        let threshold = f32::from_bits(s.duration) + f32::from_bits(s.exit_alpha_end);
        if e > threshold {
            // End.
            if w.round.game_in_progress {
                for p in &mut w.players {
                    p.is_protected_from_time_stop = false;
                }
                w.time.player_time_scale = Fix::ONE;
            }
            w.destroy(me);
        } else {
            s.update_protection(w);
        }
    }
}

/// A `DarknessAura` effect prefab at the aura (`Aura` at (0, 0.6) plus the
/// black sphere prefab's own (0, 0.7)).
fn aura_fx(cx: &mut Cx<'_>, field: &'static str) {
    let p = AbilityBody::pos(cx.w, cx.me) + Vec2::new(Fix::ZERO, Fix::from_int(13) / Fix::from_int(10));
    let o = cx.fw.owner;
    super::fx(cx.w, cx.me, field, p, Fix::ZERO, Fix::ONE, o, Vec2::ZERO);
}

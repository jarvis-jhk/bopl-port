//! Missile (`docs/spec/abilities/missile.md`): the `SparkShot` ability and
//! the steerable `Spark` rocket (`Missile` component). The rocket's
//! explosion is a `MineExplosion` whose owner is never set (killer 0).

use std::sync::Arc;

use super::Behaviour;
use super::ability_body::{self, AbilityBody, Bounds, advance_anim, play_named as play, play_named_then as play_then, prefab_parts};
use super::prefab::{self, fx, prefab_field, root_fields, vec2_field};
use super::prelude::*;
use crate::anim::SpriteAnimator;
use crate::world::Kind;

/// `SparkShot` constants.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MissileCfg {
    pub anim_data: u16,
    pub projectile_offset: Vec2,
    pub start_speed: Fix,
    pub rotation_speed: Fix,
    pub spark: Option<Arc<SparkCfg>>,
}

/// The `Spark` rocket prefab (`Missile` component).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SparkCfg {
    pub prefab: usize,
    pub min_forward_speed: Fix,
    pub accel: Fix,
    pub instability_accel: Fix,
    pub explosion: Option<usize>,
    pub parts: Vec<(PartKind, i32, bool)>,
    pub bounds: Bounds,
}

impl SparkCfg {
    pub fn from_prefab(w: &World, prefab: usize) -> Option<SparkCfg> {
        let p = w.content.prefabs.get(prefab)?;
        let f = root_fields(p, "Missile")?;
        Some(SparkCfg {
            prefab,
            min_forward_speed: fx(f, "minimumForwardsSpeed", Fix::ZERO),
            accel: fx(f, "accel", Fix::ZERO),
            instability_accel: fx(f, "instabilityAccel", Fix::ZERO),
            explosion: prefab_field(w, f, "onHitExplosionPrefab"),
            // The rocket's animators only pick sprites (not simulated).
            parts: prefab_parts(w, prefab, &[("Missile", PartKind::Behaviour(0))])
                .into_iter()
                .filter(|p| p.0 != PartKind::Animator)
                .collect(),
            bounds: Bounds::of_prefab(w, prefab),
        })
    }
}

const F_0_8F: Fix = Fix(3_435_973_888);

const CB_FIRE: u16 = 1;
const CB_IDLE: u16 = 2;
const CB_FINISH: u16 = 3;
const CB_TO_RIGHT: u16 = 4;
const CB_TO_LEFT: u16 = 5;
const CB_TO_IDLE: u16 = 6;

/// `SparkShot.State`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShotState {
    #[default]
    Enter,
    Idle,
    Right,
    Left,
    InTransition,
    Exit,
}

/// The Missile ability object (`SparkShot`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Missile {
    pub cfg: Option<Arc<MissileCfg>>,
    pub body: AbilityBody,
    pub anim: SpriteAnimator,
    pub state: ShotState,
    /// Mirror sign is +1 (facing right at cast).
    pub mirror_pos: bool,
    pub spark: Option<ObjId>,
    pub charge_time: Fix,
}

impl Missile {
    /// `InSparkShot` (if given) and `CustomAimAbilityPosition` (if given).
    fn set_player_flags(w: &mut World, owner: i32, in_shot: Option<bool>, aim: Option<Vec2>) {
        if let Some(p) = w.player_mut(owner) {
            if let Some(f) = in_shot {
                p.in_spark_shot = f;
            }
            if let Some(a) = aim {
                p.custom_aim_ability_position = a;
            }
        }
    }

    fn fire(&mut self, cx: &mut Cx<'_>) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let (me, owner) = (cx.me, cx.fw.owner);
        let w = &mut *cx.w;
        let scale = AbilityBody::scale(w, me);
        let rot = AbilityBody::rotation(w, me);
        let p = AbilityBody::pos(w, me) + Vec2::from_angle(rot).complex_mul(scale * cfg.projectile_offset);
        let up = w.objects.get(me).map_or(Vec2::UP, |o| o.tf.up);
        if let Some(sc) = &cfg.spark {
            let id = spawn_spark(w, sc, p, rot, scale, cfg.start_speed * up, owner);
            self.spark = Some(id);
        }
        Self::set_player_flags(w, owner, None, Some(p));
        play_then(w, &mut self.anim, cfg.anim_data, "enterToIdle", CB_IDLE);
        w.emit(Cue::Sound("transform1"));
    }

    fn on_code(&mut self, cx: &mut Cx<'_>, code: u16) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        match code {
            CB_FIRE => self.fire(cx),
            CB_IDLE | CB_TO_IDLE => {
                play(cx.w, &mut self.anim, cfg.anim_data, "idle");
                self.state = ShotState::Idle;
            }
            CB_FINISH => {
                Self::set_player_flags(cx.w, cx.fw.owner, Some(false), None);
                self.exit_component(cx, AbilityExitInfo::default());
            }
            CB_TO_RIGHT => {
                play(cx.w, &mut self.anim, cfg.anim_data, "right");
                self.state = ShotState::Right;
            }
            CB_TO_LEFT => {
                play(cx.w, &mut self.anim, cfg.anim_data, "left");
                self.state = ShotState::Left;
            }
            _ => {}
        }
    }

    fn transition(&mut self, w: &World, name: &str, code: u16) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        play_then(w, &mut self.anim, cfg.anim_data, name, code);
        self.state = ShotState::InTransition;
    }

    /// `SparkShot.UpdateSim`.
    fn shot_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let (me, owner) = (cx.me, cx.fw.owner);
        self.charge_time += cx.w.fixed_delta_time(owner, dt);
        // Release check.
        let protected = cx.w.player(owner).is_some_and(|p| p.is_protected_from_time_stop);
        if (!cx.w.is_time_stopped() || protected)
            && !cx.button_down()
            && self.state != ShotState::Enter
            && self.state != ShotState::Exit
        {
            self.state = ShotState::Exit;
            play_then(cx.w, &mut self.anim, cfg.anim_data, "idle", CB_FINISH);
        }
        // Ground handling.
        if !self.body.is_grounded() {
            if self.body.velocity_based_raycasts(cx.w, me, true, dt) {
                AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
                let info = AbilityExitInfo {
                    just_landed: true,
                    grounded_speed: self.body.physics.grounded_speed,
                    is_grounded: self.body.physics.is_grounded,
                    facing_right: true,
                    position: AbilityBody::pos(cx.w, me),
                    self_imposed_velocity: self.body.body.self_imposed_velocity,
                    external_velocity: self.body.body.external_velocity,
                    ground: self.body.physics.attached_ground,
                    grounded_local_position: self.body.physics.local_platform_pos,
                    radius: self.body.physics.radius,
                    ..AbilityExitInfo::default()
                };
                Self::set_player_flags(cx.w, owner, Some(false), None);
                cx.exit_body(&mut self.body, info);
                return;
            }
        } else {
            let gone = self
                .body
                .physics
                .attached_ground
                .and_then(|g| cx.w.objects.get(g))
                .is_none_or(|o| !o.active);
            if gone {
                AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
                let info = AbilityExitInfo {
                    is_grounded: false,
                    facing_right: true,
                    position: AbilityBody::pos(cx.w, me),
                    self_imposed_velocity: self.body.body.self_imposed_velocity,
                    external_velocity: self.body.body.external_velocity,
                    ground: None,
                    radius: self.body.physics.radius,
                    ..AbilityExitInfo::default()
                };
                Self::set_player_flags(cx.w, owner, Some(false), None);
                cx.exit_body(&mut self.body, info);
                return;
            }
        }
        if let Some(info) = self.body.move_(cx.w, me, Vec2::ZERO, dt, true) {
            // (The original carries on with the steering after this exit.)
            self.exit_component(cx, info);
        }
        // Steering.
        let Some(s) = self.spark.filter(|s| cx.w.objects.alive(*s)) else {
            return;
        };
        if !ability_body::collider_initialized(cx.w, s) || self.state == ShotState::Exit {
            return;
        }
        let spos = cx.w.objects.get(s).map_or(Vec2::ZERO, |o| o.tf.position);
        Self::set_player_flags(cx.w, owner, None, Some(spos));
        let mut a = cx.w.player(owner).map_or(Vec2::ZERO, |p| p.inputs.aim_vector());
        let vel = cx.w.physics.body(s).map_or(Vec2::ZERO, |b| b.velocity);
        let vdir = vel.normalized_safe();
        if let Some(r) = rocket_mut(cx.w, s) {
            r.instability = 0;
        }
        let pdt = cx.w.fixed_delta_time(owner, dt);
        let theta = if vdir.cross(a) > Fix::ZERO {
            pdt * cfg.rotation_speed
        } else {
            (-pdt) * cfg.rotation_speed
        };
        let sup = cx.w.objects.get(s).map_or(Vec2::UP, |o| o.tf.up);
        let mut u = Vec2::from_angle(theta).complex_mul(sup);
        if a.sqr_magnitude() < DEADZONE * DEADZONE {
            a = sup;
            u = sup;
        }
        let bt = cx.w.objects.get(me).map(|o| o.tf).unwrap_or_default();
        if a.dot(bt.up).abs() > F_0_8F {
            match self.state {
                ShotState::Left => self.transition(cx.w, "leftToIdle", CB_TO_IDLE),
                ShotState::Right => self.transition(cx.w, "rightToIdle", CB_TO_IDLE),
                _ => {}
            }
        } else if a.dot(bt.right) * if self.mirror_pos { Fix::ONE } else { -Fix::ONE } > Fix::ZERO {
            match self.state {
                ShotState::Idle => self.transition(cx.w, "idleToRight", CB_TO_RIGHT),
                ShotState::Left => self.transition(cx.w, "leftToIdle", CB_TO_IDLE),
                _ => {}
            }
        } else {
            match self.state {
                ShotState::Idle => self.transition(cx.w, "idleToLeft", CB_TO_LEFT),
                ShotState::Right => self.transition(cx.w, "rightToIdle", CB_TO_IDLE),
                _ => {}
            }
        }
        let speed = vel.magnitude();
        if let Some(b) = cx.w.physics.body_mut(s) {
            b.velocity = u * speed;
        }
        if let Some(o) = cx.w.objects.get_mut(s) {
            o.tf.set_up(u);
        }
    }
}

impl AbilityImpl for Missile {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let Some(p) = def.prefab else {
            return Missile::default();
        };
        let pf = &w.content.prefabs[p];
        let Some(f) = root_fields(pf, "SparkShot") else {
            return Missile::default();
        };
        let spark = prefab_field(w, f, "SparkPrefab")
            .and_then(|i| SparkCfg::from_prefab(w, i))
            .map(Arc::new);
        let cfg = MissileCfg {
            anim_data: prefab::anim_data_field(w, f, "animData").unwrap_or(0),
            projectile_offset: vec2_field(f, "projectileOffset").unwrap_or(Vec2::ZERO),
            start_speed: fx(f, "projectileStartSpeed", Fix::from_int(15)),
            rotation_speed: fx(f, "SparkRotationSpeed", Fix::from_int(100)),
            spark,
        };
        let anim = ability_body::animator(prefab::root_always_stop(pf));
        Missile {
            cfg: Some(Arc::new(cfg)),
            body: AbilityBody::from_prefab(w, p),
            anim,
            ..Missile::default()
        }
    }

    fn build(&mut self, w: &mut World, me: ObjId, fw: &mut Framework, def: &AbilityDef) {
        self.body.install(fw);
        ability_body::add_parts(w, me, fw, def, &[("SparkShot", PartKind::Behaviour(0))]);
    }

    fn body(&self) -> Option<&AbilityBody> {
        self.cfg.as_ref().map(|_| &self.body)
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        if self.cfg.is_some() {
            Some(&mut self.body)
        } else {
            None
        }
    }

    fn sprite(&self) -> crate::data::SpriteId {
        self.anim.sprite
    }

    fn look(&self, _w: &World, _me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        l.frame(&self.anim);
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        let Some(cfg) = self.cfg.clone() else {
            let i = cx.info();
            cx.exit(AbilityExitInfo::resume(&i));
            return;
        };
        let info = cx.info();
        let me = cx.me;
        self.body.owner = cx.fw.owner;
        self.body.sync_physics_to(cx.w, me, &info);
        AbilityBody::update_collider(cx.w, me);
        AbilityBody::set_rotation(cx.w, me, info.rotation);
        self.mirror_pos = info.facing_right;
        let pos = AbilityBody::pos(cx.w, me);
        Self::set_player_flags(cx.w, info.player_id, Some(true), Some(pos));
        if !info.is_grounded {
            self.body.body.external_velocity = info.external_velocity;
            self.body.body.self_imposed_velocity = info.self_imposed_velocity;
        }
        play_then(cx.w, &mut self.anim, cfg.anim_data, "enter", CB_FIRE);
        self.state = ShotState::Enter;
        self.spark = None;
        self.charge_time = Fix::ZERO;
    }

    fn update_part(&mut self, cx: &mut Cx<'_>, k: PartKind, dt: Fix, late: bool) {
        match (k, late) {
            (PartKind::Animator, false) => {
                if let Some(AnimCallback::Ability(code)) = advance_anim(cx.w, cx.fw.owner, &mut self.anim) {
                    self.on_code(cx, code);
                }
            }
            (PartKind::Bounds, false) => {
                if self.body.cfg.bounds.check(cx.w, cx.me) {
                    let info = ability_body::dead_record(cx.w, cx.me, ENVIRONMENT_ID, CauseOfDeath::Other);
                    self.exit_component(cx, info);
                }
            }
            (PartKind::Behaviour(0), false) => self.shot_update(cx, dt),
            _ => {
                if let Some(r) = self.body.update_part(cx, k, dt, late) {
                    self.exit_component(cx, r);
                }
            }
        }
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        let me = cx.me;
        if let Some((killer, cause)) = self.body.collision_contact(cx.w, me, cx.fw, c) {
            let info = ability_body::dead_record(cx.w, cx.me, killer, cause);
            self.exit_component(cx, info);
        }
    }


    /// `SparkShot.ExitAbility(info)`.
    fn exit_component(&mut self, cx: &mut Cx<'_>, mut info: AbilityExitInfo) {
        let me = cx.me;
        info.self_imposed_velocity = self.body.body.self_imposed_velocity;
        info.external_velocity = self.body.body.external_velocity;
        info.facing_right = self.mirror_pos;
        info.position = AbilityBody::pos(cx.w, me);
        info.grounded_speed = self.body.physics.grounded_speed;
        info.is_grounded = self.body.physics.is_grounded;
        info.ground = self.body.physics.attached_ground;
        info.grounded_local_position = self.body.physics.local_platform_pos;
        info.radius = self.body.physics.radius;
        Self::set_player_flags(cx.w, cx.fw.owner, Some(false), None);
        cx.exit_body(&mut self.body, info);
    }
}

// -- the rocket ------------------------------------------------------------------------

/// A `Spark` rocket.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Rocket {
    pub cfg: Arc<SparkCfg>,
    pub owner: i32,
    pub instability: i32,
    pub instable: bool,
}

/// `InstantiateFixed(Spark, pos, rot)`, scale, start velocity, owner.
pub fn spawn_spark(w: &mut World, cfg: &Arc<SparkCfg>, pos: Vec2, rot: Fix, scale: Fix, v: Vec2, owner: i32) -> ObjId {
    let r = Rocket {
        cfg: cfg.clone(),
        owner,
        instability: 0,
        instable: false,
    };
    let id = ability_body::spawn_owned(w, cfg.prefab, Kind::Behaviour(Box::new(Behaviour::Missile(Spawned::Rocket(r)))), &cfg.parts, pos, Some(rot), owner);
    w.set_collider_scale(id, scale);
    if let Some(o) = w.objects.get_mut(id)
        && let Some(c) = o.collider.as_mut()
        && let Some(b) = c.body.as_mut()
    {
        b.start_velocity = v;
    }
    id
}

/// The rocket state of an object.
pub fn rocket_mut(w: &mut World, s: ObjId) -> Option<&mut Rocket> {
    match &mut w.objects.get_mut(s)?.kind {
        Kind::Behaviour(b) => match b.as_mut() {
            Behaviour::Missile(Spawned::Rocket(r)) => Some(r),
            _ => None,
        },
        _ => None,
    }
}

/// Objects of this module.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    Rocket(Rocket),
}

impl SpawnedImpl for Spawned {
    /// The rocket's smoke trail (`particleSmokePrefab`) at the nozzle
    /// (`particleSmokeOffset` × scale, rotated with the rocket), paused in a
    /// time stop; `loopEngine` while it flies.
    fn look(&self, w: &World, me: ObjId, l: &mut crate::view::Look) {
        // `SparkShot` gives the rocket the owner's material.
        l.player_material = true;
        let Some(o) = w.objects.get(me) else {
            return;
        };
        if w.is_time_stopped() {
            return;
        }
        l.loop_sound = Some("loopEngine");
        let off = super::prefab::object_fields(w, me, "Missile")
            .and_then(|f| f.get("particleSmokeOffset"))
            .and_then(super::prefab::float_vec2)
            .unwrap_or(Vec2::new(Fix::ZERO, -Fix::from_int(7) / Fix::from_int(4)));
        let p = o.tf.position + Vec2::from_angle(o.tf.rotation).complex_mul(off * o.tf.scale);
        l.attach.push(crate::view::Attach {
            field: "particleSmokePrefab",
            position: p,
            rotation: o.tf.rotation,
            scale: o.tf.scale,
            color: None,
            shadow_color: false,
        });
    }

    fn init(&mut self, w: &mut World, _me: ObjId, part: PartKind) {
        if part == PartKind::Behaviour(0) {
            w.emit(Cue::Sound("startEngine"));
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, _dt: Fix, late: bool) {
        let Spawned::Rocket(r) = self;
        if late {
            return;
        }
        match part {
            PartKind::Behaviour(0) => {
                r.instability += 1;
                let up = w.objects.get(me).map_or(Vec2::UP, |o| o.tf.up);
                if r.instability >= 3 {
                    r.instable = true;
                    if !w.is_time_stopped() && !ability_body::is_held(w, me) {
                        w.physics.add_force(me, up * r.cfg.instability_accel);
                    }
                } else {
                    let v = w.physics.body(me).map_or(Vec2::ZERO, |b| b.velocity);
                    if up.dot(v) < r.cfg.min_forward_speed && !w.is_time_stopped() && !ability_body::is_held(w, me) {
                        w.physics.add_force(me, up * r.cfg.accel);
                    }
                }
            }
            PartKind::Bounds => {
                let b = r.cfg.bounds;
                b.check_and_destroy(w, me);
            }
            _ => {}
        }
    }

    fn on_contact(&mut self, w: &mut World, me: ObjId, c: &Contact) {
        let Spawned::Rocket(r) = self;
        if c.layer == 23 || c.layer == 24 || ability_body::is_held(w, me) {
            return;
        }
        let pos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        let scale = w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale);
        if let Some(e) = r.cfg.explosion {
            // PlayerOwnerId is never set: the explosion belongs to "player 0".
            super::explosion::spawn(w, e, pos, Some(scale), 0);
        }
        w.emit(Cue::Sound("mineExplosion"));
        w.destroy(me);
    }

    fn on_destroy(&mut self, w: &mut World, _me: ObjId, part: PartKind) {
        if part == PartKind::Behaviour(0) {
            w.emit(Cue::Sound("endEngine"));
        }
    }

    /// Item kind; Duplicator fix-ups: up := the clone's normalised
    /// velocity, item owner := the original's.
    fn interact(&mut self, w: &mut World, me: ObjId, i: super::Interaction) -> Option<Fix> {
        let Spawned::Rocket(r) = self;
        match i {
            super::Interaction::ItemKind => Some(Fix::from_int(super::ITEM_MISSILE)),
            super::Interaction::Duplicated { original, .. } => {
                let v = w
                    .objects
                    .get(me)
                    .and_then(|o| o.collider)
                    .and_then(|c| c.body)
                    .map_or(Vec2::ZERO, |b| b.start_velocity);
                if let Some(o) = w.objects.get_mut(me) {
                    o.tf.set_up(v.normalized_safe());
                }
                if let Some(Kind::Behaviour(b)) = w.objects.get(original).map(|o| &o.kind)
                    && let Behaviour::Missile(Spawned::Rocket(o)) = b.as_ref()
                {
                    r.owner = o.owner;
                }
                Some(Fix::ZERO)
            }
            _ => None,
        }
    }

    /// Clone rule: tuning kept; spawn time, steering, instability and
    /// `Item.OwnerId` (0) are runtime.
    fn duplicate(&self, _w: &World, _me: ObjId) -> Option<Spawned> {
        let Spawned::Rocket(r) = self;
        Some(Spawned::Rocket(Rocket {
            cfg: r.cfg.clone(),
            owner: 0,
            instability: 0,
            instable: false,
        }))
    }
}

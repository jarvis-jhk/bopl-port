//! Grenade (`docs/spec/abilities/grenade.md`) and the shared throw-and-hold
//! component `ThrowItem2` (canonical home; Smoke uses it with its own
//! values). The thrown items (`Grenade` and `SmokeGrenade`, both a `Grenade`
//! fuse component with a detonation) are [`GrenadeItem`]s.

use std::sync::Arc;

use super::ability_body::{self, AbilityBody, Bounds, advance_anim, play_named as play, play_named_then as play_then, prefab_parts};
use super::prefab::{self, fx, prefab_field, root_fields, vec2_field, vec2_list};
use super::prelude::*;
use super::smoke::{self, RingBuffer, SmokeBurstCfg};
use super::Behaviour;
use crate::anim::SpriteAnimator;
use crate::world::Kind;

/// `ThrowItem2` constants (Grenade and Smoke ability prefabs).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ThrowCfg {
    pub prefab: usize,
    pub anim_data: u16,
    pub a_throw_position: Vec2,
    pub g_throw_position: Vec2,
    pub grounded_dummy: Vec<Vec2>,
    pub aireal_dummy: Vec<Vec2>,
    pub disable_hitbox_time: Fix,
    pub max_throw_force: Fix,
    pub min_throw_force: Fix,
    pub gain_speed: Fix,
    pub max_slowmo_speed: Fix,
    pub knockback: Fix,
    pub max_effects: usize,
    pub item: Option<Arc<GrenadeItemCfg>>,
}

/// Code literals of ThrowItem2 (float path).
const F_0_03F: Fix = Fix(128_849_016);
const F_0_1F: Fix = Fix(429_496_736);
const F_0_6F: Fix = Fix(2_576_980_480);

const CB_GROUNDED_ENTER: u16 = 1;
const CB_AIREAL_ENTER: u16 = 2;
const CB_EXIT: u16 = 3;

/// The `ThrowItem2` component state (one per Grenade / Smoke slot).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ThrowItem2 {
    pub cfg: Option<Arc<ThrowCfg>>,
    pub body: AbilityBody,
    pub anim: SpriteAnimator,
    pub dir: Vec2,
    pub has_inputted_fire: bool,
    /// `FireInputTimeStamp`.
    pub stamp: Fix,
    pub time_passed: Fix,
    pub time_since_fired: Fix,
    pub has_fired: bool,
    pub is_in_loop: bool,
    pub is_grounded_throw: bool,
    /// The held item (`grenade` / `grenadeBody`).
    pub grenade: Option<ObjId>,
    /// Negative x scale (= facing right).
    pub mirrored: bool,
    /// `effectBuffer` (smoke clouds).
    pub effect_buffer: RingBuffer,
}

impl ThrowItem2 {
    /// Reads the component from an ability prefab.
    pub fn new(w: &World, def: &AbilityDef) -> ThrowItem2 {
        let Some(p) = def.prefab else {
            return ThrowItem2::default();
        };
        let pf = &w.content.prefabs[p];
        let Some(f) = root_fields(pf, "ThrowItem2") else {
            return ThrowItem2::default();
        };
        let item = prefab_field(w, f, "ItemPrefab")
            .and_then(|i| GrenadeItemCfg::from_prefab(w, i))
            .map(Arc::new);
        let max_effects = f.int("maxEffectsSpawnedFromGrenades").unwrap_or(0).max(0) as usize;
        let cfg = ThrowCfg {
            prefab: p,
            anim_data: prefab::anim_data_field(w, f, "animData").unwrap_or(0),
            a_throw_position: vec2_field(f, "aThrowPosition").unwrap_or(Vec2::ZERO),
            g_throw_position: vec2_field(f, "gThrowPosition").unwrap_or(Vec2::ZERO),
            grounded_dummy: vec2_list(f, "groundedDummyPositions"),
            aireal_dummy: vec2_list(f, "airealDummyPositions"),
            disable_hitbox_time: fx(f, "DisableGrenadeHitBoxTime", Fix::ZERO),
            max_throw_force: fx(f, "maxThrowForce", Fix::ZERO),
            min_throw_force: fx(f, "minThrowForce", Fix::ZERO),
            gain_speed: fx(f, "ThrowForceGainSpeed", Fix::ONE),
            max_slowmo_speed: fx(f, "maxSlowMoSpeed", Fix::ZERO),
            knockback: fx(f, "knockback", Fix::ZERO),
            max_effects,
            item,
        };
        let anim = ability_body::animator(prefab::root_always_stop(pf));
        ThrowItem2 {
            cfg: Some(Arc::new(cfg)),
            body: AbilityBody::from_prefab(w, p),
            anim,
            dir: Vec2::UP,
            effect_buffer: RingBuffer::new(max_effects),
            ..ThrowItem2::default()
        }
    }

    /// `build`: the object's component parts.
    pub fn build(&mut self, w: &mut World, me: ObjId, fw: &mut Framework, def: &AbilityDef) {
        self.body.install(fw);
        ability_body::add_parts(w, me, fw, def, &[("ThrowItem2", PartKind::Behaviour(0))]);
    }

    fn player_scale(w: &World, owner: i32) -> Fix {
        w.player(owner).map_or(Fix::ONE, |p| p.scale())
    }

    /// The thrower's look: body frame, mirrored by `mirrored`; the aim
    /// indicator (object 1) pointing along `dir`, sized 0.5..1.2 with the
    /// hold time, until the throw; the dummy item (object 2) in the hand
    /// until the real item is spawned.
    pub fn look(&self, w: &World, me: ObjId, l: &mut crate::view::Look) {
        use crate::view::PartLook;
        l.frame(&self.anim);
        let Some(cfg) = &self.cfg else {
            return;
        };
        let sx = if self.mirrored { -Fix::ONE } else { Fix::ONE };
        l.stretch = Vec2::new(sx, Fix::ONE);
        if self.has_fired {
            l.parts.push(PartLook::hidden(1));
        } else {
            let t = (if self.has_inputted_fire {
                self.stamp
            } else {
                self.time_passed
            } * cfg.gain_speed)
                .clamp(Fix::ZERO, Fix::ONE);
            let size = Fix::ONE / Fix::TWO + (Fix::from_int(7) / Fix::from_int(10)) * t;
            // Pointing along `dir` on screen (the parent may be mirrored);
            // relative to the body's rotation.
            let d = self.dir.normalized_safe();
            let world = Fix::atan2(d.y, d.x) - Fix::PI / Fix::TWO;
            let r = AbilityBody::rotation(w, me);
            let local = (world - r) * sx;
            l.parts
                .push(PartLook::new(1).posed(Vec2::ZERO, local, Vec2::new(size, size)));
        }
        if self.grenade.is_some() {
            l.parts.push(PartLook::hidden(2));
        } else {
            let list = if self.body.is_grounded() {
                &cfg.grounded_dummy
            } else {
                &cfg.aireal_dummy
            };
            let k = if self.is_in_loop {
                list.len().saturating_sub(1)
            } else {
                (self.anim.passed_sprites() - 1).max(0) as usize
            };
            let pos = list.get(k).or(list.last()).copied().unwrap_or(Vec2::ZERO);
            l.parts
                .push(PartLook::new(2).posed(pos, Fix::ZERO, Vec2::new(Fix::ONE, Fix::ONE)));
        }
    }

    /// `dummyPos()` (the hand position, by the current grounded state).
    fn hold_pos(&self, w: &World, me: ObjId, owner: i32) -> Vec2 {
        let Some(cfg) = &self.cfg else {
            return Vec2::ZERO;
        };
        let list = if self.body.is_grounded() {
            &cfg.grounded_dummy
        } else {
            &cfg.aireal_dummy
        };
        let last = list.last().copied().unwrap_or(Vec2::ZERO);
        let mut u = Self::player_scale(w, owner) * last;
        u.x = u.x * if self.mirrored { -Fix::ONE } else { Fix::ONE };
        let r = AbilityBody::rotation(w, me);
        Vec2::from_angle(r).complex_mul(u) + AbilityBody::pos(w, me)
    }

    /// The held item, if it still exists (Unity null semantics).
    fn grenade_live(&self, w: &World) -> Option<ObjId> {
        self.grenade.filter(|g| w.objects.exists(*g))
    }

    fn spawn_item(&mut self, cx: &mut Cx<'_>) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let Some(icfg) = cfg.item.clone() else {
            return;
        };
        let (me, owner) = (cx.me, cx.fw.owner);
        let pos = self.hold_pos(cx.w, me, owner);
        let rot = AbilityBody::rotation(cx.w, me);
        let scale = Self::player_scale(cx.w, owner);
        let g = spawn_item(cx.w, &icfg, pos, Some(rot), owner, Some(me));
        cx.w.set_collider_scale(g, scale);
        self.grenade = Some(g);
    }

    fn on_code(&mut self, cx: &mut Cx<'_>, code: u16) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        match code {
            CB_GROUNDED_ENTER | CB_AIREAL_ENTER => {
                let name = if code == CB_GROUNDED_ENTER { "gLoop" } else { "aLoop" };
                play(cx.w, &mut self.anim, cfg.anim_data, name);
                self.spawn_item(cx);
                self.is_in_loop = true;
            }
            CB_EXIT => self.exit_component(cx, AbilityExitInfo::default()),
            _ => {}
        }
    }

    /// `OnEnterAbility()`.
    pub fn on_enter(&mut self, cx: &mut Cx<'_>) {
        let Some(cfg) = self.cfg.clone() else {
            let i = cx.info();
            cx.exit(AbilityExitInfo::resume(&i));
            return;
        };
        let info = cx.info();
        let me = cx.me;
        self.body.owner = cx.fw.owner;
        self.dir = Vec2::UP;
        self.body.sync_physics_to(cx.w, me, &info);
        AbilityBody::update_collider(cx.w, me);
        self.is_in_loop = false;
        self.has_inputted_fire = false;
        self.mirrored = info.facing_right;
        self.time_passed = Fix::ZERO;
        self.time_since_fired = Fix::ZERO;
        if self.body.is_grounded() {
            self.body.body.self_imposed_velocity = Vec2::ZERO;
            play_then(cx.w, &mut self.anim, cfg.anim_data, "gEnter", CB_GROUNDED_ENTER);
            self.is_grounded_throw = true;
        } else {
            self.body.gravity_modifier = F_0_03F;
            self.body.body.self_imposed_velocity = info.self_imposed_velocity * F_0_1F;
            play_then(cx.w, &mut self.anim, cfg.anim_data, "aEnter", CB_AIREAL_ENTER);
            self.is_grounded_throw = false;
        }
        self.has_fired = false;
    }

    fn fire(&mut self, cx: &mut Cx<'_>) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let (me, owner) = (cx.me, cx.fw.owner);
        let w = &mut *cx.w;
        let v = self.dir;
        let t = if self.is_grounded_throw {
            cfg.g_throw_position
        } else {
            cfg.a_throw_position
        };
        let s = if self.mirrored { -Fix::ONE } else { Fix::ONE };
        let ps = Self::player_scale(w, owner);
        let r = AbilityBody::rotation(w, me);
        let p = AbilityBody::pos(w, me) + Vec2::from_angle(r).complex_mul(ps * t * s);
        let body_scale = AbilityBody::scale(w, me);
        let g = self
            .grenade_live(w)
            .filter(|g| item(w, *g).is_some_and(|i| !i.has_been_thrown));
        if let Some(g) = g {
            let delay = cfg.disable_hitbox_time * body_scale.max(Fix::ONE);
            if let Some(i) = item_mut(w, g) {
                i.hurt_owner_delay = delay;
            }
            ability_body::manual_init(w, g);
            AbilityBody::set_pos(w, g, p);
            if let Some(i) = item_mut(w, g) {
                i.has_been_thrown = true;
            }
            // grenadeBody.UpdateSim(0)
            ability_body::bopl_body_update(w, g);
            let f = cfg
                .min_throw_force
                .lerp(cfg.max_throw_force, (self.stamp * cfg.gain_speed).clamp01());
            w.physics
                .add_force(g, (body_scale * f * v) * Fix::from_int(60));
            self.time_since_fired = Fix::ZERO;
            self.body.body.self_imposed_velocity -= cfg.knockback * v;
            self.body.gravity_modifier = Fix::ONE;
            self.has_fired = true;
            w.emit(Cue::Sound("throwSmallItem"));
        } else {
            self.body.gravity_modifier = Fix::ONE;
            self.has_fired = true;
            self.time_since_fired = Fix::ZERO;
            self.body.body.self_imposed_velocity -= cfg.knockback * v;
        }
    }

    /// `ThrowItem2.UpdateSim`.
    pub fn throw_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let (me, owner) = (cx.me, cx.fw.owner);
        // 1. aim
        if !self.has_inputted_fire {
            let a = cx.w.player(owner).map_or(Vec2::ZERO, |p| p.inputs.aim_vector());
            if a.magnitude() > DEADZONE {
                let sign = if self.body.is_grounded() {
                    let n = self.body.current_normal(cx.w, me).unwrap_or(Vec2::UP);
                    n.cross(a.normalized_safe()).sign2()
                } else {
                    (-a.x).sign2()
                };
                self.mirrored = sign < 0;
                self.dir = a;
            }
        }
        // 2. hold
        if let Some(g) = self.grenade_live(cx.w)
            && !self.has_fired
            && item(cx.w, g).is_some_and(|i| !i.has_been_thrown)
        {
            let p = self.hold_pos(cx.w, me, owner);
            AbilityBody::set_pos(cx.w, g, p);
        }
        // 3. abort
        if self.is_grounded_throw && !self.body.is_grounded() {
            self.exit_component(cx, AbilityExitInfo::default());
            return;
        }
        // 4. release
        if !self.has_inputted_fire && !cx.button_down() {
            self.has_inputted_fire = true;
            self.stamp = self.time_passed;
        }
        // 5. fire
        if self.is_in_loop && self.has_inputted_fire {
            self.is_in_loop = false;
            self.fire(cx);
            let name = if self.is_grounded_throw { "gExit" } else { "aExit" };
            play_then(cx.w, &mut self.anim, cfg.anim_data, name, CB_EXIT);
        }
        let pdt = cx.w.fixed_delta_time(owner, dt);
        if self.has_fired {
            self.time_since_fired += pdt;
        }
        self.time_passed += pdt;
        // 8. movement
        if !self.body.is_grounded() {
            self.body.add_gravity_factor(cx.w);
            if !self.has_fired {
                let v = self.body.body.self_imposed_velocity;
                if v.magnitude() > cfg.max_slowmo_speed {
                    self.body.body.self_imposed_velocity = v.normalized() * cfg.max_slowmo_speed;
                }
            } else {
                let v0 = self.body.body.self_imposed_velocity;
                let h = cx.w.player(owner).map_or(Fix::ZERO, |p| p.inputs.input_vector().x);
                self.body.aireal_movement(h);
                let v1 = self.body.body.self_imposed_velocity;
                self.body.body.self_imposed_velocity = v0.lerp(v1, F_0_6F);
            }
            if self.body.velocity_based_raycasts(cx.w, me, true, dt) {
                self.exit_component(cx, AbilityExitInfo::default());
            }
        } else if let Some(info) = self.body.move_(cx.w, me, Vec2::ZERO, dt, true) {
            self.exit_component(cx, info);
        }
    }

    /// `ThrowItem2.ExitAbility(info)`.
    pub fn exit_component(&mut self, cx: &mut Cx<'_>, mut info: AbilityExitInfo) {
        let me = cx.me;
        if let Some(cfg) = self.cfg.clone()
            && let Some(g) = self.grenade_live(cx.w)
            && !self.has_fired
        {
            let body_scale = AbilityBody::scale(cx.w, me);
            if let Some(i) = item_mut(cx.w, g) {
                i.hurt_owner_delay = cfg.disable_hitbox_time * body_scale;
                i.has_been_thrown = true;
            }
            let circle_ok = cx
                .w
                .objects
                .get(g)
                .and_then(|o| o.part(PartKind::Collider))
                .is_some_and(|p| !p.destroyed);
            if circle_ok {
                if !ability_body::collider_initialized(cx.w, g) {
                    ability_body::manual_init(cx.w, g);
                }
                cx.w.physics.add_force(
                    g,
                    (body_scale * cfg.min_throw_force * Vec2::UP) * Fix::from_int(60),
                );
            }
        }
        self.grenade = None;
        info.self_imposed_velocity = self.body.body.self_imposed_velocity;
        info.external_velocity = self.body.body.external_velocity;
        info.facing_right = self.mirrored;
        info.position = AbilityBody::pos(cx.w, me);
        info.grounded_speed = self.body.physics.grounded_speed;
        info.is_grounded = self.body.physics.is_grounded;
        info.just_landed = info.is_grounded;
        info.ground = self.body.physics.attached_ground;
        info.grounded_local_position = self.body.physics.local_platform_pos;
        info.radius = self.body.physics.radius;
        cx.exit_body(&mut self.body, info);
    }

    /// `OnScaleChanged(m)`: the remembered item follows the owner's scale.
    pub fn on_scale_changed(&mut self, cx: &mut Cx<'_>, m: Fix) {
        if let Some(g) = self.grenade.filter(|g| cx.w.objects.exists(*g)) {
            let s = cx.w.objects.get(g).map_or(Fix::ONE, |o| o.tf.scale);
            cx.w.set_collider_scale(g, s * m);
        }
    }

    pub fn update_part(&mut self, cx: &mut Cx<'_>, k: PartKind, dt: Fix, late: bool) {
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
            (PartKind::Behaviour(0), false) => self.throw_update(cx, dt),
            _ => {
                if let Some(r) = self.body.update_part(cx, k, dt, late) {
                    self.exit_component(cx, r);
                }
            }
        }
    }

    pub fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        let me = cx.me;
        if let Some((killer, cause)) = self.body.collision_contact(cx.w, me, cx.fw, c) {
            let info = ability_body::dead_record(cx.w, cx.me, killer, cause);
            self.exit_component(cx, info);
        }
    }


    pub fn body_opt(&self) -> Option<&AbilityBody> {
        self.cfg.as_ref().map(|_| &self.body)
    }

    pub fn body_opt_mut(&mut self) -> Option<&mut AbilityBody> {
        if self.cfg.is_some() {
            Some(&mut self.body)
        } else {
            None
        }
    }
}

/// The Grenade ability.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Grenade(pub ThrowItem2);

impl AbilityImpl for Grenade {
    fn new(w: &World, def: &AbilityDef) -> Self {
        Grenade(ThrowItem2::new(w, def))
    }
    fn build(&mut self, w: &mut World, me: ObjId, fw: &mut Framework, def: &AbilityDef) {
        self.0.build(w, me, fw, def);
    }
    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        self.0.on_enter(cx);
    }
    fn update_part(&mut self, cx: &mut Cx<'_>, k: PartKind, dt: Fix, late: bool) {
        self.0.update_part(cx, k, dt, late);
    }
    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        self.0.on_contact(cx, c);
    }
    fn on_scale_changed(&mut self, cx: &mut Cx<'_>, m: Fix) {
        self.0.on_scale_changed(cx, m);
    }
    fn exit_component(&mut self, cx: &mut Cx<'_>, info: AbilityExitInfo) {
        self.0.exit_component(cx, info);
    }
    fn body(&self) -> Option<&AbilityBody> {
        self.0.body_opt()
    }
    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        self.0.body_opt_mut()
    }
    fn sprite(&self) -> crate::data::SpriteId {
        self.0.anim.sprite
    }
    fn look(&self, w: &World, me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        self.0.look(w, me, l);
    }
}

// -- the thrown items -----------------------------------------------------------------

/// What a grenade item does when it goes off.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Detonation {
    /// `GrenadeExplode`: spawn the explosion prefab.
    Explode { explosion: Option<usize> },
    /// `SmokeGrenadeExplode2`: burst into clouds (smoke.md).
    Smoke(Arc<SmokeBurstCfg>),
}

/// The `Grenade` item prefab (fuse + detonation).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GrenadeItemCfg {
    pub prefab: usize,
    pub timed: bool,
    pub detonation_time: Fix,
    pub detonation: Detonation,
    pub parts: Vec<(PartKind, i32, bool)>,
    pub bounds: Bounds,
}

impl GrenadeItemCfg {
    pub fn from_prefab(w: &World, prefab: usize) -> Option<GrenadeItemCfg> {
        let p = w.content.prefabs.get(prefab)?;
        let g = root_fields(p, "Grenade")?;
        let detonation = if let Some(f) = root_fields(p, "GrenadeExplode") {
            Detonation::Explode {
                explosion: prefab_field(w, f, "explosion"),
            }
        } else {
            let f = root_fields(p, "SmokeGrenadeExplode2")?;
            Detonation::Smoke(Arc::new(SmokeBurstCfg::from_fields(w, f)))
        };
        let parts = prefab_parts(
            w,
            prefab,
            &[
                ("Grenade", PartKind::Behaviour(0)),
                ("GrenadeExplode", PartKind::Behaviour(1)),
                ("SmokeGrenadeExplode2", PartKind::Behaviour(1)),
            ],
        );
        Some(GrenadeItemCfg {
            prefab,
            timed: g.boolean("timedExplosion").unwrap_or(false),
            detonation_time: fx(g, "detonationTime", Fix::ZERO),
            detonation,
            parts,
            bounds: Bounds::of_prefab(w, prefab),
        })
    }
}

/// A grenade or smoke grenade in the world.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GrenadeItem {
    pub cfg: Arc<GrenadeItemCfg>,
    pub owner: i32,
    /// The `ThrowItem2` ability object holding it.
    pub thrower: Option<ObjId>,
    pub has_been_thrown: bool,
    pub hurt_owner_delay: Fix,
    pub self_destruct_delay: Fix,
    pub detonates_on_owner: bool,
}

/// `InstantiateFixed(ItemPrefab, pos, rot)` + owner + thrower link.
pub fn spawn_item(
    w: &mut World,
    cfg: &Arc<GrenadeItemCfg>,
    pos: Vec2,
    rot: Option<Fix>,
    owner: i32,
    thrower: Option<ObjId>,
) -> ObjId {
    let it = GrenadeItem {
        cfg: cfg.clone(),
        owner,
        thrower,
        has_been_thrown: false,
        hurt_owner_delay: -Fix::ONE,
        self_destruct_delay: -Fix::ONE,
        detonates_on_owner: false,
    };
    ability_body::spawn_owned(w, cfg.prefab, Kind::Behaviour(Box::new(Behaviour::Grenade(Spawned::Item(it)))), &cfg.parts, pos, rot, owner)
}

/// The grenade item state of an object.
pub fn item(w: &World, g: ObjId) -> Option<&GrenadeItem> {
    match &w.objects.get(g)?.kind {
        Kind::Behaviour(b) => match b.as_ref() {
            Behaviour::Grenade(Spawned::Item(i)) => Some(i),
            _ => None,
        },
        _ => None,
    }
}

pub fn item_mut(w: &mut World, g: ObjId) -> Option<&mut GrenadeItem> {
    match &mut w.objects.get_mut(g)?.kind {
        Kind::Behaviour(b) => match b.as_mut() {
            Behaviour::Grenade(Spawned::Item(i)) => Some(i),
            _ => None,
        },
        _ => None,
    }
}

impl GrenadeItem {
    fn detonate(&mut self, w: &mut World, me: ObjId) {
        match &self.cfg.detonation {
            Detonation::Explode { explosion } => {
                let pos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
                let scale = w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale);
                if let Some(e) = *explosion {
                    super::explosion::spawn(w, e, pos, Some(scale), self.owner);
                }
                w.destroy(me);
            }
            Detonation::Smoke(c) => {
                let c = c.clone();
                smoke::burst(w, me, &c, self.owner, self.thrower, w.objects.alive(me));
                w.destroy(me);
            }
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        let thrower_ok = self
            .thrower
            .and_then(|t| w.objects.get(t))
            .is_some_and(|o| o.active);
        if !self.has_been_thrown && !thrower_ok {
            self.has_been_thrown = true;
            self.hurt_owner_delay = Fix::ONE;
            ability_body::manual_init(w, me);
        }
        if self.has_been_thrown && self.hurt_owner_delay > Fix::ZERO {
            self.hurt_owner_delay -= dt * w.time.player_time_scale;
            if self.hurt_owner_delay <= Fix::ZERO {
                self.detonates_on_owner = true;
            }
        }
        if self.self_destruct_delay > Fix::ZERO {
            self.self_destruct_delay -= dt * w.time.player_time_scale;
            if self.self_destruct_delay <= Fix::ZERO {
                self.detonate(w, me);
            }
        }
    }

    fn on_contact(&mut self, w: &mut World, me: ObjId, c: &Contact) {
        match &self.cfg.detonation {
            Detonation::Explode { .. } => match c.layer {
                8 => {
                    if !super::mine::has_id_holder(w, c.other) {
                        return;
                    }
                    let pid = w.objects.owner_id(c.other);
                    // Not while held by the magnet gun touching it.
                    if ability_body::holder_of(w, me) == Some(c.other) {
                        return;
                    }
                    if self.owner != pid || self.detonates_on_owner {
                        self.detonate(w, me);
                    }
                }
                9 => {
                    if projectile_ignites(w, c.other) {
                        self.detonate(w, me);
                    }
                }
                18 => self.detonate(w, me),
                _ => {}
            },
            Detonation::Smoke(_) => {
                if !self.detonates_on_owner || w.objects.get(me).is_none_or(|o| o.destroyed) {
                    return;
                }
                if c.layer == 9 && w.objects.exists(c.other) && is_projectile(w, c.other) && !projectile_ignites(w, c.other) {
                    return;
                }
                self.detonate(w, me);
            }
        }
    }
}

/// `Projectile.IgnitesExplosives` of the other object (false without a
/// Projectile component; Bow arrows: false).
fn projectile_ignites(w: &World, obj: ObjId) -> bool {
    projectile_component(w, obj).is_some_and(|f| f.boolean("IgnitesExplosives").unwrap_or(true))
}

/// The object has a `Projectile` component.
fn is_projectile(w: &World, obj: ObjId) -> bool {
    projectile_component(w, obj).is_some()
}

fn projectile_component(w: &World, obj: ObjId) -> Option<&crate::data::Value> {
    prefab::prefab_object(w, obj)?
        .component("Projectile")
        .map(|c| &c.fields)
}

/// Objects of this module.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    Item(GrenadeItem),
}

impl SpawnedImpl for Spawned {
    fn init(&mut self, _w: &mut World, _me: ObjId, part: PartKind) {
        let Spawned::Item(i) = self;
        if part == PartKind::Behaviour(0) && i.cfg.timed {
            i.self_destruct_delay = i.cfg.detonation_time;
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        let Spawned::Item(i) = self;
        if late {
            return;
        }
        match part {
            PartKind::Behaviour(0) => i.update(w, me, dt),
            PartKind::Bounds => {
                let b = i.cfg.bounds;
                b.check_and_destroy(w, me);
            }
            _ => {}
        }
    }

    fn on_contact(&mut self, w: &mut World, me: ObjId, c: &Contact) {
        let Spawned::Item(i) = self;
        i.on_contact(w, me, c);
    }

    /// Item kind, magnet fuse pause, Duplicator fix-ups (duplicator.md:
    /// no thrower, hurt-owner delay 0, not thrown, detonates on owner, not
    /// timed, fuse `detonationTime − (j + 1) · 0.1`, item owner kept).
    fn interact(&mut self, w: &mut World, _me: ObjId, it: super::Interaction) -> Option<Fix> {
        let Spawned::Item(i) = self;
        match it {
            super::Interaction::ItemKind => Some(Fix::from_int(super::ITEM_GRENADE)),
            super::Interaction::DelayFuse(k) => {
                i.self_destruct_delay += k;
                Some(Fix::ZERO)
            }
            super::Interaction::Duplicated { original, index, .. } => {
                if index >= 0 {
                    i.thrower = None;
                    i.hurt_owner_delay = Fix::ZERO;
                    i.has_been_thrown = false;
                    i.detonates_on_owner = true;
                    Arc::make_mut(&mut i.cfg).timed = false;
                    i.self_destruct_delay =
                        i.cfg.detonation_time - Fix::from_int(index + 1) * super::duplicator::GRENADE_FUSE_STEP;
                }
                if let Some(o) = item(w, original) {
                    i.owner = o.owner;
                }
                Some(Fix::ZERO)
            }
            _ => None,
        }
    }

    /// Clone rule: `timedExplosion`, `detonationTime`, `thrower`,
    /// `hasBeenThrown`, `selfDestructDelay` carry over; `hurtOwnerDelay`
    /// (−1), `DetonatesOnOwner` and `Item.OwnerId` (0) are runtime.
    fn duplicate(&self, _w: &World, _me: ObjId) -> Option<Spawned> {
        let Spawned::Item(i) = self;
        Some(Spawned::Item(GrenadeItem {
            owner: 0,
            hurt_owner_delay: -Fix::ONE,
            detonates_on_owner: false,
            ..i.clone()
        }))
    }
}


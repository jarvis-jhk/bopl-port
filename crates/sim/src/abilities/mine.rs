//! Mine (`docs/spec/abilities/mine.md`): the `ThrowItem` ability, the
//! `Mine` item and (canonical home) the spawned `Explosion` objects of the
//! shared Explosion/Shockwave components (`explosion.rs`).

use std::sync::Arc;

use super::ability_body::{self, AbilityBody, Bounds, advance_anim, play_named_then as play_then, prefab_parts};
use super::prefab::{self, fx, prefab_field, root_fields, vec2_field};
use super::explosion::{self, Explosion};
use super::prelude::*;
use super::Behaviour;
use crate::world::Kind;
use crate::anim::SpriteAnimator;

/// `ThrowItem` constants (Mine ability prefab).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MineCfg {
    pub prefab: usize,
    pub anim_data: u16,
    pub knock_back: Fix,
    pub throw_strength: Fix,
    pub min_throw_strength: Fix,
    pub aireal_throw_strength: Fix,
    pub throw_position: Vec2,
    pub only_throw_up: bool,
    pub throwing_range01: Fix,
    pub always_stop: bool,
    pub item: Option<Arc<MineItemCfg>>,
}

/// `Mine` item constants.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MineItemCfg {
    pub prefab: usize,
    pub hover_str: Fix,
    pub hover_period: Fix,
    pub time_before_armed: Fix,
    pub chase_time: Fix,
    pub chase_strength: Fix,
    pub scan_radius: Fix,
    pub shake_force: Fix,
    pub anticipation_duration: Fix,
    pub igniters: u32,
    pub scans_for: u32,
    pub explosion: Option<usize>,
    pub parts: Vec<(PartKind, i32, bool)>,
    pub bounds: Bounds,
}

impl MineItemCfg {
    pub fn from_prefab(w: &World, prefab: usize) -> Option<MineItemCfg> {
        let p = w.content.prefabs.get(prefab)?;
        let f = root_fields(p, "Mine")?;
        Some(MineItemCfg {
            prefab,
            hover_str: fx(f, "HoverStr", Fix::ONE),
            hover_period: fx(f, "HoverPeriod", Fix::ONE),
            time_before_armed: fx(f, "TimeBeforeArmed", Fix::TWO),
            chase_time: fx(f, "chaseTime", Fix::ZERO),
            chase_strength: fx(f, "chaseStrength", Fix::ONE),
            scan_radius: fx(f, "scanRadius", Fix::THREE),
            shake_force: fx(f, "shakeForce", Fix::ZERO),
            anticipation_duration: fx(f, "anticipationDuration", Fix::ONE),
            igniters: f.int("Igniters").unwrap_or(0) as u32,
            scans_for: f.int("ScansFor").unwrap_or(0) as u32,
            explosion: prefab_field(w, f, "Explosion"),
            parts: prefab_parts(w, prefab, &[("Mine", PartKind::Behaviour(0))]),
            bounds: Bounds::of_prefab(w, prefab),
        })
    }
}

const CB_PICKUP_DONE: u16 = 1;
const CB_FIRE: u16 = 2;
const CB_EXIT: u16 = 3;

/// The Mine ability object (`ThrowItem`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mine {
    pub cfg: Option<Arc<MineCfg>>,
    pub body: AbilityBody,
    pub anim: SpriteAnimator,
    pub in_anticipation: bool,
    pub fired_from_ground: bool,
    pub button_up: bool,
    pub has_fired: bool,
    /// `direction == DOWN`.
    pub down: bool,
    pub exit_velocity: Vec2,
    /// Negative x scale (= facing right).
    pub mirrored: bool,
    pub dummy: bool,
}

impl Mine {
    fn local_up(&self, w: &World, me: ObjId) -> Vec2 {
        if !self.body.is_grounded() {
            return Vec2::UP;
        }
        self.body.current_normal(w, me).unwrap_or(Vec2::UP)
    }

    /// `UpdateInputVector(v)`: the throw-direction rule.
    fn update_direction(&mut self, w: &World, me: ObjId, v: Vec2) {
        let up = self.local_up(w, me);
        self.down = if v.magnitude() > DEADZONE {
            let d = up.dot(v.normalized());
            !(d > Fix(3_435_973_836) || d > Fix(-1_288_490_188))
        } else {
            false
        };
    }

    fn input(w: &World, owner: i32) -> Vec2 {
        w.player(owner)
            .map_or(Vec2::ZERO, |p| p.inputs.input_vector())
    }

    fn fire(&mut self, cx: &mut Cx<'_>) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let (w, me) = (&mut *cx.w, cx.me);
        self.in_anticipation = false;
        self.dummy = false;
        self.has_fired = true;
        let grounded = self.body.is_grounded();
        play_then(w, &mut self.anim, cfg.anim_data, if grounded { "gExit" } else { "aExit" }, CB_EXIT);
        let scale = AbilityBody::scale(w, me);
        let tp = cfg.throw_position * scale;
        let up = self.local_up(w, me);
        let s = if self.mirrored { -Fix::ONE } else { Fix::ONE };
        let side = Vec2::new(up.y, -up.x) * s;
        let pos = AbilityBody::pos(w, me) + tp.y * up + tp.x * side;
        if grounded {
            let n = self.body.current_normal(w, me).unwrap_or(Vec2::UP);
            self.exit_velocity = -n * cfg.knock_back;
            self.body.body.self_imposed_velocity = Vec2::ZERO;
            self.fired_from_ground = true;
        } else {
            self.body.gravity_modifier = Fix::ONE;
            self.fired_from_ground = false;
        }
        let owner = cx.fw.owner;
        let v = if !self.fired_from_ground {
            Vec2::UP * cfg.aireal_throw_strength
        } else {
            let f = if self.down {
                cfg.min_throw_strength
            } else {
                cfg.throw_strength
            };
            let input = Self::input(w, owner);
            if input.magnitude() <= DEADZONE || cfg.only_throw_up {
                up * f
            } else {
                self.throwing_vector(w, me, input, cfg.throwing_range01) * f
            }
        };
        if let Some(item) = &cfg.item {
            spawn_mine(w, item, pos, scale, v, owner);
        }
        w.emit(Cue::Sound("throwSmallItem"));
    }

    /// `ThrowingVector()` (only reachable with `onlyThrowUp` off).
    fn throwing_vector(&self, w: &World, me: ObjId, input: Vec2, range: Fix) -> Vec2 {
        let up = self.local_up(w, me);
        let r = if input.normalized_safe().dot(up) <= range {
            let side = Vec2::new(-up.y, up.x);
            if side.dot(input) >= Fix::ZERO {
                (range * side + (Fix::ONE - range) * up).normalized()
            } else {
                (range * -side + (Fix::ONE - range) * up).normalized()
            }
        } else {
            input.normalized()
        };
        if input.magnitude() > DEADZONE { r } else { up }
    }

    /// `ExitAbility()` (animation end, landing after the throw).
    fn exit_plain(&mut self, cx: &mut Cx<'_>) {
        let info = AbilityExitInfo {
            self_imposed_velocity: if self.fired_from_ground {
                self.exit_velocity
            } else {
                self.body.body.self_imposed_velocity
            },
            external_velocity: self.body.body.external_velocity,
            ..AbilityExitInfo::default()
        };
        self.exit_component(cx, info);
    }

    fn on_code(&mut self, cx: &mut Cx<'_>, code: u16) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        match code {
            CB_PICKUP_DONE => {
                self.in_anticipation = true;
                play_then(cx.w, &mut self.anim, cfg.anim_data, "gLoop", CB_FIRE);
            }
            CB_FIRE => self.fire(cx),
            CB_EXIT => self.exit_plain(cx),
            _ => {}
        }
    }

    /// `ThrowItem.UpdateSim`.
    fn throw_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let owner = cx.fw.owner;
        if !self.has_fired {
            if !self.button_up {
                let v = Self::input(cx.w, owner);
                self.update_direction(cx.w, me, v);
            }
            if !cx.button_down() {
                self.button_up = true;
                let up = self.local_up(cx.w, me);
                self.update_direction(cx.w, me, -up);
            }
            if self.in_anticipation && self.button_up {
                self.fire(cx);
            }
        }
        if !cx.fw.in_use {
            return;
        }
        if !self.body.is_grounded() {
            self.body.add_gravity_factor(cx.w);
            if !self.fired_from_ground {
                let h = Self::input(cx.w, owner).x;
                self.body.aireal_movement(h);
            }
            if self.body.velocity_based_raycasts(cx.w, me, true, dt) {
                if self.has_fired {
                    self.exit_plain(cx);
                    return;
                }
                self.down = true;
                self.fire(cx);
            }
        } else if let Some(info) = self.body.move_(cx.w, me, Vec2::ZERO, dt, true) {
            self.exit_component(cx, info);
        }
    }
}

impl AbilityImpl for Mine {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let Some(p) = def.prefab else {
            return Mine::default();
        };
        let pf = &w.content.prefabs[p];
        let Some(f) = root_fields(pf, "ThrowItem") else {
            return Mine::default();
        };
        let item = prefab_field(w, f, "GrenadePrefab")
            .and_then(|i| MineItemCfg::from_prefab(w, i))
            .map(Arc::new);
        let always_stop = root_fields(pf, "SpriteAnimator")
            .and_then(|a| a.boolean("AlwaysStopDuringTimeStop"))
            .unwrap_or(false);
        let cfg = MineCfg {
            prefab: p,
            anim_data: prefab::anim_data_field(w, f, "animData").unwrap_or(0),
            knock_back: fx(f, "KnockBack", Fix::ZERO),
            throw_strength: fx(f, "throwStrength", Fix::ZERO),
            min_throw_strength: fx(f, "minThrowStrength", Fix::ZERO),
            aireal_throw_strength: fx(f, "airealThrowStrength", Fix::ZERO),
            throw_position: vec2_field(f, "throwPosition").unwrap_or(Vec2::ZERO),
            only_throw_up: f.boolean("onlyThrowUp").unwrap_or(false),
            throwing_range01: fx(f, "throwingRange01", Fix::ZERO),
            always_stop,
            item,
        };
        let anim = ability_body::animator(always_stop);
        Mine {
            cfg: Some(Arc::new(cfg)),
            body: AbilityBody::from_prefab(w, p),
            anim,
            ..Mine::default()
        }
    }

    fn build(&mut self, w: &mut World, me: ObjId, fw: &mut Framework, def: &AbilityDef) {
        self.body.install(fw);
        ability_body::add_parts(w, me, fw, def, &[("ThrowItem", PartKind::Behaviour(0))]);
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
        if self.mirrored {
            l.stretch = Vec2::new(-Fix::ONE, Fix::ONE);
        }
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
        self.in_anticipation = false;
        self.fired_from_ground = false;
        self.button_up = false;
        self.exit_velocity = Vec2::ZERO;
        self.body.sync_physics_to(cx.w, me, &info);
        AbilityBody::update_collider(cx.w, me);
        AbilityBody::set_rotation(cx.w, me, info.rotation);
        self.dummy = true;
        self.body.gravity_modifier = if info.is_grounded { Fix::ZERO } else { Fix::ONE };
        let v = Self::input(cx.w, info.player_id);
        self.update_direction(cx.w, me, v);
        self.mirrored = info.facing_right;
        self.down = false;
        self.has_fired = false;
        if info.is_grounded {
            play_then(cx.w, &mut self.anim, cfg.anim_data, "gEnter", CB_PICKUP_DONE);
        } else {
            play_then(cx.w, &mut self.anim, cfg.anim_data, "aEnter", CB_FIRE);
        }
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
                    self.exit_component(cx, ability_body::dead_record(cx.w, cx.me, ENVIRONMENT_ID, CauseOfDeath::Other));
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

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        let me = cx.me;
        if let Some((killer, cause)) = self.body.collision_contact(cx.w, me, cx.fw, c) {
            self.exit_component(cx, ability_body::dead_record(cx.w, cx.me, killer, cause));
        }
    }


    fn exit_component(&mut self, cx: &mut Cx<'_>, mut info: AbilityExitInfo) {
        let me = cx.me;
        self.dummy = false;
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
}

// -- the Mine item ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MineState {
    #[default]
    Off,
    Armed,
    Chasing,
    Anticipation,
}

/// A thrown mine.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MineItem {
    pub cfg: Arc<MineItemCfg>,
    pub owner: i32,
    pub state: MineState,
    pub time_since_spawned: Fix,
    /// `timeSinceAnticipation` (chasing and anticipation timer).
    pub timer: Fix,
    pub chase_dir: Vec2,
    pub random_offset: Fix,
    pub prev_rand: Vec2,
    pub prev_change: Vec2,
}

/// Spawns a mine item: `InstantiateFixed(Mine, pos)`, collider scale,
/// start velocity, owner.
pub fn spawn_mine(w: &mut World, cfg: &Arc<MineItemCfg>, pos: Vec2, scale: Fix, v: Vec2, owner: i32) -> ObjId {
    let m = MineItem {
        cfg: cfg.clone(),
        owner,
        state: MineState::Off,
        time_since_spawned: Fix::ZERO,
        timer: Fix::ZERO,
        chase_dir: Vec2::RIGHT,
        random_offset: Fix::ZERO,
        prev_rand: Vec2::ZERO,
        prev_change: Vec2::ZERO,
    };
    let id = ability_body::spawn_owned(w, cfg.prefab, Kind::Behaviour(Box::new(Behaviour::Mine(Spawned::Mine(m)))), &cfg.parts, pos, None, owner);
    w.set_collider_scale(id, scale);
    if let Some(o) = w.objects.get_mut(id)
        && let Some(c) = o.collider.as_mut()
        && let Some(b) = c.body.as_mut()
    {
        b.start_velocity = v;
    }
    id
}

impl MineItem {
    fn scan(&self, w: &mut World, me: ObjId) -> Option<ObjId> {
        let pos = w.objects.get(me)?.tf.position;
        let r = self.cfg.scan_radius * w.objects.get(me)?.tf.scale;
        let hits = w
            .physics
            .circle_cast_all(&w.objects, pos, r, self.cfg.scans_for, 4);
        let owner_team = w.player(self.owner).map(|p| (p.id, p.team));
        for h in hits {
            let pid = w.objects.owner_id(h);
            if has_id_holder(w, h) {
                let p = w.player(pid);
                let skip = match (p, owner_team) {
                    (None, _) => true,
                    (Some(p), Some((oid, oteam))) => p.is_invisible || pid == oid || p.team == oteam,
                    (Some(p), None) => p.is_invisible,
                };
                if skip {
                    continue;
                }
            }
            return Some(h);
        }
        None
    }

    fn detonate(&self, w: &mut World, me: ObjId) {
        let pos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        let scale = w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale);
        if let Some(e) = self.cfg.explosion {
            explosion::spawn(w, e, pos, Some(scale), self.owner);
        }
        w.emit(Cue::Sound("mineExplosion"));
        w.destroy(me);
    }

    fn late_update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        let protected = w.player(self.owner).is_some_and(|p| p.is_protected_from_time_stop);
        if w.is_time_stopped() && !protected {
            return;
        }
        let c = self.cfg.clone();
        self.time_since_spawned += dt;
        let h = (self.time_since_spawned * c.hover_period + self.random_offset).cos();
        w.physics.add_force(me, Vec2::new(Fix::ZERO, h * c.hover_str));
        match self.state {
            MineState::Off => {
                if self.time_since_spawned >= c.time_before_armed {
                    w.emit(Cue::Sound("mineArmed"));
                    self.state = MineState::Armed;
                }
            }
            MineState::Armed => {
                if !w.is_time_stopped()
                    && !ability_body::is_held(w, me)
                    && let Some(t) = self.scan(w, me)
                {
                    self.state = MineState::Chasing;
                    let tp = w.objects.get(t).map_or(Vec2::ZERO, |o| o.tf.position);
                    let mp = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
                    self.chase_dir = (tp - mp).normalized_safe();
                    w.emit(Cue::Sound("mineArmedDouble"));
                }
            }
            MineState::Chasing => {
                if w.is_time_stopped() || ability_body::is_held(w, me) {
                    return;
                }
                self.timer += w.fixed_delta_time(self.owner, dt);
                if self.timer > c.chase_time {
                    self.timer = Fix::ZERO;
                    self.state = MineState::Anticipation;
                    w.emit(Cue::Sound("mineAnticipation"));
                }
                if let Some(t) = self.scan(w, me).filter(|t| w.objects.get(*t).is_some_and(|o| !o.tf.destroyed)) {
                    let tp = w.objects.get(t).map_or(Vec2::ZERO, |o| o.tf.position);
                    let mp = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
                    let v = tp - mp;
                    if v.sqr_magnitude() < Fix::from_int(4) {
                        self.detonate(w, me);
                        return;
                    }
                    self.chase_dir = v.normalized_safe();
                }
                w.physics.add_force(me, self.chase_dir * c.chase_strength);
            }
            MineState::Anticipation => {
                if !w.is_time_stopped() {
                    self.timer += w.fixed_delta_time(self.owner, dt);
                    if self.timer >= c.anticipation_duration {
                        self.detonate(w, me);
                    } else {
                        let shake = Fix::ZERO.lerp(c.shake_force, self.timer / c.anticipation_duration);
                        if let Some(o) = w.objects.get_mut(me) {
                            o.tf.position -= self.prev_change;
                        }
                        let v = w.rng.random_inside_unit_circle() * shake * Fix::THREE;
                        let d = v - self.prev_rand;
                        if let Some(o) = w.objects.get_mut(me) {
                            o.tf.position += d;
                        }
                        self.prev_rand = v;
                        self.prev_change = d;
                    }
                }
            }
        }
    }
}

/// Whether an object carries an `IPlayerIdHolder` (bopls, ability objects,
/// items, shockwaves).
pub fn has_id_holder(w: &World, h: ObjId) -> bool {
    match w.objects.get(h).map(|o| &o.kind) {
        Some(Kind::Bopl(_)) | Some(Kind::Ability(_)) => true,
        Some(Kind::Behaviour(_)) => w.objects.get(h).is_some_and(|o| o.player_id != 0),
        _ => false,
    }
}

/// Objects of this module.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    Mine(MineItem),
    Explosion(Explosion),
    /// A stand-alone `Shockwave` object (gusts, meteor impacts;
    /// [`explosion::spawn_shockwave`]).
    Shockwave(explosion::Shockwave),
}

impl SpawnedImpl for Spawned {
    /// Mines: the owner's material and the light (object 1, owner's
    /// `_ShadowColor`) off until armed, then pulsing; blinking while
    /// chasing. Explosions: their animation frame (also on the layers with
    /// their own animator).
    fn look(&self, w: &World, me: ObjId, l: &mut crate::view::Look) {
        use crate::view::PartLook;
        match self {
            Spawned::Mine(m) => {
                l.player_material = true;
                let t = m.time_since_spawned;
                let armed_at = m.cfg.time_before_armed;
                let a = match m.state {
                    MineState::Chasing | MineState::Anticipation => {
                        let blink = Fix::from_int(3) / Fix::from_int(10);
                        let n = t % blink;
                        (Fix::TWO + n) / Fix::from_int(4)
                    }
                    _ if t < armed_at => Fix::ZERO,
                    _ => {
                        let mut a = (Fix::ONE + (t + m.random_offset).sin()) / Fix::from_int(4)
                            + Fix::from_int(2) / Fix::from_int(5);
                        let since = t - armed_at;
                        if since > Fix::ZERO && since < Fix::ONE {
                            a = a.max(Fix::ONE - since);
                        }
                        a
                    }
                };
                let mut p = PartLook::new(1);
                p.alpha = a;
                p.shadow_color = true;
                l.parts.push(p);
            }
            Spawned::Explosion(e) => {
                l.frame(&e.anim);
                let prefab = match w.objects.get(me).map(|o| o.visual) {
                    Some(crate::view::Visual::Prefab { prefab, .. }) => Some(prefab as usize),
                    _ => None,
                };
                l.frame_children(w, prefab);
            }
            Spawned::Shockwave(_) => {}
        }
    }

    fn init(&mut self, w: &mut World, me: ObjId, part: PartKind) {
        match self {
            Spawned::Mine(m) => if let PartKind::Behaviour(0) = part {
                m.time_since_spawned = Fix::ZERO;
                let r = w.rng.random_fix(Fix::ZERO, Fix::TWO);
                m.random_offset = Fix::ONE - r * m.cfg.hover_period;
                m.chase_dir = Vec2::RIGHT;
            },
            Spawned::Explosion(e) => e.init(w, me, part),
            Spawned::Shockwave(sw) => {
                if part == PartKind::Behaviour(1) {
                    sw.init(w, me);
                }
            }
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        match self {
            Spawned::Mine(m) => match (part, late) {
                (PartKind::Behaviour(0), true) => m.late_update(w, me, dt),
                (PartKind::Bounds, false) => {
                    let b = m.cfg.bounds;
                    b.check_and_destroy(w, me);
                }
                _ => {}
            },
            Spawned::Explosion(e) => e.update(w, me, part, dt, late),
            Spawned::Shockwave(sw) => {
                if part == PartKind::Behaviour(1) && !late {
                    sw.update(w, me, dt);
                }
            }
        }
    }

    fn on_contact(&mut self, w: &mut World, me: ObjId, c: &Contact) {
        let Spawned::Mine(m) = self else {
            return;
        };
        let layer = c.layer;
        if !(0..32).contains(&layer) || (1u32 << layer) & m.cfg.igniters == 0 {
            return;
        }
        if matches!(m.state, MineState::Anticipation | MineState::Off) {
            return;
        }
        if layer == 16 && !super::tesla_coil::is_chain_lightning(w, c.other) {
            return;
        }
        // Not the hurt box of the magnet gun holding it.
        if ability_body::holder_of(w, me) == Some(c.other) {
            return;
        }
        m.timer = m.cfg.anticipation_duration * Fix(3_435_973_836);
        m.state = MineState::Anticipation;
        w.emit(Cue::Sound("mineAnticipation"));
    }

    /// Item kind; Duplicator fix-ups (item / explosion owner := the
    /// original's).
    fn interact(&mut self, w: &mut World, _me: ObjId, i: super::Interaction) -> Option<Fix> {
        match (self, i) {
            (Spawned::Mine(_), super::Interaction::ItemKind) => Some(Fix::from_int(super::ITEM_MINE)),
            (Spawned::Mine(m), super::Interaction::Duplicated { original, .. }) => {
                if let Some(Kind::Behaviour(b)) = w.objects.get(original).map(|o| &o.kind)
                    && let Behaviour::Mine(Spawned::Mine(o)) = b.as_ref()
                {
                    m.owner = o.owner;
                }
                Some(Fix::ZERO)
            }
            (Spawned::Explosion(e), super::Interaction::Duplicated { original, .. }) => {
                if let Some(o) = super::explosion::info(w, original) {
                    e.owner = o.owner;
                }
                Some(Fix::ZERO)
            }
            _ => None,
        }
    }

    /// Clone rule: the mine keeps its tuning (state machine and timers
    /// reset, `Item.OwnerId` 0); an explosion keeps its flags
    /// (`PlayerOwnerId` 0).
    fn duplicate(&self, _w: &World, _me: ObjId) -> Option<Spawned> {
        Some(match self {
            Spawned::Mine(m) => Spawned::Mine(MineItem {
                cfg: m.cfg.clone(),
                owner: 0,
                state: MineState::Off,
                time_since_spawned: Fix::ZERO,
                timer: Fix::ZERO,
                chase_dir: Vec2::RIGHT,
                random_offset: Fix::ZERO,
                prev_rand: Vec2::ZERO,
                prev_change: Vec2::ZERO,
            }),
            Spawned::Explosion(e) => Spawned::Explosion(Explosion {
                owner: 0,
                first_frame: true,
                anim: crate::anim::SpriteAnimator {
                    always_stop_during_time_stop: e.anim.always_stop_during_time_stop,
                    ..Default::default()
                },
                ..e.clone()
            }),
            Spawned::Shockwave(s) => Spawned::Shockwave(s.clone()),
        })
    }

    fn lethal(&self, _w: &World, _me: ObjId) -> Option<super::Lethal> {
        match self {
            Spawned::Explosion(e) => Some(e.info().lethal()),
            Spawned::Mine(_) | Spawned::Shockwave(_) => None,
        }
    }
}


//! Throw (`docs/spec/abilities/throw.md`, component `MachoThrow2`): rip a
//! boulder out of the platform, carry it overhead, throw it. Boulders are
//! `Kind::Platform` objects with a [`BoulderState`]; spikes and tesla coils
//! near the grab spot (spikes, tesla coils, rocket engines, rope anchors)
//! travel with the boulder.

use std::sync::Arc;

use super::ability_body::{self, AbilityBody, Bounds, advance_anim, play_named as play, play_named_then as play_then};
use super::prefab::{self, fx, prefab_field, root_fields, vec2_field, vec2_list};
use super::prelude::*;
use crate::anim::SpriteAnimator;
use crate::platform::{Platform, Shake, Sticky};
use crate::surface::Surface;
use crate::view::Visual;
use crate::world::Kind;

/// `MachoThrow2` constants.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ThrowCfg {
    pub anim_data: u16,
    pub throw_speed: Fix,
    pub quick_throw_str: Fix,
    pub quick_throw_charge_scaling: Fix,
    pub min_throw_vector: Vec2,
    pub proportion_from_scale: Fix,
    pub rotation_vel_on_throw: Fix,
    pub force_from_boulder: Fix,
    pub spawn_pos_low: Vec2,
    pub sprites_before_dummy_pos: i32,
    pub dist_ground_to_hands: Fix,
    pub scale_reduction: Fix,
    pub frames_until_shrink: i32,
    pub max_dist_attachable: Fix,
    pub ang_vel_multiplier: Fix,
    pub player_radius_hack: Fix,
    pub frames_before_move: i32,
    pub frames_before_jump: i32,
    pub dummy_positions: Vec<Vec2>,
    pub dummy_prepare: Vec<Vec2>,
    pub smoke_updatables: i32,
    pub smoke_prefab: Option<usize>,
    pub boulder: Option<Arc<BoulderCfg>>,
}

/// The `Boulder` prefab.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BoulderCfg {
    pub prefab: usize,
    pub collider: Option<crate::physics::ColliderDef>,
    pub layer: u8,
    pub radius: Fix,
    pub bounciness_dropped: Fix,
    pub shake_str: Fix,
    pub parts: Vec<(PartKind, i32, bool)>,
    pub bounds: Bounds,
}

impl BoulderCfg {
    fn from_prefab(w: &World, prefab: usize) -> Option<BoulderCfg> {
        let p = w.content.prefabs.get(prefab)?;
        let root = p.objects.first()?;
        let bf = root_fields(p, "Boulder")?;
        let parts = ability_body::parts_of(
            root,
            &[
                ("StickyRoundedRectangle", PartKind::Sticky),
                ("ShakablePlatform", PartKind::Shakable),
                ("Boulder", PartKind::Behaviour(0)),
            ],
            true,
        );
        Some(BoulderCfg {
            prefab,
            collider: root.collider,
            layer: root.layer,
            radius: root.collider.map_or(Fix::ZERO, |c| c.start_radius),
            bounciness_dropped: fx(bf, "bouncinessWhenDropped01", Fix::ONE),
            shake_str: root_fields(p, "ShakablePlatform").map_or(Fix::ONE, |f| fx(f, "basicShakeStr", Fix::ONE)),
            parts,
            bounds: Bounds::of_prefab(w, prefab),
        })
    }
}

/// `Boulder` component state (on the boulder platform).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BoulderState {
    pub dropped: bool,
    pub should_drop: bool,
    pub bounciness_dropped: Fix,
    pub bounds: Bounds,
}

/// The Boulder's `UpdateSim` and its bounds check (hooked from platform.rs).
pub fn boulder_update(w: &mut World, me: ObjId, p: &mut Platform, k: PartKind) {
    let Some(b) = p.boulder.as_mut() else {
        return;
    };
    match k {
        PartKind::Behaviour(_) => {
            if !b.dropped && b.should_drop && ability_body::collider_initialized(w, me) {
                drop_boulder(w, me, b.bounciness_dropped);
                b.dropped = true;
            }
            b.should_drop = true;
        }
        PartKind::Bounds => {
            let bounds = b.bounds;
            bounds.check_and_destroy(w, me);
        }
        _ => {}
    }
}

/// `Boulder.Drop()`: gravity 1, bounciness 0.5 (body and composite).
fn drop_boulder(w: &mut World, me: ObjId, bounciness: Fix) {
    let ok = w
        .objects
        .get(me)
        .and_then(|o| o.part(PartKind::Collider))
        .is_some_and(|p| !p.destroyed)
        && ability_body::collider_initialized(w, me);
    if !ok {
        return;
    }
    if let Some(b) = w.physics.own_body_mut(me) {
        b.gravity_scale = Fix::ONE;
        b.bounciness = bounciness;
    }
    if let Some(b) = w.physics.body_mut(me) {
        b.gravity_scale = Fix::ONE;
        b.bounciness = bounciness;
    }
}

/// `Boulder.Hold()`: no drop on its next update.
pub fn boulder_hold(w: &mut World, b: ObjId) {
    if let Some(s) = boulder_state_mut(w, b) {
        s.should_drop = false;
    }
}

fn boulder_state_mut(w: &mut World, b: ObjId) -> Option<&mut BoulderState> {
    match &mut w.objects.get_mut(b)?.kind {
        Kind::Platform(p) => p.boulder.as_mut(),
        _ => None,
    }
}

fn boulder_state(w: &World, b: ObjId) -> Option<&BoulderState> {
    match &w.objects.get(b)?.kind {
        Kind::Platform(p) => p.boulder.as_ref(),
        _ => None,
    }
}

/// `InstantiateFixed(boulderPrefab, pos)` as a platform object.
fn spawn_boulder(w: &mut World, c: &BoulderCfg, pos: Vec2, platform_type: i32) -> ObjId {
    let p = Platform {
        sticky: Sticky {
            platform_type,
            base_scale: Fix::ONE,
            drop_except: -1,
            ..Sticky::default()
        },
        spring: None,
        shake: Some(Shake {
            basic_strength: c.shake_str,
            ..Shake::default()
        }),
        orbit: None,
        path: None,
        grow: None,
        time_since_last_scaled_up: Fix::ZERO,
        resize: None,
        boulder: Some(BoulderState {
            dropped: false,
            should_drop: false,
            bounciness_dropped: c.bounciness_dropped,
            bounds: c.bounds,
        }),
    };
    let total = prefab::updatables_of(w, c.prefab);
    w.open_bracket();
    let id = w.objects.create(Kind::Platform(Box::new(p)), c.layer);
    if let Some(o) = w.objects.get_mut(id) {
        o.collider = c.collider;
        o.visual = Visual::Prefab {
            prefab: c.prefab as u32,
            object: 0,
        };
        for &(k, h, en) in &c.parts {
            let i = o.add_part(k, h);
            o.parts[i].enabled = en;
        }
    }
    w.register_all(id);
    w.close_bracket();
    w.consume_hierarchy((total - c.parts.len() as i32).max(0));
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.position = pos;
        o.tf.reset_rotation();
    }
    crate::platform::update_bounds(w, id);
    id
}

const CB_ENTER: u16 = 1;
const CB_GRAB: u16 = 2;
const CB_PICKUP: u16 = 3;
const CB_TURN: u16 = 4;
const CB_BEGIN_THROW: u16 = 5;
const CB_EXIT: u16 = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThrowState {
    #[default]
    Enter,
    Pickup,
    Idle,
    Walk,
    Turn,
    ThrowAnticipation,
    Exit,
}

/// A carried attachable (deactivated while carried).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Carried {
    Spike(ObjId),
    Coil(ObjId),
    Engine(ObjId),
    /// A grappling-rope anchor on the ground (not deactivated).
    Rope(ObjId),
}

/// The Throw ability object (`MachoThrow2`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Throw {
    pub cfg: Option<Arc<ThrowCfg>>,
    pub body: AbilityBody,
    pub anim: SpriteAnimator,
    pub state: ThrowState,
    pub facing_right: bool,
    pub released: bool,
    pub held: Option<ObjId>,
    pub current_dummy_pos: Vec2,
    pub grabbing_dirt: bool,
    pub boulder_type: i32,
    /// The ground at enter (captured by the enter callback).
    pub enter_ground: Option<ObjId>,
    pub carried: Option<Carried>,
}

impl Throw {
    fn scale(w: &World, me: ObjId) -> Fix {
        AbilityBody::scale(w, me)
    }

    fn passed(&self) -> i32 {
        self.anim.passed_sprites()
    }

    /// `UpdateDirection(d)`.
    fn update_direction(&mut self, w: &World, me: ObjId, d: Vec2) {
        if self.body.ground(w).is_none() {
            return;
        }
        let f = self.body.forward(w, me).unwrap_or(Vec2::RIGHT).dot(d);
        if f > Fix::ZERO {
            self.facing_right = true;
        } else if f < Fix::ZERO {
            self.facing_right = false;
        }
    }

    /// `heldPos()`.
    fn held_pos(&self, w: &World, me: ObjId) -> Vec2 {
        let s = Self::scale(w, me);
        let mut u = self.current_dummy_pos * s;
        if !self.facing_right {
            u.x = -u.x;
        }
        if let Some(h) = self.held.filter(|h| w.objects.exists(*h))
            && let Some(cfg) = &self.cfg
            && let Some(bc) = &cfg.boulder
        {
            let hs = w.objects.get(h).map_or(Fix::ONE, |o| o.tf.scale);
            u.y += bc.radius * (hs - Fix::ONE) + cfg.dist_ground_to_hands * (s - Fix::ONE);
        }
        let t = w.objects.get(me).map(|o| o.tf).unwrap_or_default();
        t.position + t.right.complex_mul(u)
    }

    /// `UpdateHeldBoulder()`.
    fn update_held(&mut self, w: &mut World, me: ObjId) {
        let Some(h) = self.held.filter(|h| w.objects.alive(*h)) else {
            return;
        };
        let p = self.held_pos(w, me);
        let r = AbilityBody::rotation(w, me);
        if let Some(o) = w.objects.get_mut(h) {
            o.tf.position = p;
            o.tf.set_rotation(r);
        }
        let v = w.physics.body(h).map_or(Vec2::ZERO, |b| b.velocity);
        if let Some(b) = w.physics.body_mut(h) {
            b.velocity = Vec2::ZERO;
            b.angular_velocity = Fix::ZERO;
        }
        let ffb = self.cfg.as_ref().map_or(Fix::ONE, |c| c.force_from_boulder);
        let inv = self.body.cfg.inverse_mass01;
        self.body.add_force(v * ffb / inv);
    }

    fn forget_dropped(&mut self, w: &World) {
        if let Some(h) = self.held
            && w.objects.exists(h)
            && boulder_state(w, h).is_some_and(|b| b.dropped)
        {
            self.held = None;
        }
        if self.held.is_some_and(|h| !w.objects.exists(h)) {
            self.held = None;
        }
    }

    fn on_code(&mut self, cx: &mut Cx<'_>, code: u16) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let me = cx.me;
        match code {
            CB_ENTER => {
                if cfg.smoke_prefab.is_some() {
                    cx.w.consume_hierarchy(cfg.smoke_updatables);
                    let p = AbilityBody::pos(cx.w, me);
                    // Presentation: the smoke at the feet in the boulder
                    // type's colour (`boulderSmokeColors`), mirrored by
                    // facing.
                    let color = super::prefab::object_fields(cx.w, me, "MachoThrow2")
                        .and_then(|f| f.list("boulderSmokeColors").get(self.boulder_type.max(0) as usize))
                        .and_then(super::color_field);
                    if let Some(crate::view::Visual::Prefab { prefab, .. }) =
                        cx.w.objects.get(me).map(|o| o.visual)
                    {
                        cx.w.emit(Cue::Fx(crate::view::FxCue {
                            prefab,
                            field: "smokeEffectPrefab",
                            position: p,
                            rotation: AbilityBody::rotation(cx.w, me),
                            scale: if self.facing_right { Fix::ONE } else { -Fix::ONE },
                            player_id: cx.fw.owner,
                            value: Vec2::ZERO,
                            color,
                            source: me,
                        }));
                    }
                }
                cx.w.emit(Cue::Sound("pickupBoulder"));
                if let Some(g) = self.enter_ground.filter(|g| cx.w.objects.alive(*g)) {
                    self.pick_up_attachables(cx.w, me, &cfg, g);
                }
                self.grabbing_dirt = true;
                play_then(cx.w, &mut self.anim, cfg.anim_data, "grab", CB_GRAB);
            }
            CB_GRAB => {
                self.grabbing_dirt = false;
                self.state = ThrowState::Pickup;
                play_then(cx.w, &mut self.anim, cfg.anim_data, "pickup", CB_PICKUP);
            }
            CB_PICKUP => {
                self.state = ThrowState::Idle;
                self.spawn_held(cx.w, me, &cfg);
                play(cx.w, &mut self.anim, cfg.anim_data, "idle");
            }
            CB_TURN => {
                self.state = ThrowState::Walk;
                play(cx.w, &mut self.anim, cfg.anim_data, "walk");
            }
            CB_BEGIN_THROW => {
                let a = cx.w.player(cx.fw.owner).map_or(Vec2::ZERO, |p| p.inputs.aim_vector());
                self.update_direction(cx.w, me, a);
                self.fire(cx, false);
                self.state = ThrowState::Exit;
                play_then(cx.w, &mut self.anim, cfg.anim_data, "exit", CB_EXIT);
            }
            CB_EXIT => self.exit_component(cx, AbilityExitInfo::default()),
            _ => {}
        }
    }

    /// `pickUpNearbyAttachables(ground)`.
    fn pick_up_attachables(&mut self, w: &mut World, me: ObjId, cfg: &ThrowCfg, g: ObjId) {
        crate::platform::update_bounds(w, g);
        let Some(surf) = Surface::of(w, g) else {
            return;
        };
        let c = surf.total;
        let mine = self.body.physics.local_platform_pos;
        let mut best: Option<Carried> = None;
        let mut best_d = Fix::MAX;
        let children = w.objects.get(g).map(|o| o.children.clone()).unwrap_or_default();
        for ch in children {
            let Some(o) = w.objects.get(ch) else {
                continue;
            };
            if o.layer != 16 && o.layer != 21 {
                continue;
            }
            let (local, carried) = match &o.kind {
                Kind::Behaviour(b) => match b.as_ref() {
                    super::Behaviour::Spike(a) => (a.local, Carried::Spike(ch)),
                    super::Behaviour::TeslaCoil(super::tesla_coil::Spawned::Node(n)) => (n.local, Carried::Coil(ch)),
                    super::Behaviour::Engine(super::engine::Spawned::RocketEngine(e)) => {
                        (e.local, Carried::Engine(ch))
                    }
                    _ => continue,
                },
                _ => continue,
            };
            let mut d = (local - mine).abs();
            d = d.min(Fix::ONE - d);
            if d * c < cfg.max_dist_attachable && d < best_d {
                best_d = d;
                best = Some(carried);
            }
        }
        // Rope anchors on the same platform, in update-list order.
        let mut rope = None;
        for i in 0..w.updatables.len() {
            let a = w.updatables[i].obj;
            if w.objects.get(a).is_none_or(|o| o.layer != 24) {
                continue;
            }
            let Some(local) = super::grappling_hook::platform_anchor(w, a)
                .filter(|(p, _)| *p == g && w.objects.alive(g))
                .map(|(_, l)| l)
            else {
                continue;
            };
            let mut d = (local - mine).abs();
            d = d.min(Fix::ONE - d);
            if d * c < cfg.max_dist_attachable && d < best_d {
                best_d = d;
                rope = Some(a);
            }
        }
        if let Some(a) = rope {
            self.carried = Some(Carried::Rope(a));
            return;
        }
        let Some(carried) = best else {
            return;
        };
        self.carried = Some(carried);
        let id = match carried {
            Carried::Spike(s) | Carried::Coil(s) | Carried::Engine(s) | Carried::Rope(s) => s,
        };
        w.objects.set_parent(id, None);
        if let Some(o) = w.objects.get_mut(id) {
            o.active = false;
        }
        w.physics.remove_from_composite(&w.objects, id);
        let _ = me;
    }

    /// `attachPickupedAttachableToBoulder(B)`.
    fn attach_to_boulder(&mut self, w: &mut World, me: ObjId, b: ObjId) {
        let Some(carried) = self.carried else {
            return;
        };
        match carried {
            Carried::Coil(n) if w.objects.exists(n) => {
                if let Some(o) = w.objects.get_mut(n) {
                    o.active = true;
                }
                w.objects.set_parent(n, Some(b));
                let scale = w.objects.get(n).map_or(Fix::ONE, |o| o.tf.scale);
                if let Some(o) = w.objects.get_mut(n) {
                    o.tf.set_rotation(Fix::ZERO);
                }
                if let Some(body) = w.physics.own_body_mut(n) {
                    body.orientation = Fix::ZERO;
                }
                super::tesla_coil::reattach(w, n, b, scale);
            }
            Carried::Spike(s) if w.objects.exists(s) => {
                if let Some(o) = w.objects.get_mut(s) {
                    o.active = true;
                }
                w.objects.set_parent(s, Some(b));
                let scale = w.objects.get(s).map_or(Fix::ONE, |o| o.tf.scale);
                let br = w.objects.get(b).map_or(Fix::ZERO, |o| o.tf.rotation);
                if let Some(o) = w.objects.get_mut(s) {
                    o.tf.set_rotation(br + Fix::PI);
                }
                super::spike::reattach(w, s, b, scale);
            }
            Carried::Engine(e) if w.objects.exists(e) => {
                if let Some(o) = w.objects.get_mut(e) {
                    o.active = true;
                }
                w.objects.set_parent(e, Some(b));
                let br = w.objects.get(b).map_or(Fix::ZERO, |o| o.tf.rotation);
                let r = br + Fix::PI_OVER_2 + if self.facing_right { Fix::ZERO } else { Fix::PI };
                if let Some(o) = w.objects.get_mut(e) {
                    o.tf.set_rotation(r);
                }
                if let Some(body) = w.physics.own_body_mut(e) {
                    body.orientation = r;
                }
                super::engine::reattach(w, e, b);
            }
            Carried::Rope(a) if w.objects.exists(a) => {
                let up = AbilityBody::tf(w, me).up;
                super::grappling_hook::carry_anchor_to(w, a, b, up);
            }
            _ => {}
        }
    }

    /// `SpawnBoulder()` (end of the pick-up).
    fn spawn_held(&mut self, w: &mut World, me: ObjId, cfg: &ThrowCfg) {
        let Some(bc) = cfg.boulder.clone() else {
            return;
        };
        let pos = self.held_pos(w, me);
        let b = spawn_boulder(w, &bc, pos, self.boulder_type);
        self.held = Some(b);
        let p2 = self.held_pos(w, me);
        AbilityBody::set_pos(w, b, p2);
        let s = Self::scale(w, me);
        w.set_collider_scale(b, s);
        ability_body::manual_init(w, b);
        self.attach_to_boulder(w, me, b);
    }

    /// `Fire(quickThrow)`.
    fn fire(&mut self, cx: &mut Cx<'_>, quick: bool) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let (me, owner) = (cx.me, cx.fw.owner);
        let w = &mut *cx.w;
        let s = Self::scale(w, me);
        let t = w.objects.get(me).map(|o| o.tf).unwrap_or_default();
        let boulder = if quick {
            let Some(bc) = cfg.boulder.clone() else {
                return;
            };
            let mut u = if self.passed() > cfg.sprites_before_dummy_pos {
                self.current_dummy_pos
            } else {
                cfg.spawn_pos_low * s
            };
            if !self.facing_right {
                u.x = -u.x;
            }
            let pos = t.position + t.right.complex_mul(u);
            let b = spawn_boulder(w, &bc, pos, self.boulder_type);
            w.set_collider_scale(b, s);
            ability_body::manual_init(w, b);
            w.set_collider_scale(b, s);
            self.attach_to_boulder(w, me, b);
            Some(b)
        } else {
            self.held
        };
        let Some(b) = boulder.filter(|b| w.objects.exists(*b) && w.objects.active_in_hierarchy(*b)) else {
            return;
        };
        let bd = boulder_state(w, b).map_or(Fix::HALF, |s| s.bounciness_dropped);
        drop_boulder(w, b, bd);
        w.emit(Cue::Sound("throwBoulder"));
        let mut m = if quick {
            Vec2::RIGHT
        } else {
            cfg.min_throw_vector.normalized_safe()
        };
        let mut d = w.player(owner).map_or(Vec2::ZERO, |p| p.inputs.aim_vector());
        if d == Vec2::ZERO {
            d = t.right * if self.facing_right { Fix::ONE } else { -Fix::ONE };
        }
        if t.up.dot(d) < m.dot(Vec2::UP) {
            if t.right.dot(d) < Fix::ZERO {
                m.x = m.x * -Fix::ONE;
            }
            d = m.complex_mul(t.right);
        }
        let speed = if quick {
            cfg.quick_throw_str + Fix::from_int(self.passed()) * cfg.quick_throw_charge_scaling
        } else {
            cfg.throw_speed
        };
        let p = cfg.proportion_from_scale;
        let g = (s + (p - Fix::ONE)) / p;
        let inv = w.physics.body(b).map_or(Fix::ZERO, |x| x.inverse_mass);
        if let Some(body) = w.physics.body_mut(b) {
            body.velocity = d * speed * g * inv;
            body.angular_velocity = if self.facing_right {
                -cfg.rotation_vel_on_throw
            } else {
                cfg.rotation_vel_on_throw
            };
        }
        self.held = None;
    }

    /// `MachoThrow2.UpdateSim`.
    fn throw_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let (me, owner) = (cx.me, cx.fw.owner);
        let mut mv = Vec2::ZERO;
        let v = cx.w.player(owner).map_or(Vec2::ZERO, |p| p.inputs.input_vector());
        let a = self.body.input_ground_alignment(cx.w, me, v);
        let right = a > Fix::ZERO;
        let moving = a.abs() >= GROUND_ALIGNMENT_DEADZONE;
        let hp = self.held_pos(cx.w, me);
        if let Some(p) = cx.w.player_mut(owner) {
            p.custom_aim_ability_position = hp;
        }
        if !self.released && !cx.button_down() {
            self.released = true;
        }
        self.forget_dropped(cx.w);
        self.update_held(cx.w, me);
        if let Some(h) = self.held {
            if let Some(b) = boulder_state_mut(cx.w, h) {
                b.should_drop = false;
            }
            let active = cx.w.objects.active_in_hierarchy(h) && cx.w.objects.get(h).is_some_and(|o| o.part_enabled(PartKind::Behaviour(0)));
            if !active && cx.w.objects.alive(h) {
                let bd = boulder_state(cx.w, h).map_or(Fix::HALF, |s| s.bounciness_dropped);
                drop_boulder(cx.w, h, bd);
                self.held = None;
            }
        }
        match self.state {
            ThrowState::Idle | ThrowState::Turn | ThrowState::Walk if self.released => {
                self.state = ThrowState::ThrowAnticipation;
                play_then(cx.w, &mut self.anim, cfg.anim_data, "beginThrow", CB_BEGIN_THROW);
            }
            ThrowState::Pickup if self.released => {
                let aim = cx.w.player(owner).map_or(Vec2::ZERO, |p| p.inputs.aim_vector());
                self.update_direction(cx.w, me, aim);
                self.fire(cx, true);
                self.state = ThrowState::Exit;
                play_then(cx.w, &mut self.anim, cfg.anim_data, "exit", CB_EXIT);
            }
            _ => {}
        }
        match self.state {
            ThrowState::Enter => {
                if self.grabbing_dirt && self.passed() > cfg.frames_until_shrink {
                    self.shrink_platform(cx.w, &cfg, dt);
                }
            }
            ThrowState::Idle => {
                if a.abs() >= DEADZONE {
                    self.state = ThrowState::Walk;
                    let iw = prefab::anim(cx.w, cfg.anim_data, "idleToWalk");
                    let wk = prefab::anim(cx.w, cfg.anim_data, "walk");
                    let c = cx.w.content.clone();
                    self.anim.begin_then_play(&c, iw, wk, None);
                }
            }
            ThrowState::Walk => {
                if !moving {
                    self.state = ThrowState::Idle;
                    play(cx.w, &mut self.anim, cfg.anim_data, "idle");
                } else if right != self.facing_right {
                    self.facing_right = !self.facing_right;
                    self.state = ThrowState::Turn;
                    play_then(cx.w, &mut self.anim, cfg.anim_data, "turn", CB_TURN);
                }
                mv = v;
            }
            ThrowState::Turn => {
                if !moving {
                    self.state = ThrowState::Idle;
                    play(cx.w, &mut self.anim, cfg.anim_data, "idle");
                }
                mv = v;
            }
            ThrowState::Exit => {
                if self.passed() > cfg.frames_before_move {
                    mv = v;
                }
                let jp = cx.w.player(owner).is_some_and(|p| p.inputs.jump_down_this_frame);
                if self.passed() > cfg.frames_before_jump && jp {
                    self.body.jump(cx.w, me);
                    self.exit_component(cx, AbilityExitInfo::default());
                    return;
                }
            }
            _ => {}
        }
        let mut c = Fix::ONE;
        if let Some(s) = self.body.ground(cx.w).filter(|g| cx.w.objects.alive(*g)).and_then(|g| Surface::of(cx.w, g))
            && s.in_corner(AbilityBody::pos(cx.w, me))
        {
            c = (s.radius * cfg.player_radius_hack).min(Fix::from_int(10)) / Fix::from_int(10);
        }
        if let Some(info) = self.body.macho_move(cx.w, me, mv, dt, c) {
            self.exit_component(cx, info);
        }
    }

    /// `shrinkPlatform(dt)`.
    fn shrink_platform(&mut self, w: &mut World, cfg: &ThrowCfg, dt: Fix) {
        let Some(g) = self.body.physics.attached_ground.filter(|g| w.objects.alive(*g)) else {
            return;
        };
        let Some(rr) = w.physics.rr(g).copied() else {
            return;
        };
        let h = Fix::ONE / rr.b.ie.y;
        let wd = Fix::ONE / rr.b.ie.x;
        let r = rr.radius;
        let area = Fix::TWO * h * r * Fix::TWO + (Fix::TWO * h + Fix::TWO * r) * Fix::TWO * wd + Fix::PI * r * r;
        let s = w.objects.get(g).map_or(Fix::ONE, |o| o.tf.scale);
        w.set_collider_scale(g, s - s * dt * cfg.scale_reduction / area);
        let ns = w.objects.get(g).map_or(Fix::ONE, |o| o.tf.scale);
        let min = w.objects.get(g).and_then(|o| o.collider).map_or(Fix::ZERO, |c| c.min_scale);
        if ns <= min {
            w.destroy(g);
        }
    }

    fn late(&mut self, cx: &mut Cx<'_>) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let k = self.passed();
        self.current_dummy_pos = if self.state == ThrowState::Pickup && (k as usize) < cfg.dummy_positions.len() {
            cfg.dummy_positions[k as usize]
        } else if self.state == ThrowState::ThrowAnticipation && (k as usize) < cfg.dummy_prepare.len() {
            cfg.dummy_prepare[k as usize]
        } else {
            cfg.dummy_positions.last().copied().unwrap_or(Vec2::ZERO)
        };
        self.forget_dropped(cx.w);
        self.update_held(cx.w, cx.me);
    }
}

impl AbilityImpl for Throw {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let Some(p) = def.prefab else {
            return Throw::default();
        };
        let pf = &w.content.prefabs[p];
        let Some(f) = root_fields(pf, "MachoThrow2") else {
            return Throw::default();
        };
        let boulder = prefab_field(w, f, "boulderPrefab")
            .and_then(|i| BoulderCfg::from_prefab(w, i))
            .map(Arc::new);
        let smoke_prefab = prefab_field(w, f, "smokeEffectPrefab");
        let int = |k: &str, d: i64| f.int(k).unwrap_or(d) as i32;
        let cfg = ThrowCfg {
            anim_data: prefab::anim_data_field(w, f, "animData").unwrap_or(0),
            throw_speed: fx(f, "throwSpeed", Fix::ZERO),
            quick_throw_str: fx(f, "quickThrowStr", Fix::ONE),
            quick_throw_charge_scaling: fx(f, "quickThrowChargeScaling", Fix::ONE),
            min_throw_vector: vec2_field(f, "minThrowVector").unwrap_or(Vec2::ZERO),
            proportion_from_scale: fx(f, "proportionOfExtraStrFromScale", Fix::ONE),
            rotation_vel_on_throw: fx(f, "rotationVelOnThrow", Fix::ZERO),
            force_from_boulder: fx(f, "forceFromBoulder", Fix::ONE),
            spawn_pos_low: vec2_field(f, "projectileSpawnPos_low").unwrap_or(Vec2::ZERO),
            sprites_before_dummy_pos: int("spritesBeforeWeSwapToDummyPosition", 4),
            dist_ground_to_hands: fx(f, "distfromGroundToHands", Fix::ZERO),
            scale_reduction: fx(f, "scaleReductionOnGroundPlatform", Fix::ONE),
            frames_until_shrink: int("framesUntilPlatformShrinks", 0),
            max_dist_attachable: fx(f, "maxDistToPickupAttachable", Fix::ZERO),
            ang_vel_multiplier: fx(f, "angVelMultiplierOnGround01", Fix::ZERO),
            player_radius_hack: fx(f, "playerRadiusHack", Fix::ONE),
            frames_before_move: int("framesBeforeYouCanMoveAfterThrowing", 0),
            frames_before_jump: int("framesBeforeYouCanJumpAfterThrowing", 0),
            dummy_positions: vec2_list(f, "dummyPositions"),
            dummy_prepare: vec2_list(f, "dummyPositionsPrepareThrow"),
            smoke_updatables: smoke_prefab.map_or(0, |s| prefab::updatables_of(w, s)),
            smoke_prefab,
            boulder,
        };
        let anim = ability_body::animator(prefab::root_always_stop(pf));
        Throw {
            cfg: Some(Arc::new(cfg)),
            body: AbilityBody::from_prefab(w, p),
            anim,
            ..Throw::default()
        }
    }

    fn build(&mut self, w: &mut World, me: ObjId, fw: &mut Framework, def: &AbilityDef) {
        self.body.install(fw);
        ability_body::add_parts(w, me, fw, def, &[("MachoThrow2", PartKind::Behaviour(0))]);
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

    /// The macho body, mirrored by facing; during the pick-up the boulder
    /// dummy (object 1, the platform type's boulder: `params[3]` = type + 1)
    /// in the hands at `currentDummyPos`.
    fn look(&self, _w: &World, _me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        use crate::view::PartLook;
        l.frame(&self.anim);
        let sx = if self.facing_right { Fix::ONE } else { -Fix::ONE };
        l.stretch = Vec2::new(sx, Fix::ONE);
        if self.state == ThrowState::Pickup {
            let mut p = PartLook::new(1).posed(self.current_dummy_pos, Fix::ZERO, Vec2::new(sx, Fix::ONE));
            p.params[3] = Fix::from_int(self.boulder_type + 1);
            l.parts.push(p);
        } else {
            l.parts.push(PartLook::hidden(1));
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
        self.carried = None;
        self.held = None;
        self.released = false;
        let v0 = info.self_imposed_velocity;
        self.body.sync_physics_to(cx.w, me, &info);
        AbilityBody::update_collider(cx.w, me);
        self.body.body.self_imposed_velocity = v0;
        self.state = ThrowState::Enter;
        if let Some(p) = cx.w.player_mut(info.player_id) {
            p.in_machine_throw = true;
        }
        let g = self.body.ground(cx.w);
        if let Some(g) = g
            && let Some(Kind::Platform(p)) = cx.w.objects.get(g).map(|o| &o.kind)
        {
            self.boulder_type = p.sticky.platform_type;
        }
        self.grabbing_dirt = false;
        if let Some(g) = g.filter(|g| cx.w.objects.alive(*g))
            && boulder_state(cx.w, g).is_some()
            && let Some(b) = cx.w.physics.body_mut(g)
        {
            b.angular_velocity = b.angular_velocity * cfg.ang_vel_multiplier;
        }
        self.enter_ground = g;
        play_then(cx.w, &mut self.anim, cfg.anim_data, "enter", CB_ENTER);
        self.facing_right = info.facing_right;
        let aim = cx.w.player(info.player_id).map_or(Vec2::ZERO, |p| p.inputs.aim_vector());
        self.update_direction(cx.w, me, aim);
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
            (PartKind::Behaviour(0), false) => self.throw_update(cx, dt),
            (PartKind::Behaviour(0), true) => self.late(cx),
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


    /// `MachoThrow2.ExitAbility(info)`.
    fn exit_component(&mut self, cx: &mut Cx<'_>, mut info: AbilityExitInfo) {
        let (me, owner) = (cx.me, cx.fw.owner);
        if let Some(h) = self.held.filter(|h| cx.w.objects.exists(*h)) {
            let bd = boulder_state(cx.w, h).map_or(Fix::HALF, |s| s.bounciness_dropped);
            drop_boulder(cx.w, h, bd);
        }
        if let Some(p) = cx.w.player_mut(owner) {
            p.in_machine_throw = false;
        }
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        info.grounded_speed = self.body.physics.grounded_speed;
        info.is_grounded = self.body.physics.is_grounded;
        info.position = AbilityBody::pos(cx.w, me);
        info.self_imposed_velocity = self.body.body.self_imposed_velocity;
        info.ground = self.body.physics.attached_ground;
        info.grounded_local_position = self.body.physics.local_platform_pos;
        info.radius = self.body.physics.radius;
        info.facing_right = self.facing_right;
        let iv = cx.w.player(owner).map_or(Vec2::ZERO, |p| p.inputs.input_vector());
        self.update_direction(cx.w, me, iv);
        cx.exit_body(&mut self.body, info);
    }
}

/// Objects this module spawns (boulders are platforms, see [`BoulderState`]).
pub type Spawned = NoSpawn;


//! Revival (`docs/spec/abilities/revival.md`): QuickSpell + `Revive` on the
//! ability object, the `RevivePositionIndicator` marker it plants, and the
//! death branch that re-spawns clones at the markers (`ReviveEffect`,
//! `ReviveParticle`).
//!
//! Update order inside the ability object: Ability 1 (framework),
//! SpriteAnimator 2, PlayerPhysics 3, DestroyIfOutsideSceneBounds 4,
//! PlayerCollision 5, QuickSpell 6, Revive 7 (no per-tick logic),
//! DPhysicsCircle 8, PlayerBody 9.
//!
//! Quirk kept: `ReviveParticle.InitRevive` registers the particle's
//! updatable a second time (outside the spawn bracket), so its `UpdateSim`
//! runs twice per tick (the particle travels at twice its speed).

use super::ability_body::{self, AbilityBody, COLLIDER};
use super::gust::{CB_CAST, QuickSpell};
use super::prefab;
use super::prelude::*;
use super::{AbilityState, Behaviour};
use crate::bopl::{Bopl, kill_record};
use crate::data::AnimRef;
use crate::player::ExtraLife;
use crate::world::Kind;

/// Clamp margin of the revive marker inside the camera bounds (integers).
const MARGIN: Vec2 = Vec2::new(Fix::TWO, Fix::TWO);

/// `SceneBounds.ClampInsideCameraMaxBounds(p, offset)`.
pub fn clamp_inside_camera(p: Vec2, off: Vec2) -> Vec2 {
    Vec2::new(
        p.x.clamp(CAMERA_X_MIN + off.x, CAMERA_X_MAX - off.x),
        p.y.clamp(CAMERA_Y_MIN + off.y, CAMERA_Y_MAX - off.y),
    )
}

/// The Revival ability: QuickSpell plus the `Revive` component.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Revival {
    pub q: QuickSpell,
    /// `Revive.reviveIndicator`: this slot's marker.
    pub marker: Option<ObjId>,
}

/// Objects this module spawns.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    Marker,
    Effect(ReviveEffect),
    Particle(ReviveParticle),
}

/// `ReviveEffect` (layer effect): wisp, then shoots one particle per clone.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ReviveEffect {
    pub player: i32,
    /// `(clone controller, marker)` per respawn entry.
    pub revive: Vec<(Option<ObjId>, ObjId)>,
    pub anim: SpriteAnimator,
    pub a_shoot: AnimRef,
    pub particle: Option<usize>,
}

/// `ReviveParticle`: flies from the death position to a marker.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ReviveParticle {
    pub marker: ObjId,
    pub clone: Option<ObjId>,
    pub player: i32,
    pub distance_to_travel: Fix,
    pub distance_travelled: Fix,
    pub speed: Fix,
    pub has_arrived: bool,
    pub anim: SpriteAnimator,
    pub a_spawn: AnimRef,
}

const CB_SHOOT: u16 = 1;
const CB_SELF_DESTRUCT: u16 = 2;

impl AbilityImpl for Revival {
    fn exit_component(&mut self, cx: &mut Cx<'_>, r: AbilityExitInfo) {
        self.q.exit(cx, r);
    }

    fn new(w: &World, def: &AbilityDef) -> Self {
        Revival {
            q: QuickSpell::new(w, def),
            marker: None,
        }
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        self.q.enter(cx);
    }

    fn update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        if self.q.update(cx) == Some(CB_CAST) {
            let p = clamp_inside_camera(self.q.spell_position(cx), MARGIN);
            if let Some(prefab) = self.q.spell {
                let id = ability_body::spawn_prefab(
                    cx.w,
                    prefab,
                    Kind::Behaviour(Box::new(Behaviour::Revival(Spawned::Marker))),
                    &[
                        ("RevivePositionIndicator", PartKind::Behaviour(0)),
                        ("BoplBody", PartKind::BoplBody),
                        (COLLIDER, PartKind::Collider),
                    ],
                    p,
                    None,
                );
                if let Some(o) = cx.w.objects.get_mut(id) {
                    o.player_id = cx.fw.owner;
                }
                self.set_revive_flag(cx, id);
            }
            self.q.finish_cast(cx);
        }
        self.q.update_rest(cx, dt);
    }

    fn late_update(&mut self, cx: &mut Cx<'_>, _dt: Fix) {
        self.q.late_update(cx);
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        ability_body::hurt_contact(self, cx, c);
    }


    fn sprite(&self) -> crate::data::SpriteId {
        self.q.anim.sprite
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        Some(&mut self.q.b)
    }
}

/// A marker that was ended (or never existed) counts as destroyed.
fn marker_gone(w: &World, m: ObjId) -> bool {
    w.objects
        .get(m)
        .is_none_or(|o| o.part(PartKind::Behaviour(0)).is_none_or(|p| p.destroyed))
}

/// `RevivePositionIndicator.End()`: the marker leaves the simulation (its
/// visual lingers for the particle duration; presentation).
fn end_marker(w: &mut World, m: ObjId) {
    if !w.objects.exists(m) {
        return;
    }
    for k in [PartKind::Behaviour(0), PartKind::BoplBody, PartKind::Collider] {
        w.destroy_part(m, k);
    }
    if let Some(o) = w.objects.get_mut(m) {
        o.tf.destroyed = true;
    }
}

impl Revival {
    /// `Revive.SetReviveFlag(indicator)`.
    fn set_revive_flag(&mut self, cx: &mut Cx<'_>, indicator: ObjId) {
        let owner = cx.fw.owner;
        if let Some(old) = self.marker
            && !marker_gone(cx.w, old)
        {
            end_marker(cx.w, old);
            // The literal trim walk from the end.
            let gone: Vec<bool> = match cx.w.player(owner) {
                Some(p) => p
                    .respawn_positions
                    .iter()
                    .map(|m| marker_gone(cx.w, *m))
                    .collect(),
                None => Vec::new(),
            };
            if let Some(p) = cx.w.player_mut(owner) {
                let mut gone = gone;
                let mut i = p.respawn_positions.len() as i64 - 1;
                while i >= 0 && (i as usize) < p.respawn_positions.len() {
                    if gone[i as usize] {
                        p.respawn_positions.remove(i as usize);
                        gone.remove(i as usize);
                    } else {
                        i -= 1;
                    }
                }
            }
        }
        self.marker = Some(indicator);
        let me = cx.me;
        if let Some(p) = cx.w.player_mut(owner) {
            p.extra_life = ExtraLife::WillReviveOnDeath;
            p.revive_instance = Some(me);
            p.respawn_positions.push(indicator);
        }
    }
}

/// Animation data of a prefab component field.
fn anim_of(w: &World, prefab: usize, class: &str, field: &str, name: &str) -> AnimRef {
    let data = w
        .content
        .prefabs
        .get(prefab)
        .and_then(|p| p.objects.first())
        .and_then(|o| o.component(class))
        .and_then(|c| c.fields.reference(field))
        .and_then(|r| prefab::anim_data_index(w, r))
        .unwrap_or(0);
    prefab::anim(w, data, name)
}

/// The death branch for a player with revive markers (player.md §8.2 step 6).
pub fn revive_on_death(w: &mut World, me: ObjId, b: &mut Bopl, killer: i32, cause: CauseOfDeath) {
    let id = b.ctrl.player_number;
    b.unground(w, me, true, true);
    if let Some(p) = w.player_mut(id) {
        p.extra_life = ExtraLife::IsBeingRevived;
    }
    let markers = w.player(id).map(|p| p.respawn_positions.clone()).unwrap_or_default();
    let mut list = Vec::new();
    let mut first_clone = None;
    for m in markers {
        let pos = AbilityBody::pos(w, m);
        let clone = spawn_clone(w, id, b, pos);
        if first_clone.is_none() {
            first_clone = clone;
        }
        list.push((clone, m));
    }
    kill_record(w, id, killer, cause);
    if let Some(p) = w.player_mut(id) {
        p.respawn_positions.clear();
        if first_clone.is_some() {
            p.bopl = first_clone;
        }
    }
    let pos = AbilityBody::pos(w, me);
    spawn_revive_effect(w, id, pos, list);
    for a in b.ctrl.abilities.clone() {
        if w.objects.exists(a) {
            w.destroy(a);
        }
    }
    b.ctrl.abilities.clear();
    w.destroy(me);
    w.emit(Cue::Sound("reviveOnDeath"));
}

/// `PlayerCollision.SpawnClone`: a new controller with fresh copies of the
/// dead controller's slot abilities, prepared for revival at `pos`.
fn spawn_clone(w: &mut World, id: i32, dead: &Bopl, pos: Vec2) -> Option<ObjId> {
    let alive = w.player(id).map_or(0, |p| p.players_and_clones_still_alive);
    if alive > MAX_CLONES {
        return None;
    }
    if let Some(p) = w.player_mut(id) {
        p.players_and_clones_still_alive += 1;
    }
    let clone = Bopl::spawn(w, pos, id);
    let mut abilities = Vec::new();
    for a in &dead.ctrl.abilities {
        let idx = match w.objects.get(*a).map(|o| &o.kind) {
            Some(Kind::Ability(x)) => Some(x.def),
            _ => None,
        };
        if let Some(idx) = idx {
            abilities.push(super::instantiate(w, idx, id, clone));
        }
    }
    let indicators = dead.ctrl.indicators;
    w.with_kind(clone, |w, k| {
        if let Kind::Bopl(c) = k {
            c.ctrl.abilities = abilities;
            c.ctrl.indicators = indicators;
            c.prepare_to_revive(w, clone, pos);
        }
    });
    Some(clone)
}

/// `InstantiateFixed(reviveEffectPrefab, pos).Init(player, list)`.
fn spawn_revive_effect(w: &mut World, player: i32, pos: Vec2, revive: Vec<(Option<ObjId>, ObjId)>) {
    let prefab = w
        .content
        .player
        .as_ref()
        .and_then(|p| w.content.prefab(&p.prefab))
        .and_then(|p| p.objects.first())
        .and_then(|o| o.component("PlayerCollision"))
        .and_then(|c| c.fields.reference("reviveEffectPrefab"))
        .and_then(|r| prefab::prefab_index(w, r));
    let Some(prefab) = prefab else {
        return;
    };
    let particle = w
        .content
        .prefabs
        .get(prefab)
        .and_then(|p| p.objects.first())
        .and_then(|o| o.component("ReviveEffect"))
        .and_then(|c| c.fields.reference("Particle"))
        .and_then(|r| prefab::prefab_index(w, r));
    let wisp = anim_of(w, prefab, "ReviveEffect", "animData", "wisp");
    let shoot = anim_of(w, prefab, "ReviveEffect", "animData", "shoot");
    let mut anim = SpriteAnimator::default();
    anim.begin_then_do(&w.content, wisp, AnimCallback::Ability(CB_SHOOT));
    let e = ReviveEffect {
        player,
        revive,
        anim,
        a_shoot: shoot,
        particle,
    };
    ability_body::spawn_prefab(
        w,
        prefab,
        Kind::Behaviour(Box::new(Behaviour::Revival(Spawned::Effect(e)))),
        &[
            ("SpriteAnimator", PartKind::Behaviour(1)),
            ("ReviveEffect", PartKind::Behaviour(2)),
        ],
        pos,
        None,
    );
}

/// Animator advance with the time-stop rule for holder `player`.
fn advance(w: &World, anim: &mut SpriteAnimator, player: i32) -> Option<u16> {
    let go = if w.is_time_stopped() {
        !anim.always_stop_during_time_stop
            && w.player(player).is_some_and(|p| p.is_protected_from_time_stop)
    } else {
        true
    };
    if !go {
        return None;
    }
    match anim.advance(&w.content, Fix::ZERO) {
        Some(AnimCallback::Ability(c)) => Some(c),
        _ => None,
    }
}

impl ReviveEffect {
    fn shoot(&mut self, w: &mut World, me: ObjId) {
        if self.revive.is_empty() {
            w.destroy(me);
            return;
        }
        let Some(prefab) = self.particle else {
            w.destroy(me);
            return;
        };
        let a_spawn = anim_of(w, prefab, "ReviveParticle", "spawnEffect", "spawn");
        let speed = w
            .content
            .prefabs
            .get(prefab)
            .and_then(|p| p.objects.first())
            .and_then(|o| o.component("ReviveParticle"))
            .map_or(Fix::from_int(30), |c| c.fields.fix_or("speed", Fix::from_int(30)));
        for (clone, marker) in self.revive.clone() {
            let from = AbilityBody::pos(w, me);
            let target = AbilityBody::pos(w, marker);
            let v = (target - from).normalized_safe();
            let pid = clone
                .and_then(|c| match w.objects.get(c).map(|o| &o.kind) {
                    Some(Kind::Bopl(b)) => Some(b.ctrl.player_number),
                    _ => None,
                })
                .unwrap_or(1);
            let p = ReviveParticle {
                marker,
                clone,
                player: pid,
                distance_to_travel: Fix::ZERO,
                distance_travelled: Fix::ZERO,
                speed,
                has_arrived: false,
                anim: SpriteAnimator::default(),
                a_spawn,
            };
            let id = ability_body::spawn_prefab(
                w,
                prefab,
                Kind::Behaviour(Box::new(Behaviour::Revival(Spawned::Particle(p)))),
                &[
                    ("ReviveParticle", PartKind::Behaviour(3)),
                    ("SpriteAnimator", PartKind::Behaviour(4)),
                ],
                from,
                None,
            );
            if v != Vec2::ZERO {
                if let Some(o) = w.objects.get_mut(id) {
                    o.tf.set_right(v);
                }
                if let Some(o) = w.objects.get_mut(me) {
                    o.tf.set_right(v);
                }
            }
            // InitRevive: distance, and the second registration.
            let d = (AbilityBody::pos(w, marker) - AbilityBody::pos(w, id)).magnitude();
            if let Some(Kind::Behaviour(b)) = w.objects.get_mut(id).map(|o| &mut o.kind)
                && let Behaviour::Revival(Spawned::Particle(p)) = b.as_mut()
            {
                p.distance_to_travel = d;
            }
            if let Some(i) = w.objects.get(id).and_then(|o| o.part_index(PartKind::Behaviour(3))) {
                w.register(id, i);
            }
        }
        let a = self.a_shoot;
        self.anim
            .begin_then_do(&w.content, a, AnimCallback::Ability(CB_SELF_DESTRUCT));
    }
}

impl ReviveParticle {
    fn update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        if self.distance_travelled > self.distance_to_travel {
            return;
        }
        let pdt = match self.clone {
            Some(c) if w.objects.exists(c) => w.fixed_delta_time(self.player, dt),
            _ => dt,
        };
        let target = AbilityBody::pos(w, self.marker);
        let here = AbilityBody::pos(w, me);
        let v = (target - here).normalized_safe() * (self.speed * pdt);
        self.distance_travelled += (self.speed * pdt).abs();
        if self.distance_travelled >= self.distance_to_travel && !self.has_arrived {
            end_marker(w, self.marker);
            AbilityBody::set_pos(w, me, target);
            self.has_arrived = true;
            self.speed = Fix::ZERO;
            w.emit(Cue::Sound("reviveDing"));
            let a = self.a_spawn;
            self.anim
                .begin_then_do(&w.content, a, AnimCallback::Ability(CB_SELF_DESTRUCT));
            if let Some(c) = self.clone.filter(|c| w.objects.exists(*c)) {
                w.with_kind(c, |w, k| {
                    if let Kind::Bopl(b) = k {
                        b.respawn(w, c, target);
                    }
                });
            }
        } else {
            AbilityBody::set_pos(w, me, here + v);
        }
    }
}

impl super::SpawnedImpl for Spawned {
    /// The revive marker's particles take the owner's `_ShadowColor`.
    fn look(&self, _w: &World, _me: ObjId, l: &mut crate::view::Look) {
        match self {
            Spawned::Marker => l.shadow_color = true,
            Spawned::Effect(e) => {
                l.frame(&e.anim);
                l.player = e.player;
            }
            Spawned::Particle(p) => {
                l.frame(&p.anim);
                l.player = p.player;
                l.shadow_color = true;
            }
        }
    }

    /// `ISpell.AddScale(d)` (shrink/growth rays) on the revive marker:
    /// collider scale += d.
    fn interact(&mut self, w: &mut World, me: ObjId, i: super::Interaction) -> Option<Fix> {
        match (self, i) {
            (Spawned::Marker, super::Interaction::SpellAddScale(d)) => {
                let s = w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale);
                w.set_collider_scale(me, s + d);
                Some(Fix::ZERO)
            }
            _ => None,
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        if late {
            return;
        }
        match (self, part) {
            (Spawned::Marker, PartKind::Behaviour(0)) => {
                let mut p = clamp_inside_camera(AbilityBody::pos(w, me), MARGIN);
                if p.y < WATER_HEIGHT && !w.is_space() {
                    p.y = WATER_HEIGHT;
                }
                AbilityBody::set_pos(w, me, p);
            }
            (Spawned::Effect(e), PartKind::Behaviour(1)) => match advance(w, &mut e.anim, e.player) {
                Some(CB_SHOOT) => e.shoot(w, me),
                Some(CB_SELF_DESTRUCT) => w.destroy(me),
                _ => {}
            },
            (Spawned::Particle(p), PartKind::Behaviour(3)) => p.update(w, me, dt),
            (Spawned::Particle(p), PartKind::Behaviour(4)) => {
                if advance(w, &mut p.anim, p.player) == Some(CB_SELF_DESTRUCT) {
                    w.destroy(me);
                }
            }
            _ => {}
        }
    }
}

/// `PrepareToRevive`: the slot whose Revive is the player's `ReviveInstance`
/// restarts its cooldown (never matches for clones, whose abilities are new).
pub fn on_prepare_to_revive(w: &mut World, _me: ObjId, b: &mut Bopl) {
    let inst = w.player(b.ctrl.player_number).and_then(|p| p.revive_instance);
    let Some(inst) = inst else {
        return;
    };
    for (i, a) in b.ctrl.abilities.iter().enumerate() {
        let is_revive = matches!(w.objects.get(*a).map(|o| &o.kind),
            Some(Kind::Ability(x)) if matches!(x.state, AbilityState::Revival(_)));
        if is_revive
            && *a == inst
            && let Some(t) = b.ctrl.cooldown_timers.get_mut(i)
        {
            *t = Fix::ZERO;
        }
    }
}

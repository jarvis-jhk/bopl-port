//! The tutorial scene (docs/spec/tutorial.md): `TutorialGameHandler`,
//! `TutorialArrow`, `TutorialArrowPickup`, `TutorialTargetDummy` and the
//! dummies' `AnimateGround` platforms, loaded instead of the session
//! handler when a level has [`LevelData::tutorial`].
//!
//! The progress state lives in [`World::tutorial`]; the scene's objects are
//! `Kind::Tutorial` objects (their colliders in the physics lists, one
//! updatable part each). Never networked or recorded, but deterministic
//! like the rest of the simulation.

use std::sync::Arc;

use crate::abilities;
use crate::bopl::Bopl;
use crate::data::{LevelData, TutorialDef, TutorialObjDef};
use crate::fix::Fix;
use crate::object::{ObjId, PartKind};
use crate::physics::Contact;
use crate::player::ExtraLife;
use crate::vec2::Vec2;
use crate::view::{Cue, EffectKind, Visual};
use crate::world::{Kind, World};

/// Layer `wall`.
const WALL: i32 = 11;

/// What a tutorial object is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    /// `TutorialGameHandler` (its `UpdateSim`).
    Handler,
    /// `TutorialArrow` `i` (index into `TutorialDef::arrows`).
    Arrow(usize),
    /// `TutorialArrowPickup` `i`: hover start and time.
    Pickup {
        index: usize,
        start: Vec2,
        time: Fix,
    },
    /// `TutorialTargetDummy` `i`; offset from its ground's platform.
    Dummy {
        index: usize,
        offset: Vec2,
        hit: bool,
    },
    /// `AnimateGround` `i` of `TutorialDef::grounds`.
    Ground {
        index: usize,
        frames: i64,
        reversed: bool,
    },
}

/// The hint texts the handler switches (both hint sets alike).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Hints {
    pub hold_to_shoot: bool,
    pub hold_to_grenade: bool,
    pub click_to_dash: bool,
    /// The keyboard / controller indicator above the level.
    pub device_indicator: bool,
}

/// Progress of the tutorial.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct State {
    pub def: Arc<TutorialDef>,
    /// `movementArrowsAbsorbed` (spec section 3).
    pub absorbed: i32,
    pub has_dash: bool,
    pub dashed: bool,
    pub since_dash: Fix,
    jingle_pending: bool,
    /// The tutorial ended (dash done, or select): back to the main menu.
    pub done: bool,
    pub hints: Hints,
    /// Slots unlocked so far.
    pub unlocked: [bool; 3],
    pub arrows: Vec<ObjId>,
    pub pickups: Vec<ObjId>,
    pub dummies: Vec<ObjId>,
    pub grounds: Vec<ObjId>,
}

impl State {
    /// The movement hint shown now (`wasd[M]`; 0..=4 = A, S, D, W, SPACE).
    pub fn movement_hint(&self) -> Option<usize> {
        (0..5)
            .contains(&self.absorbed)
            .then_some(self.absorbed as usize)
    }
}

fn place(o: &mut crate::object::Object, t: &crate::data::TransformDef) {
    o.tf.position = t.position;
    o.tf.rotation = t.rotation;
    o.tf.right = t.right;
    o.tf.up = t.up;
    o.tf.offset = t.offset;
}

fn create_obj(w: &mut World, d: &TutorialObjDef, part: Part) -> ObjId {
    let id = w.objects.create(Kind::Tutorial(Box::new(part)), d.layer);
    let o = w.objects.get_mut(id).expect("created");
    place(o, &d.transform);
    o.active = d.active;
    o.visual = Visual::None;
    o.collider = Some(d.collider);
    o.add_part(PartKind::Tutorial, d.hier);
    o.add_part(PartKind::Collider, d.hier_collider);
    id
}

/// The tutorial part of a level load (`session::load_level`): handler,
/// arrows, pickups, dummies, grounds, and the player spawned at once with
/// its three locked abilities.
pub fn load(w: &mut World, level: &Arc<LevelData>, def: &Arc<TutorialDef>) {
    let hid = w.objects.create(Kind::Tutorial(Box::new(Part::Handler)), 0);
    w.objects
        .get_mut(hid)
        .expect("created")
        .add_part(PartKind::Tutorial, def.hier);
    let arrows = (0..def.arrows.len())
        .map(|i| create_obj(w, &def.arrows[i], Part::Arrow(i)))
        .collect();
    let pickups = (0..def.pickups.len())
        .map(|i| {
            let start = def.pickups[i].obj.transform.position;
            create_obj(
                w,
                &def.pickups[i].obj,
                Part::Pickup {
                    index: i,
                    start,
                    time: Fix::ZERO,
                },
            )
        })
        .collect();
    let grounds: Vec<ObjId> = (0..def.grounds.len())
        .map(|i| {
            let id = w.objects.create(
                Kind::Tutorial(Box::new(Part::Ground {
                    index: i,
                    frames: 0,
                    reversed: false,
                })),
                0,
            );
            let o = w.objects.get_mut(id).expect("created");
            let k = o.add_part(PartKind::Tutorial, def.grounds[i].hier);
            // Disabled until its slot is unlocked.
            o.parts[k].enabled = false;
            id
        })
        .collect();
    let dummies = (0..def.dummies.len())
        .map(|i| {
            let d = &def.dummies[i];
            let pt = level.platforms[def.grounds[d.ground].platform].transform;
            let offset = (d.obj.transform.position - pt.position)
                .complex_mul(Vec2::from_angle(-pt.rotation));
            create_obj(
                w,
                &d.obj,
                Part::Dummy {
                    index: i,
                    offset,
                    hit: false,
                },
            )
        })
        .collect();
    w.tutorial = Some(Box::new(State {
        def: def.clone(),
        absorbed: 0,
        has_dash: false,
        dashed: false,
        since_dash: Fix::ZERO,
        jingle_pending: true,
        done: false,
        hints: Hints {
            device_indicator: true,
            ..Hints::default()
        },
        unlocked: [false; 3],
        arrows,
        pickups,
        dummies,
        grounds,
    }));
    for id in w.objects.ids().collect::<Vec<_>>() {
        if matches!(w.objects.get(id).map(|o| &o.kind), Some(Kind::Tutorial(_))) {
            w.register_all(id);
        }
    }
    spawn_player(w, def);
}

/// The pre-placed player: spawned at `playerSpawn` with Bow, Grenade and
/// Dash, each locked by its cooldown (spec section 2).
fn spawn_player(w: &mut World, def: &TutorialDef) {
    let Some(p) = w.players.first_mut() else {
        return;
    };
    p.players_and_clones_still_alive = 1;
    p.still_alive_this_round = true;
    p.extra_life = ExtraLife::WillReviveOnDeath;
    p.spawn_position = def.player_spawn;
    p.won_this_round = false;
    p.kills_at_start_of_round = p.kills;
    p.cause_of_death = crate::player::CauseOfDeath::NotDeadYet;
    let id = p.id;
    let bopl = Bopl::spawn(w, def.player_spawn, id);
    let mut objs = Vec::new();
    for &a in &def.abilities {
        let o = abilities::instantiate(w, a, id, bopl);
        set_cooldown(w, o, def.locked_cooldown);
        objs.push(o);
    }
    if let Some(p) = w.players.first_mut() {
        p.bopl = Some(bopl);
        p.current_abilities = def.abilities.to_vec();
        p.abilities = def.abilities.to_vec();
    }
    if let Some(Kind::Bopl(b)) = w.objects.get_mut(bopl).map(|o| &mut o.kind) {
        b.ctrl.abilities = objs;
    }
    w.round.game_in_progress = true;
}

fn set_cooldown(w: &mut World, ability: ObjId, cd: Fix) {
    if let Some(Kind::Ability(a)) = w.objects.get_mut(ability).map(|o| &mut o.kind) {
        a.fw.cooldown = cd;
    }
}

fn player_abilities(w: &World) -> Vec<ObjId> {
    let Some(b) = w.players.first().and_then(|p| p.bopl) else {
        return Vec::new();
    };
    match w.objects.get(b).map(|o| &o.kind) {
        Some(Kind::Bopl(b)) => b.ctrl.abilities.clone(),
        _ => Vec::new(),
    }
}

/// `UnlockAbility(slot)` (spec section 5).
fn unlock(w: &mut World, slot: usize) {
    let Some(st) = w.tutorial.as_mut() else {
        return;
    };
    st.absorbed += 1;
    st.unlocked[slot.min(2)] = true;
    let cd = st.def.cooldowns[slot.min(2)];
    let ground = match slot {
        0 => Some(0),
        1 => Some(1),
        _ => None,
    }
    .and_then(|g| st.grounds.get(g).copied());
    match slot {
        0 => {
            st.hints.hold_to_shoot = true;
            st.hints.device_indicator = false;
        }
        1 => {
            st.hints.hold_to_grenade = true;
            st.hints.hold_to_shoot = false;
        }
        _ => {
            st.hints.hold_to_grenade = false;
            st.hints.click_to_dash = true;
            st.has_dash = true;
        }
    }
    if let Some(g) = ground
        && let Some(o) = w.objects.get_mut(g)
    {
        o.set_part_enabled(PartKind::Tutorial, true);
    }
    if let Some(&a) = player_abilities(w).get(slot) {
        set_cooldown(w, a, cd);
    }
}

pub fn init(w: &mut World, me: ObjId, part: &mut Part) {
    match part {
        // `Init`: `go` and the match music (the presentation starts it).
        Part::Handler => w.emit(Cue::Sound("go")),
        Part::Pickup { start, time, .. } => {
            *start = w.objects.get(me).map_or(*start, |o| o.tf.position);
            *time = Fix::ZERO;
        }
        Part::Ground { frames, .. } => *frames = 0,
        _ => {}
    }
}

pub fn update(w: &mut World, me: ObjId, part: &mut Part, dt: Fix) {
    match part {
        Part::Handler => handler_update(w, dt),
        Part::Pickup { index, start, time } => {
            let Some(def) = w.tutorial.as_ref().map(|s| s.def.clone()) else {
                return;
            };
            let p = &def.pickups[*index];
            *time += dt;
            let y = (*time * p.hover_speed).sin() * p.hover_amplitude;
            if let Some(o) = w.objects.get_mut(me) {
                o.tf.position = *start + Vec2::UP * y;
            }
        }
        Part::Ground {
            index,
            frames,
            reversed,
        } => {
            let Some(def) = w.tutorial.as_ref().map(|s| s.def.clone()) else {
                return;
            };
            let g = &def.grounds[*index];
            let t = Fix::from_long(*frames) * dt * w.time.player_time_scale - g.delay;
            let c = g.curve.eval(t);
            *frames += 1;
            let keys: Vec<Vec2> = if *reversed {
                g.keys.iter().rev().copied().collect()
            } else {
                g.keys.clone()
            };
            let home = path_point(&keys, c);
            if let Some(&pid) = w.platforms.get(g.platform)
                && let Some(Kind::Platform(p)) = w.objects.get_mut(pid).map(|o| &mut o.kind)
                && let Some(s) = &mut p.spring
            {
                s.home = home;
            }
        }
        Part::Dummy { index, offset, .. } => {
            // Rides its ground (spec section 9: the authored offset).
            let Some(def) = w.tutorial.as_ref().map(|s| s.def.clone()) else {
                return;
            };
            let g = def.dummies[*index].ground;
            let Some(&pid) = def.grounds.get(g).and_then(|g| w.platforms.get(g.platform)) else {
                return;
            };
            let Some((pos, rot)) = w.objects.get(pid).map(|o| (o.tf.position, o.tf.rotation))
            else {
                return;
            };
            let p = pos + offset.complex_mul(Vec2::from_angle(rot));
            if let Some(o) = w.objects.get_mut(me) {
                o.tf.position = p;
            }
        }
        Part::Arrow(_) => {}
    }
}

/// The point at fraction `c` of the path's length (`AnimateGround`).
fn path_point(keys: &[Vec2], c: Fix) -> Vec2 {
    if keys.len() < 2 {
        return keys.first().copied().unwrap_or_default();
    }
    let mut total = Fix::ZERO;
    for k in keys.windows(2) {
        total += (k[1] - k[0]).magnitude();
    }
    let target = c * total;
    let mut seg = 0;
    let mut frac = Fix::ZERO;
    let mut acc = Fix::ZERO;
    for j in 1..keys.len() {
        let mut len = (keys[j] - keys[j - 1]).magnitude();
        let before = acc;
        acc += len;
        if acc >= target {
            seg = j - 1;
            if len == Fix::ZERO {
                len = Fix(429_497); // (Fix)0.0001f
            }
            frac = (target - before) / len;
            break;
        }
    }
    let a = keys[seg];
    let b = keys[(seg + 1).min(keys.len() - 1)];
    a + (b - a) * frac
}

/// `TutorialGameHandler.UpdateSim` (spec section 3).
fn handler_update(w: &mut World, dt: Fix) {
    let Some(st) = w.tutorial.as_ref() else {
        return;
    };
    let absorbed = st.absorbed;
    let arrows = st.arrows.clone();
    let bow = st.pickups.first().copied();
    let has_dash = st.has_dash;
    if let Some(p) = w.players.first_mut() {
        p.extra_life = ExtraLife::PermanentRevive;
    }
    for (i, &a) in arrows.iter().enumerate() {
        if i as i32 > absorbed {
            break;
        }
        if let Some(o) = w.objects.get_mut(a)
            && !o.destroyed
            && !o.active
        {
            o.active = true;
        }
    }
    if absorbed > 3
        && let Some(b) = bow
        && let Some(o) = w.objects.get_mut(b)
        && !o.destroyed
        && !o.active
    {
        o.active = true;
    }
    let input = w
        .players
        .first()
        .map(|p| p.inputs.input)
        .unwrap_or_default();
    let mut jingle = false;
    let mut exit = false;
    if let Some(st) = w.tutorial.as_mut() {
        if has_dash && input.ability(2) {
            st.hints.click_to_dash = false;
            st.since_dash = Fix::ZERO;
            st.dashed = true;
        }
        if st.dashed {
            st.since_dash += dt;
            if st.jingle_pending && st.since_dash > st.def.jingle_delay {
                jingle = true;
                st.jingle_pending = false;
            }
            if st.since_dash > st.def.exit_delay {
                exit = true;
            }
        }
        if input.buttons & crate::input::BTN_SELECT != 0 {
            exit = true;
        }
        if exit {
            st.done = true;
        }
    }
    if jingle {
        w.emit(Cue::Sound("accept2"));
    }
}

/// Collision callbacks of tutorial objects (spec sections 4–6).
pub fn on_contact(w: &mut World, me: ObjId, part: &mut Part, c: Contact) {
    match part {
        Part::Arrow(_) => {
            if w.objects.get(me).is_none_or(|o| o.destroyed) {
                return;
            }
            if let Some(st) = w.tutorial.as_mut() {
                st.absorbed += 1;
            }
            w.destroy(me);
            w.emit(Cue::Sound("accept"));
        }
        Part::Pickup { index, .. } => {
            if w.objects.get(me).is_none_or(|o| o.destroyed) {
                return;
            }
            let slot = w
                .tutorial
                .as_ref()
                .map_or(0, |s| s.def.pickups[*index].slot);
            unlock(w, slot);
            w.emit(Cue::Sound("abilityPickup"));
            w.destroy(me);
        }
        Part::Dummy { index, hit, .. } => {
            let Some(def) = w.tutorial.as_ref().map(|s| s.def.clone()) else {
                return;
            };
            let d = &def.dummies[*index];
            if *hit || c.layer == WALL || !(0..32).contains(&c.layer) || d.mask >> c.layer & 1 == 0
            {
                return;
            }
            *hit = true;
            // Its ground flies back.
            if let Some(&g) = w.tutorial.as_ref().and_then(|s| s.grounds.get(d.ground))
                && let Some(Kind::Tutorial(p)) = w.objects.get_mut(g).map(|o| &mut o.kind)
                && let Part::Ground {
                    frames, reversed, ..
                } = p.as_mut()
            {
                *frames = 0;
                *reversed = !*reversed;
            }
            let pickup = if d.unlock_grenade {
                Some(1)
            } else if d.unlock_dash {
                Some(2)
            } else {
                None
            };
            if let Some(st) = w.tutorial.as_mut() {
                if d.unlock_grenade {
                    st.hints.hold_to_shoot = false;
                }
                if d.unlock_dash {
                    st.hints.hold_to_grenade = false;
                }
                if pickup.is_some() {
                    st.absorbed -= 1;
                }
            }
            if let Some(k) = pickup
                && let Some(&p) = w.tutorial.as_ref().and_then(|s| s.pickups.get(k))
                && let Some(o) = w.objects.get_mut(p)
            {
                o.active = true;
            }
            // Killed through its PlayerCollision (no player record).
            let pos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
            w.emit(Cue::Effect {
                kind: EffectKind::Death,
                position: pos,
                player_id: 0,
            });
            w.emit(Cue::Shake(800));
            w.destroy(me);
        }
        Part::Handler | Part::Ground { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_points() {
        let keys = [Vec2::new(Fix::ZERO, Fix::from_int(10)), Vec2::ZERO];
        assert_eq!(path_point(&keys, Fix::ZERO), keys[0]);
        assert_eq!(path_point(&keys, Fix::ONE), keys[1]);
        let mid = path_point(&keys, Fix::ONE / Fix::from_int(2));
        assert_eq!(mid, Vec2::new(Fix::ZERO, Fix::from_int(5)));
    }
}

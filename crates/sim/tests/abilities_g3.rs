//! Scenario tests for the G3 abilities (Dash, Roll, Teleport, Grappling
//! hook + ropes, Engine, Drill, Meteor) on real levels from the import
//! (skipped without an import).

use std::sync::Arc;

use bopl_content::Loaded;
use bopl_sim::abilities::AbilityState;
use bopl_sim::input::{BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2, BTN_JUMP, PlayerInput};
use bopl_sim::object::ObjId;
use bopl_sim::player::{CauseOfDeath, PlayerSetup};
use bopl_sim::surface::Surface;
use bopl_sim::view::Cue;
use bopl_sim::world::{Kind, MatchSettings, World};
use bopl_sim::{Fix, Vec2};

const DASH: usize = 2;
const ENGINE: usize = 5;
const ROLL: usize = 16;
const METEOR: usize = 20;
const TELEPORT: usize = 25;
const DRILL: usize = 26;
const HOOK: usize = 27;

const RIGHT: u8 = 1;
const UP: u8 = 64;
const LEFT: u8 = 128;
const DOWN: u8 = 192;

fn loaded() -> Option<&'static Loaded> {
    use std::sync::OnceLock;
    static L: OnceLock<Option<Loaded>> = OnceLock::new();
    L.get_or_init(|| {
        let dir = bopl_content::default_import_dir();
        if !dir.join("index.json").exists() {
            eprintln!("no import at {}; skipping", dir.display());
            return None;
        }
        Some(bopl_content::load(dir).expect("import loads"))
    })
    .as_ref()
}

fn setup(id: i32, abilities: &[usize]) -> PlayerSetup {
    PlayerSetup {
        id,
        team: id - 1,
        color: id - 1,
        abilities: abilities.to_vec(),
        uses_keyboard_and_mouse: false,
    }
}

fn world(l: &Loaded, level: usize, players: &[PlayerSetup]) -> World {
    let lv = bopl_content::load_level(l, level).unwrap();
    let mut w = World::new(
        Arc::clone(&l.content),
        players,
        MatchSettings {
            number_of_abilities: 3,
            seed: 11,
        },
    );
    bopl_sim::session::load_level(&mut w, lv);
    w
}

/// A scripted run: per tick, a closure gives each player's input.
struct Run {
    w: World,
    events: Vec<(i64, Cue)>,
}

impl Run {
    fn new(l: &Loaded, level: usize, players: &[PlayerSetup]) -> Run {
        let mut r = Run {
            w: world(l, level, players),
            events: Vec::new(),
        };
        // Players spawn on tick 167 and land soon after; slot timers start
        // at −1.5 s, so every slot here is ready by tick 420.
        r.idle(420);
        r
    }

    fn step(&mut self, inputs: &[PlayerInput]) {
        let mut full = inputs.to_vec();
        full.resize(self.w.players.len(), PlayerInput::default());
        self.w.step(&full);
        for e in &self.w.events {
            self.events.push((e.id.tick, e.cue.clone()));
        }
    }

    fn idle(&mut self, n: usize) {
        for _ in 0..n {
            self.step(&[]);
        }
    }

    fn hold(&mut self, n: usize, inputs: &[PlayerInput]) {
        for _ in 0..n {
            self.step(inputs);
        }
    }

    fn tick(&self) -> i64 {
        self.w.time.simulation_ticks
    }

    fn deaths(&self) -> Vec<(i64, i32, i32, CauseOfDeath)> {
        self.events
            .iter()
            .filter_map(|(t, c)| match c {
                Cue::PlayerDied { id, killer, cause } => Some((*t, *id, *killer, *cause)),
                _ => None,
            })
            .collect()
    }

    fn sounds(&self, name: &str) -> Vec<i64> {
        self.events
            .iter()
            .filter_map(|(t, c)| matches!(c, Cue::Sound(n) if *n == name).then_some(*t))
            .collect()
    }
}

fn input(angle: u8, buttons: u8) -> PlayerInput {
    PlayerInput::new(angle, buttons)
}

fn bopl_id(w: &World, id: i32) -> ObjId {
    w.player(id).and_then(|p| p.bopl).expect("bopl")
}

fn bopl(w: &World, id: i32) -> &bopl_sim::bopl::Bopl {
    match &w.objects.get(bopl_id(w, id)).expect("live bopl").kind {
        Kind::Bopl(b) => b,
        _ => panic!("not a bopl"),
    }
}

fn bopl_mut(w: &mut World, id: i32) -> &mut bopl_sim::bopl::Bopl {
    let b = bopl_id(w, id);
    match &mut w.objects.get_mut(b).expect("live bopl").kind {
        Kind::Bopl(b) => b,
        _ => panic!("not a bopl"),
    }
}

fn pos(w: &World, id: i32) -> Vec2 {
    w.objects.get(bopl_id(w, id)).unwrap().tf.position
}

fn in_ability(w: &World, id: i32) -> bool {
    bopl(w, id).ctrl.is_in_ability
}

fn ability_obj(w: &World, id: i32, slot: usize) -> ObjId {
    bopl(w, id).ctrl.abilities[slot]
}

fn state(w: &World, id: i32, slot: usize) -> &AbilityState {
    match &w.objects.get(ability_obj(w, id, slot)).unwrap().kind {
        Kind::Ability(a) => &a.state,
        _ => panic!("not an ability"),
    }
}

fn cooldown(w: &World, id: i32, slot: usize) -> Fix {
    match &w.objects.get(ability_obj(w, id, slot)).unwrap().kind {
        Kind::Ability(a) => a.fw.cooldown,
        _ => panic!("not an ability"),
    }
}

fn f(x: f64) -> Fix {
    Fix::from_f64(x)
}

/// Puts a grounded player on a platform at the surface point above `x`.
fn place_on(w: &mut World, id: i32, plat: ObjId, x: Fix) {
    let s = Surface::of(w, plat).expect("platform");
    let r = bopl(w, id).physics.radius;
    let probe = Vec2::new(x, s.position.y + Fix::from_int(20));
    let t = s.param_from_position(probe, r);
    let p = s.position_from_param(t, r);
    let b = bopl_mut(w, id);
    b.physics.attached_ground = Some(plat);
    b.physics.is_grounded = true;
    b.physics.local_platform_pos = t;
    b.physics.grounded_speed = Fix::ZERO;
    b.body.self_imposed_velocity = Vec2::ZERO;
    b.body.external_velocity = Vec2::ZERO;
    let o = bopl_id(w, id);
    w.objects.get_mut(o).unwrap().tf.position = p;
}

fn approx(a: Fix, b: Fix, eps: f64) -> bool {
    (a - b).abs() <= f(eps)
}

// -- Dash -------------------------------------------------------------------------

/// Dash fires on the cast tick, holds 3 ticks, exits on the 5th tick 13
/// units along the aim with the exit speed 60; the cooldown is 4 s.
#[test]
fn dash_moves_and_cools_down() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(l, 6, &[setup(1, &[DASH, DASH, DASH])]);
    let start = pos(&r.w, 1);
    let cast = r.tick() + 1;
    let mut inside = Vec::new();
    let mut origin = Vec2::ZERO;
    for _ in 0..8 {
        r.step(&[input(RIGHT, BTN_ABILITY0)]);
        if in_ability(&r.w, 1) {
            inside.push(r.tick());
            // The dash body holds still at its (ungrounded) start.
            let a = ability_obj(&r.w, 1, 0);
            origin = r.w.objects.get(a).unwrap().tf.position;
        }
        if r.tick() == cast + 4 {
            assert!(!in_ability(&r.w, 1));
            assert!((origin - start).magnitude() < f(0.1));
            let p = pos(&r.w, 1);
            assert_eq!(p, origin + Vec2::new(Fix::from_int(13), Fix::ZERO));
            let v = bopl(&r.w, 1).body.self_imposed_velocity;
            assert_eq!(v, Vec2::new(Fix::from_int(60), Fix::ZERO));
            // (The record reports "facing right" false for a dash to the
            // right, but the fall state re-faces along the held stick.)
        }
    }
    assert_eq!(inside, vec![cast, cast + 1, cast + 2, cast + 3]);
    // Keep holding: the next dash only after the 4 s cooldown.
    r.hold(300, &[input(UP, BTN_ABILITY0)]);
    let dashes = r.sounds("dash");
    assert_eq!(dashes.len(), 2, "{dashes:?}");
    assert_eq!(dashes[1] - dashes[0], 245);
    assert!(r.w.fault.is_none());
}

/// A dash straight down onto the own platform ends on its surface.
#[test]
fn dash_down_onto_platform() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(l, 6, &[setup(1, &[DASH, DASH, DASH])]);
    let plat = bopl(&r.w, 1).physics.attached_ground.unwrap();
    r.hold(1, &[input(0, BTN_JUMP)]);
    r.idle(12);
    assert!(!bopl(&r.w, 1).physics.is_grounded);
    let x = pos(&r.w, 1).x;
    r.hold(1, &[input(DOWN, BTN_ABILITY0)]);
    r.idle(4);
    assert!(!in_ability(&r.w, 1));
    let b = bopl(&r.w, 1);
    assert!(b.physics.is_grounded, "re-grounds through its own physics");
    assert_eq!(b.physics.attached_ground, Some(plat));
    assert!(approx(pos(&r.w, 1).x, x, 0.1));
}

/// Dashing through a rolling (lethal) player kills the dasher; the roller
/// is credited.
#[test]
fn dash_through_rolling_player_dies() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(
        l,
        6,
        &[setup(1, &[DASH, DASH, DASH]), setup(2, &[ROLL, ROLL, ROLL])],
    );
    let plat = bopl(&r.w, 2).physics.attached_ground.unwrap();
    let x2 = pos(&r.w, 2).x;
    place_on(&mut r.w, 1, plat, x2 - Fix::from_int(6));
    r.idle(2);
    // P2 charges a roll for 70 ticks and releases.
    r.hold(70, &[input(0, 0), input(0, BTN_ABILITY0)]);
    r.step(&[input(0, 0), input(0, 0)]);
    let roll = ability_obj(&r.w, 2, 0);
    let lethal = match &r.w.objects.get(roll).unwrap().kind {
        Kind::Ability(a) => a.fw.kill_players_on_contact,
        _ => false,
    };
    assert!(lethal, "released roll kills on contact");
    r.step(&[input(RIGHT, BTN_ABILITY0)]);
    r.idle(2);
    let d = r.deaths();
    assert!(
        d.iter().any(|&(_, id, killer, _)| id == 1 && killer == 2),
        "{d:?}"
    );
}

// -- Roll ---------------------------------------------------------------------------

/// Charge, release, roll along the platform into another player: killed by
/// the roller. The roll ends after 2 s.
#[test]
fn roll_charges_releases_and_kills() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(
        l,
        6,
        &[setup(1, &[ROLL, ROLL, ROLL]), setup(2, &[DASH, DASH, DASH])],
    );
    let plat = bopl(&r.w, 1).physics.attached_ground.unwrap();
    let x1 = pos(&r.w, 1).x;
    place_on(&mut r.w, 2, plat, x1 + Fix::from_int(4));
    r.idle(2);
    let start = pos(&r.w, 1);
    r.hold(30, &[input(0, BTN_ABILITY0)]);
    assert!(in_ability(&r.w, 1));
    if let AbilityState::Roll(roll) = state(&r.w, 1, 0) {
        assert!(roll.is_charging);
    } else {
        panic!("roll state");
    }
    // Charging stays in place.
    let a = ability_obj(&r.w, 1, 0);
    let p = r.w.objects.get(a).unwrap().tf.position;
    assert!((p - start).magnitude() < f(1.0));
    r.hold(40, &[input(0, BTN_ABILITY0)]);
    r.hold(40, &[input(0, 0)]);
    let d = r.deaths();
    assert!(
        d.iter()
            .any(|&(_, id, killer, cause)| id == 2 && killer == 1 && cause == CauseOfDeath::Other),
        "{d:?}"
    );
    r.idle(140);
    assert!(!in_ability(&r.w, 1), "the roll ends after dashTime");
    assert!(r.w.fault.is_none());
}

/// Releasing a short charge on the ground does nothing (exit at once);
/// jumping while charging cancels with a jump.
#[test]
fn roll_early_release_and_jump_cancel() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(l, 6, &[setup(1, &[ROLL, ROLL, ROLL])]);
    r.hold(10, &[input(0, BTN_ABILITY0)]);
    assert!(in_ability(&r.w, 1));
    r.step(&[input(0, 0)]);
    assert!(!in_ability(&r.w, 1));
    assert!(bopl(&r.w, 1).physics.is_grounded);
    // Cooldown 3.5 s, then jump-cancel a charge.
    r.idle(230);
    r.hold(10, &[input(0, BTN_ABILITY0)]);
    assert!(in_ability(&r.w, 1));
    r.step(&[input(0, BTN_ABILITY0 | BTN_JUMP)]);
    assert!(!in_ability(&r.w, 1));
    assert!(bopl(&r.w, 1).body.self_imposed_velocity.y > Fix::from_int(10));
}

// -- Grappling hook and ropes ----------------------------------------------------------

fn rope_of(w: &World, id: i32) -> Option<&bopl_sim::physics::ropes::RopeBody> {
    let l = bopl(w, id).body.rope?;
    w.physics.ropes.get(l.rope as usize)
}

/// A hook fired at a platform anchors there; while attached the slot's
/// cooldown is 0 and holding the button reels in.
#[test]
fn hook_anchors_and_reels_in() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(l, 6, &[setup(1, &[HOOK, HOOK, HOOK])]);
    // Aim up-right at the high middle capsule.
    let aim = bopl_sim::input::angle_byte_from_vector(Vec2::new(f(8.2), f(3.6)).normalized());
    r.step(&[input(aim, BTN_ABILITY0)]);
    r.step(&[input(aim, 0)]);
    assert!(rope_of(&r.w, 1).is_some_and(|r| r.enabled), "roped");
    let mut arrived = None;
    for _ in 0..40 {
        r.step(&[input(aim, 0)]);
        if arrived.is_none() && rope_of(&r.w, 1).is_some_and(|r| r.hook_arrived()) {
            arrived = Some(r.tick());
        }
    }
    assert!(arrived.is_some(), "the hook arrives");
    assert_eq!(r.sounds("hookshotOnHit").len(), 1, "{:?}", r.events);
    assert_eq!(cooldown(&r.w, 1, 0), Fix::ZERO, "cooldown 0 while attached");
    let sep0 = rope_of(&r.w, 1).unwrap().separation;
    r.hold(20, &[input(aim, BTN_ABILITY0)]);
    let rope = rope_of(&r.w, 1).expect("still roped");
    assert!(rope.separation < sep0, "reeling in shortens the links");
    assert!(r.w.fault.is_none());
}

/// A hook that hits nothing falls after 0.8 s; the 3 s cooldown applies.
#[test]
fn hook_miss_falls_and_cools_down() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(l, 6, &[setup(1, &[HOOK, HOOK, HOOK])]);
    r.step(&[input(UP, BTN_ABILITY0)]);
    r.idle(1);
    let idx = bopl(&r.w, 1).body.rope.unwrap().rope as usize;
    r.idle(60);
    assert!(!r.w.physics.ropes.get(idx).unwrap().enabled, "hook fell");
    assert!(bopl(&r.w, 1).body.rope.is_none());
    assert_eq!(cooldown(&r.w, 1, 0), Fix::from_int(3));
    // Not castable again before 3 s.
    r.hold(100, &[input(UP, BTN_ABILITY0)]);
    assert_eq!(r.sounds("hookshotFire").len(), 1);
}

/// A player within 2.5 is roped instead: the target holds the rope, the
/// caster does not, and the slot cools down at once.
#[test]
fn hook_on_nearby_player() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(
        l,
        6,
        &[setup(1, &[HOOK, HOOK, HOOK]), setup(2, &[DASH, DASH, DASH])],
    );
    let plat = bopl(&r.w, 1).physics.attached_ground.unwrap();
    let x1 = pos(&r.w, 1).x;
    place_on(&mut r.w, 2, plat, x1 + Fix::TWO);
    r.idle(2);
    r.step(&[input(UP, BTN_ABILITY0)]);
    r.idle(1);
    assert!(bopl(&r.w, 1).body.rope.is_none(), "caster not roped");
    let l2 = bopl(&r.w, 2).body.rope.expect("target roped");
    assert!(!l2.top);
    assert_eq!(cooldown(&r.w, 1, 0), Fix::from_int(3));
}

/// Holding the button reels the rope in until the caster arrives: the rope
/// is removed and the slot cools down again.
#[test]
fn hook_reel_in_completes() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(l, 6, &[setup(1, &[HOOK, HOOK, HOOK])]);
    let aim = bopl_sim::input::angle_byte_from_vector(Vec2::new(f(8.2), f(3.6)).normalized());
    r.step(&[input(aim, BTN_ABILITY0)]);
    r.idle(25);
    let idx = bopl(&r.w, 1).body.rope.expect("roped").rope as usize;
    assert!(r.w.physics.ropes.get(idx).unwrap().hook_arrived());
    let mut done = None;
    for _ in 0..240 {
        r.step(&[input(aim, BTN_ABILITY0)]);
        if done.is_none() && !r.w.physics.ropes.get(idx).unwrap().enabled {
            done = Some(r.tick());
            break;
        }
    }
    assert!(done.is_some(), "reel-in completes");
    r.idle(2);
    assert!(!r.sounds("hookshotLetGo").is_empty());
    assert!(!r.sounds("reelInFire").is_empty());
    assert!(bopl(&r.w, 1).body.rope.is_none());
    assert_eq!(cooldown(&r.w, 1, 0), Fix::from_int(3));
    assert!(r.w.fault.is_none());
}

/// The rope owner lets go by jumping while airborne.
#[test]
fn hook_let_go_by_jumping() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(l, 6, &[setup(1, &[HOOK, HOOK, HOOK])]);
    let aim = bopl_sim::input::angle_byte_from_vector(Vec2::new(f(8.2), f(3.6)).normalized());
    r.step(&[input(aim, BTN_ABILITY0)]);
    r.idle(30);
    r.step(&[input(0, BTN_JUMP)]);
    r.idle(10);
    assert!(!bopl(&r.w, 1).physics.is_grounded);
    assert!(bopl(&r.w, 1).body.rope.is_some());
    r.idle(1);
    r.step(&[input(0, BTN_JUMP)]);
    assert!(bopl(&r.w, 1).body.rope.is_none(), "let go");
    assert!(!r.sounds("hookshotLetGo").is_empty());
}

/// Dash keeps the rope and pushes it on exit.
#[test]
fn dash_keeps_rope() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(l, 6, &[setup(1, &[HOOK, DASH, DASH])]);
    let aim = bopl_sim::input::angle_byte_from_vector(Vec2::new(f(8.2), f(3.6)).normalized());
    r.step(&[input(aim, BTN_ABILITY0)]);
    r.idle(30);
    let link = bopl(&r.w, 1).body.rope.expect("roped");
    r.step(&[input(RIGHT, BTN_ABILITY1)]);
    assert!(in_ability(&r.w, 1));
    r.idle(4);
    assert!(!in_ability(&r.w, 1));
    assert_eq!(bopl(&r.w, 1).body.rope, Some(link), "rope handed back");
}

// -- Teleport -------------------------------------------------------------------------

/// First use places a marker without starting the cooldown; the second use
/// swaps the caster to the marker and a player near the marker to the
/// caster's spot; both are hidden for 0.6 s.
#[test]
fn teleport_marker_and_swap() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(
        l,
        6,
        &[
            setup(1, &[TELEPORT, TELEPORT, TELEPORT]),
            setup(2, &[DASH, DASH, DASH]),
        ],
    );
    let p1 = pos(&r.w, 1);
    r.step(&[input(0, BTN_ABILITY0)]);
    assert_eq!(r.sounds("swapIndicator").len(), 1);
    let marker = p1 + Vec2::new(Fix::ZERO, f(1.5));
    // Walk left a bit, then put P2 under the marker.
    r.hold(20, &[input(LEFT, 0)]);
    r.idle(10);
    let plat = bopl(&r.w, 1).physics.attached_ground.unwrap();
    place_on(&mut r.w, 2, plat, p1.x);
    r.idle(1);
    let from = pos(&r.w, 1);
    let p2 = pos(&r.w, 2);
    r.step(&[input(0, BTN_ABILITY0)]);
    assert_eq!(r.sounds("swap").len(), 1, "no cooldown after placing");
    let v = marker - from;
    let b1 = bopl_id(&r.w, 1);
    let b2 = bopl_id(&r.w, 2);
    assert!(!r.w.objects.get(b1).unwrap().active);
    assert!(!r.w.objects.get(b2).unwrap().active);
    // (Grounded players are ungrounded first: a 0.05 step along the normal.)
    let d1 = r.w.objects.get(b1).unwrap().tf.position - (from + v);
    let d2 = r.w.objects.get(b2).unwrap().tf.position - (p2 - v);
    assert!(d1.magnitude() <= f(0.051), "{d1:?}");
    assert!(d2.magnitude() <= f(0.051), "{d2:?}");
    r.idle(40);
    assert!(r.w.objects.get(b1).unwrap().active, "reappears");
    assert!(r.w.objects.get(b2).unwrap().active, "reappears");
    // Now the normal cooldown: no new marker for 3 s.
    r.hold(60, &[input(0, BTN_ABILITY0)]);
    assert_eq!(r.sounds("swapIndicator").len(), 1);
    assert!(r.w.fault.is_none());
}

// -- Engine ---------------------------------------------------------------------------

/// The engine is planted after the cast animation and pushes the platform
/// away from its side; the ability is ground-only.
#[test]
fn engine_plants_and_pushes_platform() {
    let Some(l) = loaded() else {
        return;
    };
    let mut control = Run::new(l, 6, &[setup(1, &[ENGINE, ENGINE, ENGINE])]);
    let mut r = Run::new(l, 6, &[setup(1, &[ENGINE, ENGINE, ENGINE])]);
    let plat = bopl(&r.w, 1).physics.attached_ground.unwrap();
    r.step(&[input(0, BTN_ABILITY0)]);
    control.step(&[]);
    assert!(in_ability(&r.w, 1));
    for _ in 0..60 {
        r.step(&[]);
        control.step(&[]);
    }
    assert!(!in_ability(&r.w, 1));
    assert!(bopl(&r.w, 1).physics.is_grounded);
    assert_eq!(r.sounds("plantObjectOnPlatform").len(), 1);
    assert_eq!(r.sounds("startEngine").len(), 1);
    let engines: Vec<ObjId> =
        r.w.objects
            .ids()
            .filter(|&o| {
                matches!(&r.w.objects.get(o).unwrap().kind,
                Kind::Behaviour(b) if matches!(**b, bopl_sim::abilities::Behaviour::Engine(_)))
            })
            .collect();
    assert_eq!(engines.len(), 1);
    assert_eq!(r.w.objects.get(engines[0]).unwrap().parent, Some(plat));
    r.idle(120);
    control.idle(120);
    let y = r.w.objects.get(plat).unwrap().tf.position.y;
    let yc = control.w.objects.get(plat).unwrap().tf.position.y;
    assert!(y < yc - f(0.5), "pushed down: {y:?} vs {yc:?}");
    // Airborne: not castable.
    r.idle(200);
    r.hold(1, &[input(0, BTN_JUMP)]);
    r.idle(5);
    assert!(!bopl(&r.w, 1).physics.is_grounded);
    r.hold(5, &[input(0, BTN_ABILITY0)]);
    assert!(!in_ability(&r.w, 1));
    assert!(r.w.fault.is_none());
}

/// The ring buffer of planted objects keeps 4: the fifth destroys the oldest.
#[test]
fn engine_ring_buffer_evicts_oldest() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(l, 6, &[setup(1, &[ENGINE, ENGINE, ENGINE])]);
    let mut ring = bopl_sim::abilities::smoke::RingBuffer::new(4);
    let objs: Vec<ObjId> = (0..6).map(|_| w.objects.create(Kind::Static, 0)).collect();
    for &o in &objs[..5] {
        ring.add(&mut w, o);
    }
    assert!(w.objects.get(objs[0]).unwrap().destroyed);
    assert!(
        objs[1..5]
            .iter()
            .all(|o| !w.objects.get(*o).unwrap().destroyed)
    );
    // A freed slot is reused before anything live is evicted.
    w.objects.free(objs[2]);
    ring.add(&mut w, objs[5]);
    assert!(
        objs[3..6]
            .iter()
            .all(|o| !w.objects.get(*o).unwrap().destroyed)
    );
    assert!(!w.objects.get(objs[1]).unwrap().destroyed);
}

// -- Drill -----------------------------------------------------------------------------

/// The drill burrows through its platform; its tip boxes kill a player below
/// (credited to the driller); it never kills its owner.
#[test]
fn drill_through_platform_kills_below() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(
        l,
        6,
        &[
            setup(1, &[DRILL, DRILL, DRILL]),
            setup(2, &[DASH, DASH, DASH]),
        ],
    );
    let capsule = bopl(&r.w, 1).physics.attached_ground.unwrap();
    let ground = r.w.platforms[0];
    let x1 = pos(&r.w, 1).x;
    place_on(&mut r.w, 2, ground, x1);
    r.idle(2);
    let mut under = false;
    let mut below = false;
    let cap_y = r.w.objects.get(capsule).unwrap().tf.position.y;
    for _ in 0..150 {
        r.step(&[input(0, BTN_ABILITY0)]);
        if let AbilityState::Drill(d) = state(&r.w, 1, 0) {
            under |= d.is_underground;
        }
        let a = ability_obj(&r.w, 1, 0);
        if r.w.objects.get(a).unwrap().tf.position.y < cap_y - f(3.0) {
            below = true;
        }
        if !r.deaths().is_empty() {
            break;
        }
    }
    assert!(under, "went underground");
    assert!(below, "came out below the platform");
    let d = r.deaths();
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!((d[0].1, d[0].2), (2, 1));
    assert!(r.w.fault.is_none());
}

// -- Meteor ------------------------------------------------------------------------------

/// Jump, hold, smash: the meteor lands on its platform, pushes it down and
/// kills a player in its path.
#[test]
fn meteor_smash_kills_and_pushes() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(
        l,
        6,
        &[
            setup(1, &[METEOR, METEOR, METEOR]),
            setup(2, &[DASH, DASH, DASH]),
        ],
    );
    let plat = bopl(&r.w, 1).physics.attached_ground.unwrap();
    let x1 = pos(&r.w, 1).x;
    let y0 = pos(&r.w, 1).y;
    r.hold(40, &[input(0, BTN_ABILITY0)]);
    assert!(in_ability(&r.w, 1));
    let a = ability_obj(&r.w, 1, 0);
    assert!(r.w.objects.get(a).unwrap().tf.position.y > y0 + Fix::from_int(3));
    place_on(&mut r.w, 2, plat, x1);
    r.step(&[input(0, BTN_ABILITY0)]);
    r.step(&[input(0, 0)]);
    r.idle(3);
    assert!(!in_ability(&r.w, 1));
    let b = bopl(&r.w, 1);
    assert!(b.physics.is_grounded);
    assert_eq!(b.physics.attached_ground, Some(plat));
    let d = r.deaths();
    assert!(
        d.iter().any(|&(_, id, killer, _)| id == 2 && killer == 1),
        "{d:?}"
    );
    let v = r.w.physics.body(plat).unwrap().velocity;
    assert!(v.y < Fix::ZERO, "platform pushed down: {v:?}");
    assert_eq!(r.sounds("meteor_dash").len(), 1);
    assert!(r.w.fault.is_none());
}

/// A meteor over the water hits the water box: the player comes back below
/// the water line and dies by the environment.
#[test]
fn meteor_over_water_dies() {
    let Some(l) = loaded() else {
        return;
    };
    let mut r = Run::new(
        l,
        6,
        &[
            setup(1, &[METEOR, METEOR, METEOR]),
            setup(2, &[DASH, DASH, DASH]),
        ],
    );
    {
        let b = bopl_mut(&mut r.w, 1);
        b.physics.is_grounded = false;
        b.body.self_imposed_velocity = Vec2::ZERO;
    }
    let o = bopl_id(&r.w, 1);
    r.w.objects.get_mut(o).unwrap().tf.position = Vec2::new(f(21.8), f(4.0));
    r.hold(12, &[input(0, BTN_ABILITY0)]);
    assert!(in_ability(&r.w, 1));
    r.step(&[input(0, 0)]);
    r.idle(10);
    let d = r.deaths();
    assert!(
        d.iter()
            .any(|&(_, id, killer, _)| id == 1 && killer == 1000),
        "{d:?}"
    );
    assert!(r.sounds("meteor_dash").is_empty());
    assert!(!r.sounds("dash").is_empty());
}

// -- Determinism ---------------------------------------------------------------------------

/// A deterministic pseudo-random input script per player.
fn inputs(n: usize, t: u64, seed: u64) -> Vec<PlayerInput> {
    (0..n)
        .map(|i| {
            let mut x = seed
                ^ (t / 7).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ (i as u64 + 1).wrapping_mul(0xD1B5_4A32_D192_ED03);
            x ^= x >> 33;
            x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
            x ^= x >> 29;
            let angle = if x & 3 == 0 { 0 } else { (x >> 8) as u8 };
            let mut b = 0;
            if (x >> 20) & 15 == 0 {
                b |= BTN_JUMP;
            }
            if (x >> 24) & 7 == 1 {
                b |= BTN_ABILITY0;
            }
            if (x >> 30) & 7 == 2 {
                b |= BTN_ABILITY1;
            }
            if (x >> 36) & 7 == 3 {
                b |= BTN_ABILITY2;
            }
            PlayerInput::new(angle, b)
        })
        .collect()
}

fn twelve_g3() -> Vec<PlayerSetup> {
    let set = [DASH, ENGINE, ROLL, METEOR, TELEPORT, DRILL, HOOK];
    (0..12)
        .map(|i| PlayerSetup {
            id: i + 1,
            team: i % 6,
            color: i,
            abilities: vec![
                set[i as usize % 7],
                set[(i as usize + 3) % 7],
                set[(i as usize + 5) % 7],
            ],
            uses_keyboard_and_mouse: i == 3,
        })
        .collect()
}

/// 12 players using only G3 abilities: two runs agree tick for tick over
/// several levels; a rollback (clone, mispredict, restore) equals a straight
/// run.
#[test]
fn determinism_and_rollback_twelve_players() {
    let Some(l) = loaded() else {
        return;
    };
    for &level in &[6usize, 17, 22, 40, 126] {
        let lv = bopl_content::load_level(l, level).unwrap();
        let mk = || {
            let mut w = World::new(
                Arc::clone(&l.content),
                &twelve_g3(),
                MatchSettings {
                    number_of_abilities: 3,
                    seed: 5,
                },
            );
            bopl_sim::session::load_level(&mut w, lv.clone());
            w
        };
        let (mut a, mut b) = (mk(), mk());
        let mut ropes = 0;
        let mut casts = 0;
        for t in 0..4000u64 {
            let inp = inputs(12, t, level as u64);
            a.step(&inp);
            b.step(&inp);
            casts += a
                .events
                .iter()
                .filter(|e| {
                    matches!(
                        e.cue,
                        Cue::Sound(
                            "dash"
                                | "rollCharge"
                                | "hookshotFire"
                                | "swapIndicator"
                                | "drill_start"
                                | "meteor_jump"
                                | "plantObjectOnPlatform"
                        )
                    )
                })
                .count();
            ropes = ropes.max(a.physics.ropes.bodies.iter().filter(|r| r.enabled).count());
            if t % 50 == 0 {
                assert_eq!(a.state_hash(), b.state_hash(), "level {level} tick {t}");
            }
            // Next round on the same level (rope counter and pools persist
            // across loads like in the original).
            if a.round.winner_screen && a.time.sim_time_since_level_loaded > Fix::from_int(12) {
                bopl_sim::session::load_level(&mut a, lv.clone());
                bopl_sim::session::load_level(&mut b, lv.clone());
            }
        }
        assert_eq!(a.state_hash(), b.state_hash());
        eprintln!(
            "level {level}: {casts} casts, max {ropes} ropes, deaths {}, fault {:?}",
            a.players.iter().map(|p| p.deaths).sum::<i32>(),
            a.fault
        );
        assert!(casts > 20, "level {level}: abilities used ({casts})");
        // Rollback from the middle of the run.
        let mut straight = mk();
        for t in 0..1500u64 {
            straight.step(&inputs(12, t, level as u64));
        }
        let mut rb = mk();
        for t in 0..900u64 {
            rb.step(&inputs(12, t, level as u64));
        }
        for k in 0..6u64 {
            let base = 900 + k * 100;
            let snap = rb.clone();
            for t in base..base + 9 {
                rb.step(&inputs(12, t, 777 + k));
            }
            rb = snap;
            for t in base..base + 100 {
                rb.step(&inputs(12, t, level as u64));
            }
        }
        assert_eq!(
            rb.state_hash(),
            straight.state_hash(),
            "rollback level {level}"
        );
        assert_eq!(rb.view(), straight.view());
    }
}

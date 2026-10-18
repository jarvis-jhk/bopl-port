//! Cross-module interactions wired at integration (black holes, the shared
//! shockwave, BoplBody magnet holds, Duplicator clones) on Level1 of the
//! real import (skipped without one).

use std::sync::Arc;

use bopl_content::Loaded;
use bopl_sim::abilities::{Behaviour, black_hole, duplicator, explosion};
use bopl_sim::input::PlayerInput;
use bopl_sim::object::ObjId;
use bopl_sim::player::PlayerSetup;
use bopl_sim::world::{Kind, MatchSettings, World};
use bopl_sim::{Fix, Vec2};

fn loaded() -> Option<Loaded> {
    let dir = bopl_content::default_import_dir();
    if !dir.join("index.json").exists() {
        eprintln!("no import at {}; skipping", dir.display());
        return None;
    }
    Some(bopl_content::load(dir).expect("import loads"))
}

fn world(l: &Loaded) -> World {
    let lv = bopl_content::load_level(l, 6).unwrap();
    let setups = [PlayerSetup {
        id: 1,
        team: 0,
        color: 0,
        abilities: vec![2, 2, 2],
        uses_keyboard_and_mouse: false,
    }];
    let mut w = World::new(
        Arc::clone(&l.content),
        &setups,
        MatchSettings {
            number_of_abilities: 3,
            seed: 3,
        },
    );
    bopl_sim::session::load_level(&mut w, lv);
    w
}

fn idle(w: &mut World, n: usize) {
    for _ in 0..n {
        w.step(&[PlayerInput::default()]);
    }
}

fn prefab(l: &Loaded, name: &str) -> usize {
    l.content
        .prefabs
        .iter()
        .position(|p| p.name == name)
        .unwrap_or_else(|| panic!("prefab {name}"))
}

fn hole_velocity(w: &World, h: ObjId) -> Vec2 {
    match w.objects.get(h).map(|o| &o.kind) {
        Some(Kind::Behaviour(b)) => match b.as_ref() {
            Behaviour::BlackHole(black_hole::Spawned::Hole(x)) => x.velocity,
            _ => panic!("not a hole"),
        },
        _ => panic!("not a hole"),
    }
}

fn f(x: f64) -> Fix {
    Fix((x * 4294967296.0) as i64)
}

/// Growth, the inert magnet hold and Duplicator clones of a black hole.
#[test]
fn black_hole_grow_hold_and_clone() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(&l);
    idle(&mut w, 30);
    let at = Vec2::new(Fix::ZERO, f(20.0));
    let h = black_hole::spawn_hole(&mut w, prefab(&l, "BlackHole2"), at, 1);
    idle(&mut w, 1);
    let m0 = black_hole::mass(&w, h).unwrap();
    assert_eq!(m0, Fix::from_int(15));
    // A growth-ray hit: Grow(0.02, 0) → + 1/0.02 + 1 (inverse-mass path).
    black_hole::grow(&mut w, h, Fix(85_899_346), Fix::ZERO);
    let m1 = black_hole::mass(&w, h).unwrap();
    assert!(m1 > Fix::from_int(60), "grown mass {m1:?}");
    // Magnet pickup: the holder is recorded, `Hold` clears it again.
    let fake = ObjId {
        idx: 0,
        generation: 1,
    };
    assert_eq!(black_hole::take_magnet_holder(&mut w, h, fake), None);
    black_hole::hold(&mut w, h, 3);
    assert_eq!(black_hole::take_magnet_holder(&mut w, h, fake), None);
    // Duplicator clone: mass reset (startingMass), owner 1000, then the
    // black-hole branch sets the original's mass.
    let c = duplicator::duplicate_object(&mut w, at + Vec2::new(f(8.0), Fix::ZERO), h, 480, None)
        .expect("clone");
    match w.objects.get(c).map(|o| &o.kind) {
        Some(Kind::Behaviour(b)) => match b.as_ref() {
            Behaviour::BlackHole(black_hole::Spawned::Hole(x)) => {
                assert_eq!(x.mass, Fix::from_int(15));
                assert_eq!(x.owner, 1000);
                assert_eq!(x.velocity, Vec2::ZERO);
            }
            _ => panic!("clone kind"),
        },
        _ => panic!("clone kind"),
    }
    black_hole::set_mass(&mut w, c, m1);
    idle(&mut w, 1);
    assert_eq!(black_hole::mass(&w, c), Some(m1));
}

/// The shared Shockwave (a gust's `AirWave_Shockwave`) pushes black holes
/// through `BlackHole.AddForce`, away from its centre.
#[test]
fn shockwave_pushes_black_hole() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(&l);
    idle(&mut w, 30);
    let at = Vec2::new(Fix::ZERO, f(25.0));
    let h = black_hole::spawn_hole(&mut w, prefab(&l, "BlackHole2"), at, 1);
    idle(&mut w, 1);
    assert_eq!(hole_velocity(&w, h), Vec2::ZERO);
    let s = explosion::spawn_shockwave(
        &mut w,
        prefab(&l, "AirWave_Shockwave"),
        at + Vec2::new(f(3.0), Fix::ZERO),
        1,
        Fix::ONE,
        Fix::ONE,
        &[],
    );
    assert!(w.objects.exists(s));
    idle(&mut w, 2);
    let v = hole_velocity(&w, h);
    assert!(v.x < Fix::ZERO, "pushed away from the gust: {v:?}");
    // The gust lasts its curve (0.25 s) and then destroys itself.
    idle(&mut w, 20);
    assert!(!w.objects.exists(s));
}

/// `BoplBody.Hold(3)`: held for the next two BoplBody updates, dropped on
/// the third (framesUntilDrop reaches exactly 0).
#[test]
fn bopl_body_hold_counts_down() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(&l);
    idle(&mut w, 30);
    let p = bopl_sim::abilities::pickup::spawn(
        &mut w,
        Vec2::new(Fix::ZERO, f(30.0)),
        Some(3),
        Vec2::UP,
    );
    idle(&mut w, 2);
    let holder = ObjId {
        idx: 0,
        generation: 1,
    };
    w.objects.get_mut(p).unwrap().bb.hold(3, holder);
    let held = |w: &World| bopl_sim::abilities::ability_body::is_held(w, p);
    assert!(held(&w));
    idle(&mut w, 1);
    assert!(held(&w));
    assert_eq!(
        bopl_sim::abilities::ability_body::holder_of(&w, p),
        Some(holder)
    );
    idle(&mut w, 1);
    assert!(held(&w));
    idle(&mut w, 1);
    assert!(!held(&w));
    assert_eq!(w.objects.get(p).unwrap().bb.holder, None);
}

/// `BoplBody.prevScale` starts at 1: a platform rescaled to 2 gets its
/// inverse mass halved on its next BoplBody update, once (physics.md §2.4).
#[test]
fn rescaled_platform_inverse_mass_follows_once() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(&l);
    idle(&mut w, 30);
    let p = w.platforms[0];
    let im0 = w.physics.body(p).unwrap().inverse_mass;
    w.set_collider_scale(p, Fix::TWO);
    idle(&mut w, 1);
    let im1 = w.physics.body(p).unwrap().inverse_mass;
    assert_eq!(im1, im0 * (Fix::ONE / Fix::TWO));
    idle(&mut w, 2);
    assert_eq!(w.physics.body(p).unwrap().inverse_mass, im1);
}

fn ability(l: &Loaded, name: &str) -> usize {
    l.content
        .abilities
        .iter()
        .position(|a| a.name == name)
        .unwrap()
}

fn solo(l: &Loaded, abilities: Vec<usize>) -> World {
    let lv = bopl_content::load_level(l, 6).unwrap();
    let setups = [PlayerSetup {
        id: 1,
        team: 0,
        color: 0,
        abilities,
        uses_keyboard_and_mouse: false,
    }];
    let mut w = World::new(
        Arc::clone(&l.content),
        &setups,
        MatchSettings {
            number_of_abilities: 3,
            seed: 3,
        },
    );
    bopl_sim::session::load_level(&mut w, lv);
    // Spawned, grounded, every slot ready.
    idle(&mut w, 600);
    w
}

fn bopl_of(w: &World) -> &bopl_sim::bopl::Bopl {
    match &w
        .objects
        .get(w.player(1).unwrap().bopl.unwrap())
        .unwrap()
        .kind
    {
        Kind::Bopl(b) => b,
        _ => panic!("no bopl"),
    }
}

fn in_ability(w: &World) -> bool {
    bopl_of(w).ctrl.is_in_ability
}

/// The magnet gun holds a pickup through its own BoplBody (`Hold`, the
/// counter runs at the pickup's BoplBody update) and throws it.
#[test]
fn magnet_holds_and_throws_item() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = solo(&l, vec![ability(&l, "Magnet gun"); 3]);
    let me =
        bopl_sim::abilities::ability_body::AbilityBody::pos(&w, w.player(1).unwrap().bopl.unwrap());
    let p = bopl_sim::abilities::pickup::spawn(
        &mut w,
        me + Vec2::new(f(5.0), f(1.5)),
        Some(3),
        Vec2::UP,
    );
    let mut held_ticks = 0;
    for _ in 0..80 {
        w.step(&[PlayerInput::new(1, bopl_sim::input::BTN_ABILITY0)]);
        if bopl_sim::abilities::ability_body::is_held(&w, p) {
            held_ticks += 1;
        }
    }
    assert!(in_ability(&w));
    assert!(held_ticks > 10, "the pickup was held ({held_ticks} ticks)");
    assert!(bopl_sim::abilities::ability_body::holder_of(&w, p).is_some());
    // Release: thrown along +x.
    w.step(&[PlayerInput::new(1, 0)]);
    idle(&mut w, 2);
    let v = w.physics.body(p).unwrap().velocity;
    assert!(v.x > Fix::from_int(20), "thrown: {v:?}");
    idle(&mut w, 5);
    assert!(!bopl_sim::abilities::ability_body::is_held(&w, p));
    assert!(w.fault.is_none());
}

/// Throw picks up a rocket engine planted next to the thrower: the engine
/// travels on the boulder (re-initialised there) and restarts.
#[test]
fn throw_carries_planted_engine() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = solo(
        &l,
        vec![
            ability(&l, "Engine"),
            ability(&l, "Throw"),
            ability(&l, "Dash"),
        ],
    );
    w.step(&[PlayerInput::new(0, bopl_sim::input::BTN_ABILITY0)]);
    idle(&mut w, 70);
    assert!(!in_ability(&w));
    let engine = w
        .objects
        .ids()
        .find(|&o| {
            matches!(&w.objects.get(o).unwrap().kind,
                Kind::Behaviour(b) if matches!(**b, Behaviour::Engine(_)))
        })
        .expect("planted engine");
    let ground = w.objects.get(engine).unwrap().parent.unwrap();
    // Make the Throw slot ready, then grab.
    let b = w.player(1).unwrap().bopl.unwrap();
    if let Some(Kind::Bopl(bp)) = w.objects.get_mut(b).map(|o| &mut o.kind) {
        bp.ctrl.cooldown_timers[1] = Fix::from_int(1000);
    }
    for _ in 0..91 {
        w.step(&[PlayerInput::new(0, bopl_sim::input::BTN_ABILITY1)]);
    }
    assert!(in_ability(&w));
    let boulder = w
        .objects
        .ids()
        .find(|&id| matches!(&w.objects.get(id).unwrap().kind, Kind::Platform(p) if p.boulder.is_some()))
        .expect("held boulder");
    let parent = w.objects.get(engine).map(|o| o.parent);
    assert_eq!(
        parent,
        Some(Some(boulder)),
        "engine on the boulder (was on {ground:?})"
    );
    match w.objects.get(engine).map(|o| &o.kind) {
        Some(Kind::Behaviour(bh)) => match bh.as_ref() {
            Behaviour::Engine(bopl_sim::abilities::engine::Spawned::RocketEngine(e)) => {
                assert_eq!(e.platform, boulder);
            }
            _ => panic!(),
        },
        _ => panic!(),
    }
    assert!(w.fault.is_none());
}

//! The tutorial scene in the simulation (docs/spec/tutorial.md; skipped
//! without an import): loading, the arrow / pickup / dummy chain, unlocked
//! cooldowns, the dummies' grounds, the end by dash and by select,
//! permanent revive, determinism.

use std::sync::Arc;

use bopl_content::Loaded;
use bopl_sim::Fix;
use bopl_sim::World;
use bopl_sim::input::{BTN_ABILITY2, BTN_SELECT, PlayerInput};
use bopl_sim::object::ObjId;
use bopl_sim::physics::shapes::Circle;
use bopl_sim::physics::{Contact, ShapeSnap};
use bopl_sim::player::PlayerSetup;
use bopl_sim::tutorial::Part;
use bopl_sim::vec2::Vec2;
use bopl_sim::view::Cue;
use bopl_sim::world::{Kind, MatchSettings};

fn loaded() -> Option<Loaded> {
    let dir = bopl_content::default_import_dir();
    if !dir.join("index.json").exists() {
        eprintln!("no import at {}; skipping", dir.display());
        return None;
    }
    Some(bopl_content::load(dir).expect("import loads"))
}

fn tutorial(l: &Loaded) -> World {
    let setup = PlayerSetup {
        id: 1,
        team: 1,
        color: 0,
        abilities: vec![4, 3, 2],
        uses_keyboard_and_mouse: true,
    };
    let mut w = World::new(Arc::clone(&l.content), &[setup], MatchSettings::default());
    let lv = bopl_content::load_level(l, 5).expect("tutorial level");
    bopl_sim::session::load_level(&mut w, lv);
    w
}

fn step(w: &mut World, n: usize, input: PlayerInput) -> Vec<Cue> {
    let mut cues = Vec::new();
    for _ in 0..n {
        w.step(&[input]);
        cues.extend(w.events.iter().map(|e| e.cue.clone()));
    }
    cues
}

fn player_pos(w: &World) -> Vec2 {
    let b = w.players[0].bopl.expect("spawned");
    w.objects.get(b).expect("bopl").tf.position
}

/// Moves a trigger object onto the player (arrows sit still; a pickup's
/// hover start moves with it).
fn bring(w: &mut World, id: ObjId) {
    let p = player_pos(w);
    let o = w.objects.get_mut(id).expect("object");
    o.tf.position = p;
    o.tf.offset = Vec2::ZERO;
    if let Kind::Tutorial(t) = &mut o.kind
        && let Part::Pickup { start, .. } = t.as_mut()
    {
        *start = p;
    }
}

fn hit(w: &mut World, dummy: ObjId, layer: i32) {
    let c = Contact {
        me: dummy,
        other: dummy,
        other_hier: 0,
        my_hier: 0,
        collider: ShapeSnap::Circle(Circle::default()),
        layer,
        impact_velocity: Vec2::ZERO,
        contact_point: Vec2::ZERO,
        normal: Vec2::UP,
        penetration: Fix::ZERO,
    };
    w.dispatch_contact(c);
}

fn cooldown(w: &World, slot: usize) -> Fix {
    let b = w.players[0].bopl.unwrap();
    let Some(Kind::Bopl(b)) = w.objects.get(b).map(|o| &o.kind) else {
        panic!("no bopl")
    };
    bopl_sim::abilities::cooldown(w, b.ctrl.abilities[slot])
}

#[test]
fn tutorial_chain() {
    let Some(l) = loaded() else { return };
    let mut w = tutorial(&l);
    let st = w.tutorial.as_ref().expect("tutorial state");
    assert_eq!(st.arrows.len(), 4);
    assert_eq!(st.pickups.len(), 3);
    assert_eq!(st.dummies.len(), 2);
    assert_eq!(st.grounds.len(), 2);
    assert_eq!(st.def.abilities, [4, 3, 2], "Bow, Grenade, Dash");
    let (arrows, pickups, dummies) = (st.arrows.clone(), st.pickups.clone(), st.dummies.clone());
    let cues = step(&mut w, 120, PlayerInput::default());
    assert!(cues.contains(&Cue::Sound("go")));
    assert!(w.players[0].bopl.is_some());
    // Locked abilities.
    assert!(cooldown(&w, 0) > Fix::from_int(1000));
    let active = |w: &World, id: ObjId| w.objects.get(id).is_some_and(|o| o.active && !o.destroyed);
    assert!(active(&w, arrows[0]) && !active(&w, arrows[1]) && !active(&w, pickups[0]));
    // The four arrows, one after the other.
    for (k, &a) in arrows.iter().enumerate() {
        if k > 0 {
            step(&mut w, 1, PlayerInput::default());
        }
        assert!(active(&w, a) || k == 3, "arrow {k} shown");
        if k == 3 && !active(&w, a) {
            // Already touched on appearing (it sits on the spawn spot).
            assert_eq!(w.tutorial.as_ref().unwrap().absorbed, 4);
            break;
        }
        // (Move the player away from the next arrow's spot first: arrow 3
        // sits where the player spawned.)
        bring(&mut w, a);
        let cues = step(&mut w, 1, PlayerInput::default());
        assert!(cues.contains(&Cue::Sound("accept")), "arrow {k} sound");
        assert_eq!(w.tutorial.as_ref().unwrap().absorbed, k as i32 + 1);
    }
    step(&mut w, 2, PlayerInput::default());
    assert!(active(&w, pickups[0]), "bow pickup after four arrows");
    assert_eq!(
        w.tutorial.as_ref().unwrap().movement_hint(),
        Some(4),
        "SPACE"
    );
    // Bow: unlock slot 0, the right ground descends.
    let ground_platform = |w: &World, g: usize| {
        let def = &w.tutorial.as_ref().unwrap().def;
        w.objects
            .get(w.platforms[def.grounds[g].platform])
            .unwrap()
            .tf
            .position
    };
    let y0 = ground_platform(&w, 0).y;
    bring(&mut w, pickups[0]);
    let cues = step(&mut w, 2, PlayerInput::default());
    assert!(cues.contains(&Cue::Sound("abilityPickup")));
    let st = w.tutorial.as_ref().unwrap();
    assert!(st.unlocked[0] && st.hints.hold_to_shoot && !st.hints.device_indicator);
    assert_eq!(cooldown(&w, 0), Fix::ONE);
    step(&mut w, 120, PlayerInput::default());
    let y1 = ground_platform(&w, 0).y;
    assert!(
        y1 < y0 - Fix::from_int(10),
        "the ground came down: {y0:?} -> {y1:?}"
    );
    // The dummy on that ground rides along.
    let on_ground0 = dummies
        .iter()
        .copied()
        .zip(w.tutorial.as_ref().unwrap().def.dummies.iter())
        .find(|(_, d)| d.ground == 0)
        .map(|(id, _)| id)
        .unwrap();
    let dy = w.objects.get(on_ground0).unwrap().tf.position.y;
    assert!(dy < Fix::from_int(40), "dummy came down: {dy:?}");
    // Hit the right dummy (projectile): the grenade pickup appears.
    let grenade_dummy = dummies
        .iter()
        .copied()
        .zip(w.tutorial.as_ref().unwrap().def.dummies.iter())
        .find(|(_, d)| d.unlock_grenade)
        .map(|(id, _)| id)
        .expect("grenade dummy");
    hit(&mut w, grenade_dummy, 11); // wall: ignored
    assert!(!active(&w, pickups[1]));
    hit(&mut w, grenade_dummy, 9);
    assert!(active(&w, pickups[1]));
    assert_eq!(w.tutorial.as_ref().unwrap().absorbed, 4);
    assert!(
        w.events.iter().any(|e| matches!(e.cue, Cue::Effect { .. })),
        "death effect"
    );
    step(&mut w, 2, PlayerInput::default());
    assert!(!w.objects.alive(grenade_dummy), "dummy destroyed");
    // Grenade, then the left dummy (explosion), then dash.
    bring(&mut w, pickups[1]);
    step(&mut w, 2, PlayerInput::default());
    assert!(w.tutorial.as_ref().unwrap().unlocked[1]);
    assert_eq!(cooldown(&w, 1), Fix::ONE);
    let dash_dummy = dummies
        .iter()
        .copied()
        .find(|&d| d != grenade_dummy)
        .unwrap();
    hit(&mut w, dash_dummy, 18);
    assert!(active(&w, pickups[2]));
    bring(&mut w, pickups[2]);
    step(&mut w, 2, PlayerInput::default());
    let st = w.tutorial.as_ref().unwrap();
    assert!(st.has_dash && st.hints.click_to_dash);
    assert!(!st.done);
    let dash = PlayerInput::new(1, BTN_ABILITY2);
    step(&mut w, 1, dash);
    assert!(w.tutorial.as_ref().unwrap().dashed);
    let cues = step(&mut w, 70, PlayerInput::default());
    assert!(cues.contains(&Cue::Sound("accept2")), "jingle");
    assert!(
        w.tutorial.as_ref().unwrap().done,
        "back to the menu after 1 s"
    );
}

#[test]
fn select_exits_and_death_revives() {
    let Some(l) = loaded() else { return };
    let mut w = tutorial(&l);
    step(&mut w, 120, PlayerInput::default());
    // Jump off the capsule to the left into the water: permanent revive.
    let mut died = false;
    for t in 0..900 {
        let input = match t % 120 {
            0..=40 => PlayerInput::new(128, 0),
            41..=50 => PlayerInput::new(128, bopl_sim::input::BTN_JUMP),
            _ => PlayerInput::new(128, 0),
        };
        let cues = step(&mut w, 1, input);
        if cues.iter().any(|c| matches!(c, Cue::Effect { .. })) {
            died = true;
        }
    }
    assert!(died, "fell into the water");
    assert!(w.players[0].is_alive(), "revived");
    assert!(!w.tutorial.as_ref().unwrap().done);
    step(&mut w, 1, PlayerInput::new(0, BTN_SELECT));
    assert!(w.tutorial.as_ref().unwrap().done);
}

#[test]
fn tutorial_is_deterministic() {
    let Some(l) = loaded() else { return };
    let script = |t: usize| match t % 200 {
        0..=60 => PlayerInput::new(128, 0),
        61..=70 => PlayerInput::new(65, 1),
        _ => PlayerInput::new(1, 0),
    };
    let mut a = tutorial(&l);
    let mut b = tutorial(&l);
    for t in 0..1500 {
        a.step(&[script(t)]);
        b.step(&[script(t)]);
        if t % 100 == 0 {
            assert_eq!(a.state_hash(), b.state_hash(), "tick {t}");
        }
    }
    let snap = a.clone();
    a.step(&[script(1500)]);
    let mut c = snap;
    c.step(&[script(1500)]);
    assert_eq!(a.state_hash(), c.state_hash());
}

//! Scenario tests for the G2 abilities (Grenade, Smoke, Mine, Missile,
//! Rock, Throw, Tesla coil, Spike) on a real imported level (Level1);
//! skipped without an import (`cache/import` or the user data dir).
//!
//! Layout used by most tests: four players on Level1; 1 and 2 idle on the
//! small upper capsules, 3 (the caster, team 2) and 4 (the target, team 3)
//! on the long bottom platform at x ≈ −14.6 and +13.1.

use std::sync::Arc;

use bopl_content::Loaded;
use bopl_sim::abilities::{AbilityState, Behaviour, smoke, tesla_coil};
use bopl_sim::input::{BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2, BTN_JUMP, PlayerInput};
use bopl_sim::object::ObjId;
use bopl_sim::player::{CauseOfDeath, PlayerSetup};
use bopl_sim::view::Cue;
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

fn idx(l: &Loaded, name: &str) -> usize {
    l.content
        .abilities
        .iter()
        .position(|a| a.name == name)
        .unwrap_or_else(|| panic!("ability {name}"))
}

fn setup(id: i32, team: i32, abilities: Vec<usize>) -> PlayerSetup {
    PlayerSetup {
        id,
        team,
        color: id - 1,
        abilities,
        uses_keyboard_and_mouse: false,
    }
}

fn world(l: &Loaded, level: usize, players: &[PlayerSetup], seed: u32) -> World {
    let lv = bopl_content::load_level(l, level).unwrap();
    let mut w = World::new(
        Arc::clone(&l.content),
        players,
        MatchSettings {
            number_of_abilities: 3,
            seed,
        },
    );
    bopl_sim::session::load_level(&mut w, lv);
    w
}

const RIGHT: u8 = 1;
const LEFT: u8 = 128;
const UP: u8 = 64;
/// ≈ 45° up-right.
const UP_RIGHT: u8 = 33;

fn bopl(w: &World, id: i32) -> Option<ObjId> {
    w.player(id)?.bopl.filter(|b| w.objects.exists(*b))
}

fn bopl_pos(w: &World, id: i32) -> Option<Vec2> {
    Some(w.objects.get(bopl(w, id)?)?.tf.position)
}

fn grounded(w: &World, id: i32) -> bool {
    match bopl(w, id).and_then(|b| w.objects.get(b)).map(|o| &o.kind) {
        Some(Kind::Bopl(b)) => b.physics.is_grounded && !b.ctrl.is_in_ability,
        _ => false,
    }
}

fn in_ability(w: &World, id: i32) -> bool {
    match bopl(w, id).and_then(|b| w.objects.get(b)).map(|o| &o.kind) {
        Some(Kind::Bopl(b)) => b.ctrl.is_in_ability,
        _ => false,
    }
}

/// The ability object of slot `k` of player `id`.
fn slot_obj(w: &World, id: i32, k: usize) -> Option<ObjId> {
    match w.objects.get(bopl(w, id)?)?.kind {
        Kind::Bopl(ref b) => b.ctrl.abilities.get(k).copied(),
        _ => None,
    }
}

fn timer(w: &World, id: i32, k: usize) -> Fix {
    match bopl(w, id).and_then(|b| w.objects.get(b)).map(|o| &o.kind) {
        Some(Kind::Bopl(b)) => b.ctrl.cooldown_timers[k],
        _ => Fix::ZERO,
    }
}

/// Makes every ability slot of every spawned player ready.
fn ready_all(w: &mut World) {
    for i in 0..w.players.len() {
        if let Some(b) = w.players[i].bopl
            && let Some(Kind::Bopl(b)) = w.objects.get_mut(b).map(|o| &mut o.kind)
        {
            for t in b.ctrl.cooldown_timers.iter_mut() {
                *t = Fix::from_int(1000);
            }
        }
    }
}

type Events = Vec<(i64, Cue)>;

fn step_n(w: &mut World, n: usize, inputs: &[PlayerInput], events: &mut Events) {
    for _ in 0..n {
        w.step(inputs);
        for e in &w.events {
            events.push((e.id.tick, e.cue.clone()));
        }
    }
}

/// Steps with neutral input until all players are spawned and grounded.
fn settle(w: &mut World) {
    let n = w.players.len();
    for _ in 0..600 {
        w.step(&vec![PlayerInput::default(); n]);
        let ids: Vec<i32> = w.players.iter().map(|p| p.id).collect();
        if w.time.simulation_ticks > 200 && ids.iter().all(|&i| grounded(w, i)) {
            return;
        }
    }
    panic!("players did not settle");
}

fn four(l: &Loaded, a3: Vec<usize>, a4: Vec<usize>) -> World {
    let mut w = world(
        l,
        6,
        &[
            setup(1, 0, vec![2, 2, 2]),
            setup(2, 1, vec![2, 2, 2]),
            setup(3, 2, a3),
            setup(4, 3, a4),
        ],
        1,
    );
    settle(&mut w);
    w
}

/// Walks player `id` toward x = `target` (others idle) until within `tol`.
fn walk_to(w: &mut World, id: i32, target: Fix, tol: Fix) {
    let mut ev = Vec::new();
    walk_to_ev(w, id, target, tol, &mut ev);
}

fn walk_to_ev(w: &mut World, id: i32, target: Fix, tol: Fix, ev: &mut Events) {
    let n = w.players.len();
    for _ in 0..600 {
        let Some(p) = bopl_pos(w, id) else {
            break;
        };
        if (p.x - target).abs() < tol {
            break;
        }
        let dir = if p.x < target { RIGHT } else { LEFT };
        step_n(w, 1, &input_for(n, id, PlayerInput::new(dir, 0)), ev);
    }
    step_n(w, 30, &vec![PlayerInput::default(); n], ev);
}

fn input_for(n: usize, id: i32, i: PlayerInput) -> Vec<PlayerInput> {
    let mut v = vec![PlayerInput::default(); n];
    v[(id - 1) as usize] = i;
    v
}

fn spawned_of(w: &World, f: impl Fn(&Behaviour) -> bool) -> Vec<ObjId> {
    w.objects
        .ids()
        .filter(|&id| {
            let o = w.objects.get(id).unwrap();
            matches!(&o.kind, Kind::Behaviour(b) if f(b)) && !o.destroyed
        })
        .collect()
}

fn died(events: &[(i64, Cue)], id: i32) -> Option<(i32, CauseOfDeath)> {
    events.iter().find_map(|(_, c)| match c {
        Cue::PlayerDied {
            id: i,
            killer,
            cause,
        } if *i == id => Some((*killer, *cause)),
        _ => None,
    })
}

fn sound_ticks(events: &[(i64, Cue)], s: &str) -> Vec<i64> {
    events
        .iter()
        .filter(|(_, c)| matches!(c, Cue::Sound(n) if *n == s))
        .map(|(t, _)| *t)
        .collect()
}

fn has_sound(events: &[(i64, Cue)], s: &str) -> bool {
    !sound_ticks(events, s).is_empty()
}

#[allow(dead_code)]
fn print_events(ev: &[(i64, Cue)]) {
    for (t, c) in ev {
        eprintln!("  {t}: {c:?}");
    }
}

fn cooldown_reset_on_cast(w: &World, id: i32) -> bool {
    timer(w, id, 0) < Fix::ONE
}

// -- Grenade ----------------------------------------------------------------------------

/// Holding a grenade for its 3 s fuse blows it up in the hand: the thrower
/// dies by accident (killer = itself), a nearby enemy dies credited to it.
#[test]
fn grenade_fuse_in_hand_kills_thrower_and_neighbour() {
    let Some(l) = loaded() else {
        return;
    };
    let g = idx(&l, "Grenade");
    let mut w = four(&l, vec![g, 2, 2], vec![2, 2, 2]);
    let x3 = bopl_pos(&w, 3).unwrap().x;
    walk_to(&mut w, 4, x3 + Fix::from_int(3), Fix::ONE);
    ready_all(&mut w);
    let mut ev = Vec::new();
    let hold = input_for(4, 3, PlayerInput::new(0, BTN_ABILITY0));
    step_n(&mut w, 1, &hold, &mut ev);
    assert!(in_ability(&w, 3), "cast");
    assert!(cooldown_reset_on_cast(&w, 3));
    // gEnter: 15 ticks, the item spawns on the 16th animator update; its
    // fuse (3 s = 180 ticks of Grenade.UpdateSim) starts the tick after.
    step_n(&mut w, 250, &hold, &mut ev);
    let boom = sound_ticks(&ev, "explosion");
    assert_eq!(boom.len(), 1, "one explosion");
    assert_eq!(died(&ev, 3), Some((3, CauseOfDeath::Accident)));
    assert_eq!(died(&ev, 4), Some((3, CauseOfDeath::Other)));
    assert!(w.fault.is_none());
}

/// A quick tap throws the grenade with the minimum charge (stamp = 1/60:
/// force ≈ 56.65); it flies to the enemy and kills it on contact.
#[test]
fn grenade_throw_hits_enemy() {
    let Some(l) = loaded() else {
        return;
    };
    let g = idx(&l, "Grenade");
    let mut w = four(&l, vec![g, 2, 2], vec![2, 2, 2]);
    let x3 = bopl_pos(&w, 3).unwrap().x;
    walk_to(&mut w, 4, x3 + Fix::from_int(10), Fix::ONE);
    ready_all(&mut w);
    let mut ev = Vec::new();
    step_n(
        &mut w,
        1,
        &input_for(4, 3, PlayerInput::new(UP_RIGHT, BTN_ABILITY0)),
        &mut ev,
    );
    assert!(in_ability(&w, 3));
    // Release at once, keep aiming.
    let aim = input_for(4, 3, PlayerInput::new(UP_RIGHT, 0));
    let mut thrown_v = None;
    let idle = vec![PlayerInput::default(); 4];
    for _ in 0..240 {
        // Keep aiming while in the ability, then stand still.
        let inp = if in_ability(&w, 3) { &aim } else { &idle };
        step_n(&mut w, 1, inp, &mut ev);
        let items = spawned_of(&w, |b| matches!(b, Behaviour::Grenade(_)));
        if thrown_v.is_none()
            && let Some(&it) = items.first()
            && let Some(b) = w.physics.body(it)
            && b.velocity != Vec2::ZERO
        {
            thrown_v = Some(b.velocity);
        }
        if died(&ev, 4).is_some() {
            break;
        }
    }
    let v = thrown_v.expect("grenade thrown");
    // Δv = F/m (impulse via force × 60 over one step): ≈ 56.65 / 3 ≈ 18.9.
    let s = v.magnitude().to_f64();
    assert!((17.0..20.0).contains(&s), "throw speed {s}");
    assert!(has_sound(&ev, "throwSmallItem"));
    assert_eq!(
        died(&ev, 4),
        Some((3, CauseOfDeath::Other)),
        "enemy killed by the thrower"
    );
    assert!(died(&ev, 3).is_none());
}

// -- Smoke ------------------------------------------------------------------------------

fn smoke_buffer(w: &World, id: i32) -> (usize, usize) {
    let a = slot_obj(w, id, 0).unwrap();
    match &w.objects.get(a).unwrap().kind {
        Kind::Ability(x) => match &x.state {
            AbilityState::Smoke(s) => (s.0.effect_buffer.slots.len(), s.0.effect_buffer.live(w)),
            _ => panic!("not smoke"),
        },
        _ => panic!("no ability object"),
    }
}

/// A smoke grenade bursts into four clouds on contact; the slot's ring
/// buffer keeps at most 40 live clouds (the 41st evicts the oldest); an
/// explosion ignites the clouds, which explode (killer = thrower).
#[test]
fn smoke_burst_ring_buffer_and_ignition() {
    let Some(l) = loaded() else {
        return;
    };
    let s = idx(&l, "Smoke");
    let g = idx(&l, "Grenade");
    let mut w = four(&l, vec![s, g, 2], vec![2, 2, 2]);
    assert_eq!(smoke_buffer(&w, 3).0, 40);
    let mut ev = Vec::new();
    let mut casts = 0;
    for _ in 0..12 {
        ready_all(&mut w);
        let before = smoke::clouds(&w).len();
        step_n(
            &mut w,
            1,
            &input_for(4, 3, PlayerInput::new(UP, BTN_ABILITY0)),
            &mut ev,
        );
        assert!(in_ability(&w, 3));
        casts += 1;
        // Thrown straight up at minimum force: bursts when it lands back.
        for _ in 0..300 {
            step_n(
                &mut w,
                1,
                &input_for(4, 3, PlayerInput::new(UP, 0)),
                &mut ev,
            );
            if smoke::clouds(&w).len() != before && !in_ability(&w, 3) {
                break;
            }
        }
        let after = smoke::clouds(&w).len();
        let live = smoke_buffer(&w, 3).1;
        eprintln!("cast {casts}: clouds {before} -> {after}, buffer live {live}");
        if casts == 1 {
            assert_eq!(after, 4, "four clouds per burst");
            assert!(has_sound(&ev, "smokeBombExplosion"));
        }
        assert!(after <= 40 + 4);
        step_n(&mut w, 2, &input_for(4, 3, PlayerInput::default()), &mut ev);
        assert!(smoke::clouds(&w).len() <= 40, "at most 40 clouds per slot");
    }
    assert_eq!(smoke_buffer(&w, 3).1, 40);
    assert!(died(&ev, 3).is_none(), "smoke itself is harmless");
    // Grenade into the clouds: ignition chain, smoke explosions kill the
    // thrower (accident) - the clouds surround it.
    ready_all(&mut w);
    let hold = input_for(4, 3, PlayerInput::new(0, BTN_ABILITY1));
    step_n(&mut w, 260, &hold, &mut ev);
    assert!(has_sound(&ev, "explosion"));
    assert!(has_sound(&ev, "smokeExplosion"), "clouds ignited");
    assert!(smoke::clouds(&w).len() < 40, "burnt clouds are gone");
    assert_eq!(died(&ev, 3).map(|d| d.0), Some(3));
}

// -- Mine -------------------------------------------------------------------------------

/// Tap-placed mine arms after 2 s, chases the enemy and explodes; the kill
/// is credited to the thrower.
#[test]
fn mine_arms_chases_and_kills() {
    let Some(l) = loaded() else {
        return;
    };
    let m = idx(&l, "Mine");
    let mut w = four(&l, vec![m, 2, 2], vec![2, 2, 2]);
    ready_all(&mut w);
    let mut ev = Vec::new();
    step_n(
        &mut w,
        1,
        &input_for(4, 3, PlayerInput::new(0, BTN_ABILITY0)),
        &mut ev,
    );
    assert!(in_ability(&w, 3));
    let t0 = w.time.simulation_ticks;
    step_n(
        &mut w,
        40,
        &input_for(4, 3, PlayerInput::default()),
        &mut ev,
    );
    assert!(!in_ability(&w, 3), "mine cast over");
    let fire = sound_ticks(&ev, "throwSmallItem");
    // Tap: released during gEnter → fires on the gEnter callback (update #6).
    // Update #k of the ability happens in tick t0 + k − 2 (#1 is the cast tick).
    assert_eq!(fire, vec![t0 + 4], "fire tick");
    let mines = spawned_of(&w, |b| {
        matches!(
            b,
            Behaviour::Mine(bopl_sim::abilities::mine::Spawned::Mine(_))
        )
    });
    assert_eq!(mines.len(), 1);
    // Move the caster away, bring the enemy near the mine.
    let mx = w.objects.get(mines[0]).unwrap().tf.position.x;
    walk_to_ev(&mut w, 3, mx - Fix::from_int(6), Fix::TWO, &mut ev);
    walk_to_ev(&mut w, 4, mx + Fix::from_int(4), Fix::ONE, &mut ev);
    step_n(&mut w, 300, &[PlayerInput::default(); 4], &mut ev);
    assert!(has_sound(&ev, "mineArmed"));
    assert!(has_sound(&ev, "mineExplosion"));
    assert_eq!(died(&ev, 4), Some((3, CauseOfDeath::Other)));
    assert!(died(&ev, 3).is_none());
}

/// Holding through the grounded loop fires on update #22 (6 + 16).
#[test]
fn mine_hold_fires_after_loop() {
    let Some(l) = loaded() else {
        return;
    };
    let m = idx(&l, "Mine");
    let mut w = four(&l, vec![m, 2, 2], vec![2, 2, 2]);
    ready_all(&mut w);
    let mut ev = Vec::new();
    let hold = input_for(4, 3, PlayerInput::new(0, BTN_ABILITY0));
    step_n(&mut w, 1, &hold, &mut ev);
    let t0 = w.time.simulation_ticks;
    step_n(&mut w, 40, &hold, &mut ev);
    assert_eq!(sound_ticks(&ev, "throwSmallItem"), vec![t0 + 20]);
}

// -- Missile ----------------------------------------------------------------------------

/// The rocket launches after the enter animation, is steered toward the
/// enemy and its MineExplosion (owner never set) kills it: killer 0.
#[test]
fn missile_steers_and_kills_with_owner_zero() {
    let Some(l) = loaded() else {
        return;
    };
    let ms = idx(&l, "Missile");
    let mut w = four(&l, vec![ms, 2, 2], vec![2, 2, 2]);
    // Open sky above the caster (not under the upper capsules).
    walk_to(&mut w, 3, Fix::from_int(-7), Fix::ONE);
    ready_all(&mut w);
    let mut ev = Vec::new();
    step_n(
        &mut w,
        1,
        &input_for(4, 3, PlayerInput::new(UP, BTN_ABILITY0)),
        &mut ev,
    );
    assert!(in_ability(&w, 3));
    // Steer at player 4; the rocket's arc meets player 2 on the right upper
    // capsule first (deterministic).
    let p4 = bopl_pos(&w, 4).unwrap();
    for _ in 0..400 {
        // Steer at the enemy.
        let rockets = spawned_of(&w, |b| matches!(b, Behaviour::Missile(_)));
        let aim = match rockets.first() {
            Some(&r) => {
                let rp = w.objects.get(r).unwrap().tf.position;
                bopl_sim::input::angle_byte_from_direction(p4 - rp)
            }
            None => UP,
        };
        let a = aim.max(1);
        step_n(
            &mut w,
            1,
            &input_for(4, 3, PlayerInput::new(a, BTN_ABILITY0)),
            &mut ev,
        );
        if died(&ev, 2).is_some() {
            break;
        }
    }
    assert!(has_sound(&ev, "transform1"));
    assert!(has_sound(&ev, "mineExplosion"));
    assert_eq!(
        died(&ev, 2),
        Some((0, CauseOfDeath::Other)),
        "kill credited to nobody"
    );
    assert!(died(&ev, 3).is_none());
    assert_eq!(w.player(3).unwrap().kills, 0);
}

// -- Rock -------------------------------------------------------------------------------

/// The rock lasts 2.5 s (+0.2 s shrink) and kills whoever it touches.
#[test]
fn rock_rolls_into_enemy() {
    let Some(l) = loaded() else {
        return;
    };
    let r = idx(&l, "Rock");
    let mut w = four(&l, vec![r, 2, 2], vec![2, 2, 2]);
    let x3 = bopl_pos(&w, 3).unwrap().x;
    walk_to(&mut w, 4, x3 + Fix::from_int(6), Fix::ONE);
    ready_all(&mut w);
    let mut ev = Vec::new();
    // Run right, then cast: the rock keeps the running speed.
    step_n(
        &mut w,
        8,
        &input_for(4, 3, PlayerInput::new(RIGHT, 0)),
        &mut ev,
    );
    step_n(
        &mut w,
        1,
        &input_for(4, 3, PlayerInput::new(RIGHT, BTN_ABILITY0)),
        &mut ev,
    );
    assert!(in_ability(&w, 3));
    assert!(has_sound(&ev, "rockBounce"));
    let t0 = w.time.simulation_ticks;
    let mut exit_at = None;
    for _ in 0..300 {
        step_n(
            &mut w,
            1,
            &input_for(4, 3, PlayerInput::new(RIGHT, 0)),
            &mut ev,
        );
        if exit_at.is_none() && !in_ability(&w, 3) {
            exit_at = Some(w.time.simulation_ticks - t0);
        }
    }
    assert_eq!(died(&ev, 4), Some((3, CauseOfDeath::Other)));
    let e = exit_at.expect("rock ended");
    // 2.5 s + 0.2 s of owner time ≈ 162 ticks.
    assert!((160..=166).contains(&e), "rock lasted {e} ticks");
}

// -- Throw ------------------------------------------------------------------------------

/// Grabbing shrinks the platform; the boulder is held overhead and thrown
/// on release as a dynamic platform.
#[test]
fn throw_grabs_and_throws_boulder() {
    let Some(l) = loaded() else {
        return;
    };
    let t = idx(&l, "Throw");
    let mut w = four(&l, vec![t, 2, 2], vec![2, 2, 2]);
    // Away from the upper capsules (the held boulder is ~8 units up).
    walk_to(&mut w, 3, Fix::from_int(-7), Fix::ONE);
    ready_all(&mut w);
    let ground = match w.objects.get(bopl(&w, 3).unwrap()).unwrap().kind {
        Kind::Bopl(ref b) => b.physics.attached_ground.unwrap(),
        _ => unreachable!(),
    };
    let s0 = w.objects.get(ground).unwrap().tf.scale;
    let mut ev = Vec::new();
    let hold = input_for(4, 3, PlayerInput::new(0, BTN_ABILITY0));
    step_n(&mut w, 1, &hold, &mut ev);
    assert!(in_ability(&w, 3));
    // enter (25) + grab (21) + pickup (30) updates, then idle holding.
    step_n(&mut w, 90, &hold, &mut ev);
    let s1 = w.objects.get(ground).unwrap().tf.scale;
    assert!(s1 < s0, "platform shrank: {s0:?} -> {s1:?}");
    let boulders: Vec<ObjId> = w
        .objects
        .ids()
        .filter(|&id| matches!(&w.objects.get(id).unwrap().kind, Kind::Platform(p) if p.boulder.is_some()))
        .collect();
    assert_eq!(boulders.len(), 1, "one held boulder");
    let b = boulders[0];
    assert!(has_sound(&ev, "pickupBoulder"));
    // Release → beginThrow (8 ticks) → fire on its callback.
    let mut v = None;
    for _ in 0..30 {
        step_n(
            &mut w,
            1,
            &input_for(4, 3, PlayerInput::new(RIGHT, 0)),
            &mut ev,
        );
        if v.is_none() && has_sound(&ev, "throwBoulder") {
            v = w.physics.body(b).map(|b| b.velocity);
        }
    }
    let v = v.expect("boulder thrown");
    // Full throw: velocity = aim · 720 · scale · (1/12) = 60 along +x.
    // (y: one physics step of gravity since the throw.)
    assert_eq!(
        v.x,
        Fix::from_int(720) * (Fix::ONE / Fix::from_int(12)),
        "throw velocity {v:?}"
    );
    step_n(&mut w, 60, &[PlayerInput::default(); 4], &mut ev);
    assert!(!in_ability(&w, 3));
    assert!(w.fault.is_none());
}

// -- Tesla coil -------------------------------------------------------------------------

/// Two coils connect with a beam after the orb charge-up; a third removes
/// the oldest (two per slot); the beam kills with killer 0.
#[test]
fn tesla_coils_pair_limit_and_beam_kills() {
    let Some(l) = loaded() else {
        return;
    };
    let tc = idx(&l, "Tesla coil");
    let mut w = four(&l, vec![tc, 2, 2], vec![2, 2, 2]);
    let mut ev = Vec::new();
    let x3 = bopl_pos(&w, 3).unwrap().x;
    // Coil 1 here, coil 2 eight units to the right.
    ready_all(&mut w);
    step_n(
        &mut w,
        1,
        &input_for(4, 3, PlayerInput::new(0, BTN_ABILITY0)),
        &mut ev,
    );
    step_n(&mut w, 45, &[PlayerInput::default(); 4], &mut ev);
    assert!(!in_ability(&w, 3));
    assert_eq!(tesla_coil::nodes(&w).len(), 1);
    walk_to(&mut w, 3, x3 + Fix::from_int(8), Fix::ONE);
    ready_all(&mut w);
    step_n(
        &mut w,
        1,
        &input_for(4, 3, PlayerInput::new(0, BTN_ABILITY0)),
        &mut ev,
    );
    step_n(&mut w, 45, &[PlayerInput::default(); 4], &mut ev);
    assert_eq!(tesla_coil::nodes(&w).len(), 2);
    assert!(has_sound(&ev, "loadLightning"), "pair connects");
    step_n(&mut w, 60, &[PlayerInput::default(); 4], &mut ev);
    assert!(has_sound(&ev, "lightning"), "beam active");
    assert_eq!(tesla_coil::beams(&w).len(), 1);
    // Third coil: the oldest goes.
    let x = bopl_pos(&w, 3).unwrap().x;
    walk_to(&mut w, 3, x + Fix::from_int(4), Fix::ONE);
    ready_all(&mut w);
    step_n(
        &mut w,
        1,
        &input_for(4, 3, PlayerInput::new(0, BTN_ABILITY0)),
        &mut ev,
    );
    step_n(&mut w, 45, &[PlayerInput::default(); 4], &mut ev);
    assert_eq!(tesla_coil::nodes(&w).len(), 2, "two coils per slot");
    step_n(&mut w, 70, &[PlayerInput::default(); 4], &mut ev);
    assert_eq!(tesla_coil::beams(&w).len(), 1, "new pair has a beam");
    // Both live coils are lethal terrain now.
    let nodes = tesla_coil::nodes(&w);
    assert!(nodes.iter().all(|n| w.objects.get(*n).unwrap().layer == 16));
    // Walk the caster away and the enemy into the pair.
    let mid = nodes
        .iter()
        .map(|n| w.objects.get(*n).unwrap().tf.position.x)
        .fold(Fix::ZERO, |a, x| a + x)
        / Fix::TWO;
    walk_to(&mut w, 3, mid - Fix::from_int(14), Fix::TWO);
    walk_to(&mut w, 4, mid, Fix::ONE);
    // The beam joins the orbs ≈ 3.25 above the surface: standing bopls pass
    // under it; jumping into it kills.
    step_n(&mut w, 30, &[PlayerInput::default(); 4], &mut ev);
    assert!(died(&ev, 4).is_none(), "walking under the beam is safe");
    step_n(
        &mut w,
        2,
        &input_for(4, 4, PlayerInput::new(0, BTN_JUMP)),
        &mut ev,
    );
    step_n(&mut w, 40, &[PlayerInput::default(); 4], &mut ev);
    assert_eq!(
        died(&ev, 4),
        Some((0, CauseOfDeath::Other)),
        "killed by the lightning (nobody)"
    );
    assert!(w.fault.is_none());
}

// -- Spike ------------------------------------------------------------------------------

/// The spike comes out of the far side of the platform, welded to it; a
/// second cast replaces it; touching it kills (killer 0).
#[test]
fn spike_casts_replaces_and_is_lethal() {
    let Some(l) = loaded() else {
        return;
    };
    let sp = idx(&l, "Spike");
    let mut w = four(&l, vec![sp, 2, 2], vec![2, 2, 2]);
    let mut ev = Vec::new();
    ready_all(&mut w);
    step_n(
        &mut w,
        1,
        &input_for(4, 3, PlayerInput::new(0, BTN_ABILITY0)),
        &mut ev,
    );
    let t0 = w.time.simulation_ticks;
    step_n(&mut w, 50, &[PlayerInput::default(); 4], &mut ev);
    let spikes = spawned_of(&w, |b| matches!(b, Behaviour::Spike(_)));
    assert_eq!(spikes.len(), 1);
    assert_eq!(
        sound_ticks(&ev, "spikeRock"),
        vec![t0 + 21],
        "CastSpike on update #23"
    );
    let p3 = bopl_pos(&w, 3).unwrap();
    let sp_pos = w.objects.get(spikes[0]).unwrap().tf.position;
    assert!(
        sp_pos.y < p3.y - Fix::from_int(5),
        "spike below the platform: {sp_pos:?}"
    );
    // Recast: the old spike is replaced.
    ready_all(&mut w);
    step_n(
        &mut w,
        1,
        &input_for(4, 3, PlayerInput::new(0, BTN_ABILITY0)),
        &mut ev,
    );
    step_n(&mut w, 50, &[PlayerInput::default(); 4], &mut ev);
    let spikes2 = spawned_of(&w, |b| matches!(b, Behaviour::Spike(_)));
    assert_eq!(spikes2.len(), 1);
    assert_ne!(spikes2[0], spikes[0]);
    // The cooldown restarted on the cast: an immediate recast fails.
    assert!(timer(&w, 3, 0) < Fix::THREE);
    step_n(
        &mut w,
        1,
        &input_for(4, 3, PlayerInput::new(0, BTN_ABILITY0)),
        &mut ev,
    );
    assert!(!in_ability(&w, 3), "spike on cooldown");
    assert!(w.fault.is_none());
}

/// A spike cast on an upper capsule shoots down out of its underside and
/// kills the player standing below; the kill is credited to nobody (0).
#[test]
fn spike_kills_player_below_with_owner_zero() {
    let Some(l) = loaded() else {
        return;
    };
    let sp = idx(&l, "Spike");
    let mut w = world(
        &l,
        6,
        &[
            setup(1, 0, vec![sp, 2, 2]),
            setup(2, 1, vec![2, 2, 2]),
            setup(3, 2, vec![2, 2, 2]),
            setup(4, 3, vec![2, 2, 2]),
        ],
        1,
    );
    settle(&mut w);
    ready_all(&mut w);
    let mut ev = Vec::new();
    step_n(
        &mut w,
        1,
        &input_for(4, 1, PlayerInput::new(0, BTN_ABILITY0)),
        &mut ev,
    );
    assert!(in_ability(&w, 1));
    step_n(&mut w, 60, &[PlayerInput::default(); 4], &mut ev);
    assert!(has_sound(&ev, "spikeRock"));
    assert_eq!(
        died(&ev, 3),
        Some((0, CauseOfDeath::Other)),
        "killed by the spike (nobody)"
    );
    assert!(died(&ev, 1).is_none());
    assert_eq!(w.player(1).unwrap().kills, 0);
}

// -- determinism ----------------------------------------------------------------------

/// A deterministic pseudo-random input script per player (abilities pressed
/// often, held for a while).
fn inputs(n: usize, t: u64, seed: u64) -> Vec<PlayerInput> {
    (0..n)
        .map(|i| {
            let mut x = seed
                ^ (t / 11).wrapping_mul(0x9E37_79B9_7F4A_7C15)
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

fn g2_twelve(l: &Loaded) -> Vec<PlayerSetup> {
    let names = [
        "Grenade",
        "Smoke",
        "Mine",
        "Missile",
        "Rock",
        "Throw",
        "Tesla coil",
        "Spike",
    ];
    let ids: Vec<usize> = names.iter().map(|n| idx(l, n)).collect();
    (0..12)
        .map(|i| PlayerSetup {
            id: i + 1,
            team: i % 6,
            color: i,
            abilities: (0..3)
                .map(|k| ids[(i as usize * 3 + k) % ids.len()])
                .collect(),
            uses_keyboard_and_mouse: i == 5,
        })
        .collect()
}

fn run(w: &mut World, from: u64, to: u64, seed: u64) {
    let n = w.players.len();
    for t in from..to {
        w.step(&inputs(n, t, seed));
    }
}

/// 12 players using only G2 abilities: identical state hashes for two
/// worlds with the same inputs over many rounds and level loads.
#[test]
fn g2_determinism_twelve_players() {
    let Some(l) = loaded() else {
        return;
    };
    let levels = [6usize, 18, 28, 40];
    let lv: Vec<_> = levels
        .iter()
        .map(|&i| bopl_content::load_level(&l, i).unwrap())
        .collect();
    let setups = g2_twelve(&l);
    let mk = || {
        let mut w = World::new(
            Arc::clone(&l.content),
            &setups,
            MatchSettings {
                number_of_abilities: 3,
                seed: 11,
            },
        );
        bopl_sim::session::load_level(&mut w, lv[0].clone());
        w
    };
    let (mut a, mut b) = (mk(), mk());
    let mut next = 0;
    let mut rounds = 0;
    let mut spawned_kinds = std::collections::BTreeSet::new();
    for chunk in 0..60u64 {
        run(&mut a, chunk * 100, chunk * 100 + 100, 5);
        run(&mut b, chunk * 100, chunk * 100 + 100, 5);
        assert_eq!(a.state_hash(), b.state_hash(), "tick {}", (chunk + 1) * 100);
        for id in a.objects.ids() {
            match &a.objects.get(id).unwrap().kind {
                Kind::Behaviour(x) => {
                    spawned_kinds.insert(format!("{:?}", std::mem::discriminant(x.as_ref())));
                }
                Kind::Platform(p) if p.boulder.is_some() => {
                    spawned_kinds.insert("boulder".to_string());
                }
                _ => {}
            }
        }
        if a.round.winner_screen && a.time.sim_time_since_level_loaded > Fix::from_int(20) {
            rounds += 1;
            next = (next + 1) % lv.len();
            bopl_sim::session::load_level(&mut a, lv[next].clone());
            bopl_sim::session::load_level(&mut b, lv[next].clone());
        }
    }
    let deaths: i32 = a.players.iter().map(|p| p.deaths).sum();
    eprintln!(
        "{rounds} rounds, {deaths} deaths, spawned kinds {}, fault {:?}",
        spawned_kinds.len(),
        a.fault
    );
    assert!(rounds >= 2, "only {rounds} rounds");
    assert!(
        spawned_kinds.len() >= 5,
        "abilities were used: {spawned_kinds:?}"
    );
    assert!(a.fault.is_none());
}

/// Rollback with G2 abilities in flight: snapshot, mispredict, restore,
/// re-simulate equals a straight run.
#[test]
fn g2_rollback_equals_straight_run() {
    let Some(l) = loaded() else {
        return;
    };
    let setups = g2_twelve(&l);
    let mk = || world(&l, 18, &setups, 3);
    let mut straight = mk();
    run(&mut straight, 0, 2400, 1);
    let mut rb = mk();
    run(&mut rb, 0, 600, 1);
    for k in 0..20u64 {
        let base = 600 + k * 90;
        let snap = rb.clone();
        run(&mut rb, base, base + 8, 777 + k);
        rb = snap;
        run(&mut rb, base, base + 90, 1);
    }
    assert_eq!(rb.time.simulation_ticks, straight.time.simulation_ticks);
    assert_eq!(rb.state_hash(), straight.state_hash());
    assert_eq!(rb.view(), straight.view());
}

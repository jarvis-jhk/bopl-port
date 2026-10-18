//! Simulation tests on real imported levels (skipped without an import in
//! `cache/import` or the user data dir).

use std::sync::Arc;
use std::time::Instant;

use bopl_content::Loaded;
use bopl_sim::bopl::State;
use bopl_sim::input::{BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2, BTN_JUMP, PlayerInput};
use bopl_sim::player::{CauseOfDeath, PlayerSetup};
use bopl_sim::view::Cue;
use bopl_sim::world::{Kind, MatchSettings, World};

fn loaded() -> Option<Loaded> {
    let dir = bopl_content::default_import_dir();
    if !dir.join("index.json").exists() {
        eprintln!("no import at {}; skipping", dir.display());
        return None;
    }
    Some(bopl_content::load(dir).expect("import loads"))
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

fn solo() -> Vec<PlayerSetup> {
    vec![PlayerSetup {
        id: 1,
        team: 0,
        color: 0,
        abilities: vec![4, 7, 2],
        uses_keyboard_and_mouse: false,
    }]
}

fn bopl_state(w: &World, id: i32) -> Option<(State, bool, bopl_sim::Vec2)> {
    let p = w.player(id)?;
    let o = w.objects.get(p.bopl?)?;
    match &o.kind {
        Kind::Bopl(b) => Some((b.ctrl.state, b.physics.is_grounded, o.tf.position)),
        _ => None,
    }
}

const RIGHT: u8 = 1; // angle byte of +x
const LEFT: u8 = 128; // ≈ π

/// One bopl spawns, lands, walks around a rounded platform, jumps and dies
/// in the water (Level1).
#[test]
fn sandbox_spawn_walk_jump_die() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(&l, 6, &solo(), 1);
    let mut spawned_at = None;
    let mut landed_at = None;
    let mut died_at = None;
    let mut won_at = None;
    let mut ran = false;
    let mut airborne_after_jump = false;
    let mut corner_param = Vec::new();
    for t in 0..2000i64 {
        let input = match t {
            0..=219 => PlayerInput::default(),
            // Walk right along the capsule and around its rounded right end
            // (the bopl stops on the vertical side, where "right" no longer
            // points along the surface).
            220..=300 => PlayerInput::new(RIGHT, 0),
            // Walk back up around the corner and along the top.
            301..=345 => PlayerInput::new(LEFT, 0),
            // Jump left off the platform, keep steering left into the water.
            346..=347 => PlayerInput::new(LEFT, BTN_JUMP),
            _ => PlayerInput::new(LEFT, 0),
        };
        w.step(&[input]);
        for e in &w.events {
            match e.cue {
                Cue::PlayerSpawned { id: 1 } => spawned_at = Some(e.id.tick),
                Cue::PlayerSound("slimeLand", 1) if landed_at.is_none() => {
                    landed_at = Some(e.id.tick)
                }
                Cue::PlayerDied {
                    id: 1,
                    killer,
                    cause,
                } => {
                    assert_eq!(killer, 1000);
                    assert_eq!(cause, CauseOfDeath::Accident);
                    died_at = Some(e.id.tick);
                }
                Cue::RoundWon { team: 0 } => won_at = Some(e.id.tick),
                _ => {}
            }
        }
        if let Some((st, g, _)) = bopl_state(&w, 1) {
            if (221..300).contains(&t) {
                if st == State::Run {
                    ran = true;
                }
                if let Some(Kind::Bopl(b)) = w
                    .player(1)
                    .and_then(|p| w.objects.get(p.bopl?))
                    .map(|o| &o.kind)
                {
                    corner_param.push(b.physics.local_platform_pos);
                }
                assert!(g, "stays attached while walking (tick {t})");
            }
            if t == 300 {
                // On the platform's right side: normal ≈ +x.
                let n = w
                    .player(1)
                    .and_then(|p| p.bopl)
                    .and_then(|b| w.objects.get(b))
                    .map(|o| o.tf.up)
                    .unwrap();
                assert!(
                    n.x > bopl_sim::Fix::from_f32(0.9),
                    "on the right side, up = {n:?}"
                );
            }
            if t == 352 {
                airborne_after_jump = !g;
            }
        }
        if won_at.is_some() {
            break;
        }
    }
    // Spawn: delay 1.0 s (61 updates) + 1 tick, Init next tick, then the
    // spawn animation (Σ|duration| = 105) fires on its 106th advance.
    assert_eq!(spawned_at, Some(167));
    let landed = landed_at.expect("lands");
    assert!((170..240).contains(&landed), "landed at {landed}");
    assert!(ran, "ran");
    // The surface parameter advanced monotonically (clockwise walking).
    assert!(corner_param.windows(2).filter(|p| p[1] < p[0]).count() <= 1);
    assert!(airborne_after_jump, "jump leaves the platform");
    let died = died_at.expect("dies in the water");
    // Round end is seen one tick later (the match handler updates first).
    assert_eq!(won_at, Some(died + 1));
    let p = w.player(1).unwrap();
    assert_eq!(p.deaths, 1);
    assert_eq!(p.games_won, 1);
    assert!(w.fault.is_none());
}

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
            if (x >> 24) & 63 == 1 {
                b |= BTN_ABILITY0;
            }
            if (x >> 30) & 63 == 2 {
                b |= BTN_ABILITY1;
            }
            if (x >> 36) & 63 == 3 {
                b |= BTN_ABILITY2;
            }
            PlayerInput::new(angle, b)
        })
        .collect()
}

fn twelve() -> Vec<PlayerSetup> {
    (0..12)
        .map(|i| PlayerSetup {
            id: i + 1,
            team: i % 6,
            color: i,
            abilities: vec![1, (2 + i as usize * 3) % 29 + 2, (5 + i as usize) % 29 + 2],
            uses_keyboard_and_mouse: i == 3,
        })
        .collect()
}

fn run(w: &mut World, from: u64, to: u64, seed: u64) {
    let n = w.players.len();
    for t in from..to {
        w.step(&inputs(n, t, seed));
    }
}

/// Same inputs → identical state hash over 10 000 ticks, 12 players, several
/// levels (grass, snow, space, orbiting and path platforms). When a round has
/// reached the winner screen the next level of the list is loaded, so the
/// run covers many rounds and level loads.
#[test]
fn determinism_twelve_players() {
    let Some(l) = loaded() else {
        return;
    };
    let levels = [6usize, 17, 18, 22, 28, 40, 64, 126];
    let lv: Vec<_> = levels
        .iter()
        .map(|&i| bopl_content::load_level(&l, i).unwrap())
        .collect();
    for (start, &level) in levels.iter().enumerate() {
        let mk = || {
            let mut w = World::new(
                Arc::clone(&l.content),
                &twelve(),
                MatchSettings {
                    number_of_abilities: 3,
                    seed: 7,
                },
            );
            bopl_sim::session::load_level(&mut w, lv[start].clone());
            w
        };
        let (mut a, mut b) = (mk(), mk());
        let mut rounds = 0;
        let mut next = start;
        let mut hashes = Vec::new();
        for chunk in 0..100u64 {
            run(&mut a, chunk * 100, chunk * 100 + 100, level as u64);
            run(&mut b, chunk * 100, chunk * 100 + 100, level as u64);
            assert_eq!(
                a.state_hash(),
                b.state_hash(),
                "level {level} tick {}",
                (chunk + 1) * 100
            );
            hashes.push(a.state_hash());
            if a.round.winner_screen
                && a.time.sim_time_since_level_loaded > bopl_sim::Fix::from_int(20)
            {
                rounds += 1;
                next = (next + 1) % lv.len();
                bopl_sim::session::load_level(&mut a, lv[next].clone());
                bopl_sim::session::load_level(&mut b, lv[next].clone());
            }
        }
        hashes.dedup();
        assert!(hashes.len() > 50, "level {level}: state did not evolve");
        let deaths: i32 = a.players.iter().map(|p| p.deaths).sum();
        let won: i32 = a.players.iter().map(|p| p.games_won).sum();
        eprintln!(
            "start level {level}: {rounds} rounds, {deaths} deaths, {won} wins, hash {:016x}, fault {:?}",
            a.state_hash(),
            a.fault
        );
        assert!(rounds >= 2, "level {level}: only {rounds} rounds");
        assert!(a.fault.is_none());
    }
}

/// Rollback: snapshot, run ahead with other inputs, restore, re-run the
/// real inputs → the same state as a run that never rolled back.
#[test]
fn clone_and_replay_equals_straight_run() {
    let Some(l) = loaded() else {
        return;
    };
    let mut straight = world(&l, 18, &twelve(), 3);
    run(&mut straight, 0, 3000, 1);
    let mut rb = world(&l, 18, &twelve(), 3);
    run(&mut rb, 0, 1200, 1);
    for k in 0..20u64 {
        let base = 1200 + k * 90;
        let snap = rb.clone();
        // Mispredicted inputs for 8 ticks...
        run(&mut rb, base, base + 8, 999 + k);
        // ...roll back and re-simulate with the real ones.
        rb = snap;
        run(&mut rb, base, base + 90, 1);
    }
    assert_eq!(rb.time.simulation_ticks, straight.time.simulation_ticks);
    assert_eq!(rb.state_hash(), straight.state_hash());
    assert_eq!(rb.checksum(), straight.checksum());
    assert_eq!(rb.view(), straight.view());
}

/// Cost of a rollback frame: clone + step, 12 players (release only).
#[test]
fn clone_and_step_cost() {
    let Some(l) = loaded() else {
        return;
    };
    if cfg!(debug_assertions) {
        return;
    }
    let mut w = world(&l, 18, &twelve(), 5);
    run(&mut w, 0, 400, 2);
    // Best of several batches: a busy machine slows some batches down, the
    // fastest one still shows the real cost.
    let n = 400u64;
    let mut keep = 0u64;
    let mut per = f64::MAX;
    let mut clone = f64::MAX;
    for b in 0..5u64 {
        let t0 = Instant::now();
        for t in 400 + b * n..400 + (b + 1) * n {
            let c = w.clone();
            keep ^= c.time.simulation_ticks as u64;
            w.step(&inputs(12, t, 2));
        }
        per = per.min(t0.elapsed().as_secs_f64() * 1e6 / n as f64);
        let t1 = Instant::now();
        for _ in 0..n {
            let c = w.clone();
            keep ^= c.time.simulation_ticks as u64;
        }
        clone = clone.min(t1.elapsed().as_secs_f64() * 1e6 / n as f64);
    }
    eprintln!("clone+step {per:.1} µs, clone {clone:.1} µs ({keep})");
    // `BOPL_SKIP_TIMING=1` skips the bound (e.g. on a loaded CI runner).
    if std::env::var_os("BOPL_SKIP_TIMING").is_none() {
        assert!(per < 2000.0, "clone+step {per} µs");
    }
}

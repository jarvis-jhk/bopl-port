//! All 29 abilities (plus Random) at once: 12 players on several levels of
//! the real import (skipped without one), level changes at the winner
//! screen, two independent runs compared tick for tick, a rollback
//! (clone, mispredict, restore, replay) compared with a straight run, and
//! a check that every ability was actually cast.
//!
//! Prints `FP <level> <tick> <hash>` checkpoints (`--nocapture`), used to
//! compare state hashes across refactors.

use std::sync::Arc;

use bopl_content::Loaded;
use bopl_sim::Fix;
use bopl_sim::input::{BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2, BTN_JUMP, PlayerInput};
use bopl_sim::player::PlayerSetup;
use bopl_sim::world::{MatchSettings, World};

fn loaded() -> Option<Loaded> {
    let dir = bopl_content::default_import_dir();
    if !dir.join("index.json").exists() {
        eprintln!("no import at {}; skipping", dir.display());
        return None;
    }
    Some(bopl_content::load(dir).expect("import loads"))
}

/// A deterministic pseudo-random input script per player (sticks change
/// every 7 ticks, buttons are held for runs of ticks so charged abilities
/// get released too).
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

/// 12 players × 3 slots covering every ability of the list (except the
/// null ability) at least once; `rot` rotates the assignment.
fn setups(l: &Loaded, rot: usize) -> Vec<PlayerSetup> {
    let all: Vec<usize> = (0..l.content.abilities.len())
        .filter(|&i| l.content.abilities[i].name != "NullAbility")
        .collect();
    let n = all.len();
    (0..12)
        .map(|i| PlayerSetup {
            id: i + 1,
            team: i % 6,
            color: i,
            abilities: (0..3)
                .map(|k| all[(i as usize * 3 + k + rot) % n])
                .collect(),
            uses_keyboard_and_mouse: i == 4,
        })
        .collect()
}

fn mk(l: &Loaded, level: &Arc<bopl_sim::data::LevelData>, rot: usize) -> World {
    let mut w = World::new(
        Arc::clone(&l.content),
        &setups(l, rot),
        MatchSettings {
            number_of_abilities: 3,
            seed: 11 + rot as u32,
        },
    );
    bopl_sim::session::load_level(&mut w, level.clone());
    w
}

/// A hash of what the players see (positions, life, scores), independent
/// of internal bookkeeping such as hierarchy numbers.
fn behaviour(w: &World) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |x: i64| {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for p in w.view().players {
        mix(p.position.x.0);
        mix(p.position.y.0);
        mix(p.alive as i64);
        mix(p.deaths as i64);
        mix(p.kills as i64);
    }
    h
}

/// Records casts: a slot timer that went down restarted its cooldown.
fn note_casts(w: &World, prev: &mut Vec<Vec<Fix>>, cast: &mut [u32]) {
    let v = w.view();
    if prev.len() != v.players.len() {
        *prev = v
            .players
            .iter()
            .map(|p| p.slots.iter().map(|s| s.timer).collect())
            .collect();
        return;
    }
    for (p, pv) in v.players.iter().zip(prev.iter_mut()) {
        for (k, s) in p.slots.iter().enumerate() {
            if k < pv.len() && s.timer < pv[k] && s.ability < cast.len() {
                cast[s.ability] += 1;
            }
            if k < pv.len() {
                pv[k] = s.timer;
            } else {
                pv.push(s.timer);
            }
        }
    }
}

#[test]
fn all_abilities_twelve_players_determinism_and_rollback() {
    let Some(l) = loaded() else {
        return;
    };
    let levels = [6usize, 17, 18, 22, 28, 40, 126];
    let lv: Vec<_> = levels
        .iter()
        .map(|&i| bopl_content::load_level(&l, i).unwrap())
        .collect();
    let mut cast = vec![0u32; l.content.abilities.len()];
    // Checkpoint spacing of the `FP` lines (`BOPL_FP_STEP`, default 500).
    let step: u64 = std::env::var("BOPL_FP_STEP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);
    for (start, &level) in levels.iter().enumerate() {
        let rot = start * 5;
        let (mut a, mut b) = (mk(&l, &lv[start], rot), mk(&l, &lv[start], rot));
        let mut prev = Vec::new();
        let mut cur = start;
        for t in 0..5000u64 {
            let inp = inputs(12, t, level as u64 * 31 + 7);
            a.step(&inp);
            b.step(&inp);
            note_casts(&a, &mut prev, &mut cast);
            if t % 50 == 0 {
                assert_eq!(a.state_hash(), b.state_hash(), "level {level} tick {t}");
            }
            if t % step == 0 {
                eprintln!(
                    "FP {level} {t} {:016x} {:016x}",
                    a.state_hash(),
                    behaviour(&a)
                );
            }
            // Next level at the winner screen.
            if a.round.winner_screen && a.time.sim_time_since_level_loaded > Fix::from_int(10) {
                cur = (cur + 1) % lv.len();
                bopl_sim::session::load_level(&mut a, lv[cur].clone());
                bopl_sim::session::load_level(&mut b, lv[cur].clone());
            }
        }
        assert_eq!(a.state_hash(), b.state_hash());
        assert_eq!(a.view(), b.view());
        eprintln!(
            "FP {level} end {:016x} deaths {} fault {:?}",
            a.state_hash(),
            a.players.iter().map(|p| p.deaths).sum::<i32>(),
            a.fault
        );
        // Rollback in the middle of the run: mispredicted inputs for 8
        // ticks, restore, replay the real ones.
        let real = |t: u64| inputs(12, t, level as u64 * 31 + 7);
        let mut straight = mk(&l, &lv[start], rot);
        for t in 0..1800u64 {
            straight.step(&real(t));
        }
        let mut rb = mk(&l, &lv[start], rot);
        for t in 0..1000u64 {
            rb.step(&real(t));
        }
        for k in 0..8u64 {
            let base = 1000 + k * 100;
            let snap = rb.clone();
            for t in base..base + 8 {
                rb.step(&inputs(12, t, 4242 + k));
            }
            rb = snap;
            for t in base..base + 100 {
                rb.step(&real(t));
            }
        }
        assert_eq!(
            rb.state_hash(),
            straight.state_hash(),
            "rollback level {level}"
        );
        assert_eq!(rb.view(), straight.view(), "rollback view level {level}");
    }
    let never: Vec<&str> = l
        .content
        .abilities
        .iter()
        .enumerate()
        // Random is resolved into another ability when the bopl spawns
        // (abilities.md §13.1); it is in the slot lists but never cast.
        .filter(|(i, a)| a.name != "NullAbility" && a.name != "Random" && cast[*i] == 0)
        .map(|(_, a)| a.name.as_str())
        .collect();
    eprintln!(
        "casts: {:?}",
        l.content
            .abilities
            .iter()
            .zip(&cast)
            .map(|(a, c)| format!("{} {c}", a.name))
            .collect::<Vec<_>>()
    );
    assert!(never.is_empty(), "never cast: {never:?}");
}

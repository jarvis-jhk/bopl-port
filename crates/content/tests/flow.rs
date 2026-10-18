//! Session flow on real levels (skipped without an import): level bag,
//! ability-select circles, the mid-game wheel and the winner-screen inputs.

use std::sync::Arc;

use bopl_content::Loaded;
use bopl_sim::flow::{self, Circle, FlowRequest};
use bopl_sim::input::{BTN_JUMP, BTN_SELECT, PlayerInput};
use bopl_sim::player::PlayerSetup;
use bopl_sim::view::Cue;
use bopl_sim::world::{MatchSettings, World};

fn loaded() -> Option<Loaded> {
    let dir = bopl_content::default_import_dir();
    if !dir.join("index.json").exists() {
        eprintln!("no import at {}; skipping", dir.display());
        return None;
    }
    Some(bopl_content::load(dir).expect("import loads"))
}

fn setup(id: i32, team: i32) -> PlayerSetup {
    PlayerSetup {
        id,
        team,
        color: id - 1,
        abilities: vec![4, 7, 2],
        uses_keyboard_and_mouse: false,
    }
}

fn session(l: &Loaded, players: &[PlayerSetup]) -> World {
    let mut w = World::new(
        Arc::clone(&l.content),
        players,
        MatchSettings {
            number_of_abilities: 3,
            seed: 7,
        },
    );
    flow::begin_session(&mut w, l.game.level_indices().len());
    let lv = bopl_content::load_level(l, 6).unwrap();
    flow::load_level(&mut w, 0, lv);
    w
}

const RIGHT: u8 = 1;
const LEFT: u8 = 128;
const UP: u8 = 65; // ≈ π/2

/// Walks off Level1's left edge into the water (the sandbox test's path).
fn suicide(t: i64) -> PlayerInput {
    match t {
        0..=219 => PlayerInput::default(),
        220..=300 => PlayerInput::new(RIGHT, 0),
        301..=345 => PlayerInput::new(LEFT, 0),
        346..=347 => PlayerInput::new(LEFT, BTN_JUMP),
        _ => PlayerInput::new(LEFT, 0),
    }
}

fn press(b: u8) -> PlayerInput {
    PlayerInput::new(0, b)
}

#[test]
fn solo_round_winner_screen_next_level_and_back() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = session(&l, &[setup(1, 0)]);
    let mut t = 0i64;
    while w.flow.winner_screen_at.is_none() {
        assert!(t < 3000, "no winner screen");
        w.step(&[suicide(t)]);
        t += 1;
    }
    // The solo player died (loser circle), then won: a winner circle, no wheel.
    assert!(matches!(
        w.flow.players[0].circle,
        Circle::Winner { slot: 0, .. }
    ));
    let next = w.flow.next.expect("next level drawn at round end");
    assert_ne!(next, 0, "Level1 is marked played");
    // Inputs are refused for 2 s after the winner screen appeared.
    w.step(&[press(BTN_JUMP)]);
    assert_eq!(w.flow.request, None);
    for _ in 0..flow::WINNER_INPUT_DELAY_TICKS {
        w.step(&[PlayerInput::default()]);
    }
    w.step(&[press(BTN_JUMP)]);
    assert_eq!(w.flow.request, Some(FlowRequest::NextLevel(next)));
    assert!(w.events.iter().any(|e| e.cue == Cue::Sound("startGame")));
    // Load it; the circles reset and the next seed differs.
    let seed1 = w.settings.seed;
    let bi = l.game.level_indices()[next];
    flow::load_level(&mut w, next, bopl_content::load_level(&l, bi).unwrap());
    assert_eq!(w.flow.levels_loaded, 2);
    assert_ne!(w.settings.seed, seed1);
    assert!(w.flow.players.iter().all(|p| p.circle == Circle::Hidden));
    assert_eq!(w.players[0].games_won, 1);
}

#[test]
fn loser_wheel_changes_abilities_and_blocks_next_level() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = session(&l, &[setup(1, 0), setup(2, 1)]);
    let mut t = 0i64;
    let idle = PlayerInput::default();
    // Player 1 jumps into the water, player 2 waits.
    while w.flow.winner_screen_at.is_none() {
        assert!(t < 3000, "no winner screen");
        w.step(&[suicide(t), idle]);
        t += 1;
    }
    assert!(matches!(
        w.flow.players[1].circle,
        Circle::Winner { slot: 0, .. }
    ));
    let Circle::Loser { since, slot } = w.flow.players[0].circle else {
        panic!(
            "player 1 has a loser circle: {:?}",
            w.flow.players[0].circle
        );
    };
    assert_eq!(slot, 1, "losers fill from the right");
    while w.time.simulation_ticks - since <= flow::WHEEL_DELAY_TICKS + 1 {
        w.step(&[idle; 2]);
    }
    // Open (stick up), release, change the top row (left = next ability).
    w.step(&[PlayerInput::new(UP, 0), idle]);
    assert!(w.flow.players[0].wheel.open);
    w.step(&[idle; 2]);
    let before = w.players[0].abilities[0];
    w.step(&[PlayerInput::new(LEFT, 0), idle]);
    assert_eq!(w.players[0].abilities[0], before + 1);
    w.step(&[idle; 2]);
    // Next level is refused while the wheel is open.
    for _ in 0..flow::WINNER_INPUT_DELAY_TICKS {
        w.step(&[idle; 2]);
    }
    w.step(&[idle, press(BTN_JUMP)]);
    assert_eq!(w.flow.request, None);
    assert!(w.flow.players[0].wiggle_at.is_some());
    assert!(w.events.iter().any(|e| e.cue == Cue::Sound("smallError")));
    // Close (jump), wait for the close animation, then back to select wins
    // over next level in the same tick.
    w.step(&[press(BTN_JUMP), idle]);
    assert!(!w.flow.players[0].wheel.open);
    for _ in 0..flow::WHEEL_CLOSE_TICKS {
        w.step(&[idle; 2]);
    }
    w.step(&[press(BTN_SELECT), press(BTN_JUMP)]);
    assert_eq!(w.flow.request, Some(FlowRequest::BackToSelect));
}

#[test]
fn flow_is_deterministic_and_rolls_back() {
    let Some(l) = loaded() else {
        return;
    };
    let input = |t: i64| {
        let a = if (500..700).contains(&t) {
            PlayerInput::new(UP, BTN_JUMP * ((t / 7) % 2) as u8)
        } else {
            PlayerInput::default()
        };
        [suicide(t), a]
    };
    let run = |n: i64| {
        let mut w = session(&l, &[setup(1, 0), setup(2, 1)]);
        for t in 0..n {
            w.step(&input(t));
        }
        w
    };
    let a = run(900);
    let b = run(900);
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(a.flow, b.flow);
    // Rollback: snapshot, mispredict, restore, replay = straight run.
    let mut w = run(450);
    let snap = w.clone();
    for t in 450..600 {
        w.step(&[
            PlayerInput::new(UP, BTN_JUMP),
            PlayerInput::new(LEFT, BTN_SELECT),
        ]);
        let _ = t;
    }
    let mut w = snap;
    for t in 450..900 {
        w.step(&input(t));
    }
    assert_eq!(w.state_hash(), a.state_hash());
}

/// The online leave rule (docs/net.md, `flow::set_gone`): a player who
/// left is inert (does not keep the round going, cannot win), does not
/// spawn in the next level, and the session goes back to character select
/// once fewer than two teams with connected players remain.
#[test]
fn players_who_left_are_inert_removed_and_end_the_session() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = session(&l, &[setup(1, 0), setup(2, 1), setup(3, 2)]);
    let mut t = 0i64;
    while w.flow.winner_screen_at.is_none() {
        assert!(t < 3000, "no winner screen");
        if t == 200 {
            flow::set_gone(&mut w, 2);
        }
        w.step(&[suicide(t), PlayerInput::default(), PlayerInput::default()]);
        assert!(w.flow.request.is_none(), "two connected teams remain");
        t += 1;
    }
    // Player 3 left while alive: team 1 (player 2) won alone.
    assert!(w.players[2].is_alive());
    assert_eq!(
        w.players.iter().map(|p| p.games_won).collect::<Vec<_>>(),
        [0, 1, 0]
    );
    // Next level: the player who left does not spawn.
    let at = w.flow.winner_screen_at.unwrap();
    while w.flow.request.is_none() {
        assert!(w.time.simulation_ticks < at + 400, "no next level");
        let next = if w.time.simulation_ticks % 2 == 0 {
            press(BTN_JUMP)
        } else {
            PlayerInput::default()
        };
        w.step(&[PlayerInput::default(), next, PlayerInput::default()]);
    }
    let Some(FlowRequest::NextLevel(bag)) = w.flow.request else {
        panic!("expected the next level, got {:?}", w.flow.request);
    };
    let build = l.game.level_indices()[bag];
    flow::load_level(&mut w, bag, bopl_content::load_level(&l, build).unwrap());
    for _ in 0..400 {
        w.step(&[PlayerInput::default(); 3]);
    }
    assert!(w.players[0].bopl.is_some() && w.players[1].bopl.is_some());
    assert!(w.players[2].bopl.is_none() && !w.players[2].is_alive());
    assert!(w.flow.request.is_none());
    // Player 1 leaves too: one team left, back to character select.
    flow::set_gone(&mut w, 0);
    w.step(&[PlayerInput::default(); 3]);
    assert_eq!(w.flow.request, Some(FlowRequest::BackToSelect));
}

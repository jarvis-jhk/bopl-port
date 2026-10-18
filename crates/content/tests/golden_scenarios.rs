//! Regression numbers for the golden scenarios (`scripts/dev/golden/`,
//! `docs/review/feel-pass.md`): each scenario's replay run through OUR sim
//! (with the replay viewer's input rule) must still produce the recorded
//! event ticks and end state. The numbers in `golden_expected.txt` come from
//! our sim only, after the scenario was compared with the original; when a
//! deliberate sim change moves them, re-run the comparison for the affected
//! scenarios and re-bless with `BOPL_GOLDEN_BLESS=1 cargo test --release -p
//! bopl-content --test golden_scenarios`. Skipped without an import.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use bopl_content::scenario::Scenario;
use bopl_sim::view::Cue;
use bopl_sim::world::World;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// One line per scenario: name, end tick, a trajectory hash (every player's
/// position, scale and life every 10 ticks: behaviour, not struct layout),
/// then the events.
fn run(loaded: &bopl_content::Loaded, name: &str, sc: &Scenario) -> String {
    let rep = sc.replay();
    let lv = bopl_content::load_level(loaded, rep.build_index()).unwrap();
    let mut w = World::new(Arc::clone(&loaded.content), &rep.setups(), rep.settings());
    bopl_sim::session::load_level(&mut w, lv);
    let mut last = Vec::new();
    let mut ev = String::new();
    let mut h = 0xcbf29ce484222325u64;
    let mut mix = |v: i64| {
        for b in v.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
    };
    let ticks = rep.frames.len() + 300;
    for t in 0..ticks {
        let inp = rep.viewer_inputs(t, &w, &mut last);
        w.step(&inp);
        let tick = w.time.simulation_ticks;
        if tick % 10 == 0 {
            for p in &w.players {
                mix(p.position.x.0);
                mix(p.position.y.0);
                mix(p.scale().0);
                mix(p.is_alive() as i64);
            }
        }
        for e in &w.events {
            match &e.cue {
                Cue::PlayerDied { id, .. } => write!(ev, " died{id}@{tick}").unwrap(),
                Cue::RoundWon { team } => write!(ev, " won{team}@{tick}").unwrap(),
                Cue::RoundDraw => write!(ev, " draw@{tick}").unwrap(),
                Cue::WinnerScreen => write!(ev, " winner@{tick}").unwrap(),
                Cue::Fault(_) => write!(ev, " fault@{tick}").unwrap(),
                _ => {}
            }
        }
    }
    format!("{name} {} {h:016x}{ev}", w.time.simulation_ticks)
}

#[test]
fn golden_scenarios_unchanged() {
    let dir = bopl_content::default_import_dir();
    if !dir.join("index.json").exists() {
        eprintln!("no import at {}; skipping", dir.display());
        return;
    }
    let loaded = bopl_content::load(dir).unwrap();
    let sdir = root().join("scripts/dev/golden/scenarios");
    let mut names: Vec<_> = std::fs::read_dir(&sdir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "txt"))
        .collect();
    names.sort();
    let mut got = String::new();
    for p in &names {
        let name = p.file_stem().unwrap().to_string_lossy().to_string();
        let sc = Scenario::parse(&std::fs::read_to_string(p).unwrap()).unwrap();
        got.push_str(&run(&loaded, &name, &sc));
        got.push('\n');
    }
    let exp_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden_expected.txt");
    if std::env::var("BOPL_GOLDEN_BLESS").is_ok() {
        std::fs::write(&exp_path, &got).unwrap();
        return;
    }
    let exp = std::fs::read_to_string(&exp_path).expect("tests/golden_expected.txt");
    let mut bad = Vec::new();
    for (g, e) in got.lines().zip(exp.lines()) {
        if g != e {
            bad.push(format!("expected {e}\n     got {g}"));
        }
    }
    assert_eq!(
        got.lines().count(),
        exp.lines().count(),
        "scenario count changed"
    );
    assert!(
        bad.is_empty(),
        "golden scenarios changed:\n{}",
        bad.join("\n")
    );
}

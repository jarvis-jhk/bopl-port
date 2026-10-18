//! `sim-trace`: runs a scripted input sequence (or a `.rep` replay) through
//! the simulation and prints per-tick player state as CSV.
//!
//! ```text
//! sim-trace [--data DIR] [--level BUILD_INDEX] [--players N] [--seed S]
//!           [--ticks N] [--script SCRIPT] [--replay FILE.rep] [--out FILE]
//!           [--every N] [--events]
//! ```
//!
//! SCRIPT is a `;`-separated list of `TICKS:KEYS` steps for player 1, keys
//! from `L R U D` (direction), `J` (jump), `1 2 3` (ability buttons), e.g.
//! `200:;40:R;1:RJ;30:R;60:`. Columns: tick, player, state, grounded,
//! platform, x, y (floats), x_raw, y_raw, vx, vy, alive, spawned.

use std::io::Write;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bopl_content::replay::Replay;
use bopl_sim::input::{
    BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2, BTN_JUMP, PlayerInput, angle_byte_from_direction,
};
use bopl_sim::player::PlayerSetup;
use bopl_sim::vec2::Vec2;
use bopl_sim::world::{Kind, MatchSettings, World};

fn parse_keys(k: &str) -> PlayerInput {
    let mut d = Vec2::ZERO;
    let mut b = 0;
    for c in k.chars() {
        match c {
            'L' => d.x -= bopl_sim::Fix::ONE,
            'R' => d.x += bopl_sim::Fix::ONE,
            'U' => d.y += bopl_sim::Fix::ONE,
            'D' => d.y -= bopl_sim::Fix::ONE,
            'J' => b |= BTN_JUMP,
            '1' => b |= BTN_ABILITY0,
            '2' => b |= BTN_ABILITY1,
            '3' => b |= BTN_ABILITY2,
            _ => {}
        }
    }
    PlayerInput::new(angle_byte_from_direction(d), b)
}

/// Expands a script into per-tick inputs for one player.
pub fn script(s: &str) -> Result<Vec<PlayerInput>> {
    let mut out = Vec::new();
    for step in s.split(';').filter(|x| !x.trim().is_empty()) {
        let (n, k) = step.split_once(':').unwrap_or((step, ""));
        let n: usize = n
            .trim()
            .parse()
            .with_context(|| format!("bad step {step}"))?;
        out.extend(std::iter::repeat_n(parse_keys(k.trim()), n));
    }
    Ok(out)
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut data = bopl_content::default_import_dir();
    let mut level = 6usize;
    let mut nplayers = 1usize;
    let mut seed = 1u32;
    let mut ticks = 900usize;
    let mut scr = String::new();
    let mut rep: Option<String> = None;
    let mut out: Option<String> = None;
    let mut every = 1usize;
    let mut events = false;
    while let Some(a) = args.next() {
        let mut v = || args.next().context("missing value");
        match a.as_str() {
            "--data" => data = v()?.into(),
            "--level" => level = v()?.parse()?,
            "--players" => nplayers = v()?.parse()?,
            "--seed" => seed = v()?.parse()?,
            "--ticks" => ticks = v()?.parse()?,
            "--script" => scr = v()?,
            "--replay" => rep = Some(v()?),
            "--out" => out = Some(v()?),
            "--every" => every = v()?.parse()?,
            "--events" => events = true,
            "-h" | "--help" => {
                println!("see the doc comment in crates/content/src/bin/sim-trace.rs");
                return Ok(());
            }
            _ => bail!("unknown argument {a}"),
        }
    }
    let loaded = bopl_content::load(&data)?;
    let mut setups: Vec<PlayerSetup> = (0..nplayers)
        .map(|i| PlayerSetup {
            id: i as i32 + 1,
            team: i as i32,
            color: i as i32,
            abilities: vec![4, 7, 2],
            uses_keyboard_and_mouse: false,
        })
        .collect();
    let mut settings = MatchSettings {
        number_of_abilities: 3,
        seed,
    };
    let mut frames: Vec<Vec<PlayerInput>> = Vec::new();
    let mut replay: Option<Replay> = None;
    if let Some(r) = &rep {
        let r = Replay::parse(&std::fs::read(r)?)?;
        level = r.build_index();
        settings = r.settings();
        setups = r.setups();
        for t in 0..r.frames.len() {
            frames.push(r.inputs(t));
        }
        if ticks == 900 {
            ticks = frames.len() + 120;
        }
        eprintln!(
            "replay: level {} seed {} players {:?} frames {}",
            level,
            r.seed,
            r.players
                .iter()
                .map(|p| (p.id, p.team, p.abilities))
                .collect::<Vec<_>>(),
            r.frames.len()
        );
        replay = Some(r);
    } else {
        let s = script(&scr)?;
        for t in 0..ticks {
            let mut f = vec![PlayerInput::default(); nplayers];
            f[0] = s.get(t).copied().unwrap_or_default();
            frames.push(f);
        }
    }
    let lv = bopl_content::load_level(&loaded, level)?;
    let mut w = World::new(Arc::clone(&loaded.content), &setups, settings);
    bopl_sim::session::load_level(&mut w, lv);
    let mut o: Box<dyn Write> = match &out {
        Some(p) => Box::new(std::io::BufWriter::new(std::fs::File::create(p)?)),
        None => Box::new(std::io::BufWriter::new(std::io::stdout())),
    };
    writeln!(
        o,
        "tick,player,state,grounded,platform,x,y,x_raw,y_raw,vx,vy,alive,spawned"
    )?;
    let mut last = Vec::new();
    for t in 0..ticks {
        let inputs = match &replay {
            // The replay viewer's rule for time stops (net-original.md §8).
            Some(r) => r.viewer_inputs(t, &w, &mut last),
            None => frames
                .get(t)
                .cloned()
                .unwrap_or_else(|| vec![PlayerInput::default(); setups.len()]),
        };
        w.step(&inputs);
        if events {
            for e in &w.events {
                writeln!(o, "# {} {:?}", e.id.tick, e.cue)?;
            }
        }
        if t % every != 0 {
            continue;
        }
        for p in &w.players {
            let b = p.bopl.and_then(|b| w.objects.get(b));
            let (state, grounded, plat, pos, vel, spawned) =
                match b.map(|o| (&o.kind, o.tf.position)) {
                    Some((Kind::Bopl(bp), pos)) => (
                        format!("{:?}", bp.ctrl.state),
                        bp.physics.is_grounded,
                        bp.physics
                            .attached_ground
                            .and_then(|g| w.platforms.iter().position(|x| *x == g))
                            .map_or(-1, |i| i as i64),
                        pos,
                        bp.body.velocity(),
                        bp.ctrl.has_spawned,
                    ),
                    _ => ("-".into(), false, -1, Vec2::ZERO, Vec2::ZERO, false),
                };
            writeln!(
                o,
                "{},{},{},{},{},{:.4},{:.4},{},{},{:.4},{:.4},{},{}",
                w.time.simulation_ticks,
                p.id,
                state,
                grounded as u8,
                plat,
                pos.x.to_f64(),
                pos.y.to_f64(),
                pos.x.0,
                pos.y.0,
                vel.x.to_f64(),
                vel.y.to_f64(),
                p.is_alive() as u8,
                spawned as u8
            )?;
        }
    }
    eprintln!(
        "end: tick {} round decided {} fault {:?} hash {:016x}",
        w.time.simulation_ticks,
        w.round.decided,
        w.fault,
        w.state_hash()
    );
    for p in &w.players {
        eprintln!(
            "player {}: alive {} deaths {} died at tick {} cause {:?} won {}",
            p.id,
            p.is_alive(),
            p.deaths,
            if p.timestamp_when_killed == i64::MAX {
                -1
            } else {
                p.timestamp_when_killed
            },
            p.cause_of_death,
            p.games_won
        );
    }
    Ok(())
}

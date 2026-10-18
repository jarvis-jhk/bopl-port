//! `bopl-xcheck`: runs a headless bot match on the simulation only (no
//! renderer, no SDL) and prints the complete state hash at regular ticks, so
//! the output of the Linux and the Windows build can be compared line by line
//! (`scripts/check-determinism.sh`). Online play between platforms relies on
//! these being equal.
//!
//! The bots are the game's own (`crates/game/src/bots.rs`, compiled in by
//! path) and the session follows `SimMatch` (next level when the flow asks
//! for it), so this is the same match as `bopl-port --headless --bot-match`
//! with the same `--players/--teams/--level/--seed`.
//!
//! ```text
//! bopl-xcheck --data DIR [--players 12] [--teams T] [--level 6] [--seed 1]
//!             [--ticks 6000] [--every 60]
//! ```
//!
//! Output: `lut <fnv64 of both lookup tables>`, then
//! `tick level checksum_raw state_hash` every `--every` ticks and at the
//! end, then `final <fnv64 over all lines>`.

#[allow(dead_code)]
#[path = "../../../crates/game/src/bots.rs"]
mod bots;

use std::hash::Hasher;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bopl_sim::flow::{self, FlowRequest};
use bopl_sim::input::PlayerInput;
use bopl_sim::player::PlayerSetup;
use bopl_sim::world::{Fnv64, MatchSettings, World};

/// Same as the game's `bot_setups` (game_app.rs).
fn bot_setups(n: usize, teams: Option<usize>, abilities: usize) -> Vec<PlayerSetup> {
    const COLORS: [i32; 12] = [0, 6, 2, 3, 4, 5, 1, 7, 8, 9, 10, 11];
    (0..n)
        .map(|i| PlayerSetup {
            id: i as i32 + 1,
            team: match teams {
                Some(t) => (i % t.max(1)) as i32,
                None => i as i32,
            },
            color: COLORS[i % 12],
            abilities: (0..3)
                .map(|k| 2 + (i * 3 + k * 7) % (abilities.max(3) - 2))
                .collect(),
            uses_keyboard_and_mouse: false,
        })
        .collect()
}

fn main() -> Result<()> {
    let mut data: Option<String> = None;
    let (mut players, mut teams, mut level, mut seed) = (12usize, None, 6usize, 1u32);
    let (mut ticks, mut every) = (6000u64, 60u64);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut v = || args.next().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--data" => data = Some(v()?),
            "--players" => players = v()?.parse()?,
            "--teams" => teams = Some(v()?.parse()?),
            "--level" => level = v()?.parse()?,
            "--seed" => seed = v()?.parse()?,
            "--ticks" => ticks = v()?.parse()?,
            "--every" => every = v()?.parse::<u64>()?.max(1),
            "-h" | "--help" => {
                println!("see the doc comment in scripts/xcheck/src/main.rs");
                return Ok(());
            }
            _ => bail!("unknown argument {a}"),
        }
    }
    let data = data
        .map(Into::into)
        .unwrap_or_else(bopl_content::default_import_dir);
    let loaded =
        bopl_content::load(&data).with_context(|| format!("loading {}", data.display()))?;
    let bag = loaded.game.level_indices();
    let mut out = Fnv64::default();
    let mut emit = |line: String| {
        println!("{line}");
        out.write(line.as_bytes());
        out.write(b"\n");
    };

    let luts = bopl_sim::lut::luts();
    let mut h = Fnv64::default();
    for v in luts.sin.iter().chain(luts.tan.iter()) {
        h.write(&v.to_le_bytes());
    }
    emit(format!("lut {:016x}", h.finish()));

    let setups = bot_setups(players, teams, loaded.content.abilities.len());
    let settings = MatchSettings {
        number_of_abilities: 3,
        seed,
    };
    let mut w = World::new(Arc::clone(&loaded.content), &setups, settings);
    flow::begin_session(&mut w, bag.len());
    let bag_index = |bi: usize| bag.iter().position(|&b| b == bi).unwrap_or(0);
    flow::load_level(
        &mut w,
        bag_index(level),
        bopl_content::load_level(&loaded, level)?,
    );
    let mut bots: Vec<bots::Bot> = (0..players)
        .map(|i| {
            let mut b = bots::Bot::new(i, seed);
            b.policy = bots::FlowPolicy::NextLevel;
            b
        })
        .collect();
    let mut view = w.view();
    for t in 1..=ticks {
        let inputs: Vec<PlayerInput> = bots.iter_mut().map(|b| b.input(&view)).collect();
        w.step(&inputs);
        match w.flow.request.take() {
            Some(FlowRequest::NextLevel(b)) => {
                level = bag.get(b).copied().unwrap_or(6);
                flow::load_level(&mut w, b, bopl_content::load_level(&loaded, level)?);
            }
            Some(FlowRequest::BackToSelect) => {
                emit(format!("{t} back-to-select"));
                break;
            }
            None => {}
        }
        view = w.view();
        if t % every == 0 || t == ticks {
            emit(format!(
                "{t} {level} {:016x} {:016x}",
                w.checksum().raw(),
                w.state_hash()
            ));
        }
    }
    let f = out.finish();
    println!("final {f:016x}");
    Ok(())
}

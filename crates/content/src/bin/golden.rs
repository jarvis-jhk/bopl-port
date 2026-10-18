//! `golden`: golden comparisons of our simulation with the original's replay
//! viewer (`docs/reference-capture.md`, "Golden comparisons";
//! `docs/review/feel-pass.md`).
//!
//! ```text
//! golden gen SCENARIO.txt OUT.rep          compile a scenario (bopl_content::scenario)
//! golden frames FILE.rep                   number of frames in a replay
//! golden sim FILE.rep OUT.csv [--ticks N] [--shift N] [--platforms F] [--cooldowns F]
//!                                          our sim: per tick and player, + OUT.csv.events
//! golden track VIDEO OUT.csv --ref REF.png --colors C1,C2,..
//!              [--water-y PX] [--tol T] [--open R] [--start S] [--frames N]
//!                                          track bopls (player colours) and the camera
//!                                          in a capture of the original
//! golden compare SIM.csv TRACK.csv [--out FRAMES.csv] [--thr UNITS] [--fit-frames N]
//!                                          align frames to ticks and report divergences
//! ```
//!
//! The capture must be 1920x1080 (camera size 30 → 18 px per unit, camera
//! y fixed at 8). Tracking finds, per player colour (`_ShadowColor` of the
//! player material, read from the import), the largest blob after a
//! morphological opening (removes slime trails and thin icons) and its
//! centroid; the camera x comes from matching the static platforms against
//! `REF.png` (our renderer's screenshot of the level at camera x = 0, e.g.
//! `bopl-port --headless --level L --camera-x 0 --time 4`).
//! Nothing from the original is stored by this tool outside the paths given.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bopl_content::replay::Replay;
use bopl_content::scenario::Scenario;
use bopl_sim::fix::Fix;
use bopl_sim::view::Cue;
use bopl_sim::world::{Kind, World};

const W: usize = 1920;
const H: usize = 1080;
const PPU: f64 = 18.0; // pixels per unit (1080 / (2 × 30))
const CAM_Y: f64 = 8.0;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        println!("see the doc comment in crates/content/src/bin/golden.rs");
        return Ok(());
    };
    let rest = &args[1..];
    match cmd.as_str() {
        "gen" => {
            let sc = Scenario::parse(&std::fs::read_to_string(&rest[0])?)
                .with_context(|| rest[0].clone())?;
            std::fs::write(&rest[1], sc.replay().write())?;
            eprintln!(
                "{}: level {} players {} ticks {}",
                rest[1],
                sc.level,
                sc.players.len(),
                sc.ticks
            );
        }
        "frames" => {
            let r = Replay::parse(&std::fs::read(&rest[0])?)?;
            println!("{}", r.frames.len());
        }
        "sim" => sim(rest)?,
        "track" => track(rest)?,
        "compare" => compare(rest)?,
        "-h" | "--help" => println!("see the doc comment in crates/content/src/bin/golden.rs"),
        _ => bail!("unknown command {cmd}"),
    }
    Ok(())
}

fn opt<'a>(a: &'a [String], name: &str) -> Option<&'a str> {
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1))
        .map(String::as_str)
}

// -- our sim ----------------------------------------------------------------------

/// The original's camera follow (levels.md §7) at a fixed frame time.
fn camera_step(x: f64, alive_xs: &[f64], dt: f64) -> f64 {
    let (mut lo, mut hi) = (0.0f64, 0.0f64);
    for &v in alive_xs {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let target = (lo + hi) * 0.5;
    let d = dt.min(0.0667) * 1.5;
    let x = (1.0 - d) * x + d * target;
    let w = 30.0 * 16.0 / 9.0;
    let x = x.clamp(-97.27 + w + 1.0, 97.6 - w - 1.0);
    (x * 60.0).round() / 60.0
}

fn sim(a: &[String]) -> Result<()> {
    let r = Replay::parse(&std::fs::read(&a[0])?)?;
    let out = &a[1];
    let ticks: usize = match opt(a, "--ticks") {
        Some(t) => t.parse()?,
        None => r.frames.len() + 600,
    };
    let loaded = bopl_content::load(bopl_content::default_import_dir())?;
    let lv = bopl_content::load_level(&loaded, r.build_index())?;
    let mut w = World::new(Arc::clone(&loaded.content), &r.setups(), r.settings());
    bopl_sim::session::load_level(&mut w, lv);
    let mut o = std::io::BufWriter::new(std::fs::File::create(out)?);
    let mut ev = std::io::BufWriter::new(std::fs::File::create(format!("{out}.events"))?);
    writeln!(o, "tick,player,alive,x,y,bx,by,bopl,scale,cam,upx,upy")?;
    let mut plat = match opt(a, "--platforms") {
        Some(p) => {
            let mut f = std::io::BufWriter::new(std::fs::File::create(p)?);
            writeln!(f, "tick,platform,x,y,rot,scale")?;
            Some(f)
        }
        None => None,
    };
    let mut cds = match opt(a, "--cooldowns") {
        Some(p) => {
            let mut f = std::io::BufWriter::new(std::fs::File::create(p)?);
            writeln!(f, "tick,player,slot,ability,timer,cooldown")?;
            Some(f)
        }
        None => None,
    };
    let mut cam = 0.0f64;
    let mut frozen = false;
    let mut stopped = false;
    let shift: i64 = opt(a, "--shift").map_or(Ok(0), str::parse)?;
    let mut last = Vec::new();
    for t in 0..ticks {
        // --shift N: frame t is applied N ticks later (timing experiments).
        let ft = t as i64 - shift;
        let inp = if ft < 0 {
            vec![bopl_sim::input::PlayerInput::default(); r.players.len()]
        } else {
            r.viewer_inputs(ft as usize, &w, &mut last)
        };
        w.step(&inp);
        let tick = w.time.simulation_ticks;
        if w.is_time_stopped() != stopped {
            stopped = w.is_time_stopped();
            writeln!(
                ev,
                "{tick} time_{}",
                if stopped { "stopped" } else { "resumed" }
            )?;
        }
        for e in &w.events {
            match &e.cue {
                Cue::PlayerSpawned { id } => writeln!(ev, "{tick} spawned {id}")?,
                Cue::PlayerDied { id, killer, cause } => {
                    writeln!(ev, "{tick} died {id} killer {killer} {cause:?}")?
                }
                Cue::RoundWon { team } => writeln!(ev, "{tick} round_won {team}")?,
                Cue::RoundDraw => writeln!(ev, "{tick} round_draw")?,
                Cue::WinnerScreen => {
                    writeln!(ev, "{tick} winner_screen")?;
                    frozen = true;
                }
                Cue::SuddenDeath => writeln!(ev, "{tick} sudden_death")?,
                Cue::Fault(f) => writeln!(ev, "{tick} fault {f:?}")?,
                Cue::Shake(s) => writeln!(ev, "{tick} shake {s}")?,
                _ => {}
            }
        }
        if let Some(f) = cds.as_mut() {
            for pv in w.view().players {
                for (k, sl) in pv.slots.iter().enumerate() {
                    writeln!(
                        f,
                        "{tick},{},{k},{},{:.4},{:.4}",
                        pv.id,
                        sl.ability,
                        sl.timer.to_f64(),
                        sl.cooldown.to_f64()
                    )?;
                }
            }
        }
        if let Some(f) = plat.as_mut() {
            for pv in w.view().platforms {
                writeln!(
                    f,
                    "{tick},{},{:.4},{:.4},{:.4},{:.4}",
                    pv.index,
                    pv.position.x.to_f64(),
                    pv.position.y.to_f64(),
                    pv.rotation.to_f64(),
                    pv.scale.to_f64()
                )?;
            }
        }
        let alive: Vec<f64> = w
            .players
            .iter()
            .filter(|p| p.is_alive())
            .map(|p| p.position.x.to_f64())
            .collect();
        if !frozen {
            cam = camera_step(cam, &alive, 1.0 / 60.0);
        }
        for p in &w.players {
            let b = p.bopl.and_then(|b| w.objects.get(b));
            let (bp, active, up) = match b {
                Some(o) if matches!(o.kind, Kind::Bopl(_)) => (o.tf.position, o.active, o.tf.up),
                _ => (
                    p.position,
                    false,
                    bopl_sim::vec2::Vec2::new(Fix::ZERO, Fix::ONE),
                ),
            };
            // While in an ability the bopl object is inactive and the ability
            // object (at the player's position) is what is drawn.
            let (bp, up) = if active {
                (bp, up)
            } else {
                (p.position, bopl_sim::vec2::Vec2::new(Fix::ZERO, Fix::ONE))
            };
            writeln!(
                o,
                "{tick},{},{},{:.5},{:.5},{:.5},{:.5},{},{:.4},{:.4},{:.4},{:.4}",
                p.id,
                p.is_alive() as u8,
                p.position.x.to_f64(),
                p.position.y.to_f64(),
                bp.x.to_f64(),
                bp.y.to_f64(),
                active as u8,
                p.scale().to_f64(),
                cam,
                up.x.to_f64(),
                up.y.to_f64()
            )?;
        }
    }
    eprintln!(
        "{out}: {ticks} ticks, round decided {} fault {:?} hash {:016x}",
        w.round.decided,
        w.fault,
        w.state_hash()
    );
    Ok(())
}

// -- tracking ---------------------------------------------------------------------

/// `_ShadowColor` (the visible body colour) of each player colour, from the import.
fn player_colors() -> Result<Vec<[u8; 3]>> {
    let dir = bopl_content::default_import_dir();
    let g: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("globals.json"))?)?;
    let m: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("materials.json"))?)?;
    let mut out = Vec::new();
    for c in g["player_colors"]["fields"]["playerColors"]
        .as_array()
        .context("player colours")?
    {
        let r = c["playerMaterial"]["$ref"].as_str().unwrap_or("");
        let col = &m[r]["colors"]["_ShadowColor"];
        let f = |i: usize| (col[i].as_f64().unwrap_or(0.0) * 255.0).round() as u8;
        out.push([f(0), f(1), f(2)]);
    }
    Ok(out)
}

fn read_png_rgb(path: &str) -> Result<Vec<u8>> {
    let o = Command::new("ffmpeg")
        .args([
            "-loglevel",
            "error",
            "-i",
            path,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()?;
    if o.stdout.len() != W * H * 3 {
        bail!("{path}: expected a {W}x{H} image");
    }
    Ok(o.stdout)
}

struct CamMatcher {
    /// (x, y, rgb) samples of static platform pixels in the reference.
    pts: Vec<(i32, i32, [u8; 3])>,
}

impl CamMatcher {
    /// Static-geometry samples of the reference `r` (camera x 0). With a
    /// second reference `r1` at camera x `cam1`, a sample is kept where the
    /// pixel moves with the camera (platforms) and not where it stays
    /// (parallax backdrop, UI); without it, by colour (grass levels).
    fn new(r: &[u8], r1: Option<(&[u8], f64)>, water_y: usize) -> CamMatcher {
        let px = |img: &[u8], x: usize, y: usize| {
            let i = (y * W + x) * 3;
            [img[i] as i32, img[i + 1] as i32, img[i + 2] as i32]
        };
        let dist = |a: [i32; 3], b: [i32; 3]| {
            (a[0] - b[0])
                .abs()
                .max((a[1] - b[1]).abs())
                .max((a[2] - b[2]).abs())
        };
        let mut pts = Vec::new();
        for y in (60..water_y.min(H - 2)).step_by(4) {
            for x in (2..W - 2).step_by(4) {
                let c = px(r, x, y);
                let keep = match r1 {
                    Some((r1, cam1)) => {
                        let sh = (cam1 * PPU).round() as i64;
                        let x1 = x as i64 - sh;
                        let edge = dist(px(r, x + 2, y), px(r, x - 2, y))
                            + dist(px(r, x, y + 2), px(r, x, y - 2))
                            > 25;
                        edge && x1 >= 0
                            && x1 < W as i64
                            && dist(c, px(r1, x1 as usize, y)) < 14
                            && dist(c, px(r1, x, y)) > 30
                    }
                    None => c[2] < c[0].max(c[1]) - 10 || c[0] + c[1] + c[2] < 200,
                };
                if keep {
                    pts.push((x as i32, y as i32, [c[0] as u8, c[1] as u8, c[2] as u8]));
                }
            }
        }
        CamMatcher { pts }
    }

    /// Matching samples when the scene is shifted left by `s` pixels.
    fn score(&self, f: &[u8], s: i32) -> (u32, u32) {
        let (mut ok, mut n) = (0, 0);
        for &(x, y, c) in &self.pts {
            let fx = x - s;
            if fx < 0 || fx >= W as i32 {
                continue;
            }
            n += 1;
            let i = (y as usize * W + fx as usize) * 3;
            let d = (f[i] as i32 - c[0] as i32)
                .abs()
                .max((f[i + 1] as i32 - c[1] as i32).abs())
                .max((f[i + 2] as i32 - c[2] as i32).abs());
            if d < 40 {
                ok += 1;
            }
        }
        (ok, n)
    }

    /// Best shift near `prev` (±`range`), sub-pixel, and its match fraction.
    fn find(&self, f: &[u8], prev: i32, range: i32) -> (f64, f64) {
        let mut best = (prev, -1.0f64);
        let mut sc = BTreeMap::new();
        for s in prev - range..=prev + range {
            let (ok, n) = self.score(f, s);
            let v = if n > 0 { ok as f64 / n as f64 } else { 0.0 };
            sc.insert(s, v);
            if v > best.1 {
                best = (s, v);
            }
        }
        let (s, v) = best;
        let l = *sc.get(&(s - 1)).unwrap_or(&v);
        let r = *sc.get(&(s + 1)).unwrap_or(&v);
        let den = l - 2.0 * v + r;
        let sub = if den < -1e-9 {
            (0.5 * (l - r) / den).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        (s as f64 + sub, v)
    }
}

const CELL: usize = 3;
/// Blobs reported per colour (largest first).
const NBLOB: usize = 6;
/// CSV fields per blob in the track file.
const BLOB_FIELDS: usize = 11;
/// Frames whose camera match is below this are not compared (loading,
/// fades, winner screen); spikes, boulders and smoke over the platforms
/// lower it to about 0.5 in play.
const MIN_FIT: f64 = 0.35;
const CW: usize = W / CELL;
const CH: usize = H / CELL;

struct Blob {
    cx: f64,
    cy: f64,
    area: usize,
    /// Pixel bounding box (inclusive).
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
    /// Caps: mean x of the pixels within `CAP` of the top and of the bottom
    /// edge, mean y of those near the left and the right edge (the bopl's
    /// head when it stands on the opposite side; trails lie on the surface).
    caps: [f64; 4],
}

const CAP: usize = 14;

/// Square erosion/dilation of a cell mask with radius `r` (separable).
fn morph(m: &[bool], r: usize, erode: bool) -> Vec<bool> {
    let mut tmp = vec![false; m.len()];
    for y in 0..CH {
        for x in 0..CW {
            let (lo, hi) = (x.saturating_sub(r), (x + r).min(CW - 1));
            let row = &m[y * CW + lo..=y * CW + hi];
            tmp[y * CW + x] = if erode {
                x >= r && x + r < CW && row.iter().all(|&b| b)
            } else {
                row.iter().any(|&b| b)
            };
        }
    }
    let mut out = vec![false; m.len()];
    for y in 0..CH {
        let (lo, hi) = (y.saturating_sub(r), (y + r).min(CH - 1));
        for x in 0..CW {
            let mut v = erode;
            for yy in lo..=hi {
                let b = tmp[yy * CW + x];
                if erode && !b {
                    v = false;
                    break;
                }
                if !erode && b {
                    v = true;
                    break;
                }
            }
            if erode && (y < r || y + r >= CH) {
                v = false;
            }
            out[y * CW + x] = v;
        }
    }
    out
}

/// Connected components (4-neighbour) of a cell mask: lists of cells.
fn components(m: &[bool]) -> Vec<Vec<usize>> {
    let mut seen = vec![false; m.len()];
    let mut out = Vec::new();
    for s in 0..m.len() {
        if !m[s] || seen[s] {
            continue;
        }
        let mut comp = Vec::new();
        let mut stack = vec![s];
        seen[s] = true;
        while let Some(c) = stack.pop() {
            comp.push(c);
            let (x, y) = (c % CW, c / CW);
            let mut push = |n: usize| {
                if m[n] && !seen[n] {
                    seen[n] = true;
                    stack.push(n);
                }
            };
            if x > 0 {
                push(c - 1);
            }
            if x + 1 < CW {
                push(c + 1);
            }
            if y > 0 {
                push(c - CW);
            }
            if y + 1 < CH {
                push(c + CW);
            }
        }
        out.push(comp);
    }
    out
}

/// The largest blobs of one colour (largest first).
fn blobs(f: &[u8], col: [u8; 3], tol: i32, water_y: usize, open_r: usize) -> (Vec<Blob>, usize) {
    let mut pix = vec![false; W * H];
    let mut cnt = vec![0u8; CW * CH];
    let mut total = 0;
    let t2 = tol * tol;
    for y in 0..water_y.min(H) {
        for x in 0..W {
            if x < 110 && y < 110 {
                continue; // the "?" button
            }
            let i = (y * W + x) * 3;
            let dr = f[i] as i32 - col[0] as i32;
            let dg = f[i + 1] as i32 - col[1] as i32;
            let db = f[i + 2] as i32 - col[2] as i32;
            if dr * dr + dg * dg + db * db < t2 {
                pix[y * W + x] = true;
                cnt[(y / CELL) * CW + x / CELL] += 1;
                total += 1;
            }
        }
    }
    let m: Vec<bool> = cnt.iter().map(|&c| c >= 5).collect();
    // Close (fills the eyes inside the body ring), then open (removes thin
    // slime trails and icon strokes).
    let closed = morph(&morph(&m, open_r + 1, false), open_r + 1, true);
    let closed: Vec<bool> = closed.iter().zip(&m).map(|(a, b)| *a || *b).collect();
    let opened = morph(&morph(&closed, open_r, true), open_r, false);
    let opened: Vec<bool> = opened.iter().zip(&m).map(|(a, b)| *a && *b).collect();
    // Group opened pieces that are close (body, eyes and splat of one bopl).
    let group = |mask: &[bool]| -> Vec<Vec<usize>> {
        let near = morph(mask, 3, false);
        let mut label = vec![usize::MAX; mask.len()];
        let comps = components(&near);
        for (i, c) in comps.iter().enumerate() {
            for &cell in c {
                label[cell] = i;
            }
        }
        let mut out = vec![Vec::new(); comps.len()];
        for (cell, &on) in mask.iter().enumerate() {
            if on && label[cell] != usize::MAX {
                out[label[cell]].push(cell);
            }
        }
        out.retain(|c| !c.is_empty());
        out
    };
    let mut comps: Vec<Vec<usize>> = group(&opened);
    if comps.is_empty() {
        comps = group(&m).into_iter().filter(|c| c.len() >= 4).collect();
    }
    // Bopls first: groups with eye whites (near-white pixels inside the
    // bounding box; slime trails on platforms have none), then by size.
    comps.retain(|c| c.len() >= 3);
    let mut keyed: Vec<(bool, usize, Vec<usize>)> = comps
        .into_iter()
        .map(|c| {
            let (mut x0, mut x1, mut y0, mut y1) = (CW, 0, CH, 0);
            for &cell in &c {
                x0 = x0.min(cell % CW);
                x1 = x1.max(cell % CW);
                y0 = y0.min(cell / CW);
                y1 = y1.max(cell / CW);
            }
            let mut white = 0;
            for y in y0 * CELL..(y1 + 1) * CELL {
                for x in x0 * CELL..(x1 + 1) * CELL {
                    let i = (y * W + x) * 3;
                    if f[i] > 225 && f[i + 1] > 225 && f[i + 2] > 225 {
                        white += 1;
                    }
                }
            }
            (white >= 12, c.len(), c)
        })
        .collect();
    keyed.sort_by_key(|k| std::cmp::Reverse((k.0, k.1)));
    let comps: Vec<Vec<usize>> = keyed.into_iter().map(|k| k.2).collect();
    let mut out = Vec::new();
    for c in comps.into_iter().take(NBLOB) {
        let (mut sx, mut sy, mut n) = (0.0, 0.0, 0usize);
        let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
        for &cell in &c {
            let (cx0, cy0) = ((cell % CW) * CELL, (cell / CW) * CELL);
            for y in cy0..cy0 + CELL {
                for x in cx0..cx0 + CELL {
                    if pix[y * W + x] {
                        sx += x as f64 + 0.5;
                        sy += y as f64 + 0.5;
                        n += 1;
                        x0 = x0.min(x);
                        x1 = x1.max(x);
                        y0 = y0.min(y);
                        y1 = y1.max(y);
                    }
                }
            }
        }
        if n > 0 {
            let mut acc = [(0.0f64, 0usize); 4];
            for &cell in &c {
                let (cx0, cy0) = ((cell % CW) * CELL, (cell / CW) * CELL);
                for y in cy0..cy0 + CELL {
                    for x in cx0..cx0 + CELL {
                        if !pix[y * W + x] {
                            continue;
                        }
                        let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                        if y <= y0 + CAP {
                            acc[0].0 += fx;
                            acc[0].1 += 1;
                        }
                        if y + CAP >= y1 {
                            acc[1].0 += fx;
                            acc[1].1 += 1;
                        }
                        if x <= x0 + CAP {
                            acc[2].0 += fy;
                            acc[2].1 += 1;
                        }
                        if x + CAP >= x1 {
                            acc[3].0 += fy;
                            acc[3].1 += 1;
                        }
                    }
                }
            }
            out.push(Blob {
                cx: sx / n as f64,
                cy: sy / n as f64,
                area: n,
                x0,
                x1,
                y0,
                y1,
                caps: acc.map(|(v, k)| v / k.max(1) as f64),
            });
        }
    }
    (out, total)
}

fn track(a: &[String]) -> Result<()> {
    let video = &a[0];
    let out = &a[1];
    let refp = opt(a, "--ref").context("--ref REF.png")?;
    let colors: Vec<usize> = opt(a, "--colors")
        .context("--colors")?
        .split(',')
        .map(|s| s.parse())
        .collect::<Result<_, _>>()?;
    let water_y: usize = opt(a, "--water-y").map_or(Ok(H), str::parse)?;
    let tol: i32 = opt(a, "--tol").map_or(Ok(40), str::parse)?;
    let open_r: usize = opt(a, "--open").map_or(Ok(2), str::parse)?;
    let start = opt(a, "--start").unwrap_or("0");
    let palette = player_colors()?;
    let cols: Vec<[u8; 3]> = colors.iter().map(|&c| palette[c % palette.len()]).collect();
    let r0 = read_png_rgb(refp)?;
    let r1 = match opt(a, "--ref2") {
        Some(p) => Some((
            read_png_rgb(p)?,
            opt(a, "--ref2-cam").map_or(Ok(10.0), str::parse)?,
        )),
        None => None,
    };
    let cam = CamMatcher::new(&r0, r1.as_ref().map(|(i, c)| (i.as_slice(), *c)), water_y);
    eprintln!("camera samples: {}", cam.pts.len());
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-loglevel", "error", "-ss", start, "-i", video]);
    if let Some(n) = opt(a, "--frames") {
        cmd.args(["-frames:v", n]);
    }
    cmd.args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"]);
    let mut child = cmd.stdout(Stdio::piped()).spawn()?;
    let mut rd = BufReader::with_capacity(1 << 24, child.stdout.take().unwrap());
    let mut o = std::io::BufWriter::new(std::fs::File::create(out)?);
    write!(o, "frame,dup,cam,camfit")?;
    for c in &colors {
        write!(o, ",c{c}total")?;
        for j in 0..NBLOB {
            write!(
                o,
                ",c{c}_{j}x,c{c}_{j}y,c{c}_{j}a,c{c}_{j}x0,c{c}_{j}x1,c{c}_{j}y0,c{c}_{j}y1,\
                 c{c}_{j}tx,c{c}_{j}bx,c{c}_{j}ly,c{c}_{j}ry"
            )?;
        }
    }
    writeln!(o)?;
    let mut f = vec![0u8; W * H * 3];
    let mut prev_hash = 0u64;
    let mut prev_shift: Option<i32> = None;
    let mut k = 0usize;
    while rd.read_exact(&mut f).is_ok() {
        let mut h = 0xcbf29ce484222325u64;
        for b in f.iter().step_by(97) {
            h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
        }
        let dup = h == prev_hash;
        prev_hash = h;
        // Full search only every 20 frames while lost (menus, fly-in, winner screen).
        let (s, fit) = match prev_shift {
            Some(p) => {
                let (s, fit) = cam.find(&f, p, 24);
                if fit < 0.5 && k.is_multiple_of(20) {
                    cam.find(&f, 0, 800)
                } else {
                    (s, fit)
                }
            }
            None => cam.find(&f, 0, 800),
        };
        prev_shift = Some(s.round() as i32);
        write!(o, "{k},{},{:.4},{:.3}", dup as u8, s / PPU, fit)?;
        let found: Vec<(Vec<Blob>, usize)> = std::thread::scope(|sc| {
            let hs: Vec<_> = cols
                .iter()
                .map(|&c| {
                    let f = &f;
                    sc.spawn(move || blobs(f, c, tol, water_y, open_r))
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (bl, total) in found {
            write!(o, ",{total}")?;
            for j in 0..NBLOB {
                match bl.get(j) {
                    Some(b) => write!(
                        o,
                        ",{:.2},{:.2},{},{},{},{},{},{:.2},{:.2},{:.2},{:.2}",
                        b.cx,
                        b.cy,
                        b.area,
                        b.x0,
                        b.x1,
                        b.y0,
                        b.y1,
                        b.caps[0],
                        b.caps[1],
                        b.caps[2],
                        b.caps[3]
                    )?,
                    None => write!(o, ",,,0,,,,,,,,")?,
                }
            }
        }
        writeln!(o)?;
        k += 1;
        if k.is_multiple_of(600) {
            eprintln!("{video}: {k} frames");
        }
    }
    let _ = child.wait();
    eprintln!("{out}: {k} frames");
    Ok(())
}

// -- comparison -------------------------------------------------------------------

fn read_csv(path: &str) -> Result<(Vec<String>, Vec<Vec<String>>)> {
    let f = BufReader::new(std::fs::File::open(path).with_context(|| path.to_string())?);
    let mut lines = f.lines();
    let head: Vec<String> = lines
        .next()
        .context("empty csv")??
        .split(',')
        .map(String::from)
        .collect();
    let mut rows = Vec::new();
    for l in lines {
        let l = l?;
        if l.starts_with('#') || l.is_empty() {
            continue;
        }
        rows.push(l.split(',').map(String::from).collect());
    }
    Ok((head, rows))
}

#[derive(Clone, Copy, Default)]
struct SimP {
    alive: bool,
    x: f64,
    y: f64,
    scale: f64,
    cam: f64,
    ux: f64,
    uy: f64,
}

/// A tracked blob in world units: centroid, bounding box and caps.
#[derive(Clone, Copy)]
struct BlobW {
    cx: f64,
    cy: f64,
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
    /// x of the top and bottom caps, y of the left and right caps.
    caps: [f64; 4],
}

struct TrackFrame {
    cam: f64,
    fit: f64,
    /// Per player (list order): the blobs of its colour.
    p: Vec<Vec<BlobW>>,
}

/// Visual offset of the blob's extreme point along the bopl's up axis
/// (slime splats lie on the surface side, so the far side is stable).
#[derive(Clone, Copy)]
struct Model {
    k_up: f64,
    /// Only the vertical extreme (camera independent; for levels without
    /// static platforms, where the camera cannot be measured).
    y_only: bool,
}

/// (extreme along the up axis − position along it, head − position across
/// it). Up axes within 30° of a screen axis use that bounding-box edge and the
/// cap centroid next to it (the head, away from the slime trail on the
/// surface); in between, the support of the ellipse inscribed in the box and
/// the centroid.
fn features(b: &BlobW, p: &SimP) -> (f64, f64) {
    let n = (p.ux * p.ux + p.uy * p.uy).sqrt().max(1e-9);
    let (ux, uy) = (p.ux / n, p.uy / n);
    let c30 = 0.866;
    if uy >= c30 {
        return (b.top - p.y, b.caps[0] - p.x);
    }
    if uy <= -c30 {
        return (p.y - b.bottom, p.x - b.caps[1]);
    }
    if ux >= c30 {
        return (b.right - p.x, p.y - b.caps[3]);
    }
    if ux <= -c30 {
        return (p.x - b.left, b.caps[2] - p.y);
    }
    let (bx, by) = ((b.left + b.right) * 0.5, (b.top + b.bottom) * 0.5);
    let (hw, hh) = ((b.right - b.left) * 0.5, (b.top - b.bottom) * 0.5);
    let support = bx * ux + by * uy + ((hw * ux).powi(2) + (hh * uy).powi(2)).sqrt();
    let along = support - (p.x * ux + p.y * uy);
    let across = (b.cx - p.x) * -uy + (b.cy - p.y) * ux;
    (along, across)
}

impl Model {
    fn dist1(&self, b: &BlobW, p: &SimP) -> Option<f64> {
        if self.y_only {
            let n = (p.ux * p.ux + p.uy * p.uy).sqrt().max(1e-9);
            let uy = p.uy / n;
            if uy.abs() < 0.7 {
                return None;
            }
            let e = if uy > 0.0 {
                b.top - p.y
            } else {
                p.y - b.bottom
            };
            return Some((e - self.k_up * p.scale).abs());
        }
        let (e, q) = features(b, p);
        let de = e - self.k_up * p.scale;
        Some((de * de + q * q).sqrt())
    }

    /// Distance of the best blob and its index.
    fn dist(&self, bs: &[BlobW], p: &SimP) -> Option<(f64, usize)> {
        bs.iter()
            .enumerate()
            .filter_map(|(i, b)| self.dist1(b, p).map(|d| (d, i)))
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
    }
}

struct Cmp {
    sim: Vec<Vec<SimP>>,
    tr: Vec<TrackFrame>,
    np: usize,
    y_only: bool,
}

impl Cmp {
    fn sp(&self, t: i64, pi: usize) -> Option<&SimP> {
        if t < 1 {
            return None;
        }
        self.sim.get(t as usize)?.get(pi).filter(|p| p.alive)
    }

    /// Whether the sim position is inside frame `k`'s view (off-screen bopls
    /// only show as arrows in their colour, which must not count).
    fn on_screen(&self, k: usize, p: &SimP) -> bool {
        let half_w = 30.0 * 16.0 / 9.0 - 1.0;
        (p.y - CAM_Y).abs() < 29.0 && (self.y_only || (p.x - self.tr[k].cam).abs() < half_w)
    }
}

fn compare(a: &[String]) -> Result<()> {
    let (sh, srows) = read_csv(&a[0])?;
    let (th, trows) = read_csv(&a[1])?;
    let thr: f64 = opt(a, "--thr").map_or(Ok(0.6), str::parse)?;
    let size = opt(a, "--size").unwrap_or("1920x1080");
    let (w, h) = size.split_once('x').context("--size WxH")?;
    let (w, h): (f64, f64) = (w.parse()?, h.parse()?);
    let ppu = h / 60.0;
    let col = |hd: &[String], n: &str| hd.iter().position(|x| x == n);
    let c = |n: &str| col(&sh, n).with_context(|| format!("sim column {n}"));
    let cols = [
        c("tick")?,
        c("player")?,
        c("alive")?,
        c("bx")?,
        c("by")?,
        c("scale")?,
        c("cam")?,
        c("upx")?,
        c("upy")?,
    ];
    let mut sim: Vec<Vec<SimP>> = Vec::new();
    let mut ids: Vec<i64> = Vec::new();
    for r in &srows {
        let t: usize = r[cols[0]].parse()?;
        let id: i64 = r[cols[1]].parse()?;
        if !ids.contains(&id) {
            ids.push(id);
        }
        let pi = ids.iter().position(|&x| x == id).unwrap();
        while sim.len() <= t {
            sim.push(Vec::new());
        }
        while sim[t].len() <= pi {
            sim[t].push(SimP::default());
        }
        let g = |i: usize| -> Result<f64> { Ok(r[cols[i]].parse()?) };
        sim[t][pi] = SimP {
            alive: r[cols[2]] == "1",
            x: g(3)?,
            y: g(4)?,
            scale: g(5)?,
            cam: g(6)?,
            ux: g(7)?,
            uy: g(8)?,
        };
    }
    let np = ids.len();
    // Track columns: c<colour>total, then c<colour>_<j>{x,y,a,x0,x1,y0,y1}.
    let tot: Vec<usize> = th
        .iter()
        .enumerate()
        .filter(|(_, n)| n.ends_with("total"))
        .map(|(i, _)| i)
        .collect();
    if tot.len() < np {
        bail!("track has {} colours, sim {np} players", tot.len());
    }
    let (tcam, tfit) = (col(&th, "cam").unwrap(), col(&th, "camfit").unwrap());
    let amin = 100.0 * (ppu / 18.0).powi(2);
    let amax = 60000.0 * (ppu / 18.0).powi(2);
    let mut tr = Vec::new();
    for r in &trows {
        let cam: f64 = r[tcam].parse()?;
        let wx = |px: f64| cam + (px - w / 2.0) / ppu;
        let wy = |py: f64| CAM_Y + (h / 2.0 - py) / ppu;
        let mut p = Vec::new();
        for &ti in tot.iter().take(np) {
            let mut bs = Vec::new();
            for j in 0..NBLOB {
                let b = ti + 1 + BLOB_FIELDS * j;
                let area: f64 = r[b + 2].parse().unwrap_or(0.0);
                if area < amin || area > amax {
                    continue;
                }
                let v = |i: usize| -> Result<f64> { Ok(r[b + i].parse()?) };
                bs.push(BlobW {
                    cx: wx(v(0)?),
                    cy: wy(v(1)?),
                    left: wx(v(3)?),
                    right: wx(v(4)? + 1.0),
                    top: wy(v(5)?),
                    bottom: wy(v(6)? + 1.0),
                    caps: [wx(v(7)?), wx(v(8)?), wy(v(9)?), wy(v(10)?)],
                });
            }
            p.push(bs);
        }
        tr.push(TrackFrame {
            cam,
            fit: r[tfit].parse()?,
            p,
        });
    }
    if a.iter().any(|x| x == "--y-only") {
        for f in &mut tr {
            f.fit = 1.0;
        }
    }
    let cmp = Cmp {
        sim,
        tr,
        np,
        y_only: a.iter().any(|x| x == "--y-only"),
    };
    let nf = cmp.tr.len();
    // Ticks of the spawn animation are skipped (the visual grows; scale 1 in the sim).
    let first_alive = (1..cmp.sim.len()).find(|&t| cmp.sim[t].iter().any(|p| p.alive));
    let skip_to: i64 = match opt(a, "--skip-ticks") {
        Some(v) => v.parse()?,
        None => first_alive.map_or(0, |t| t as i64 + 110),
    };
    // 1. Monotonic alignment of frames to ticks (dynamic time warping): the
    //    viewer runs 0..4 ticks per rendered frame and falls behind real time
    //    when the original renders below 15 fps, so no fixed offset fits.
    //    Two passes: the second with the visual model calibrated on the first.
    let y_only = a.iter().any(|x| x == "--y-only");
    let fps: f64 = opt(a, "--fps").map_or(Ok(60.0), str::parse)?;
    let mut m = Model { k_up: 0.9, y_only };
    let mut path = Vec::new();
    // --map FILE: a given frame,tick alignment (scripts/dev/golden/refine.py
    // matches whole frames against our renders) instead of the warping.
    let given: Option<Vec<(usize, i64)>> = match opt(a, "--map") {
        Some(p) => {
            let (_, rows) = read_csv(p)?;
            Some(
                rows.iter()
                    .filter_map(|r| Some((r.first()?.parse().ok()?, r.get(1)?.parse().ok()?)))
                    .filter(|&(k, _): &(usize, i64)| k < nf)
                    .collect(),
            )
        }
        None => None,
    };
    for pass in 0..2 {
        path = match &given {
            Some(g) => g.clone(),
            None => align(&cmp, m, 60.0 / fps),
        };
        let mut ks = Vec::new();
        for &(k, t) in &path {
            for pi in 0..np {
                if let Some(p) = cmp.sp(t, pi)
                    && t >= skip_to
                    && cmp.on_screen(k, p)
                    && let Some((_, i)) = m.dist(&cmp.tr[k].p[pi], p)
                {
                    let b = &cmp.tr[k].p[pi][i];
                    let e = if y_only {
                        if p.uy >= 0.0 {
                            b.top - p.y
                        } else {
                            p.y - b.bottom
                        }
                    } else {
                        features(b, p).0
                    };
                    ks.push(e / p.scale);
                }
            }
        }
        if pass == 0 && !ks.is_empty() {
            m.k_up = median(&mut ks);
        }
    }
    // cs[k] = k − tick(k), interpolated between aligned frames.
    let mut cs = vec![i64::MIN; nf];
    for &(k, t) in &path {
        cs[k] = k as i64 - t;
    }
    let mut last = path.first().map_or(0, |&(k, t)| k as i64 - t);
    for c in cs.iter_mut() {
        if *c == i64::MIN {
            *c = last;
        } else {
            last = *c;
        }
    }
    // Playback rate: ticks per second of capture over 2 s windows.
    let mut rates = Vec::new();
    let win_frames = (2.0 * fps) as usize;
    let mut j = 0;
    for i in 0..path.len() {
        while j < path.len() && path[j].0 < path[i].0 + win_frames {
            j += 1;
        }
        if j < path.len() && j > i {
            let (k0, t0) = path[i];
            let (k1, t1) = path[j - 1];
            if k1 > k0 + win_frames / 2 {
                rates.push((t1 - t0) as f64 / ((k1 - k0) as f64 / fps));
            }
        }
    }
    let mut rs = rates.clone();
    let rmed = median(&mut rs);
    let rmin = rs.first().copied().unwrap_or(0.0);
    println!(
        "fit: {} frames aligned (ticks {}..{}); playback {rmed:.1} ticks/s median, slowest 2 s {rmin:.1}{}; visual model: extreme along up = position + {:.3} × scale; checks from tick {skip_to}",
        path.len(),
        path.first().map_or(0, |p| p.1),
        path.last().map_or(0, |p| p.1),
        if rmin < 45.0 {
            " (SLOW: the original fell behind real time)"
        } else {
            ""
        },
        m.k_up
    );
    // 3. Per frame residuals at the best tick within ±3.
    let mut out = match opt(a, "--out") {
        Some(p) => Some(std::io::BufWriter::new(std::fs::File::create(p)?)),
        None => None,
    };
    if let Some(o) = out.as_mut() {
        write!(o, "frame,tick,cam,simcam,fit")?;
        for id in &ids {
            write!(
                o,
                ",p{id}seen,p{id}bx,p{id}by,p{id}sx,p{id}sy,p{id}d,p{id}dt"
            )?;
        }
        writeln!(o)?;
    }
    let mut first_div: Vec<Option<(usize, i64, f64)>> = vec![None; np];
    let mut win: Vec<Vec<(f64, usize, i64)>> = vec![Vec::new(); np];
    let mut all_resid: Vec<Vec<f64>> = vec![Vec::new(); np];
    let mut maxd_before = vec![0.0f64; np];
    let mut resid: Vec<Vec<f64>> = vec![Vec::new(); np];
    let mut last_seen: Vec<Option<i64>> = vec![None; np];
    let mut seen_dead: Vec<usize> = vec![0; np];
    let mut cam_d: Vec<f64> = Vec::new();
    for (k, f) in cmp.tr.iter().enumerate() {
        let t0 = k as i64 - cs[k];
        if t0 < 1 || t0 as usize >= cmp.sim.len() || f.fit <= MIN_FIT {
            continue;
        }
        let simcam = cmp.sim[t0 as usize].first().map_or(0.0, |p| p.cam);
        cam_d.push(f.cam - simcam);
        if let Some(o) = out.as_mut() {
            write!(o, "{k},{t0},{:.4},{simcam:.4},{:.3}", f.cam, f.fit)?;
        }
        for pi in 0..np {
            let mut row = (0u8, 0.0, 0.0, 0.0, 0.0, -1.0, 0i64);
            if !f.p[pi].is_empty() {
                last_seen[pi] = Some(t0);
                let mut b = (f64::MAX, 0i64, 0usize);
                for dt in -3i64..=3 {
                    if let Some(p) = cmp.sp(t0 + dt, pi)
                        && cmp.on_screen(k, p)
                        && let Some((d, i)) = m.dist(&f.p[pi], p)
                        && d < b.0
                    {
                        b = (d, dt, i);
                    }
                }
                if b.0 < f64::MAX {
                    let p = cmp.sp(t0 + b.1, pi).unwrap();
                    let bl = &f.p[pi][b.2];
                    row = (1, bl.cx, bl.cy, p.x, p.y, b.0, b.1);
                    if t0 >= skip_to {
                        all_resid[pi].push(b.0);
                    }
                    if first_div[pi].is_none() && t0 >= skip_to {
                        // Diverged when the median of the last 15 sightings
                        // exceeds the threshold (single-frame tracking spikes
                        // and animation squash do not count).
                        resid[pi].push(b.0);
                        win[pi].push((b.0, k, t0 + b.1));
                        if win[pi].len() > 30 {
                            win[pi].remove(0);
                        }
                        let mut v: Vec<f64> = win[pi].iter().map(|w| w.0).collect();
                        if win[pi].len() == 30 && median(&mut v) > thr {
                            let (d, k1, t1) =
                                *win[pi].iter().find(|w| w.0 > thr).unwrap_or(&win[pi][0]);
                            first_div[pi] = Some((k1, t1, d));
                        } else if b.0 <= thr {
                            maxd_before[pi] = maxd_before[pi].max(b.0);
                        }
                    }
                } else {
                    row.0 = 2;
                    seen_dead[pi] += 1;
                }
            }
            if let Some(o) = out.as_mut() {
                write!(
                    o,
                    ",{},{:.3},{:.3},{:.3},{:.3},{:.3},{}",
                    row.0, row.1, row.2, row.3, row.4, row.5, row.6
                )?;
            }
        }
        if let Some(o) = out.as_mut() {
            writeln!(o)?;
        }
    }
    let mut sim_death: Vec<Option<i64>> = vec![None; np];
    for t in 1..cmp.sim.len() {
        for (pi, d) in sim_death.iter_mut().enumerate() {
            if d.is_none()
                && cmp.sim[t - 1].get(pi).is_some_and(|p| p.alive)
                && cmp.sim[t].get(pi).is_some_and(|p| !p.alive)
            {
                *d = Some(t as i64);
            }
        }
    }
    if !cam_d.is_empty() {
        let mut v: Vec<f64> = cam_d.iter().map(|x| x.abs()).collect();
        let med = median(&mut v);
        println!(
            "camera: |measured − modelled at 60 fps| median {med:.3}, max {:.3} units (the original's follow depends on its frame rate)",
            v.last().unwrap()
        );
    }
    for pi in 0..np {
        let div = match first_div[pi] {
            Some((k, t, d)) => format!("DIVERGES at tick {t} (frame {k}, {d:.2} units)"),
            None => "no divergence".into(),
        };
        let mut r = resid[pi].clone();
        let med = median(&mut r);
        // Death: the last frame (tick via the alignment) with a blob within
        // 1.5 units of the position the sim had just before its death.
        let vanish = sim_death[pi].map(|td| {
            let pd = cmp.sim[(td - 1).max(1) as usize][pi];
            let mut last = None;
            for (k, f) in cmp.tr.iter().enumerate() {
                let t0 = k as i64 - cs[k];
                if t0 < td - 90 || t0 > td + 60 || f.fit <= MIN_FIT {
                    continue;
                }
                let near = f.p[pi].iter().any(|b| {
                    let mut q = pd;
                    q.ux = 0.0;
                    q.uy = 1.0;
                    m.dist1(b, &pd).is_some_and(|d| d < 1.5)
                        || m.dist1(b, &q).is_some_and(|d| d < 1.5)
                });
                if near {
                    last = Some(t0);
                }
            }
            last
        });
        let a = &all_resid[pi];
        let n = a.len().max(1) as f64;
        println!(
            "player {}: {div}; residual median {med:.3}, max (≤ thr) before {:.3}; last seen tick {:?}; sim death tick {:?} (original: last sighting near it at tick {:?}); seen while dead in sim: {} frames",
            ids[pi],
            maxd_before[pi],
            last_seen[pi],
            sim_death[pi],
            vanish.flatten(),
            seen_dead[pi]
        );
        println!(
            "score p{}: n {} mean {:.3} >1 {:.1}% >2 {:.1}%",
            ids[pi],
            a.len(),
            a.iter().sum::<f64>() / n,
            100.0 * a.iter().filter(|&&d| d > 1.0).count() as f64 / n,
            100.0 * a.iter().filter(|&&d| d > 2.0).count() as f64 / n
        );
    }
    Ok(())
}

/// Dynamic time warping of the usable frames (camera measured, something
/// tracked) to ticks: per frame the cost is the sum over players of the capped
/// distance to the sim at that tick; between frames the tick advances by
/// 0..=5 × (ticks per frame) × gap, with a small penalty for leaving real
/// time. Returns (frame, tick) pairs.
fn align(cmp: &Cmp, m: Model, r: f64) -> Vec<(usize, i64)> {
    let frames: Vec<usize> = (0..cmp.tr.len())
        .filter(|&k| cmp.tr[k].fit > MIN_FIT && cmp.tr[k].p.iter().any(|b| !b.is_empty()))
        .collect();
    let nt = cmp.sim.len();
    if frames.is_empty() || nt < 2 {
        return Vec::new();
    }
    let cost = |k: usize, t: usize| -> f32 {
        // Seen but dead in the sim: full cap; alive but not seen (tracking
        // miss or hidden): a quarter of it.
        let mut c = 0.0f64;
        for pi in 0..cmp.np {
            let seen = !cmp.tr[k].p[pi].is_empty();
            match cmp.sp(t as i64, pi) {
                Some(p) if !cmp.on_screen(k, p) => {}
                Some(p) => {
                    c += match m.dist(&cmp.tr[k].p[pi], p) {
                        // Dead zone: pose-dependent blob biases (< 0.6, e.g.
                        // a bopl on a wall merged with its trail) must not
                        // pull the alignment away from real time.
                        Some((d, _)) => (d - 0.6).clamp(0.0, 1.4),
                        None if seen => 0.0,
                        None => 0.5,
                    }
                }
                None if seen => c += 1.4,
                None => {}
            }
        }
        c as f32
    };
    let lambda = 1.0f32;
    let mut prev: Vec<f32> = (0..nt).map(|t| cost(frames[0], t)).collect();
    let mut back: Vec<Vec<u8>> = vec![vec![0u8; nt]];
    for u in 1..frames.len() {
        let g = (frames[u] - frames[u - 1]) as f64;
        let dmax = ((5.0 * r * g).ceil() as usize).clamp(1, 255);
        let expect = (r * g) as f32;
        let mut cur = vec![f32::MAX; nt];
        let mut bk = vec![0u8; nt];
        for t in 0..nt {
            let mut best = (f32::MAX, 0u8);
            for d in 0..=dmax.min(t) {
                let v = prev[t - d] + lambda * (d as f32 - expect).abs();
                if v < best.0 {
                    best = (v, d as u8);
                }
            }
            cur[t] = best.0 + cost(frames[u], t);
            bk[t] = best.1;
        }
        prev = cur;
        back.push(bk);
    }
    let mut t = (0..nt)
        .min_by(|&a, &b| prev[a].partial_cmp(&prev[b]).unwrap())
        .unwrap();
    let mut path = vec![(frames[frames.len() - 1], t as i64)];
    for u in (1..frames.len()).rev() {
        t -= back[u][t] as usize;
        path.push((frames[u - 1], t as i64));
    }
    path.reverse();
    path
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

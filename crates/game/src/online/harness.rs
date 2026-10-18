//! Headless multi-process test of online play: real game processes (no
//! window, no GPU, no audio) play a session with bots over a Warpzone
//! server and compare their final checksums.
//!
//! ```sh
//! scripts/net/dev-server.sh &          # http://127.0.0.1:3024, udp 3504
//! export WARPZONE_SERVER=http://127.0.0.1:3024
//! bopl-port --net-harness --locals 3x4 --levels 2
//! bopl-port --net-harness --locals 12x1 --delay 40 --jitter 10 --loss 3
//! ```
//!
//! Orchestrator (`--net-harness`): starts machine 0 as host of a public
//! room (`--private`: unlisted, with a join code); the others join with
//! `--warpzone-join CODE` (odd machines, or all in private rooms) or
//! `--warpzone-join <room id>` (the launcher path); `--spectators N` more
//! machines watch. Every machine (`--net-peer`) plays its local slots with
//! bots ([`crate::bots::Bot`]: they fight, losers use the ability wheel,
//! they press "next level" on the winner screen). After `--levels N`
//! levels the host's first player picks "back to character select" on the
//! winner screen, which ends the session in the sim; each player reports
//! the confirmed end frame and checksum through `finish_match` and prints a
//! `RESULT` line. `SUMMARY PASS`: every remaining machine reported, all
//! checksums equal, the host saw agreement, no desync. `--leave M@F` /
//! `--crash M@F` make machine M leave / exit at frame F (the sim's leave
//! rule: inert, removed, back to character select when fewer than two
//! teams remain); `--desync M@F` corrupts machine M's state at frame F
//! (PASS = the desync is detected and reported).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bopl_net::warpzone_core::client::{Event, HostOptions};
use bopl_net::warpzone_core::protocol::api::{Entry, Visibility};
use bopl_net::{LinkSim, MatchSettings, NetConfig, NetEvent, NetSession, SlotRequest, TeamMode};
use bopl_sim::PlayerInput;
use bopl_sim::input::BTN_SELECT;

use super::adapter::{NetWorld, OnlineSettings, SlotExtra, new_world};
use super::runner::OnlineMatch;
use crate::bots::{Bot, FlowPolicy};
use crate::sim_match::LevelCache;

const STEP: Duration = Duration::from_micros(16_667);
const DEFAULT_SERVER: &str = "http://127.0.0.1:3024";

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn num<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    arg(args, name)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// `"3x4"` (3 machines with 4 slots) or `"2,4,6"`.
pub fn parse_locals(s: &str) -> Vec<usize> {
    if let Some((m, n)) = s.split_once('x') {
        return vec![n.parse().unwrap_or(1); m.parse().unwrap_or(2)];
    }
    s.split(',').filter_map(|x| x.trim().parse().ok()).collect()
}

fn parse_at(s: Option<String>) -> Option<(usize, i32)> {
    let (m, f) = s?
        .split_once('@')
        .map(|(a, b)| (a.to_string(), b.to_string()))?;
    Some((m.parse().ok()?, f.parse().ok()?))
}

fn root() -> PathBuf {
    std::env::var_os("BOPL_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

/// `--net-harness ...`: run the machines, compare, print `SUMMARY`.
pub fn orchestrate(args: &[String]) -> Result<i32> {
    let locals = parse_locals(&arg(args, "--locals").unwrap_or_else(|| "2x1".into()));
    let spectators: usize = num(args, "--spectators", 0);
    let tag = arg(args, "--tag").unwrap_or_else(|| "h".into());
    let leave = parse_at(arg(args, "--leave"));
    let crash = parse_at(arg(args, "--crash"));
    let desync = parse_at(arg(args, "--desync"));
    let timeout: u64 = num(args, "--timeout", 900);
    let server = std::env::var("WARPZONE_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.into());
    let exe = std::env::current_exe()?;
    let homes = root().join("scratch/net/homes");
    let n = locals.len() + spectators;
    let mut pass: Vec<String> = Vec::new();
    for k in [
        "--delay",
        "--jitter",
        "--loss",
        "--levels",
        "--teams",
        "--first-level",
        "--max-frames",
        "--input-delay",
        "--data",
        "--abilities",
    ] {
        if let Some(v) = arg(args, k) {
            pass.push(k.into());
            pass.push(v);
        }
    }
    for k in ["--relay", "--private"] {
        if flag(args, k) {
            pass.push(k.into());
        }
    }
    println!(
        "net-harness: {n} machines, slots {locals:?}, {spectators} spectators, {} ({server})",
        pass.join(" ")
    );
    let (tx, rx) = mpsc::channel::<(usize, String)>();
    let mut children: Vec<Child> = Vec::new();
    let total_slots: usize = locals.iter().sum();
    let spawn = |i: usize, extra: Vec<String>, children: &mut Vec<Child>| -> Result<()> {
        let home = homes.join(format!("{tag}-m{i}"));
        std::fs::create_dir_all(&home)?;
        let mut c = Command::new(&exe);
        let first_slot: usize = locals.iter().take(i).sum();
        c.arg("--net-peer").args([
            "--index",
            &i.to_string(),
            "--slots",
            &locals.get(i).copied().unwrap_or(0).to_string(),
            "--first-slot",
            &first_slot.to_string(),
            "--total-slots",
            &total_slots.to_string(),
        ]);
        c.args(&pass).args(extra);
        for (opt, at) in [
            ("--leave-at", leave),
            ("--crash-at", crash),
            ("--desync-at", desync),
        ] {
            if let Some((m, f)) = at
                && m == i
            {
                c.args([opt, &f.to_string()]);
            }
        }
        c.env("WARPZONE_HOME", &home)
            .env("WARPZONE_SERVER", &server)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = c.spawn().context("spawn peer")?;
        let out = child.stdout.take().context("stdout")?;
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send((i, line));
            }
        });
        children.push(child);
        Ok(())
    };
    spawn(
        0,
        vec!["--host".into(), "--expect".into(), n.to_string()],
        &mut children,
    )?;
    let t0 = Instant::now();
    let (mut room, mut code) = (String::new(), String::new());
    while room.is_empty() {
        match rx.recv_timeout(Duration::from_secs(90)) {
            Ok((i, l)) => {
                println!("[m{i}] {l}");
                if let Some(rest) = l.strip_prefix("ROOM ") {
                    let mut it = rest.split_whitespace();
                    room = it.next().unwrap_or_default().into();
                    code = it.next().unwrap_or_default().into();
                }
                if l.starts_with("ERROR") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    if room.is_empty() {
        eprintln!("net-harness: the host opened no room (is scripts/net/dev-server.sh running?)");
        for c in &mut children {
            let _ = c.kill();
        }
        return Ok(1);
    }
    for i in 1..n {
        let by_code = code != "-" && (flag(args, "--private") || i % 2 == 1);
        let extra = vec![
            "--warpzone-join".into(),
            if by_code { code.clone() } else { room.clone() },
        ];
        spawn(i, extra, &mut children)?;
    }
    drop(tx);
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let mut results: Vec<(usize, String)> = Vec::new();
    while let Ok((i, l)) = rx.recv_timeout(
        deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1)),
    ) {
        println!("[m{i}] {l}");
        if l.starts_with("RESULT") {
            results.push((i, l));
        } else if l.starts_with("ERROR") {
            break; // fail fast
        }
        if Instant::now() > deadline {
            break;
        }
    }
    for c in &mut children {
        let _ = c.kill();
        let _ = c.wait();
    }
    let field = |l: &str, k: &str| {
        l.split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{k}=")).map(String::from))
            .unwrap_or_default()
    };
    let expected: Vec<usize> = (0..n)
        .filter(|i| leave.map(|l| l.0) != Some(*i) && crash.map(|c| c.0) != Some(*i))
        .collect();
    let mut got: Vec<usize> = results.iter().map(|r| r.0).collect();
    got.sort();
    let mut sums: Vec<String> = results.iter().map(|(_, l)| field(l, "checksum")).collect();
    sums.sort();
    sums.dedup();
    let sum = |k: &str| -> u64 {
        results
            .iter()
            .map(|(_, l)| field(l, k).parse::<u64>().unwrap_or(0))
            .sum()
    };
    let agreed = results.iter().all(|(_, l)| field(l, "agreed") == "true");
    // With `--desync M@F` (machine M corrupts its state at frame F) the
    // test passes when the desync was detected.
    let ok = if desync.is_some() {
        got == expected && sum("desyncs") > 0
    } else {
        got == expected && sums.len() == 1 && agreed && sum("desyncs") == 0
    };
    let first = results
        .iter()
        .find(|(i, _)| *i == 0)
        .or(results.first())
        .map(|r| r.1.clone())
        .unwrap_or_default();
    println!(
        "SUMMARY {} machines={n} reported={} checksums_equal={} agreed={agreed} frame={} levels={} scores={} capped={} rollbacks={} resim={} maxdepth={} desyncs={} resumes={} time={:.1}s",
        if ok { "PASS" } else { "FAIL" },
        got.len(),
        sums.len() == 1,
        field(&first, "frame"),
        field(&first, "levels"),
        field(&first, "scores"),
        field(&first, "capped"),
        sum("rollbacks"),
        sum("resim"),
        results
            .iter()
            .map(|(_, l)| field(l, "maxdepth").parse::<u64>().unwrap_or(0))
            .max()
            .unwrap_or(0),
        sum("desyncs"),
        sum("resumes"),
        t0.elapsed().as_secs_f32()
    );
    Ok(if ok { 0 } else { 1 })
}

fn say(s: impl AsRef<str>) {
    println!("{}", s.as_ref());
}

/// Abilities for test slot `g` (varied, deterministic, never the null
/// entry).
fn test_abilities(g: usize, count: usize) -> [usize; 3] {
    let span = count.saturating_sub(2).max(1);
    [0, 1, 2].map(|k| 2 + (g * 3 + k * 7) % span)
}

/// `--net-peer ...`: one machine.
pub fn peer(args: &[String], levels: Rc<LevelCache>) -> Result<i32> {
    let index: usize = num(args, "--index", 0);
    let nslots: usize = num(args, "--slots", 1);
    let first_slot: usize = num(args, "--first-slot", 0);
    let host = flag(args, "--host");
    let expect: usize = num(args, "--expect", 2);
    let leave_at: Option<i32> = arg(args, "--leave-at").and_then(|v| v.parse().ok());
    let crash_at: Option<i32> = arg(args, "--crash-at").and_then(|v| v.parse().ok());
    let mut desync_at: Option<i32> = arg(args, "--desync-at").and_then(|v| v.parse().ok());
    let want_levels: u32 = num(args, "--levels", 2);
    let teams: Option<usize> = arg(args, "--teams").and_then(|v| v.parse().ok());
    let max_frames: Option<i32> = arg(args, "--max-frames").and_then(|v| v.parse().ok());
    let ability_count = levels.content.abilities.len();
    let mut cfg = NetConfig::new(&super::net_version());
    // Flags win over BOPL_NET_DELAY_MS / _JITTER_MS / _LOSS.
    if ["--delay", "--jitter", "--loss"]
        .iter()
        .any(|k| flag(args, k))
    {
        cfg.link_sim = LinkSim {
            delay_ms: num(args, "--delay", 0u32),
            jitter_ms: num(args, "--jitter", 0u32),
            loss: num(args, "--loss", 0.0f32) / 100.0,
        };
    }
    cfg.force_relay = cfg.force_relay || flag(args, "--relay");
    cfg.content_id = super::local_content_id(&levels.data);
    if let Some(d) = arg(args, "--input-delay").and_then(|v| v.parse().ok()) {
        cfg.ggrs.input_delay = d;
    }
    let home = std::env::var("WARPZONE_HOME").unwrap_or_default();
    let h = home
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let mut name = format!("bopl{index}x{:04x}", h & 0xffff);
    cfg.auto_register = Some(name.clone());
    let mut net: NetSession<NetWorld> = NetSession::new(cfg).map_err(|e| anyhow::anyhow!("{e}"))?;
    let slots: Vec<SlotRequest> = (0..nslots)
        .map(|k| {
            let g = first_slot + k;
            SlotRequest {
                name: format!("M{index}P{}", k + 1),
                colour: (g % 12) as u8,
                team: teams.map(|t| (g % t.max(1)) as u8),
                extra: SlotExtra {
                    abilities: test_abilities(g, ability_count),
                    mouse: false,
                }
                .encode(),
            }
        })
        .collect();
    net.set_local_slots(slots);
    net.set_spectate(nslots == 0);
    net.set_ready(true);
    if host {
        let os = OnlineSettings {
            number_of_abilities: num(args, "--abilities", 3u8),
            first_level: arg(args, "--first-level").and_then(|v| v.parse().ok()),
        };
        let _ = net.set_settings(MatchSettings {
            team_mode: TeamMode::Teams(12),
            mode: match teams {
                Some(t) => format!("{t} teams"),
                None => "free-for-all".into(),
            },
            map: "Level1".into(),
            game: os.encode(),
        });
    }
    let t0 = Instant::now();
    let mut logged_in = false;
    let mut requested = false;
    let mut started = false;
    let mut m: Option<OnlineMatch> = None;
    let mut bots: Vec<(usize, Bot)> = Vec::new();
    let mut ended: Option<(Instant, bool)> = None;
    let mut next = Instant::now();
    let mut last_log = Instant::now();
    let mut last_levels = 0u32;
    loop {
        if t0.elapsed() > Duration::from_secs(1800) {
            say("ERROR timeout");
            return Ok(1);
        }
        net.poll();
        while let Some(e) = net.next_event() {
            match e {
                NetEvent::Warpzone(b) => match *b {
                    Event::LoggedIn {
                        account: Some(a), ..
                    } => {
                        say(format!("logged in as {}", a.username));
                        logged_in = true;
                    }
                    Event::Registered(a) => {
                        say(format!("registered {}", a.username));
                        logged_in = true;
                    }
                    Event::Error { op, code, message } => {
                        say(format!("warpzone error {op}/{code}: {message}"));
                        if op == "register" {
                            name = format!(
                                "{}{}",
                                &name[..name.len().min(12)],
                                t0.elapsed().as_nanos() % 1000
                            );
                            net.register(&name);
                        } else if op == "join" || op == "host" {
                            say("ERROR room");
                            return Ok(1);
                        }
                    }
                    Event::RoomHosted { room, code, .. } => say(format!(
                        "ROOM {} {}",
                        room.id,
                        code.unwrap_or_else(|| "-".into())
                    )),
                    _ => {}
                },
                NetEvent::Welcome { machine } => say(format!("welcome as {machine}")),
                NetEvent::MatchStarted(ms) => {
                    say(format!(
                        "match {} started: {} players, {} spectators, local {:?}{}",
                        ms.match_id,
                        ms.players.len(),
                        ms.spectators.len(),
                        net.local_slots(),
                        if net.is_spectator() {
                            " (spectating)"
                        } else {
                            ""
                        }
                    ));
                    let world = match new_world(&ms, levels.clone()) {
                        Ok(w) => w,
                        Err(e) => {
                            say(format!("ERROR {e:#}"));
                            return Ok(1);
                        }
                    };
                    let mut om = OnlineMatch::new(world);
                    om.stop_at = max_frames;
                    m = Some(om);
                    bots = net
                        .local_slots()
                        .iter()
                        .map(|&s| {
                            let mut b = Bot::new(s, (ms.seed as u32) ^ (index as u32 * 7919));
                            b.policy = FlowPolicy::NextLevel;
                            (s, b)
                        })
                        .collect();
                    next = Instant::now();
                }
                NetEvent::Running => say("running"),
                NetEvent::PlayersDropped {
                    machines,
                    slots,
                    from_frame,
                } => say(format!(
                    "resumed at frame {from_frame} without {machines:?} (slots {slots:?})"
                )),
                NetEvent::Desync { frame, machine, .. } => {
                    say(format!("DESYNC at frame {frame} with {machine}"))
                }
                NetEvent::MatchEnded {
                    agreed, reports, ..
                } => {
                    say(format!(
                        "match ended: {} reports, agreed={agreed}",
                        reports.len()
                    ));
                    ended = Some((Instant::now(), agreed));
                }
                NetEvent::MatchAborted { reason } | NetEvent::RoomLeft { reason } => {
                    if ended.is_none() {
                        say(format!("ERROR aborted: {reason}"));
                        return Ok(1);
                    }
                }
                NetEvent::Rejected { reason } => {
                    say(format!("ERROR rejected: {reason}"));
                    return Ok(1);
                }
                _ => {}
            }
        }
        if logged_in && !requested {
            requested = true;
            if host {
                net.host(HostOptions {
                    max_players: 12,
                    visibility: if flag(args, "--private") {
                        Visibility::Unlisted
                    } else {
                        Visibility::Public
                    },
                    entry: Entry::Open,
                    preset: Some(if teams.is_some() { "teams" } else { "ffa" }.into()),
                    ..Default::default()
                });
            } else if let Some(j) = arg(args, "--warpzone-join") {
                super::join(&mut net, &j);
            }
        }
        if host
            && !started
            && net.lobby().machines.len() >= expect
            && net.lobby().machines.iter().all(|m| m.ready || m.spectate)
        {
            match net.start_match(None) {
                Ok(_) => started = true,
                Err(e) => say(format!("start: {e}")),
            }
        }
        if let Some((t, agreed)) = ended
            && (!host || t.elapsed() > Duration::from_secs(2))
        {
            let s = net
                .last_match_stats()
                .cloned()
                .unwrap_or_else(|| net.stats());
            let (frame, sum) = m.as_ref().and_then(|m| m.done).unwrap_or((0, 0));
            let (lv, scores) = m
                .as_ref()
                .map(|m| {
                    let w = &m.game.sim.world;
                    (
                        w.flow.levels_loaded,
                        w.players
                            .iter()
                            .map(|p| p.games_won.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                    )
                })
                .unwrap_or_default();
            if let Some(om) = &m {
                say(format!("final: {}", om.summary()));
            }
            say(format!(
                "RESULT machine={index} slots={nslots} spectator={} frame={frame} checksum={sum:032x} agreed={agreed} levels={lv} scores={scores} capped={} rollbacks={} resim={} maxdepth={} stalls={} desyncs={} resumes={}",
                nslots == 0,
                m.as_ref().is_some_and(|m| m.capped()),
                s.rollbacks,
                s.resimulated_frames,
                s.max_rollback_depth,
                s.prediction_stalls,
                s.desyncs,
                s.resumes,
            ));
            // Stay in the room a while (screenshots of the lobby).
            let linger: u64 = num(args, "--linger", 0);
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(linger) {
                net.poll();
                while net.next_event().is_some() {}
                std::thread::sleep(Duration::from_millis(10));
            }
            return Ok(0);
        }
        let now = Instant::now();
        if now >= next {
            let step = if net.frames_ahead() > 0 {
                STEP * 11 / 10
            } else {
                STEP
            };
            next += step;
            if now > next + STEP * 10 {
                next = now;
            }
            if let Some(om) = m.as_mut()
                && net.current_match().is_some()
            {
                let v = om.game.sim.world.view();
                let mut local: Vec<(usize, PlayerInput)> =
                    bots.iter_mut().map(|(s, b)| (*s, b.input(&v))).collect();
                // The host's first player ends the session after the
                // wanted number of levels: "back to character select" on
                // the winner screen (a sim input like any other).
                if host
                    && v.flow.levels_loaded >= want_levels
                    && let Some(at) = v.flow.winner_screen_at
                    && let Some(first) = local.first_mut()
                {
                    let t = v.tick - at;
                    first.1 = if t >= 125 && (t - 125) % 20 < 4 {
                        PlayerInput::new(0, BTN_SELECT)
                    } else {
                        PlayerInput::default()
                    };
                }
                om.step(&mut net, &local);
                let lv = om.game.sim.world.flow.levels_loaded;
                if lv != last_levels {
                    last_levels = lv;
                    say(format!("frame {}: {}", om.frame, om.summary()));
                }
                let f = net.current_frame();
                if leave_at.is_some_and(|l| f >= l) {
                    say(format!("leaving at frame {f}"));
                    net.leave();
                    let t = Instant::now();
                    while t.elapsed() < Duration::from_millis(500) {
                        net.poll();
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    return Ok(0);
                }
                if let Some(d) = desync_at
                    && om.frame >= d
                {
                    // Test: corrupt this machine's state (the level RNG).
                    say(format!("corrupting the state at frame {}", om.frame));
                    om.game.sim.world.rng = bopl_sim::rng::Rng::new(0xdead_beef);
                    desync_at = None;
                }
                if crash_at.is_some_and(|c| f >= c) {
                    say(format!("crashing at frame {f}"));
                    std::process::exit(3);
                }
                if last_log.elapsed() > Duration::from_secs(15) {
                    last_log = Instant::now();
                    let s = net.stats();
                    say(format!(
                        "frame {} confirmed {} rollbacks {} stalls {} {}",
                        s.current_frame,
                        s.confirmed_frame,
                        s.rollbacks,
                        s.prediction_stalls,
                        om.summary()
                    ));
                }
            }
        }
        let wait = next
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(2));
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }
}

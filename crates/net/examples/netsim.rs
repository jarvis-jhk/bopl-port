//! netsim: the toy game over bopl-net, to test it end to end.
//!
//! ```sh
//! scripts/net/dev-server.sh &                    # local warpzone-server, http://127.0.0.1:3024
//! export WARPZONE_SERVER=http://127.0.0.1:3024
//! cargo run -p bopl-net --example netsim -- run --locals 4x3 --frames 3600
//! cargo run -p bopl-net --example netsim -- run --locals 12x1 --delay 50 --jitter 20 --loss 5
//! cargo run -p bopl-net --example netsim -- run --locals 4x3 --relay --spectators 1
//! cargo run -p bopl-net --example netsim -- run --locals 2,4,6 --teams 3
//! cargo run -p bopl-net --example netsim -- run --locals 4x1 --leave 2@900 --crash 3@1500
//! cargo run -p bopl-net --example netsim -- run --locals 3x1 --desync 1@600
//! cargo run -p bopl-net --example netsim -- memory --locals 12x1 --delay 40 --loss 5
//! ```
//!
//! `run`: one OS process per machine against a Warpzone server. Machine 0
//! hosts a public room; odd machines join by code, even ones through
//! `--warpzone-join` (the launcher path); the last `--spectators` machines
//! have no slots and watch. Each machine gets its own `WARPZONE_HOME` under
//! `scratch/net/homes/`. At the target frame every player reports its
//! checksum through `finish_match`; `SUMMARY PASS` when all remaining
//! machines (spectators included) have the same one and no desync was seen.
//! With `--desync M@F` machine M corrupts its state at frame F, and PASS
//! means the desync was detected and reported.
//!
//! `memory`: the same in one process over in-memory rooms (no server).

use bopl_net::toy::{ToyMachine, ToySettings};
use bopl_net::warpzone_core::client::{Event, HostOptions};
use bopl_net::warpzone_core::protocol::api::{Entry, Visibility};
use bopl_net::{
    DesyncPolicy, LinkSim, MatchSettings, MemHub, NetConfig, NetEvent, NetSession, NetStats,
    SlotRequest, TeamMode,
};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Warpzone events arrive boxed; unbox them for matching.
#[allow(clippy::large_enum_variant)]
enum Ev {
    Wz(Event),
    Net(NetEvent),
}

/// 60 Hz.
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") => std::process::exit(orchestrate(&args)),
        Some("memory") => std::process::exit(memory(&args)),
        Some("peer") => std::process::exit(peer(&args)),
        _ => {
            eprintln!(
                "usage: netsim run|memory [--locals 3,3,3,3 | 4x3] [--spectators N] [--frames N] [--delay MS] [--jitter MS] [--loss PCT] [--relay] [--leave M@F] [--crash M@F] [--desync M@F] [--abort-on-desync] [--input-delay N] [--max-prediction N] [--teams N] [--tag NAME]"
            );
            std::process::exit(2);
        }
    }
}

fn parse_locals(s: &str) -> Vec<usize> {
    if let Some((m, n)) = s.split_once('x') {
        return vec![n.parse().expect("locals"); m.parse().expect("machines")];
    }
    s.split(',')
        .map(|x| x.trim().parse().expect("locals"))
        .collect()
}

fn parse_at(s: Option<String>) -> Option<(usize, i32)> {
    let s = s?;
    let (m, f) = s.split_once('@')?;
    Some((m.parse().ok()?, f.parse().ok()?))
}

fn root() -> PathBuf {
    std::env::var_os("BOPL_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn num(args: &[String], k: &str) -> f32 {
    arg(args, k)
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(0.0)
}

fn config(args: &[String], version: &str) -> NetConfig {
    let mut cfg = NetConfig::new(version);
    cfg.link_sim = LinkSim {
        delay_ms: num(args, "--delay") as u32,
        jitter_ms: num(args, "--jitter") as u32,
        loss: num(args, "--loss") / 100.0,
    };
    cfg.force_relay = flag(args, "--relay");
    if let Some(d) = arg(args, "--input-delay").and_then(|v| v.parse().ok()) {
        cfg.ggrs.input_delay = d;
    }
    if let Some(d) = arg(args, "--max-prediction").and_then(|v| v.parse().ok()) {
        cfg.ggrs.max_prediction = d;
    }
    if flag(args, "--abort-on-desync") {
        cfg.desync_policy = DesyncPolicy::Abort;
    }
    cfg
}

fn settings(args: &[String]) -> MatchSettings {
    let teams: u8 = arg(args, "--teams")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    MatchSettings {
        team_mode: if teams >= 2 {
            TeamMode::Teams(teams)
        } else {
            TeamMode::Ffa
        },
        mode: "netsim".into(),
        map: "toy arena".into(),
        game: ToySettings::default().to_bytes(),
    }
}

fn slot_requests(index: usize, n: usize) -> Vec<SlotRequest> {
    (0..n)
        .map(|i| SlotRequest {
            name: format!("m{index}p{i}"),
            colour: (index * 2 + i) as u8,
            team: None,
            // Three ability choices (the toy's `PlayerSlot::extra`).
            extra: vec![(index + i) as u8 % 4, 1, 2],
        })
        .collect()
}

/// The `RESULT` line of one machine.
fn result_line(index: usize, nslots: usize, m: &ToyMachine, s: &NetStats) -> String {
    let relayed = s
        .machines
        .iter()
        .filter(|x| x.relayed == Some(true))
        .count();
    let rtt = s
        .machines
        .iter()
        .filter_map(|x| x.ggrs.as_ref().map(|g| g.ping))
        .max()
        .unwrap_or(0);
    let agreed = m.result.as_ref().is_some_and(|r| r.agreed);
    let reported = m.result.as_ref().map_or(0, |r| r.desyncs.len());
    let aborted = match m.aborted.as_deref() {
        Some(r) if r.contains("desync") => "desync",
        Some(_) => "other",
        None => "no",
    };
    format!(
        "RESULT machine={index} slots={nslots} spectator={} aborted={aborted} frame={} checksum={:032x} agreed={agreed} rollbacks={} resim={} maxdepth={} stalls={} desyncs={} desync_reports={reported} resumes={} relayed_links={relayed} sent={} fwd={} simdrop={} frag={}{}",
        nslots == 0,
        m.target,
        m.target_checksum().unwrap_or(0),
        s.rollbacks,
        s.resimulated_frames,
        s.max_rollback_depth,
        s.prediction_stalls,
        s.desyncs,
        s.resumes,
        s.transport.sent_datagrams,
        s.transport.forwarded,
        s.transport.sim_dropped,
        s.transport.fragmented,
        if rtt > 0 {
            format!(" ping={rtt}")
        } else {
            String::new()
        },
    )
}

/// Compare the `RESULT` lines and print `SUMMARY`; returns the exit code.
fn summarise(
    machines: usize,
    expected: &[usize],
    results: &[(usize, String)],
    expect_desync: bool,
    t0: Instant,
) -> i32 {
    let field = |l: &str, k: &str| {
        l.split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{k}=")).map(String::from))
            .unwrap_or_default()
    };
    let sum = |k: &str| -> u64 {
        results
            .iter()
            .map(|(_, l)| field(l, k).parse::<u64>().unwrap_or(0))
            .sum()
    };
    let mut sums: Vec<String> = results.iter().map(|(_, l)| field(l, "checksum")).collect();
    sums.sort();
    sums.dedup();
    let mut got: Vec<usize> = results.iter().map(|r| r.0).collect();
    got.sort();
    let depth = results
        .iter()
        .map(|(_, l)| field(l, "maxdepth").parse::<u64>().unwrap_or(0))
        .max()
        .unwrap_or(0);
    let desyncs = sum("desyncs");
    let reports = sum("desync_reports");
    let all_agreed = results.iter().all(|(_, l)| field(l, "agreed") == "true");
    let all_aborted =
        !results.is_empty() && results.iter().all(|(_, l)| field(l, "aborted") == "desync");
    let ok = if expect_desync {
        // The desync must have been seen, and either reported to the host
        // with disagreeing final checksums, or (abort policy) have aborted
        // the match everywhere.
        desyncs > 0 && ((reports > 0 && !all_agreed) || all_aborted)
    } else {
        got == expected && sums.len() == 1 && desyncs == 0 && all_agreed
    };
    println!(
        "SUMMARY {} machines={machines} reported={} checksums_equal={} checksum={} rollbacks={} resim={} maxdepth={depth} desyncs={desyncs} desync_reports={reports} time={:.1}s",
        if ok { "PASS" } else { "FAIL" },
        got.len(),
        sums.len() == 1,
        sums.first().cloned().unwrap_or_default(),
        sum("rollbacks"),
        sum("resim"),
        t0.elapsed().as_secs_f32()
    );
    if ok { 0 } else { 1 }
}

// ------------------------------------------------------------------ orchestrator

fn orchestrate(args: &[String]) -> i32 {
    let locals = parse_locals(&arg(args, "--locals").unwrap_or_else(|| "3,3,3,3".into()));
    let spectators: usize = arg(args, "--spectators")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let frames: i32 = arg(args, "--frames")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3600);
    let tag = arg(args, "--tag").unwrap_or_else(|| "ns".into());
    let leave = parse_at(arg(args, "--leave"));
    let crash = parse_at(arg(args, "--crash"));
    let desync = parse_at(arg(args, "--desync"));
    let server = std::env::var("WARPZONE_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.into());
    let exe = std::env::current_exe().expect("exe");
    let homes = root().join("scratch/net/homes");
    let n = locals.len() + spectators;
    let slots_of = |i: usize| locals.get(i).copied().unwrap_or(0);
    let mut passthrough: Vec<String> = Vec::new();
    for k in [
        "--delay",
        "--jitter",
        "--loss",
        "--input-delay",
        "--max-prediction",
        "--teams",
    ] {
        if let Some(v) = arg(args, k) {
            passthrough.push(k.into());
            passthrough.push(v);
        }
    }
    for k in ["--relay", "--abort-on-desync"] {
        if flag(args, k) {
            passthrough.push(k.into());
        }
    }
    println!(
        "netsim: {} machines, slots {:?}, {spectators} spectators, {frames} frames, {} ({server})",
        n,
        locals,
        passthrough.join(" ")
    );

    let (tx, rx) = mpsc::channel::<(usize, String)>();
    let mut children: Vec<Child> = Vec::new();
    let spawn = |i: usize, extra: Vec<String>, children: &mut Vec<Child>| {
        let home = homes.join(format!("{tag}-m{i}"));
        std::fs::create_dir_all(&home).expect("home");
        let mut c = Command::new(&exe);
        c.arg("peer").args([
            "--index",
            &i.to_string(),
            "--slots",
            &slots_of(i).to_string(),
            "--frames",
            &frames.to_string(),
        ]);
        c.args(&passthrough).args(extra);
        for (opt, at) in [
            ("--leave-at", leave),
            ("--crash-at", crash),
            ("--corrupt-at", desync),
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
        let mut child = c.spawn().expect("spawn peer");
        let out = child.stdout.take().unwrap();
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send((i, line));
            }
        });
        children.push(child);
    };
    spawn(
        0,
        vec!["--host".into(), "--expect".into(), n.to_string()],
        &mut children,
    );
    let t0 = Instant::now();
    let (mut room, mut code) = (String::new(), String::new());
    while room.is_empty() {
        match rx.recv_timeout(Duration::from_secs(30)) {
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
        eprintln!(
            "netsim: host did not open a room (is the server running? scripts/net/dev-server.sh)"
        );
        for c in &mut children {
            let _ = c.kill();
        }
        return 1;
    }
    for i in 1..n {
        let how = if i % 2 == 1 && code != "-" {
            vec!["--code".into(), code.clone()]
        } else {
            vec!["--warpzone-join".into(), room.clone()]
        };
        spawn(i, how, &mut children);
    }
    drop(tx);
    let mut results: Vec<(usize, String)> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60) + STEP * frames as u32 * 2;
    while let Ok((i, l)) = rx.recv_timeout(
        deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1)),
    ) {
        println!("[m{i}] {l}");
        if l.starts_with("RESULT") {
            results.push((i, l));
        }
        if Instant::now() > deadline {
            break;
        }
    }
    for c in &mut children {
        let _ = c.kill();
        let _ = c.wait();
    }
    let expected: Vec<usize> = (0..n)
        .filter(|i| leave.map(|l| l.0) != Some(*i) && crash.map(|c| c.0) != Some(*i))
        .collect();
    summarise(n, &expected, &results, desync.is_some(), t0)
}

// ------------------------------------------------------------------ one machine (process)

fn say(s: String) {
    println!("{s}");
}

fn peer(args: &[String]) -> i32 {
    let index: usize = arg(args, "--index")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let nslots: usize = arg(args, "--slots")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let frames: i32 = arg(args, "--frames")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3600);
    let host = flag(args, "--host");
    let expect: usize = arg(args, "--expect")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    let leave_at: Option<i32> = arg(args, "--leave-at").and_then(|v| v.parse().ok());
    let crash_at: Option<i32> = arg(args, "--crash-at").and_then(|v| v.parse().ok());
    let corrupt_at: Option<i32> = arg(args, "--corrupt-at").and_then(|v| v.parse().ok());

    let mut cfg = config(args, "netsim-1");
    let home = std::env::var("WARPZONE_HOME").unwrap_or_default();
    let h = home
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let mut name = format!("bp{index}x{:04x}", h & 0xffff);
    cfg.auto_register = Some(name.clone());
    let net: NetSession<_> = match NetSession::new(cfg) {
        Ok(n) => n,
        Err(e) => {
            say(format!("ERROR {e}"));
            return 1;
        }
    };
    let mut m = ToyMachine::new(net, frames, index as u64 * 1000 + 17);
    m.corrupt_at = corrupt_at;
    m.net.set_local_slots(slot_requests(index, nslots));
    m.net.set_spectate(nslots == 0);
    m.net.set_ready(true);
    if host {
        let _ = m.net.set_settings(settings(args));
    }

    let t0 = Instant::now();
    let mut logged_in = false;
    let mut requested_room = false;
    let mut started = false;
    let mut next = Instant::now();
    let mut ended_at: Option<Instant> = None;
    let mut last_report = Instant::now();
    let mut match_t0 = Instant::now();
    loop {
        if t0.elapsed() > Duration::from_secs(60) + STEP * frames as u32 * 3 {
            say("ERROR timeout".into());
            return 1;
        }
        for e in m.poll() {
            let e = match e {
                NetEvent::Warpzone(b) => Ev::Wz(*b),
                other => Ev::Net(other),
            };
            match e {
                Ev::Wz(Event::LoggedIn {
                    account: Some(a), ..
                }) => {
                    say(format!("logged in as {}", a.username));
                    logged_in = true;
                }
                Ev::Wz(Event::Registered(a)) => {
                    say(format!("registered {}", a.username));
                    logged_in = true;
                }
                Ev::Wz(Event::Error { op, code, message }) => {
                    say(format!("warpzone error {op}/{code}: {message}"));
                    if op == "register" {
                        name = format!(
                            "{}{}",
                            &name[..name.len().min(12)],
                            t0.elapsed().as_nanos() % 1000
                        );
                        m.net.register(&name);
                    } else if op == "join" || op == "host" {
                        say("ERROR room".into());
                        return 1;
                    }
                }
                Ev::Wz(Event::RoomHosted { room, code, .. }) => {
                    say(format!(
                        "ROOM {} {}",
                        room.id,
                        code.unwrap_or_else(|| "-".into())
                    ));
                }
                Ev::Wz(Event::PeerConnected { peer, relayed }) => say(format!(
                    "peer {peer} connected {}",
                    if relayed { "via relay" } else { "direct" }
                )),
                Ev::Wz(Event::PeerPathChanged { peer, relayed }) => say(format!(
                    "peer {peer} now {}",
                    if relayed { "relayed" } else { "direct" }
                )),
                Ev::Net(NetEvent::Welcome { machine }) => say(format!("welcome as {machine}")),
                Ev::Net(NetEvent::MatchStarted(ms)) => {
                    say(format!(
                        "match {} started: {} slots, {} spectators, seed {:#x}, local {:?}{}",
                        ms.match_id,
                        ms.players.len(),
                        ms.spectators.len(),
                        ms.seed,
                        m.net.local_slots(),
                        if m.net.is_spectator() {
                            " (spectating)"
                        } else {
                            ""
                        }
                    ));
                    next = Instant::now();
                    match_t0 = Instant::now();
                }
                Ev::Net(NetEvent::Running) => say(format!(
                    "running ({} ms after match start)",
                    match_t0.elapsed().as_millis()
                )),
                Ev::Net(NetEvent::MachineLeft { machine }) => {
                    say(format!("{machine} left the room"))
                }
                Ev::Net(NetEvent::MatchFrozen { gone }) => {
                    say(format!("frozen, dropping {gone:?}"))
                }
                Ev::Net(NetEvent::PlayersDropped {
                    machines,
                    slots,
                    from_frame,
                }) => say(format!(
                    "resumed at frame {from_frame} without {machines:?} (slots {slots:?})"
                )),
                Ev::Net(NetEvent::Disconnected { machine }) => {
                    say(format!("{machine} disconnected (ggrs)"))
                }
                Ev::Net(NetEvent::Desync { frame, machine, .. }) => {
                    say(format!("DESYNC at frame {frame} with {machine}"))
                }
                Ev::Net(NetEvent::DesyncReported(r)) => say(format!(
                    "desync report: {} vs {} at frame {}",
                    r.reporter, r.with, r.frame
                )),
                Ev::Net(NetEvent::MatchEnded {
                    agreed,
                    ref reports,
                    ref desyncs,
                    ..
                }) => {
                    say(format!(
                        "match ended, {} reports, agreed={agreed}, {} desync reports",
                        reports.len(),
                        desyncs.len()
                    ));
                    ended_at = Some(Instant::now());
                }
                Ev::Net(NetEvent::MatchAborted { reason })
                    if corrupt_at.is_some() || reason.contains("desync") =>
                {
                    // Desync abort policy: report what we saw.
                    say(format!("aborted: {reason}"));
                    ended_at.get_or_insert_with(Instant::now);
                }
                Ev::Net(NetEvent::MatchAborted { reason } | NetEvent::RoomLeft { reason }) => {
                    if ended_at.is_none() {
                        say(format!("ERROR aborted: {reason}"));
                        return 1;
                    }
                }
                Ev::Net(NetEvent::Rejected { reason }) => {
                    say(format!("ERROR rejected: {reason}"));
                    return 1;
                }
                _ => {}
            }
        }
        if logged_in && !requested_room {
            requested_room = true;
            if host {
                m.net.host(HostOptions {
                    max_players: 12,
                    visibility: Visibility::Public,
                    entry: Entry::Open,
                    preset: Some("ffa".into()),
                    ..Default::default()
                });
            } else if let Some(c) = arg(args, "--code") {
                m.net.join_code(&c);
            } else if let Some(r) = arg(args, "--warpzone-join") {
                let action =
                    m.net
                        .handle_launch(&["netsim".to_string(), "--warpzone-join".into(), r]);
                say(format!("launch: {action:?}"));
            }
        }
        if host && !started && m.net.lobby().machines.len() >= expect {
            match m.net.start_match(None) {
                Ok(_) => started = true,
                Err(e) => say(format!("start: {e}")),
            }
        }
        if let Some(t) = ended_at {
            // Host: linger so the End message reaches everybody.
            if !host || t.elapsed() > Duration::from_secs(2) {
                let s = m
                    .net
                    .last_match_stats()
                    .cloned()
                    .unwrap_or_else(|| m.net.stats());
                say(result_line(index, nslots, &m, &s));
                return 0;
            }
        }
        let now = Instant::now();
        if now >= next {
            // Ahead of the others: run 10% slower until they catch up.
            let step = if m.net.frames_ahead() > 0 {
                STEP * 11 / 10
            } else {
                STEP
            };
            next += step;
            if now > next + STEP * 10 {
                next = now; // we were stalled (e.g. synchronising); don't sprint
            }
            m.step();
            let f = m.net.current_frame();
            if leave_at.is_some_and(|l| f >= l) {
                say(format!("leaving at frame {f}"));
                m.net.leave();
                let t = Instant::now();
                while t.elapsed() < Duration::from_millis(500) {
                    m.net.poll();
                    std::thread::sleep(Duration::from_millis(5));
                }
                return 0;
            }
            if crash_at.is_some_and(|c| f >= c) {
                say(format!("crashing at frame {f}"));
                std::process::exit(3);
            }
            if last_report.elapsed() > Duration::from_secs(10) && m.net.current_match().is_some() {
                last_report = Instant::now();
                let s = m.net.stats();
                let links: Vec<String> = s
                    .machines
                    .iter()
                    .map(|x| {
                        format!(
                            "{}:{}ms{}{}",
                            x.machine,
                            x.ggrs.as_ref().map(|g| g.ping).unwrap_or(0),
                            if x.relayed == Some(true) { "R" } else { "" },
                            if x.via_host { "H" } else { "" }
                        )
                    })
                    .collect();
                say(format!(
                    "frame {} confirmed {} ahead {} rollbacks {} stalls {} buffered {} links [{}]",
                    s.current_frame,
                    s.confirmed_frame,
                    s.frames_ahead,
                    s.rollbacks,
                    s.prediction_stalls,
                    s.spectator_buffered,
                    links.join(" ")
                ));
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

// ------------------------------------------------------------------ all machines in one process

fn memory(args: &[String]) -> i32 {
    let locals = parse_locals(&arg(args, "--locals").unwrap_or_else(|| "3,3,3,3".into()));
    let spectators: usize = arg(args, "--spectators")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let frames: i32 = arg(args, "--frames")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3600);
    let leave = parse_at(arg(args, "--leave"));
    let crash = parse_at(arg(args, "--crash"));
    let desync = parse_at(arg(args, "--desync"));
    let n = locals.len() + spectators;
    println!(
        "netsim memory: {n} machines, slots {locals:?}, {spectators} spectators, {frames} frames"
    );
    let hub = MemHub::new();
    let mut ms: Vec<ToyMachine> = (0..n)
        .map(|i| {
            let mut net = NetSession::memory(hub.join(&format!("m{i}")), config(args, "memory"));
            let k = locals.get(i).copied().unwrap_or(0);
            net.set_local_slots(slot_requests(i, k));
            net.set_spectate(k == 0);
            let mut m = ToyMachine::new(net, frames, i as u64 * 1000 + 17);
            if let Some((who, f)) = desync
                && who == i
            {
                m.corrupt_at = Some(f);
            }
            m
        })
        .collect();
    let _ = ms[0].net.set_settings(settings(args));
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_secs(60) + STEP * frames as u32 * 3;
    let mut started = false;
    let mut next = Instant::now();
    let mut gone = vec![false; n];
    let mut results: Vec<(usize, String)> = Vec::new();
    let mut done = vec![false; n];
    while Instant::now() < deadline {
        for (i, m) in ms.iter_mut().enumerate() {
            if gone[i] || done[i] {
                continue;
            }
            for e in m.poll() {
                match e {
                    NetEvent::PlayersDropped {
                        machines,
                        from_frame,
                        ..
                    } if i == 0 => {
                        println!("[m0] resumed at frame {from_frame} without {machines:?}")
                    }
                    NetEvent::Desync { frame, machine, .. } => {
                        println!("[m{i}] DESYNC at frame {frame} with {machine}")
                    }
                    _ => {}
                }
            }
            if m.result.is_some() || m.aborted.is_some() {
                let s = m
                    .net
                    .last_match_stats()
                    .cloned()
                    .unwrap_or_else(|| m.net.stats());
                let line = result_line(i, locals.get(i).copied().unwrap_or(0), m, &s);
                println!("[m{i}] {line}");
                results.push((i, line));
                done[i] = true;
            }
        }
        if !started
            && ms[0].net.lobby().machines.len() == n
            && ms[1..].iter().all(|m| m.net.me().is_some())
        {
            started = ms[0].net.start_match(Some(0x7f)).is_ok();
        }
        if Instant::now() >= next {
            next += STEP;
            for (i, m) in ms.iter_mut().enumerate() {
                if gone[i] || done[i] {
                    continue;
                }
                m.step();
                let f = m.net.current_frame();
                if leave.is_some_and(|(w, at)| w == i && f >= at) {
                    println!("[m{i}] leaving at frame {f}");
                    m.net.leave();
                    gone[i] = true;
                }
                if crash.is_some_and(|(w, at)| w == i && f >= at) {
                    println!("[m{i}] crashing at frame {f}");
                    gone[i] = true;
                }
            }
        }
        if started && (0..n).all(|i| gone[i] || done[i]) {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let expected: Vec<usize> = (0..n).filter(|i| !gone[*i]).collect();
    summarise(n, &expected, &results, desync.is_some(), t0)
}

//! Headless scripted device input (`--drive FILE`): synthetic keyboard and
//! controller states per tick plus named screenshots, for walking through
//! the whole flow (menus, character select, rounds, wheel, winner screen)
//! without a window.
//!
//! One command per line (`#` comments). Each line waits `DELAY` ticks
//! (60/s) after the previous line, then runs; `wait` lines block until a
//! condition holds:
//!
//! ```text
//! DELAY press DEVICE BUTTON [TICKS]  hold a button (default 4 ticks); DEVICE
//!                                    `loser` = a player who can use the wheel
//! DELAY stick DEVICE X Y TICKS       hold the move axis
//! DELAY bot DEVICE                   (match) a bot plays this device's player;
//!                                    the device's own presses still win
//! DELAY wait COND                    menu | tutorial | charsel | match | round | winner
//!                                    (inputs accepted) | wheel N|any (player N /
//!                                    anyone may open the wheel) | level (a new
//!                                    level) | machines N (online: N machines
//!                                    in the room) | busy (online: a remote
//!                                    player has the wheel open)
//! DELAY shot LABEL                   screenshot <stem>-LABEL.png
//! DELAY key NAME [TICKS]             hold a physical key or mouse button
//!                                    (binding names: `J`, `Space`, `Mouse1`;
//!                                    default 4 ticks): a raw press (key
//!                                    rebinding, typing) and the keyboard
//!                                    players' buttons through the bindings
//! DELAY button PAD NAME [TICKS]      the same for a controller button (`a`,
//!                                    `leftshoulder`, `lefttrigger`, ...)
//! DELAY type TEXT                    type letters / digits (one key per tick;
//!                                    `_` = Backspace, `>` = Return)
//! DELAY point X Y                   the mouse pointer at output pixel X Y (keyboard
//!                                    + mouse; `key Mouse1` clicks)
//! DELAY quit
//! ```
//!
//! Devices: `km` (keyboard + mouse), `kl`, `kr` (keyboard halves),
//! `pad0`..`pad11` (controllers). Buttons: `jump a0 a1 a2 start select up
//! down left right accept back`.

use anyhow::{Context, Result, bail};
use glam::Vec2;

use crate::input::{Buttons, DeviceId, InputState};

#[derive(Clone, Debug, PartialEq)]
pub enum Cond {
    Menu,
    Tutorial,
    CharSel,
    Match,
    Round,
    Winner,
    Wheel(usize),
    Level,
    /// Online: at least this many machines in the room.
    Machines(usize),
    /// Online: a remote player has the wheel open (busy dots).
    Busy,
}

/// A device, or the device of a player who may use the ability wheel.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Dev(DeviceId),
    Loser,
}

#[derive(Clone, Debug, PartialEq)]
enum Cmd {
    Press(Target, Buttons, u64),
    Stick(Target, Vec2, u64),
    Bot(DeviceId),
    /// A physical key / button: (device, binding name, ticks).
    Key(DeviceId, String, u64),
    Type(String),
    Wait(Cond),
    Shot(String),
    /// The keyboard + mouse pointer at output pixel (x, y) from now on.
    Point(Vec2),
    Quit,
}

/// What the app reports to the driver each tick.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// `menu`, `charsel` or `match`.
    pub screen: &'static str,
    pub round_live: bool,
    /// Winner screen up and taking inputs.
    pub winner_ready: bool,
    /// Per player: the wheel can be opened.
    pub wheel_ready: Vec<bool>,
    pub level_generation: u32,
    /// The device of the first player who can open (or has open) the wheel.
    pub loser: Option<DeviceId>,
    /// Online: machines in the room.
    pub machines: usize,
    /// Online: a remote player's wheel is open.
    pub remote_wheel_open: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Driver {
    cmds: Vec<(u64, Cmd)>,
    next: usize,
    /// App tick at which the next command is due.
    due: u64,
    holds: Vec<(DeviceId, Buttons, u64, u64)>,
    /// Physical keys / pad buttons held: (device, name, from, to).
    keys: Vec<(DeviceId, String, u64, u64)>,
    /// Raw presses of this tick.
    raw: Vec<crate::input::RawPress>,
    /// Keys still to type (one per tick).
    typing: std::collections::VecDeque<String>,
    sticks: Vec<(DeviceId, Vec2, u64, u64)>,
    devices: Vec<DeviceId>,
    prev: Vec<(DeviceId, Buttons)>,
    shot: Option<String>,
    bots: Vec<DeviceId>,
    done: bool,
    /// Level generation when the current `wait level` started.
    level_gen: Option<u32>,
    started: bool,
    /// The mouse pointer (`point X Y`), output pixels.
    pointer: Option<Vec2>,
}

fn device(s: &str) -> Result<DeviceId> {
    Ok(match s {
        "km" => DeviceId::KeyboardMouse,
        "kl" => DeviceId::KeyboardLeft,
        "kr" => DeviceId::KeyboardRight,
        p if p.starts_with("pad") => DeviceId::Controller(p[3..].parse().context("pad number")?),
        _ => bail!("unknown device {s:?}"),
    })
}

fn target(s: &str) -> Result<Target> {
    Ok(if s == "loser" {
        Target::Loser
    } else {
        Target::Dev(device(s)?)
    })
}

fn button(s: &str) -> Result<Buttons> {
    Ok(match s {
        "jump" => Buttons::JUMP,
        "a0" => Buttons::ABILITY0,
        "a1" => Buttons::ABILITY1,
        "a2" => Buttons::ABILITY2,
        "start" => Buttons::START,
        "select" => Buttons::SELECT,
        "up" => Buttons::UP,
        "down" => Buttons::DOWN,
        "left" => Buttons::LEFT,
        "right" => Buttons::RIGHT,
        "accept" => Buttons::ACCEPT,
        "back" => Buttons::BACK,
        _ => bail!("unknown button {s:?}"),
    })
}

impl Driver {
    pub fn parse(text: &str) -> Result<Self> {
        let mut d = Driver::default();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let w: Vec<&str> = line.split_whitespace().collect();
            let ctx = || format!("line {}: {line}", n + 1);
            let delay: u64 = w[0].parse().with_context(ctx)?;
            let arg = |i: usize| w.get(i).copied().with_context(ctx);
            let cmd = match arg(1)? {
                "press" => Cmd::Press(
                    target(arg(2)?)?,
                    button(arg(3)?)?,
                    w.get(4).map_or(Ok(4), |s| s.parse()).with_context(ctx)?,
                ),
                "stick" => Cmd::Stick(
                    target(arg(2)?)?,
                    Vec2::new(arg(3)?.parse()?, arg(4)?.parse()?),
                    arg(5)?.parse()?,
                ),
                "bot" => Cmd::Bot(device(arg(2)?)?),
                "key" => Cmd::Key(
                    DeviceId::KeyboardMouse,
                    arg(2)?.to_string(),
                    w.get(3).map_or(Ok(4), |s| s.parse()).with_context(ctx)?,
                ),
                "button" => Cmd::Key(
                    device(arg(2)?)?,
                    arg(3)?.to_string(),
                    w.get(4).map_or(Ok(4), |s| s.parse()).with_context(ctx)?,
                ),
                "type" => Cmd::Type(w[2..].join(" ")),
                "wait" => Cmd::Wait(match arg(2)? {
                    "menu" => Cond::Menu,
                    "tutorial" => Cond::Tutorial,
                    "charsel" => Cond::CharSel,
                    "match" => Cond::Match,
                    "round" => Cond::Round,
                    "winner" => Cond::Winner,
                    "level" => Cond::Level,
                    "busy" => Cond::Busy,
                    "machines" => Cond::Machines(arg(3)?.parse().with_context(ctx)?),
                    "wheel" => Cond::Wheel(match arg(3)? {
                        "any" => usize::MAX,
                        n => n.parse().with_context(ctx)?,
                    }),
                    c => bail!("{}: unknown condition {c:?}", ctx()),
                }),
                "shot" => Cmd::Shot(arg(2)?.to_string()),
                "point" => Cmd::Point(Vec2::new(arg(2)?.parse()?, arg(3)?.parse()?)),
                "quit" => Cmd::Quit,
                c => bail!("{}: unknown command {c:?}", ctx()),
            };
            let dvs = match &cmd {
                Cmd::Press(Target::Dev(dv), ..)
                | Cmd::Stick(Target::Dev(dv), ..)
                | Cmd::Bot(dv) => vec![*dv],
                Cmd::Key(DeviceId::KeyboardMouse, ..) | Cmd::Type(_) => vec![
                    DeviceId::KeyboardMouse,
                    DeviceId::KeyboardLeft,
                    DeviceId::KeyboardRight,
                ],
                Cmd::Key(dv, ..) => vec![*dv],
                _ => vec![],
            };
            for dv in dvs {
                if !d.devices.contains(&dv) {
                    d.devices.push(dv);
                }
            }
            d.cmds.push((delay, cmd));
        }
        if !d.devices.contains(&DeviceId::KeyboardMouse) {
            d.devices.insert(0, DeviceId::KeyboardMouse);
        }
        Ok(d)
    }

    fn holds_cond(&self, c: &Cond, s: &Status) -> bool {
        match c {
            Cond::Menu => s.screen == "menu",
            Cond::Tutorial => s.screen == "tutorial",
            Cond::CharSel => s.screen == "charsel",
            Cond::Match => s.screen == "match",
            Cond::Round => s.screen == "match" && s.round_live,
            Cond::Winner => s.screen == "match" && s.winner_ready,
            Cond::Wheel(n) if *n == usize::MAX => s.loser.is_some(),
            Cond::Wheel(n) => s.wheel_ready.get(*n).copied().unwrap_or(false),
            Cond::Level => self.level_gen.is_some_and(|g| s.level_generation != g),
            Cond::Machines(n) => s.machines >= *n,
            Cond::Busy => s.remote_wheel_open,
        }
    }

    /// Runs the commands due at `tick` (call once per tick, before
    /// [`Driver::devices`]).
    pub fn advance(&mut self, tick: u64, status: &Status) {
        if !self.started {
            self.started = true;
            self.due = tick + self.cmds.first().map_or(0, |c| c.0);
        }
        self.shot = None;
        self.raw.clear();
        if let Some(k) = self.typing.pop_front() {
            self.raw.push(crate::input::RawPress {
                device: DeviceId::KeyboardMouse,
                name: k.clone(),
            });
            self.keys.push((DeviceId::KeyboardMouse, k, tick, tick + 1));
        }
        while self.next < self.cmds.len() && tick >= self.due {
            let cmd = self.cmds[self.next].1.clone();
            match cmd {
                Cmd::Wait(c) => {
                    // `wait level` waits for a level loaded after it began.
                    if c == Cond::Level && self.level_gen.is_none() {
                        self.level_gen = Some(status.level_generation);
                    }
                    if !self.holds_cond(&c, status) {
                        return;
                    }
                    self.level_gen = None;
                }
                Cmd::Press(t, b, n) => {
                    if let Some(d) = Self::resolve(t, status) {
                        self.holds.push((d, b, tick, tick + n));
                    }
                }
                Cmd::Stick(t, v, n) => {
                    if let Some(d) = Self::resolve(t, status) {
                        self.sticks.push((d, v, tick, tick + n));
                    }
                }
                Cmd::Bot(d) => self.bots.push(d),
                Cmd::Key(d, name, n) => {
                    self.raw.push(crate::input::RawPress {
                        device: d,
                        name: name.clone(),
                    });
                    self.keys.push((d, name, tick, tick + n));
                }
                Cmd::Type(text) => {
                    for c in text.chars() {
                        self.typing.push_back(match c {
                            '_' => "Backspace".into(),
                            '>' => "Return".into(),
                            ' ' => "Space".into(),
                            c => c.to_ascii_uppercase().to_string(),
                        });
                    }
                }
                Cmd::Shot(s) => self.shot = Some(s),
                Cmd::Point(p) => self.pointer = Some(p),
                Cmd::Quit => self.done = true,
            }
            self.next += 1;
            self.due = tick + self.cmds.get(self.next).map_or(0, |c| c.0);
            if self.shot.is_some() {
                // One screenshot per tick.
                return;
            }
        }
    }

    fn resolve(t: Target, s: &Status) -> Option<DeviceId> {
        match t {
            Target::Dev(d) => Some(d),
            Target::Loser => {
                if s.loser.is_none() {
                    log::warn!("driver: no loser device");
                }
                s.loser
            }
        }
    }

    /// Raw key / button presses of this tick.
    pub fn raw(&self) -> &[crate::input::RawPress] {
        &self.raw
    }

    /// Device states for `tick` (physical keys through `bindings`).
    pub fn devices(
        &mut self,
        tick: u64,
        bindings: &crate::input::Bindings,
        out: &mut Vec<(DeviceId, InputState)>,
    ) {
        self.holds.retain(|h| h.3 > tick);
        self.sticks.retain(|h| h.3 > tick);
        self.keys.retain(|h| h.3 > tick);
        let keys = self.keys.clone();
        let held_key = |d: DeviceId, n: &str| {
            keys.iter()
                .any(|(kd, kn, from, to)| *kd == d && kn == n && (*from..*to).contains(&tick))
        };
        let kb = crate::input::KeyboardKeys::new(bindings).buttons(|k| {
            keys.iter().any(|(kd, kn, from, to)| {
                *kd == DeviceId::KeyboardMouse
                    && (*from..*to).contains(&tick)
                    && crate::input::Key::from_name(kn) == Some(k)
            })
        });
        for dev in self.devices.clone() {
            let mut held = match dev {
                DeviceId::KeyboardMouse => kb[0],
                DeviceId::KeyboardLeft => kb[1],
                DeviceId::KeyboardRight => kb[2],
                DeviceId::Controller(_) => {
                    let key = crate::input::device_key(dev, &[]);
                    crate::input::controller_buttons(bindings, &key, |n| held_key(dev, n))
                }
            };
            let mut axis = Vec2::ZERO;
            for &(d, b, from, to) in &self.holds {
                if d == dev && (from..to).contains(&tick) {
                    held = held | b;
                }
            }
            for &(d, v, from, to) in &self.sticks {
                if d == dev && (from..to).contains(&tick) {
                    axis = v;
                }
            }
            // Directions from the stick, like the controllers' menu bits.
            if axis.x < -0.4 && axis.x.abs() > axis.y.abs() {
                held = held | Buttons::LEFT;
            }
            if axis.x > 0.4 && axis.x.abs() > axis.y.abs() {
                held = held | Buttons::RIGHT;
            }
            if axis.y > 0.4 && axis.y.abs() > axis.x.abs() {
                held = held | Buttons::UP;
            }
            if axis.y < -0.4 && axis.y.abs() > axis.x.abs() {
                held = held | Buttons::DOWN;
            }
            if axis == Vec2::ZERO {
                let h = |b: Buttons| held.contains(b) as i32 as f32;
                axis = Vec2::new(
                    h(Buttons::RIGHT) - h(Buttons::LEFT),
                    h(Buttons::UP) - h(Buttons::DOWN),
                );
            }
            let prev = self
                .prev
                .iter()
                .find(|(d, _)| *d == dev)
                .map_or(Buttons::default(), |(_, b)| *b);
            let pressed = Buttons(held.0 & !prev.0);
            let released = Buttons(prev.0 & !held.0);
            match self.prev.iter_mut().find(|(d, _)| *d == dev) {
                Some(p) => p.1 = held,
                None => self.prev.push((dev, held)),
            }
            out.push((
                dev,
                InputState {
                    move_axis: axis,
                    aim_axis: axis.normalize_or_zero(),
                    pointer: if dev == DeviceId::KeyboardMouse {
                        self.pointer
                    } else {
                        None
                    },
                    held,
                    pressed,
                    released,
                },
            ));
        }
    }

    /// Devices bots play for (in matches).
    pub fn bots(&self) -> &[DeviceId] {
        &self.bots
    }

    /// The screenshot label requested this tick.
    pub fn shot(&self) -> Option<&str> {
        self.shot.as_deref()
    }

    pub fn done(&self) -> bool {
        self.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_with_waits() {
        let mut d =
            Driver::parse("10 press pad0 accept 3\n2 wait winner\n0 shot x\n5 quit").unwrap();
        let mut st = Status {
            screen: "match",
            ..Default::default()
        };
        let pad = |o: &Vec<(DeviceId, InputState)>| {
            o.iter()
                .find(|(id, _)| *id == DeviceId::Controller(0))
                .unwrap()
                .1
        };
        let mut out = Vec::new();
        for t in 1..=9 {
            d.advance(t, &st);
            out.clear();
            d.devices(t, &crate::input::Bindings::default(), &mut out);
            assert!(!pad(&out).held.contains(Buttons::ACCEPT), "{t}");
        }
        d.advance(11, &st);
        out.clear();
        d.devices(11, &crate::input::Bindings::default(), &mut out);
        assert!(pad(&out).pressed.contains(Buttons::ACCEPT));
        for t in 12..40 {
            d.advance(t, &st);
            assert!(d.shot().is_none());
        }
        st.winner_ready = true;
        d.advance(40, &st);
        assert_eq!(d.shot(), Some("x"));
        d.advance(41, &st);
        assert!(!d.done());
        d.advance(46, &st);
        assert!(d.done());
        assert!(Driver::parse("1 frobnicate").is_err());
    }

    #[test]
    fn physical_keys_and_typing() {
        let mut d = Driver::parse("1 key K 3\n0 button pad2 leftshoulder 2\n5 type ab_>").unwrap();
        let st = Status::default();
        let mut b = crate::input::Bindings::default();
        b.set(
            crate::input::Scheme::KeyboardMouse,
            crate::input::Action::Jump,
            vec!["K".into()],
        );
        b.set(
            crate::input::Scheme::Controller,
            crate::input::Action::Ability2,
            vec!["leftshoulder".into()],
        );
        let mut out = Vec::new();
        let mut raws = Vec::new();
        let mut jump_ticks = 0;
        for t in 1..=12 {
            d.advance(t, &st);
            raws.extend(d.raw().iter().map(|r| (r.device, r.name.clone())));
            out.clear();
            d.devices(t, &b, &mut out);
            let km = out
                .iter()
                .find(|(id, _)| *id == DeviceId::KeyboardMouse)
                .unwrap()
                .1;
            if km.held.contains(Buttons::JUMP) {
                jump_ticks += 1;
            }
            if t == 2 {
                let pad = out
                    .iter()
                    .find(|(id, _)| *id == DeviceId::Controller(2))
                    .unwrap()
                    .1;
                assert!(pad.held.contains(Buttons::ABILITY2));
            }
        }
        assert_eq!(jump_ticks, 3, "K held for 3 ticks, bound to jump");
        let names: Vec<&str> = raws.iter().map(|r| r.1.as_str()).collect();
        assert_eq!(
            names,
            ["K", "leftshoulder", "A", "B", "Backspace", "Return"]
        );
        assert_eq!(raws[1].0, DeviceId::Controller(2));
    }
}

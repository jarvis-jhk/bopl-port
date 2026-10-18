//! Simulated controllers for tests (`--virtual-pads FILE`, docs/platform.md
//! "Testing on a private X server"): SDL virtual joysticks
//! (`SDL_JoystickAttachVirtualEx`) that look like standard game
//! controllers, driven by commands appended to a text file. The file is
//! read from where the previous frame stopped, so a test script can
//! `echo "tap 0 a" >> FILE` while the game runs. Hot-plug goes through
//! SDL's normal device events, exactly like a real pad.
//!
//! Commands (one per line, `#` comments):
//!
//! ```text
//! attach [N]            attach N pads (default 1); they get the next pad numbers
//! detach P              unplug pad P (numbers are never reused)
//! button P NAME 0|1     hold / release a button (a b x y back guide start
//!                       ls rs lb rb up down left right)
//! tap P NAME            press now, release about 100 ms later
//! axis P NAME V         lx ly rx ry (-1..1, y down like SDL) lt rt (0..1)
//! center P              all axes to rest
//! ```

use std::collections::BTreeMap;
use std::ffi::CString;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use sdl2::sys;

struct Pad {
    joystick: *mut sys::SDL_Joystick,
    instance: i32,
}

pub struct VirtualPads {
    path: PathBuf,
    offset: u64,
    partial: String,
    pads: BTreeMap<usize, Pad>,
    next: usize,
    releases: Vec<(Instant, usize, i32)>,
    _name: CString,
}

const BUTTONS: [&str; 15] = [
    "a", "b", "x", "y", "back", "guide", "start", "ls", "rs", "lb", "rb", "up", "down", "left",
    "right",
];
const AXES: [&str; 6] = ["lx", "ly", "rx", "ry", "lt", "rt"];

impl VirtualPads {
    pub fn new(path: PathBuf) -> VirtualPads {
        log::info!("virtual pads: reading commands from {}", path.display());
        VirtualPads {
            path,
            offset: 0,
            partial: String::new(),
            pads: BTreeMap::new(),
            next: 0,
            releases: Vec::new(),
            _name: CString::new("bopl-port virtual pad").unwrap(),
        }
    }

    /// Runs new commands and due releases. Call once per frame, before
    /// polling SDL events.
    pub fn poll(&mut self) {
        let now = Instant::now();
        let due: Vec<_> = self
            .releases
            .iter()
            .filter(|r| r.0 <= now)
            .copied()
            .collect();
        self.releases.retain(|r| r.0 > now);
        for (_, p, b) in due {
            self.set_button(p, b, false);
        }
        let Ok(mut f) = std::fs::File::open(&self.path) else {
            return;
        };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.offset {
            self.offset = 0; // truncated: start over
        }
        if len == self.offset || f.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut buf = String::new();
        if f.read_to_string(&mut buf).is_err() {
            return;
        }
        self.offset += buf.len() as u64;
        self.partial.push_str(&buf);
        while let Some(i) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=i).collect();
            log::debug!("virtual pads: {}", line.trim());
            if let Err(e) = self.command(line.trim()) {
                log::warn!("virtual pads: {e} in {line:?}");
            }
        }
    }

    fn command(&mut self, line: &str) -> Result<(), String> {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            return Ok(());
        }
        let w: Vec<&str> = line.split_whitespace().collect();
        let pad = |i: usize| -> Result<usize, String> {
            w.get(i)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| "pad number expected".to_string())
        };
        let button = |i: usize| -> Result<i32, String> {
            let n = w.get(i).copied().unwrap_or("");
            BUTTONS
                .iter()
                .position(|b| *b == n)
                .map(|b| b as i32)
                .ok_or_else(|| format!("unknown button {n:?}"))
        };
        match w[0] {
            "attach" => {
                let n: usize = w.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
                for _ in 0..n {
                    self.attach()?;
                }
            }
            "detach" => {
                let p = pad(1)?;
                let pd = self.pads.remove(&p).ok_or("no such pad")?;
                // SAFETY: the joystick was opened by us; the device index of
                // a virtual joystick is looked up by its instance id.
                unsafe {
                    let n = sys::SDL_NumJoysticks();
                    for i in 0..n {
                        if sys::SDL_JoystickGetDeviceInstanceID(i) == pd.instance {
                            sys::SDL_JoystickClose(pd.joystick);
                            sys::SDL_JoystickDetachVirtual(i);
                            break;
                        }
                    }
                }
                log::info!("virtual pads: detached pad {p}");
            }
            "button" => {
                let (p, b) = (pad(1)?, button(2)?);
                let on = w.get(3).is_some_and(|s| *s == "1" || *s == "down");
                self.set_button(p, b, on);
            }
            "tap" => {
                let (p, b) = (pad(1)?, button(2)?);
                self.set_button(p, b, true);
                self.releases
                    .push((Instant::now() + Duration::from_millis(100), p, b));
            }
            "axis" => {
                let p = pad(1)?;
                let n = w.get(2).copied().unwrap_or("");
                let a = AXES
                    .iter()
                    .position(|x| *x == n)
                    .ok_or_else(|| format!("unknown axis {n:?}"))?;
                let v: f32 = w
                    .get(3)
                    .and_then(|s| s.parse().ok())
                    .ok_or("value expected")?;
                self.set_axis(p, a, v);
            }
            "center" => {
                let p = pad(1)?;
                for a in 0..AXES.len() {
                    self.set_axis(p, a, 0.0);
                }
            }
            other => return Err(format!("unknown command {other:?}")),
        }
        Ok(())
    }

    fn attach(&mut self) -> Result<(), String> {
        // SAFETY: zeroed descriptor with the version set, as SDL requires;
        // the name outlives the joystick (owned by self).
        unsafe {
            let mut d: sys::SDL_VirtualJoystickDesc = std::mem::zeroed();
            d.version = sys::SDL_VIRTUAL_JOYSTICK_DESC_VERSION as u16;
            d.type_ = sys::SDL_JoystickType::SDL_JOYSTICK_TYPE_GAMECONTROLLER as u16;
            d.naxes = AXES.len() as u16;
            d.nbuttons = BUTTONS.len() as u16;
            d.button_mask = (1u32 << BUTTONS.len()) - 1;
            d.axis_mask = (1u32 << AXES.len()) - 1;
            d.name = self._name.as_ptr();
            let index = sys::SDL_JoystickAttachVirtualEx(&d);
            if index < 0 {
                return Err(format!("attach: {}", sdl2::get_error()));
            }
            let js = sys::SDL_JoystickOpen(index);
            if js.is_null() {
                return Err(format!("open: {}", sdl2::get_error()));
            }
            let instance = sys::SDL_JoystickInstanceID(js);
            let p = self.next;
            self.next += 1;
            self.pads.insert(
                p,
                Pad {
                    joystick: js,
                    instance,
                },
            );
            for a in 0..AXES.len() {
                self.set_axis(p, a, 0.0);
            }
            log::info!("virtual pads: attached pad {p} (joystick instance {instance})");
        }
        Ok(())
    }

    fn set_button(&mut self, p: usize, b: i32, on: bool) {
        if let Some(pd) = self.pads.get(&p) {
            // SAFETY: open virtual joystick.
            unsafe { sys::SDL_JoystickSetVirtualButton(pd.joystick, b, on as u8) };
        }
    }

    /// Sticks -1..1 (y down), triggers 0..1 (the virtual mapping reads the
    /// full axis range as the trigger range).
    fn set_axis(&mut self, p: usize, a: usize, v: f32) {
        let raw = if a >= 4 {
            (-32768.0 + v.clamp(0.0, 1.0) * 65535.0) as i32
        } else {
            (v.clamp(-1.0, 1.0) * 32767.0) as i32
        };
        if let Some(pd) = self.pads.get(&p) {
            // SAFETY: open virtual joystick.
            unsafe { sys::SDL_JoystickSetVirtualAxis(pd.joystick, a as i32, raw as i16) };
        }
    }
}

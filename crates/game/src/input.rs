//! Input devices: keyboard + mouse, two keyboard-only halves (for more than
//! one player on a keyboard) and any number of SDL game controllers
//! (hot-plugged), each turned into a normalised [`InputState`] per tick.
//!
//! Default bindings follow the original (`docs/spec/ui.md` section 1):
//! controllers = left stick (fallback d-pad, right stick) move/aim, South
//! jump, West ability slot 0, East slot 1, North slot 2, Start, Back =
//! select; keyboard + mouse = W A S D / arrows, Space jump, left / right /
//! middle mouse (and the side buttons) for slots 0 / 1 / 2, Enter start,
//! Esc / Backspace select, aim at the mouse. The keyboard halves are a
//! bopl-port addition (`docs/platform.md`, "Input").

use std::collections::{BTreeMap, HashSet};

use glam::Vec2;
use sdl2::controller::{Axis, Button, GameController};
use sdl2::event::Event;
use sdl2::keyboard::Scancode;
use sdl2::mouse::MouseButton;
use serde::{Deserialize, Serialize};

/// A local input device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceId {
    KeyboardMouse,
    /// Keyboard only, left half (W A S D); shares keys with `KeyboardMouse`,
    /// so only one of the two plays.
    KeyboardLeft,
    /// Keyboard only, right half (arrows and the keys around them).
    KeyboardRight,
    /// SDL joystick instance id (stable while the controller is plugged in).
    Controller(u32),
}

impl DeviceId {
    #[allow(dead_code)] // Lobby / player slots (M4).
    pub fn is_keyboard(self) -> bool {
        !matches!(self, DeviceId::Controller(_))
    }

    pub fn label(self) -> &'static str {
        match self {
            DeviceId::KeyboardMouse => "Keyboard + mouse",
            DeviceId::KeyboardLeft => "Keyboard (left)",
            DeviceId::KeyboardRight => "Keyboard (right)",
            DeviceId::Controller(_) => "Controller",
        }
    }
}

/// A remappable action. The order of the ability slots on screen is
/// slot 0 left, slot 2 top, slot 1 right (ui.md 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Action {
    Jump,
    Ability0,
    Ability1,
    Ability2,
    Left,
    Right,
    Up,
    Down,
    Start,
    Select,
}

pub const ACTIONS: [Action; 10] = [
    Action::Jump,
    Action::Ability0,
    Action::Ability1,
    Action::Ability2,
    Action::Left,
    Action::Right,
    Action::Up,
    Action::Down,
    Action::Start,
    Action::Select,
];

impl Action {
    /// Controllers move with the sticks / d-pad (not remappable).
    pub fn is_move(self) -> bool {
        matches!(
            self,
            Action::Left | Action::Right | Action::Up | Action::Down
        )
    }

    fn button(self) -> Buttons {
        match self {
            Action::Jump => Buttons::JUMP,
            Action::Ability0 => Buttons::ABILITY0,
            Action::Ability1 => Buttons::ABILITY1,
            Action::Ability2 => Buttons::ABILITY2,
            Action::Left => Buttons::LEFT,
            Action::Right => Buttons::RIGHT,
            Action::Up => Buttons::UP,
            Action::Down => Buttons::DOWN,
            Action::Start => Buttons::START,
            Action::Select => Buttons::SELECT,
        }
    }
}

/// An input scheme with its own bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Scheme {
    KeyboardMouse,
    KeyboardLeft,
    KeyboardRight,
    Controller,
}

/// Key / button names per action: SDL scancode names (`Space`,
/// `Right Shift`, ...) and `Mouse1`..`Mouse5` for keyboards, SDL game
/// controller button names (`a`, `x`, `start`, ...) plus `lefttrigger` /
/// `righttrigger` for controllers.
///
/// Per player (ui.md 6): `devices` holds the binding of one device (one
/// character-select box), keyed by [`device_key`]; a device without its own
/// entry uses the per-type `map` (the defaults, or a binding stored by an
/// older version), and missing entries use the defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bindings {
    pub map: BTreeMap<Scheme, BTreeMap<Action, Vec<String>>>,
    pub devices: BTreeMap<String, BTreeMap<Action, Vec<String>>>,
}

/// Device key of the keyboard + mouse player.
pub const KEY_KEYBOARD_MOUSE: &str = "keyboard+mouse";
/// Device key of the left keyboard half.
pub const KEY_KEYBOARD_LEFT: &str = "keyboard-left";
/// Device key of the right keyboard half.
pub const KEY_KEYBOARD_RIGHT: &str = "keyboard-right";
/// Device key of a controller that reports neither a serial number nor a
/// GUID (all such controllers share it: per type).
pub const KEY_CONTROLLER: &str = "controller";

/// The bindings key of a device: fixed for the keyboard schemes; for a
/// controller the key the input hub derived when it was connected
/// (`pad:serial:…`, else `pad:guid:…#n`, else `controller`). A controller
/// missing from `controllers` (the headless driver's pads) gets a key from
/// its instance id.
pub fn device_key(d: DeviceId, controllers: &[ControllerInfo]) -> String {
    match d {
        DeviceId::KeyboardMouse => KEY_KEYBOARD_MOUSE.into(),
        DeviceId::KeyboardLeft => KEY_KEYBOARD_LEFT.into(),
        DeviceId::KeyboardRight => KEY_KEYBOARD_RIGHT.into(),
        DeviceId::Controller(i) => controllers
            .iter()
            .find(|c| c.id == d)
            .map(|c| c.key.clone())
            .unwrap_or_else(|| format!("pad:instance:{i}")),
    }
}

/// A controller's bindings key from its serial number and GUID string;
/// `taken` tells which keys connected controllers already use (identical
/// controllers without a serial get `#1`, `#2`, ... by connection order).
pub fn controller_key(
    serial: Option<&str>,
    guid: Option<&str>,
    taken: impl Fn(&str) -> bool,
) -> String {
    if let Some(s) = serial.map(str::trim).filter(|s| !s.is_empty()) {
        return format!("pad:serial:{s}");
    }
    let Some(g) = guid.filter(|g| !g.is_empty() && g.bytes().any(|b| b != b'0')) else {
        return KEY_CONTROLLER.into();
    };
    (1..)
        .map(|n| format!("pad:guid:{g}#{n}"))
        .find(|k| !taken(k))
        .unwrap_or_default()
}

impl Default for Bindings {
    fn default() -> Self {
        use Action::*;
        let table = |rows: &[(Action, &[&str])]| -> BTreeMap<Action, Vec<String>> {
            rows.iter()
                .map(|(a, ks)| (*a, ks.iter().map(|k| k.to_string()).collect()))
                .collect()
        };
        let mut map = BTreeMap::new();
        map.insert(
            Scheme::KeyboardMouse,
            table(&[
                (Jump, &["Space"]),
                (Ability0, &["Mouse1"]),
                (Ability1, &["Mouse2"]),
                (Ability2, &["Mouse3", "Mouse4", "Mouse5"]),
                (Left, &["A", "Left"]),
                (Right, &["D", "Right"]),
                (Up, &["W", "Up"]),
                (Down, &["S", "Down"]),
                (Start, &["Return"]),
                (Select, &["Escape", "Backspace"]),
            ]),
        );
        map.insert(
            Scheme::KeyboardLeft,
            table(&[
                (Jump, &["Space"]),
                (Ability0, &["F"]),
                (Ability1, &["H"]),
                (Ability2, &["G"]),
                (Left, &["A"]),
                (Right, &["D"]),
                (Up, &["W"]),
                (Down, &["S"]),
                (Start, &["Tab"]),
                (Select, &["Escape"]),
            ]),
        );
        map.insert(
            Scheme::KeyboardRight,
            table(&[
                (Jump, &["Right Shift", "Keypad 0"]),
                (Ability0, &[",", "Keypad 1"]),
                (Ability1, &["/", "Keypad 3"]),
                (Ability2, &[".", "Keypad 2"]),
                (Left, &["Left"]),
                (Right, &["Right"]),
                (Up, &["Up"]),
                (Down, &["Down"]),
                (Start, &["Keypad Enter"]),
                (Select, &["Delete"]),
            ]),
        );
        map.insert(
            Scheme::Controller,
            table(&[
                (Jump, &["a"]),
                (Ability0, &["x"]),
                (Ability1, &["b"]),
                (Ability2, &["y"]),
                (Start, &["start"]),
                (Select, &["back"]),
            ]),
        );
        Bindings {
            map,
            devices: BTreeMap::new(),
        }
    }
}

impl Bindings {
    /// The names bound to `a` in scheme `s` (defaults when unset).
    pub fn get(&self, s: Scheme, a: Action) -> Vec<String> {
        self.map
            .get(&s)
            .and_then(|m| m.get(&a))
            .cloned()
            .unwrap_or_else(|| {
                Bindings::default()
                    .map
                    .get(&s)
                    .and_then(|m| m.get(&a))
                    .cloned()
                    .unwrap_or_default()
            })
    }

    #[allow(dead_code)] // Tests; per-type bindings of older settings files.
    pub fn set(&mut self, s: Scheme, a: Action, names: Vec<String>) {
        self.map.entry(s).or_default().insert(a, names);
    }

    /// The names bound to `a` for the device with bindings key `key` (its
    /// own binding, else the per-type one of scheme `s`, else the default).
    pub fn get_dev(&self, key: &str, s: Scheme, a: Action) -> Vec<String> {
        self.devices
            .get(key)
            .and_then(|m| m.get(&a))
            .cloned()
            .unwrap_or_else(|| self.get(s, a))
    }

    /// Binds `a` of the device `key` (that device only).
    pub fn set_dev(&mut self, key: &str, a: Action, names: Vec<String>) {
        self.devices
            .entry(key.to_string())
            .or_default()
            .insert(a, names);
    }
}

/// A key, mouse button or controller button that went down (binding
/// capture, "press any button to join").
#[derive(Clone, Debug, PartialEq)]
pub struct RawPress {
    /// `KeyboardMouse` for keys and mouse buttons, else the controller.
    pub device: DeviceId,
    /// Binding name (see [`Bindings`]).
    pub name: String,
}

/// A keyboard key or mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Scan(Scancode),
    /// Mouse button 0..4 (left, right, middle, back, forward).
    Mouse(usize),
}

impl Key {
    /// A binding name (`Space`, `Mouse1`, ...) as a key.
    pub fn from_name(n: &str) -> Option<Key> {
        match n {
            "Mouse1" => Some(Key::Mouse(0)),
            "Mouse2" => Some(Key::Mouse(1)),
            "Mouse3" => Some(Key::Mouse(2)),
            "Mouse4" => Some(Key::Mouse(3)),
            "Mouse5" => Some(Key::Mouse(4)),
            n => Scancode::from_name(n).map(Key::Scan),
        }
    }
}

fn keys_of(b: &Bindings, s: Scheme, a: Action) -> Vec<Key> {
    b.get_dev(keyboard_key(s), s, a)
        .iter()
        .filter_map(|n| Key::from_name(n))
        .collect()
}

/// The device key of a keyboard scheme.
pub fn keyboard_key(s: Scheme) -> &'static str {
    match s {
        Scheme::KeyboardMouse => KEY_KEYBOARD_MOUSE,
        Scheme::KeyboardLeft => KEY_KEYBOARD_LEFT,
        Scheme::KeyboardRight => KEY_KEYBOARD_RIGHT,
        Scheme::Controller => KEY_CONTROLLER,
    }
}

/// The keyboard schemes in [`KeyboardKeys`] order.
pub const KEYBOARD_SCHEMES: [Scheme; 3] = [
    Scheme::KeyboardMouse,
    Scheme::KeyboardLeft,
    Scheme::KeyboardRight,
];

/// Resolved keyboard bindings of the three keyboard schemes.
#[derive(Clone, Debug, Default)]
pub struct KeyboardKeys([BTreeMap<Action, Vec<Key>>; 3]);

impl KeyboardKeys {
    pub fn new(b: &Bindings) -> Self {
        Self(KEYBOARD_SCHEMES.map(|s| ACTIONS.iter().map(|&a| (a, keys_of(b, s, a))).collect()))
    }

    /// Held buttons of the keyboard + mouse player and the two halves for
    /// the keys that are down (menu accept/back included).
    pub fn buttons(&self, down: impl Fn(Key) -> bool) -> [Buttons; 3] {
        let bound = |scheme: usize, a: Action| {
            self.0[scheme]
                .get(&a)
                .is_some_and(|ks| ks.iter().any(|&k| down(k)))
        };
        let scan = |s: &[Scancode]| s.iter().any(|&sc| down(Key::Scan(sc)));
        let accept_keys = scan(&[Scancode::Space, Scancode::Return, Scancode::KpEnter]);
        let back_keys = scan(&[Scancode::Escape, Scancode::Backspace]);
        std::array::from_fn(|scheme| {
            let mut b = Buttons::default();
            for a in ACTIONS {
                b.set(a.button(), bound(scheme, a));
            }
            if scheme == 0 {
                b.set(Buttons::ACCEPT, accept_keys);
                b.set(Buttons::BACK, back_keys);
            } else {
                b.set(
                    Buttons::ACCEPT,
                    bound(scheme, Action::Jump) || bound(scheme, Action::Start),
                );
                b.set(Buttons::BACK, bound(scheme, Action::Select));
            }
            b
        })
    }
}

/// Held buttons of the controller with bindings key `key` for its pressed
/// buttons (binding names, `lefttrigger` / `righttrigger` included): the
/// bound actions plus menu accept (South, Start) and back (East, Back).
pub fn controller_buttons(b: &Bindings, key: &str, down: impl Fn(&str) -> bool) -> Buttons {
    let mut out = Buttons::default();
    for a in ACTIONS {
        if !a.is_move() {
            out.set(
                a.button(),
                b.get_dev(key, Scheme::Controller, a)
                    .iter()
                    .any(|n| down(n)),
            );
        }
    }
    out.set(Buttons::ACCEPT, down("a") || down("start"));
    out.set(Buttons::BACK, down("b") || down("back"));
    out
}

/// Logical buttons, as a bit set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Buttons(pub u32);

impl Buttons {
    pub const JUMP: Buttons = Buttons(1 << 0);
    pub const ABILITY0: Buttons = Buttons(1 << 1);
    pub const ABILITY1: Buttons = Buttons(1 << 2);
    pub const ABILITY2: Buttons = Buttons(1 << 3);
    pub const START: Buttons = Buttons(1 << 4);
    pub const SELECT: Buttons = Buttons(1 << 5);
    pub const UP: Buttons = Buttons(1 << 6);
    pub const DOWN: Buttons = Buttons(1 << 7);
    pub const LEFT: Buttons = Buttons(1 << 8);
    pub const RIGHT: Buttons = Buttons(1 << 9);
    /// Menu accept: Space, Enter, South, Start (ui.md 1).
    pub const ACCEPT: Buttons = Buttons(1 << 10);
    /// Menu back: Esc, Backspace, East, Select.
    pub const BACK: Buttons = Buttons(1 << 11);

    pub fn contains(self, b: Buttons) -> bool {
        self.0 & b.0 == b.0
    }

    #[allow(dead_code)] // Virtual inputs (bots, replays).
    pub fn set(&mut self, b: Buttons, on: bool) {
        if on {
            self.0 |= b.0;
        } else {
            self.0 &= !b.0;
        }
    }
}

impl std::ops::BitOr for Buttons {
    type Output = Buttons;
    fn bitor(self, o: Buttons) -> Buttons {
        Buttons(self.0 | o.0)
    }
}

/// One device's input for one simulation tick. Axes are y-up, clamped to
/// the unit disk after the dead zone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InputState {
    pub move_axis: Vec2,
    /// Stick aim (right stick, or the move stick when the right one is
    /// centred); zero when centred. Keyboards: the move keys.
    pub aim_axis: Vec2,
    /// Mouse position in window pixels (keyboard + mouse only).
    pub pointer: Option<Vec2>,
    pub held: Buttons,
    /// Went down since the previous tick (taps shorter than a tick count).
    pub pressed: Buttons,
    pub released: Buttons,
}

/// Radial dead zone with rescaling, then clamp to the unit disk.
pub fn deadzone(v: Vec2, dz: f32) -> Vec2 {
    let len = v.length();
    if len <= dz {
        return Vec2::ZERO;
    }
    let scaled = (len - dz) / (1.0 - dz);
    v / len * scaled.min(1.0)
}

const STICK_DEADZONE: f32 = 0.2;
/// Menu moves need the axis beyond 0.4 (ui.md 1).
const MENU_DEADZONE: f32 = 0.4;
const TRIGGER_THRESHOLD: f32 = 0.35;

fn axis_value(raw: i16) -> f32 {
    (raw as f32 / 32767.0).clamp(-1.0, 1.0)
}

#[derive(Default)]
struct DeviceSlot {
    held: Buttons,
    latched_press: Buttons,
    latched_release: Buttons,
    last_held: Buttons,
}

impl DeviceSlot {
    fn set(&mut self, b: Buttons, down: bool) {
        if down && !self.held.contains(b) {
            self.latched_press.set(b, true);
        }
        if !down && self.held.contains(b) {
            self.latched_release.set(b, true);
        }
        self.held.set(b, down);
    }

    fn take_edges(&mut self) -> (Buttons, Buttons) {
        let pressed = Buttons(self.latched_press.0 | (self.held.0 & !self.last_held.0));
        let released = Buttons(self.latched_release.0 | (!self.held.0 & self.last_held.0));
        self.latched_press = Buttons::default();
        self.latched_release = Buttons::default();
        self.last_held = self.held;
        (pressed, released)
    }
}

/// A connected controller, for menus and `--list-devices`.
#[derive(Clone, Debug)]
pub struct ControllerInfo {
    pub id: DeviceId,
    pub name: String,
    pub has_rumble: bool,
    /// Bindings key ([`controller_key`]).
    pub key: String,
}

/// Tracks every local device.
pub struct InputHub {
    subsystem: sdl2::GameControllerSubsystem,
    /// Open controllers by instance id: the controller, its state and its
    /// bindings key.
    controllers: BTreeMap<u32, (GameController, DeviceSlot, String)>,
    keyboard: DeviceSlot,
    keyboard_left: DeviceSlot,
    keyboard_right: DeviceSlot,
    keys_down: HashSet<Scancode>,
    mouse_buttons: [bool; 5],
    pointer: Option<Vec2>,
    bindings: Bindings,
    /// Resolved keyboard bindings per scheme (KeyboardMouse, Left, Right).
    keys: KeyboardKeys,
    raw: Vec<RawPress>,
    trig_down: BTreeMap<u32, [bool; 2]>,
}

#[allow(dead_code)] // Part of the API the menus and the match use (M4+).
impl InputHub {
    pub fn new(subsystem: sdl2::GameControllerSubsystem) -> Self {
        // Optional extra mappings next to the executable (SDL's built-in
        // database covers common pads).
        if let Some(path) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("gamecontrollerdb.txt")))
            .filter(|p| p.exists())
        {
            match subsystem.load_mappings(&path) {
                Ok(n) => log::info!("loaded {n} controller mappings from {}", path.display()),
                Err(e) => log::warn!("controller mappings {}: {e}", path.display()),
            }
        }
        let mut hub = Self {
            subsystem,
            controllers: BTreeMap::new(),
            keyboard: DeviceSlot::default(),
            keyboard_left: DeviceSlot::default(),
            keyboard_right: DeviceSlot::default(),
            keys_down: HashSet::new(),
            mouse_buttons: [false; 5],
            pointer: None,
            bindings: Bindings::default(),
            keys: Default::default(),
            raw: Vec::new(),
            trig_down: BTreeMap::new(),
        };
        hub.set_bindings(Bindings::default());
        hub
    }

    /// Applies remapped bindings.
    pub fn set_bindings(&mut self, b: Bindings) {
        self.keys = KeyboardKeys::new(&b);
        self.bindings = b;
        self.update_keyboard();
    }

    /// Key / button presses since the last call.
    pub fn take_raw(&mut self) -> Vec<RawPress> {
        std::mem::take(&mut self.raw)
    }

    /// Feeds one SDL event. Returns true if it was an input event.
    pub fn handle_event(&mut self, event: &Event) -> bool {
        match event {
            Event::ControllerDeviceAdded { which, .. } => match self.subsystem.open(*which) {
                Ok(c) => {
                    let id = c.instance_id();
                    if !self.controllers.contains_key(&id) {
                        let (serial, guid) = controller_ids(id);
                        let key = controller_key(serial.as_deref(), guid.as_deref(), |k| {
                            self.controllers.values().any(|(_, _, ck)| ck == k)
                        });
                        log::info!("controller connected: #{id} {} ({key})", c.name());
                        self.controllers.insert(id, (c, DeviceSlot::default(), key));
                    }
                }
                Err(e) => log::warn!("cannot open controller {which}: {e}"),
            },
            Event::ControllerDeviceRemoved { which, .. } => {
                if let Some((c, _, _)) = self.controllers.remove(which) {
                    log::info!("controller disconnected: #{which} {}", c.name());
                }
                self.trig_down.remove(which);
            }
            Event::ControllerButtonDown { which, button, .. }
            | Event::ControllerButtonUp { which, button, .. } => {
                let down = matches!(event, Event::ControllerButtonDown { .. });
                log::debug!(
                    "controller #{which} {button:?} {}",
                    if down { "down" } else { "up" }
                );
                if down {
                    self.raw.push(RawPress {
                        device: DeviceId::Controller(*which),
                        name: button.string(),
                    });
                }
                if let Some((_, slot, _)) = self.controllers.get_mut(which) {
                    for b in menu_button(*button) {
                        slot.set(b, down);
                    }
                }
            }
            Event::KeyDown {
                scancode: Some(sc),
                repeat: false,
                ..
            } => {
                self.keys_down.insert(*sc);
                self.raw.push(RawPress {
                    device: DeviceId::KeyboardMouse,
                    name: sc.name().to_string(),
                });
                self.update_keyboard();
            }
            Event::KeyUp {
                scancode: Some(sc), ..
            } => {
                self.keys_down.remove(sc);
                self.update_keyboard();
            }
            Event::MouseButtonDown { mouse_btn, .. } | Event::MouseButtonUp { mouse_btn, .. } => {
                let down = matches!(event, Event::MouseButtonDown { .. });
                let i = match mouse_btn {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                    MouseButton::X1 => 3,
                    MouseButton::X2 => 4,
                    _ => return true,
                };
                self.mouse_buttons[i] = down;
                if down {
                    self.raw.push(RawPress {
                        device: DeviceId::KeyboardMouse,
                        name: format!("Mouse{}", i + 1),
                    });
                }
                self.update_keyboard();
            }
            Event::MouseMotion { x, y, .. } => self.pointer = Some(Vec2::new(*x as f32, *y as f32)),
            _ => return false,
        }
        true
    }

    fn update_keyboard(&mut self) {
        let st = self.keys.buttons(|k| match k {
            Key::Scan(sc) => self.keys_down.contains(&sc),
            Key::Mouse(m) => self.mouse_buttons[m],
        });
        for (slot, b) in [
            &mut self.keyboard,
            &mut self.keyboard_left,
            &mut self.keyboard_right,
        ]
        .into_iter()
        .zip(st)
        {
            for bit in 0..12 {
                let one = Buttons(1 << bit);
                slot.set(one, b.contains(one));
            }
        }
    }

    /// Connected devices: keyboard + mouse first, then controllers by id.
    pub fn devices(&self) -> Vec<DeviceId> {
        std::iter::once(DeviceId::KeyboardMouse)
            .chain(self.controllers.keys().map(|&i| DeviceId::Controller(i)))
            .collect()
    }

    pub fn controllers(&self) -> Vec<ControllerInfo> {
        self.controllers
            .iter()
            .map(|(&i, (c, _, key))| ControllerInfo {
                id: DeviceId::Controller(i),
                name: c.name(),
                has_rumble: c.has_rumble(),
                key: key.clone(),
            })
            .collect()
    }

    /// Rumble a controller (0..1 strengths); ignored when unsupported.
    pub fn rumble(&mut self, id: DeviceId, low: f32, high: f32, ms: u32) {
        if let DeviceId::Controller(i) = id
            && let Some((c, _, _)) = self.controllers.get_mut(&i)
        {
            let q = |x: f32| (x.clamp(0.0, 1.0) * 65535.0) as u16;
            let _ = c.set_rumble(q(low), q(high), ms);
        }
    }

    /// Samples every device for one tick (consumes press/release edges).
    pub fn snapshot(&mut self, out: &mut Vec<(DeviceId, InputState)>) {
        out.clear();
        let axis = |slot: &DeviceSlot| {
            let h = |b: Buttons| slot.held.contains(b) as i32 as f32;
            Vec2::new(
                h(Buttons::RIGHT) - h(Buttons::LEFT),
                h(Buttons::UP) - h(Buttons::DOWN),
            )
            .clamp_length_max(1.0)
        };
        for (id, pointer) in [
            (DeviceId::KeyboardMouse, self.pointer),
            (DeviceId::KeyboardLeft, None),
            (DeviceId::KeyboardRight, None),
        ] {
            let slot = match id {
                DeviceId::KeyboardMouse => &mut self.keyboard,
                DeviceId::KeyboardLeft => &mut self.keyboard_left,
                _ => &mut self.keyboard_right,
            };
            let mv = axis(slot);
            let (pressed, released) = slot.take_edges();
            out.push((
                id,
                InputState {
                    move_axis: mv,
                    aim_axis: mv.normalize_or_zero(),
                    pointer,
                    held: slot.held,
                    pressed,
                    released,
                },
            ));
        }
        // Controllers: axes and bound buttons are polled (taps on triggers
        // count too), menu buttons come from events.
        for (&id, (c, slot, key)) in &mut self.controllers {
            let stick = |x, y| {
                deadzone(
                    Vec2::new(axis_value(c.axis(x)), -axis_value(c.axis(y))),
                    STICK_DEADZONE,
                )
            };
            let left = stick(Axis::LeftX, Axis::LeftY);
            let right = stick(Axis::RightX, Axis::RightY);
            let dpad = Vec2::new(
                c.button(Button::DPadRight) as i32 as f32
                    - c.button(Button::DPadLeft) as i32 as f32,
                c.button(Button::DPadUp) as i32 as f32 - c.button(Button::DPadDown) as i32 as f32,
            );
            // Left stick, then d-pad, then right stick (ui.md 1).
            let mv = if left != Vec2::ZERO {
                left
            } else if dpad != Vec2::ZERO {
                dpad.normalize()
            } else {
                right
            };
            let trig = [
                axis_value(c.axis(Axis::TriggerLeft)) > TRIGGER_THRESHOLD,
                axis_value(c.axis(Axis::TriggerRight)) > TRIGGER_THRESHOLD,
            ];
            let was = self.trig_down.entry(id).or_default();
            for (i, name) in ["lefttrigger", "righttrigger"].iter().enumerate() {
                if trig[i] && !was[i] {
                    self.raw.push(RawPress {
                        device: DeviceId::Controller(id),
                        name: (*name).into(),
                    });
                }
            }
            *was = trig;
            let held = |names: &[String]| {
                names.iter().any(|n| match n.as_str() {
                    "lefttrigger" => trig[0],
                    "righttrigger" => trig[1],
                    n => Button::from_string(n).is_some_and(|b| c.button(b)),
                })
            };
            for a in ACTIONS {
                if !a.is_move() {
                    slot.set(
                        a.button(),
                        held(&self.bindings.get_dev(key, Scheme::Controller, a)),
                    );
                }
            }
            let menu = deadzone(mv, MENU_DEADZONE);
            slot.set(Buttons::LEFT, menu.x < -0.0 && menu.x.abs() > menu.y.abs());
            slot.set(Buttons::RIGHT, menu.x > 0.0 && menu.x.abs() > menu.y.abs());
            slot.set(Buttons::UP, menu.y > 0.0 && menu.y.abs() > menu.x.abs());
            slot.set(Buttons::DOWN, menu.y < 0.0 && menu.y.abs() > menu.x.abs());
            let (pressed, released) = slot.take_edges();
            out.push((
                DeviceId::Controller(id),
                InputState {
                    move_axis: mv,
                    aim_axis: if right != Vec2::ZERO { right } else { mv },
                    pointer: None,
                    held: slot.held,
                    pressed,
                    released,
                },
            ));
        }
    }
}

/// Serial number and GUID string of an open controller (SDL 2.0.14+ for the
/// serial; `None` when the driver reports none).
fn controller_ids(instance: u32) -> (Option<String>, Option<String>) {
    use sdl2::sys;
    use std::ffi::CStr;
    // SAFETY: the controller with this instance id was just opened and
    // stays open (the hub owns it); SDL returns pointers it owns (or null)
    // that are copied at once; the GUID buffer is 33 bytes as SDL asks.
    unsafe {
        let raw = sys::SDL_GameControllerFromInstanceID(instance as sys::SDL_JoystickID);
        if raw.is_null() {
            return (None, None);
        }
        let p = sys::SDL_GameControllerGetSerial(raw);
        let serial = (!p.is_null())
            .then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
            .filter(|s| !s.trim().is_empty());
        let joy = sys::SDL_GameControllerGetJoystick(raw);
        let guid = (!joy.is_null()).then(|| {
            let g = sys::SDL_JoystickGetGUID(joy);
            let mut buf = [0 as std::ffi::c_char; 33];
            sys::SDL_JoystickGetGUIDString(g, buf.as_mut_ptr(), buf.len() as i32);
            CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
        });
        (serial, guid)
    }
}

/// Controller buttons that arrive as events and drive menus (gameplay
/// buttons are polled from the bindings in `snapshot`).
fn menu_button(b: Button) -> Vec<Buttons> {
    match b {
        Button::A | Button::Start => vec![Buttons::ACCEPT],
        Button::B | Button::Back => vec![Buttons::BACK],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadzone_rescales() {
        assert_eq!(deadzone(Vec2::new(0.1, 0.1), 0.2), Vec2::ZERO);
        let v = deadzone(Vec2::new(0.6, 0.0), 0.2);
        assert!((v.x - 0.5).abs() < 1e-6);
        let v = deadzone(Vec2::new(1.0, 1.0), 0.2);
        assert!((v.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn short_taps_are_latched() {
        let mut s = DeviceSlot::default();
        s.set(Buttons::JUMP, true);
        s.set(Buttons::JUMP, false);
        let (p, r) = s.take_edges();
        assert!(p.contains(Buttons::JUMP) && r.contains(Buttons::JUMP));
        let (p, r) = s.take_edges();
        assert_eq!((p, r), (Buttons::default(), Buttons::default()));
        s.set(Buttons::ABILITY0, true);
        let (p, _) = s.take_edges();
        assert!(p.contains(Buttons::ABILITY0));
        let (p, _) = s.take_edges();
        assert!(!p.contains(Buttons::ABILITY0), "held is not pressed again");
    }

    #[test]
    fn rebound_keys_drive_the_buttons() {
        let mut b = Bindings::default();
        b.set(Scheme::KeyboardMouse, Action::Jump, vec!["K".into()]);
        b.set(Scheme::KeyboardLeft, Action::Ability0, vec!["Q".into()]);
        let keys = KeyboardKeys::new(&b);
        let down = |names: &'static [&'static str]| {
            move |k: Key| names.iter().any(|n| Key::from_name(n) == Some(k))
        };
        let [km, kl, kr] = keys.buttons(down(&["K"]));
        assert!(km.contains(Buttons::JUMP) && !kl.contains(Buttons::JUMP));
        assert!(!kr.contains(Buttons::JUMP));
        let [km, _, _] = keys.buttons(down(&["Space"]));
        assert!(!km.contains(Buttons::JUMP), "Space no longer jumps");
        assert!(km.contains(Buttons::ACCEPT), "but still accepts in menus");
        let [_, kl, _] = keys.buttons(down(&["Q"]));
        assert!(kl.contains(Buttons::ABILITY0));
        let [_, kl, _] = keys.buttons(down(&["F"]));
        assert!(!kl.contains(Buttons::ABILITY0));
        // Controllers: the bound names, menu accept/back fixed.
        b.set(
            Scheme::Controller,
            Action::Jump,
            vec!["rightshoulder".into()],
        );
        let pad = controller_buttons(&b, "pad:serial:1", |n| n == "rightshoulder");
        assert!(pad.contains(Buttons::JUMP) && !pad.contains(Buttons::ACCEPT));
        let pad = controller_buttons(&b, "pad:serial:1", |n| n == "a");
        assert!(!pad.contains(Buttons::JUMP) && pad.contains(Buttons::ACCEPT));
        // Settings round trip (scheme and action names as JSON keys).
        let j = serde_json::to_string(&b).unwrap();
        let back: Bindings = serde_json::from_str(&j).unwrap();
        assert_eq!(back, b);
    }

    #[test]
    fn per_device_bindings() {
        let mut b = Bindings::default();
        b.set_dev("pad:serial:A", Action::Jump, vec!["y".into()]);
        b.set_dev(KEY_KEYBOARD_LEFT, Action::Jump, vec!["Q".into()]);
        // Only that pad jumps with Y; another pad keeps the defaults.
        assert!(controller_buttons(&b, "pad:serial:A", |n| n == "y").contains(Buttons::JUMP));
        assert!(!controller_buttons(&b, "pad:serial:A", |n| n == "a").contains(Buttons::JUMP));
        assert!(controller_buttons(&b, "pad:serial:B", |n| n == "a").contains(Buttons::JUMP));
        // The per-type binding (older settings) is the fallback.
        b.set(
            Scheme::Controller,
            Action::Jump,
            vec!["rightshoulder".into()],
        );
        let rs = |n: &str| n == "rightshoulder";
        assert!(controller_buttons(&b, "pad:serial:B", rs).contains(Buttons::JUMP));
        assert!(!controller_buttons(&b, "pad:serial:A", rs).contains(Buttons::JUMP));
        // Keyboard halves are devices of their own.
        let keys = KeyboardKeys::new(&b);
        let [km, kl, _] = keys.buttons(|k| k == Key::from_name("Q").unwrap());
        assert!(kl.contains(Buttons::JUMP) && !km.contains(Buttons::JUMP));
        // Settings round trip, and settings without `devices` still load.
        let j = serde_json::to_string(&b).unwrap();
        assert_eq!(serde_json::from_str::<Bindings>(&j).unwrap(), b);
        let old: Bindings = serde_json::from_str(r#"{"map":{}}"#).unwrap();
        assert!(old.devices.is_empty());
        assert_eq!(
            old.get_dev("x", Scheme::Controller, Action::Jump),
            vec!["a"]
        );
    }

    #[test]
    fn controller_keys() {
        let none = |_: &str| false;
        assert_eq!(
            controller_key(Some(" 12ab "), Some("0300"), none),
            "pad:serial:12ab"
        );
        let g = "030000005e0400008e02000014010000";
        assert_eq!(
            controller_key(None, Some(g), none),
            format!("pad:guid:{g}#1")
        );
        let first = format!("pad:guid:{g}#1");
        let taken = |k: &str| k == first;
        assert_eq!(
            controller_key(Some(""), Some(g), taken),
            format!("pad:guid:{g}#2")
        );
        assert_eq!(controller_key(None, Some("0000"), none), KEY_CONTROLLER);
        assert_eq!(controller_key(None, None, none), KEY_CONTROLLER);
        assert_eq!(device_key(DeviceId::KeyboardLeft, &[]), KEY_KEYBOARD_LEFT);
        assert_eq!(device_key(DeviceId::Controller(7), &[]), "pad:instance:7");
    }

    #[test]
    fn default_bindings_resolve() {
        let b = Bindings::default();
        for s in [
            Scheme::KeyboardMouse,
            Scheme::KeyboardLeft,
            Scheme::KeyboardRight,
        ] {
            for a in ACTIONS {
                let keys = keys_of(&b, s, a);
                assert!(!keys.is_empty(), "{s:?} {a:?} has no key");
            }
        }
        for a in ACTIONS.iter().filter(|a| !a.is_move()) {
            for n in b.get(Scheme::Controller, *a) {
                assert!(Button::from_string(&n).is_some(), "{n}");
            }
        }
        assert_eq!(
            keys_of(&b, Scheme::KeyboardMouse, Action::Ability2).len(),
            3
        );
    }
}

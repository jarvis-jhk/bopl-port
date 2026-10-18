//! Key / button remapping (ui.md 6): the steps of a rebinding, which
//! presses may be bound, clashes, display names, and turning the chosen
//! controls into [`Bindings`] for one device (one binding per player box:
//! each keyboard scheme and each controller by its serial / GUID key,
//! stored in `settings.json`).
//!
//! The original binds, in order, jump and the three ability buttons in
//! on-screen order (left = slot 0, top = slot 2, right = slot 1) and, for
//! keyboard players, up, left, down and right. A control already chosen in
//! this rebinding is refused (`smallError`, the clashing entry wiggles);
//! Esc (keyboards) or Back (controllers) cancels. bopl-port additions: the
//! keyboard halves bind like the keyboard (without the mouse); their own
//! Start / Select keys and keys of the other half cannot be chosen; Start,
//! Back, Guide and the d-pad of a controller stay reserved like in the
//! original; a cancelled rebinding keeps the previous binding (the original
//! leaves the cleared one); the keyboard + mouse player keeps the arrow
//! keys as a second binding of the four directions unless one of them was
//! chosen for another action.

use crate::input::{Action, Bindings, DeviceId, RawPress, Scheme, keyboard_key};

/// Actions bound by the steps, in order (ui.md 6).
pub const STEPS: [Action; 8] = [
    Action::Jump,
    Action::Ability0,
    Action::Ability2,
    Action::Ability1,
    Action::Up,
    Action::Left,
    Action::Down,
    Action::Right,
];

/// Localisation keys of the steps (`KeyListKeyboardMouse`; spaces become
/// line breaks and `_` a space on screen).
pub const STEP_KEYS: [&str; 8] = [
    "click jump",
    "click ability_left",
    "click ability_top",
    "click ability_right",
    "click move_up",
    "click move_left",
    "click move_down",
    "click move_right",
];

/// Steps shown with the "mouse only" hint (`MouseOnlyKeyList`; the
/// original shows it without enforcing it).
pub const MOUSE_ONLY: [bool; 8] = [false, true, true, true, false, false, false, false];

/// Circle indicator sprite per step (`CircleIndicators_keyboard`; the
/// controller list is the first four) and its rotation in degrees.
pub const STEP_SPRITES: [&str; 8] = [
    "sharedassets2.assets:10",
    "sharedassets2.assets:26",
    "sharedassets2.assets:17",
    "sharedassets2.assets:22",
    "sharedassets2.assets:23",
    "sharedassets2.assets:23",
    "sharedassets2.assets:23",
    "sharedassets2.assets:23",
];
pub const STEP_ROTATIONS: [f32; 8] = [0.0, 0.0, 0.0, 0.0, 90.0, 180.0, 270.0, 0.0];

/// The binding scheme of a device.
pub fn scheme_of(d: DeviceId) -> Scheme {
    match d {
        DeviceId::KeyboardMouse => Scheme::KeyboardMouse,
        DeviceId::KeyboardLeft => Scheme::KeyboardLeft,
        DeviceId::KeyboardRight => Scheme::KeyboardRight,
        DeviceId::Controller(_) => Scheme::Controller,
    }
}

/// Number of steps of a scheme.
pub fn step_count(s: Scheme) -> usize {
    match s {
        Scheme::Controller => 4,
        _ => 8,
    }
}

/// A readable name of a binding name (`InputControl.displayName` style).
pub fn display_name(name: &str) -> String {
    let fixed = match name {
        "Mouse1" => "Left Button",
        "Mouse2" => "Right Button",
        "Mouse3" => "Middle Button",
        "Mouse4" => "Back",
        "Mouse5" => "Forward",
        "a" => "A",
        "b" => "B",
        "x" => "X",
        "y" => "Y",
        "leftshoulder" => "Left Shoulder",
        "rightshoulder" => "Right Shoulder",
        "lefttrigger" => "Left Trigger",
        "righttrigger" => "Right Trigger",
        "leftstick" => "Left Stick Press",
        "rightstick" => "Right Stick Press",
        "misc1" => "Share",
        "touchpad" => "Touchpad",
        "paddle1" => "Paddle 1",
        "paddle2" => "Paddle 2",
        "paddle3" => "Paddle 3",
        "paddle4" => "Paddle 4",
        _ => "",
    };
    if fixed.is_empty() {
        name.to_string()
    } else {
        fixed.to_string()
    }
}

/// What a press did to a rebinding.
#[derive(Clone, Debug, PartialEq)]
pub enum RebindEvent {
    /// Bound to step `n` (`navigateMenu`).
    Bound(usize),
    /// Already chosen for step `n` (`smallError`, the entry wiggles).
    Clash(usize),
    /// Not bindable (reserved, the other keyboard half's key).
    Refused,
    /// The last step was bound: the new binding (`accept2`).
    Done(Bindings),
    /// Esc / Back (`decline2`): the previous binding stays.
    Cancelled,
}

/// A rebinding in progress for one box.
#[derive(Clone, Debug, PartialEq)]
pub struct Rebind {
    pub scheme: Scheme,
    /// Controller of a controller box.
    pub device: DeviceId,
    /// Bindings key of the device ([`crate::input::device_key`]).
    pub key: String,
    pub step: usize,
    /// Chosen names per step (`None` until bound).
    pub chosen: Vec<Option<String>>,
    /// Presses of the tick that started the rebinding are not bound.
    pub armed: bool,
}

impl Rebind {
    pub fn new(device: DeviceId, key: String) -> Self {
        let scheme = scheme_of(device);
        Self {
            scheme,
            device,
            key,
            step: 0,
            chosen: vec![None; step_count(scheme)],
            armed: false,
        }
    }

    /// Whether a raw press belongs to this box's device.
    fn mine(&self, p: &RawPress) -> bool {
        match self.device {
            DeviceId::Controller(_) => p.device == self.device,
            _ => p.device == DeviceId::KeyboardMouse,
        }
    }

    fn is_mouse(name: &str) -> bool {
        name.starts_with("Mouse") && name.len() == 6
    }

    fn cancels(&self, name: &str, current: &Bindings) -> bool {
        match self.scheme {
            Scheme::Controller => name == "back",
            Scheme::KeyboardMouse => name == "Escape",
            s => {
                name == "Escape"
                    || current
                        .get_dev(&self.key, s, Action::Select)
                        .iter()
                        .any(|n| n == name)
            }
        }
    }

    fn reserved(&self, name: &str, current: &Bindings) -> bool {
        match self.scheme {
            Scheme::Controller => {
                matches!(name, "start" | "back" | "guide") || name.starts_with("dp")
            }
            Scheme::KeyboardMouse => false,
            s => {
                if Self::is_mouse(name) {
                    return true;
                }
                let other = if s == Scheme::KeyboardLeft {
                    Scheme::KeyboardRight
                } else {
                    Scheme::KeyboardLeft
                };
                let own = [Action::Start, Action::Select]
                    .iter()
                    .any(|&a| current.get_dev(&self.key, s, a).iter().any(|n| n == name));
                let theirs = crate::input::ACTIONS.iter().any(|&a| {
                    current
                        .get_dev(keyboard_key(other), other, a)
                        .iter()
                        .any(|n| n == name)
                });
                own || theirs
            }
        }
    }

    /// Feeds one raw press (call [`Rebind::arm`] after the starting tick).
    pub fn feed(&mut self, p: &RawPress, current: &Bindings) -> Option<RebindEvent> {
        if !self.armed || !self.mine(p) {
            return None;
        }
        let name = p.name.as_str();
        if self.cancels(name, current) {
            return Some(RebindEvent::Cancelled);
        }
        if self.reserved(name, current) {
            return Some(RebindEvent::Refused);
        }
        if let Some(i) = self.chosen.iter().position(|c| c.as_deref() == Some(name)) {
            return Some(RebindEvent::Clash(i));
        }
        self.chosen[self.step] = Some(name.to_string());
        self.step += 1;
        if self.step >= self.chosen.len() {
            return Some(RebindEvent::Done(self.result(current)));
        }
        Some(RebindEvent::Bound(self.step - 1))
    }

    /// Presses from now on are bound.
    pub fn arm(&mut self) {
        self.armed = true;
    }

    /// `current` with this device's steps replaced by the chosen controls
    /// (other devices keep theirs).
    fn result(&self, current: &Bindings) -> Bindings {
        let mut b = current.clone();
        let chosen: Vec<String> = self.chosen.iter().flatten().cloned().collect();
        for (k, name) in self.chosen.iter().enumerate() {
            let Some(name) = name else { continue };
            let action = STEPS[k];
            let mut names = vec![name.clone()];
            if self.scheme == Scheme::KeyboardMouse && action.is_move() {
                let arrow = match action {
                    Action::Up => "Up",
                    Action::Down => "Down",
                    Action::Left => "Left",
                    _ => "Right",
                };
                if !chosen.iter().any(|c| c == arrow) {
                    names.push(arrow.to_string());
                }
            }
            b.set_dev(&self.key, action, names);
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: &str) -> RawPress {
        RawPress {
            device: DeviceId::KeyboardMouse,
            name: n.into(),
        }
    }

    #[test]
    fn keyboard_mouse_rebinding() {
        let cur = Bindings::default();
        let mut r = Rebind::new(DeviceId::KeyboardMouse, "keyboard+mouse".into());
        assert_eq!(r.feed(&key("J"), &cur), None, "not armed yet");
        r.arm();
        assert_eq!(r.feed(&key("J"), &cur), Some(RebindEvent::Bound(0)));
        assert_eq!(r.feed(&key("J"), &cur), Some(RebindEvent::Clash(0)));
        for (k, n) in ["Mouse1", "Mouse3", "Mouse2", "I", "Left"]
            .iter()
            .enumerate()
        {
            assert_eq!(r.feed(&key(n), &cur), Some(RebindEvent::Bound(k + 1)));
        }
        assert_eq!(r.feed(&key("K"), &cur), Some(RebindEvent::Bound(6)));
        let Some(RebindEvent::Done(b)) = r.feed(&key("L"), &cur) else {
            panic!("not done");
        };
        let s = Scheme::KeyboardMouse;
        let g = |a| b.get_dev("keyboard+mouse", s, a);
        assert_eq!(g(Action::Jump), vec!["J"]);
        assert_eq!(g(Action::Ability0), vec!["Mouse1"]);
        assert_eq!(g(Action::Ability2), vec!["Mouse3"]);
        assert_eq!(g(Action::Ability1), vec!["Mouse2"]);
        assert_eq!(g(Action::Up), vec!["I", "Up"]);
        // "Left" was chosen for left itself: no second copy.
        assert_eq!(g(Action::Left), vec!["Left"]);
        assert_eq!(g(Action::Right), vec!["L", "Right"]);
        // The per-type tables and other devices untouched.
        assert_eq!(b.map, cur.map);
        assert_eq!(b.devices.len(), 1);
    }

    #[test]
    fn cancel_and_reserved() {
        let cur = Bindings::default();
        let mut r = Rebind::new(DeviceId::KeyboardLeft, "keyboard-left".into());
        r.arm();
        assert_eq!(r.feed(&key("Mouse1"), &cur), Some(RebindEvent::Refused));
        // Right half's jump and the left half's own start.
        assert_eq!(
            r.feed(&key("Right Shift"), &cur),
            Some(RebindEvent::Refused)
        );
        assert_eq!(r.feed(&key("Tab"), &cur), Some(RebindEvent::Refused));
        assert_eq!(r.feed(&key("Q"), &cur), Some(RebindEvent::Bound(0)));
        assert_eq!(r.feed(&key("Escape"), &cur), Some(RebindEvent::Cancelled));
        let pad = DeviceId::Controller(3);
        let mut r = Rebind::new(pad, "pad:serial:P3".into());
        r.arm();
        let btn = |n: &str| RawPress {
            device: pad,
            name: n.into(),
        };
        assert_eq!(r.feed(&key("Q"), &cur), None, "keyboard is not this pad");
        assert_eq!(
            r.feed(
                &RawPress {
                    device: DeviceId::Controller(4),
                    name: "a".into()
                },
                &cur
            ),
            None
        );
        assert_eq!(r.feed(&btn("dpup"), &cur), Some(RebindEvent::Refused));
        assert_eq!(r.feed(&btn("start"), &cur), Some(RebindEvent::Refused));
        for (k, n) in ["rightshoulder", "a", "lefttrigger"].iter().enumerate() {
            assert_eq!(r.feed(&btn(n), &cur), Some(RebindEvent::Bound(k)));
        }
        let Some(RebindEvent::Done(b)) = r.feed(&btn("x"), &cur) else {
            panic!()
        };
        let s = Scheme::Controller;
        let g = |k: &str, a| b.get_dev(k, s, a);
        assert_eq!(g("pad:serial:P3", Action::Jump), vec!["rightshoulder"]);
        assert_eq!(g("pad:serial:P3", Action::Ability0), vec!["a"]);
        assert_eq!(g("pad:serial:P3", Action::Ability2), vec!["lefttrigger"]);
        assert_eq!(g("pad:serial:P3", Action::Ability1), vec!["x"]);
        assert_eq!(g("pad:serial:P3", Action::Start), vec!["start"]);
        // Another controller keeps the defaults.
        assert_eq!(g("pad:serial:P4", Action::Jump), vec!["a"]);
        let mut r = Rebind::new(pad, "pad:serial:P3".into());
        r.arm();
        assert_eq!(r.feed(&btn("back"), &cur), Some(RebindEvent::Cancelled));
    }

    #[test]
    fn names() {
        assert_eq!(display_name("Mouse2"), "Right Button");
        assert_eq!(display_name("leftshoulder"), "Left Shoulder");
        assert_eq!(display_name("Right Shift"), "Right Shift");
    }
}

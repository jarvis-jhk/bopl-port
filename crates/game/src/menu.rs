//! Main menu (ui.md 3) on the original's `MainMenu` canvas: the main list,
//! the settings and language sub-menus and the credits.

use bopl_render::Color;
use glam::Vec2;

use crate::app::DrawContext;
use crate::audio::Audio;
use crate::input::{Buttons, DeviceId, InputState};
use crate::shared::Shared;
use crate::ui::{Canvas, L10n, UiDraw, ui_pass};

pub enum MenuOutcome {
    Stay,
    Play,
    /// The tutorial, started by this device (a controller takes the player).
    Tutorial(DeviceId),
    /// Online (Warpzone): host a room or join one.
    Online(crate::online::scene::OnlineIntent),
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sub {
    Main,
    Settings,
    Language,
    Credits,
    /// Ours: host / host private / join by code / room list / name / back.
    Online,
    /// Ours: the room browser (public rooms from Warpzone).
    Rooms,
}

/// Rows of the online sub-menu (texts; the original has no such menu).
const ONLINE_ITEMS: [&str; 6] = [
    "host game",
    "host private",
    "join code",
    "room list",
    "name",
    "back",
];
/// Rows of the room browser: a page of rooms, refresh, back.
const ROOM_ITEMS: [&str; 8] = [
    "rooms:0", "rooms:1", "rooms:2", "rooms:3", "rooms:4", "rooms:5", "refresh", "back",
];
/// Index of the "join code" and "name" rows in the online sub-menu.
const ROW_CODE: usize = 2;
const ROW_NAME: usize = 4;

/// A text being typed in the online sub-menu.
#[derive(Clone, Debug, PartialEq)]
enum Typing {
    Code(String),
    Name(String),
}

/// One selectable row: node, its border image and texts.
#[derive(Clone, Debug)]
struct Item {
    node: usize,
    border: Option<usize>,
    texts: Vec<usize>,
    selected_border: Color,
    unselected_border: Color,
    selected_text: Color,
    unselected_text: Color,
    /// Not available in bopl-port (yet): drawn dimmed, refuses.
    disabled: bool,
}

/// A settings row with its value texts.
#[derive(Clone, Debug)]
struct SettingRow {
    item: usize,
    /// Value child nodes in value order (one is shown).
    values: Vec<usize>,
}

pub struct MainMenu {
    canvas: Canvas,
    l10n: L10n,
    sub: Sub,
    selected: usize,
    menus: [Option<usize>; 6],
    items: [Vec<Item>; 6],
    /// All rows of the room browser (`items[Rooms]` holds the shown ones).
    room_rows: Vec<Item>,
    /// Join code or player name being typed (online sub-menu).
    typing: Option<Typing>,
    /// The room browser's Warpzone session (while the room list is open).
    browser: Option<crate::online::browser::Browser>,
    /// The error / status line (`disconnect_errorLog`).
    error_line: Option<usize>,
    /// A message for the error line (e.g. why an online join failed).
    pub error: Option<String>,
    /// The device of the last menu input (the tutorial starts on it).
    last_device: DeviceId,
    settings_rows: Vec<SettingRow>,
    /// The display options when the settings opened (Back restores them;
    /// Save applies the new ones).
    display_at_open: crate::shared::Display,
    /// The resolution row's two numbers.
    res_texts: (Option<usize>, Option<usize>),
    /// Seconds since the current sub-menu opened (slide-in).
    since: f32,
    /// The list being left and the seconds since (it falls away).
    leaving: Option<(Sub, f32)>,
    /// Each list's `MainMenu` animation (index = `Sub`).
    anims: [MenuAnim; 6],
    time: f32,
    size: (u32, u32),
    hover_prev: Option<usize>,
}

/// A list's `MainMenu` animation (ui.md 3): items slide in from the left
/// along `AnimateInOptions` (x offset over time), `optionAnimateDelay`
/// apart from the top; a list being left falls away along
/// `AnimateOutOptions` (y offset), the bottom item first.
#[derive(Clone, Debug)]
struct MenuAnim {
    /// Unity curve keys `[time, value, in slope, out slope]`.
    slide_in: Vec<[f32; 4]>,
    slide_out: Vec<[f32; 4]>,
    delay: f32,
}

impl MenuAnim {
    /// Fallback without the scene: the main list's authored values.
    fn fallback() -> Self {
        Self {
            slide_in: vec![
                [0.0009, -3500.0, 183.7, 183.7],
                [0.2015, -3506.8, 8915.0, 8915.0],
                [0.6499, 0.0, -1003.1, -1003.1],
            ],
            slide_out: vec![
                [0.0, 0.0, -1.2, -1.2],
                [0.3277, -1893.5, -5039.1, -5039.1],
                [1.0006, -3500.0, -227.8, -227.8],
            ],
            delay: 0.07,
        }
    }

    fn from_component(c: &serde_json::Value) -> Self {
        let mut a = Self::fallback();
        let keys = crate::fx::curve_keys(&c["AnimateInOptions"]);
        if !keys.is_empty() {
            a.slide_in = keys;
        }
        let keys = crate::fx::curve_keys(&c["AnimateOutOptions"]);
        if !keys.is_empty() {
            a.slide_out = keys;
        }
        if let Some(d) = c["optionAnimateDelay"].as_f64() {
            a.delay = d as f32;
        }
        a
    }

    /// Item `i`'s x offset `t` s after the list was enabled.
    fn x_in(&self, i: usize, t: f32) -> f32 {
        crate::level_view::eval_curve(&self.slide_in, t - self.delay * i as f32)
    }

    /// Item `i` of `n`'s y offset `t` s after the list was disabled.
    fn y_out(&self, i: usize, n: usize, t: f32) -> f32 {
        let k = n.saturating_sub(1 + i) as f32;
        crate::level_view::eval_curve(&self.slide_out, t - self.delay * k)
    }

    /// Every item of `n` has fallen away.
    fn out_done(&self, n: usize, t: f32) -> bool {
        let end = self.slide_out.last().map_or(0.0, |k| k[0]);
        t > end + self.delay * n.saturating_sub(1) as f32
    }
}

fn color_of(j: &serde_json::Value) -> Color {
    let f = |k: &str| j[k].as_f64().unwrap_or(1.0) as f32;
    Color::rgba(f("r"), f("g"), f("b"), f("a"))
}

impl MainMenu {
    pub fn new(shared: &mut Shared) -> Self {
        let data = shared.data.clone();
        let scene = data.scene(0).ok();
        let objects = scene.as_ref().map(|s| s.objects.as_slice()).unwrap_or(&[]);
        let root = objects
            .iter()
            .position(|o| o.name == "Canvas (1)")
            .unwrap_or(0);
        let mut canvas = if objects.is_empty() {
            Canvas::from_prefab(&data, "", Vec2::new(4096.0, 2160.0)).unwrap_or_else(|| Canvas {
                nodes: Vec::new(),
                reference: Vec2::new(4096.0, 2160.0),
                match_mode: 1,
                match_wh: 0.0,
                constant: false,
                world_root: None,
            })
        } else {
            Canvas::from_objects(&data, objects, root)
        };
        let names = [
            "mainMenu_leaveACTIVE",
            "SettingsMenu_leaveACTIVE",
            "LanguageMenu_leaveACTIVE",
            "Credits_leaveACTIVE",
        ];
        let found = names.map(|n| canvas.find(n));
        // Each list's animation from its `MainMenu` script (the online
        // list and the room browser are copies of the main list).
        let anim_of = |name: &str| {
            objects
                .iter()
                .find(|o| o.name == name)
                .and_then(|o| o.component("MainMenu"))
                .map_or_else(MenuAnim::fallback, |c| MenuAnim::from_component(&c.fields))
        };
        let anims = [
            anim_of(names[0]),
            anim_of(names[1]),
            anim_of(names[2]),
            anim_of(names[3]),
            anim_of(names[0]),
            anim_of(names[0]),
        ];
        if let Some(sm) = found[1] {
            add_camera_zoom_row(&mut canvas, sm);
        }
        // The online sub-menu and the room browser: copies of the main
        // list with our rows.
        let online = found[0].map(|m| clone_list(&mut canvas, m, &ONLINE_ITEMS, "online"));
        let rooms = found[0].map(|m| clone_list(&mut canvas, m, &ROOM_ITEMS, "rooms"));
        let menus = [found[0], found[1], found[2], found[3], online, rooms];
        // Items: the direct children with a selection border.
        let mut items: [Vec<Item>; 6] = Default::default();
        let mut settings_rows = Vec::new();
        for (k, m) in menus.iter().enumerate() {
            let Some(m) = *m else { continue };
            for &c in &canvas.nodes[m].children.clone() {
                let name = canvas.nodes[c].name.clone();
                if name.ends_with(":unused") {
                    continue;
                }
                // The border: first child image with a SelectionBorder in
                // the original (the `Border…`/`BORDER` child).
                let border = canvas.nodes[c]
                    .children
                    .iter()
                    .copied()
                    .find(|&b| canvas.nodes[b].name.to_lowercase().starts_with("border"));
                let Some(b) = border else { continue };
                // Selection colours from the scene's SelectionBorder.
                let obj = objects.iter().find(|o| {
                    o.name == canvas.nodes[b].name
                        && o.parent.is_some_and(|p| objects[p].name == name)
                });
                let sb = obj.and_then(|o| o.component("SelectionBorder"));
                let get = |key: &str, d: Color| sb.map_or(d, |c| color_of(&c.fields[key]));
                let texts: Vec<usize> = descendants(&canvas, c)
                    .into_iter()
                    .filter(|&t| canvas.nodes[t].text.is_some())
                    .collect();
                let disabled = name == "invite friend";
                items[k].push(Item {
                    node: c,
                    border: Some(b),
                    texts,
                    selected_border: get("SelectedBorderColor", Color::rgb(1.0, 0.6, 0.14)),
                    unselected_border: get("UnSelectedBorderColor", Color::rgb(0.125, 0.6, 0.933)),
                    selected_text: get("SelectedTextColor", Color::WHITE),
                    unselected_text: get("UnselectedTextColor", Color::rgb(0.078, 0.369, 0.573)),
                    disabled,
                });
                if k == 1 {
                    let values: Vec<usize> = canvas.nodes[c]
                        .children
                        .iter()
                        .copied()
                        .filter(|&v| {
                            let n = &canvas.nodes[v];
                            n.text.is_some()
                                && n.name != "text"
                                && n.name != ":"
                                && !n.name.starts_with("text")
                        })
                        .collect();
                    settings_rows.push(SettingRow {
                        item: items[k].len() - 1,
                        values,
                    });
                }
            }
        }
        // Hide the Steam / debug bits bopl-port has no use for; the error
        // line shows our messages.
        let discord = canvas.find("discord-link");
        canvas.set_active(discord, false);
        let error_line = canvas.find("disconnect_errorLog");
        canvas.set_active(error_line, false);
        let room_rows = items[Sub::Rooms as usize].clone();
        let mut m = Self {
            l10n: L10n::new(&data, shared.settings.language),
            canvas,
            sub: Sub::Main,
            selected: 0,
            menus,
            items,
            settings_rows,
            display_at_open: shared.settings.display(),
            res_texts: (None, None),
            since: 0.0,
            leaving: None,
            anims,
            time: 0.0,
            size: (1920, 1080),
            hover_prev: None,
            room_rows,
            typing: None,
            browser: None,
            error_line,
            error: None,
            last_device: DeviceId::KeyboardMouse,
        };
        if let Some(r) = m
            .settings_rows
            .iter()
            .find(|r| m.canvas.nodes[m.items[1][r.item].node].name == "Resolution")
        {
            let node = m.items[1][r.item].node;
            m.res_texts = (m.canvas.child(node, "1280"), m.canvas.child(node, "720"));
        }
        m.show(Sub::Main);
        m.apply_settings(shared);
        m
    }

    fn show(&mut self, sub: Sub) {
        // The list being left falls away (its items stay drawn until
        // gone); the first list shown just slides in.
        if self.menus[self.sub as usize].is_some() && self.sub != sub && self.time > 0.0 {
            self.leaving = Some((self.sub, 0.0));
        } else {
            self.leaving = None;
        }
        self.sub = sub;
        self.selected = 0;
        self.since = 0.0;
        self.update_visible();
    }

    /// The current list and the one falling away are active.
    fn update_visible(&mut self) {
        let leaving = self.leaving.map(|l| l.0);
        for (k, m) in self.menus.iter().enumerate() {
            let on = k == self.sub as usize || leaving.is_some_and(|l| l as usize == k);
            self.canvas.set_active(*m, on);
        }
    }

    fn items(&self) -> &Vec<Item> {
        &self.items[self.sub as usize]
    }

    /// Shows the current setting values.
    fn apply_settings(&mut self, shared: &Shared) {
        let s = &shared.settings;
        let rows = self.settings_rows.clone();
        for row in &rows {
            let item = &self.items[1][row.item];
            let name = self.canvas.nodes[item.node].name.clone();
            let shown: Option<usize> = match name.as_str() {
                "sfxVolume" => Some(s.sfx_volume as usize),
                "MusicVolume" => Some(s.music_volume as usize),
                "Abilities" => Some(s.number_of_abilities.saturating_sub(1)),
                "ScreenShake" => Some(if s.screen_shake { 0 } else { 1 }),
                "CameraZoom" => Some(if s.camera_zoom { 0 } else { 1 }),
                // Value children in authored order: off, on, high.
                "Rumble" => Some(s.rumble as usize),
                // off, on.
                "Vsync" => Some(s.vsync as usize),
                // windowed, borderless, fullscreen.
                "Fullscreen" => Some(s.screen as usize),
                // names and avatars, names, nothing.
                "Hide" => Some(2 - s.hide as usize),
                _ => None,
            };
            if let Some(v) = shown {
                for (k, &n) in row.values.iter().enumerate() {
                    self.canvas.nodes[n].active = k == v;
                }
            }
        }
        // Resolution: the chosen one, else the desktop's (windowed: the
        // window's default).
        let (w, h) = s
            .resolution
            .or(shared.platform.desktop)
            .unwrap_or((1280, 720));
        let (tw, th) = self.res_texts;
        self.canvas.set_text(tw, &w.to_string());
        self.canvas.set_text(th, &h.to_string());
    }

    fn change_setting(&mut self, shared: &mut Shared, audio: &mut Audio, dir: i32) {
        let Some(item) = self.items[1].get(self.selected) else {
            return;
        };
        let name = self.canvas.nodes[item.node].name.clone();
        let s = &mut shared.settings;
        let wrap = |v: i32, n: i32| (v + dir).rem_euclid(n);
        match name.as_str() {
            "sfxVolume" => {
                s.sfx_volume = wrap(s.sfx_volume as i32, 11) as u32;
                shared.sounds.sfx_volume = s.sfx_volume as f32;
                shared.sounds.apply_volumes(audio);
                shared.sounds.play(audio, "arrowShoot");
            }
            "MusicVolume" => {
                s.music_volume = wrap(s.music_volume as i32, 11) as u32;
                let v = s.music_volume as f32;
                shared.sounds.set_music_volume(audio, v);
            }
            "Abilities" => {
                s.number_of_abilities = wrap(s.number_of_abilities as i32 - 1, 3) as usize + 1
            }
            "ScreenShake" => s.screen_shake = !s.screen_shake,
            "CameraZoom" => s.camera_zoom = !s.camera_zoom,
            "Rumble" => {
                use crate::shared::Rumble;
                s.rumble =
                    [Rumble::Off, Rumble::On, Rumble::High][wrap(s.rumble as i32, 3) as usize]
            }
            "Vsync" => s.vsync = !s.vsync,
            "Fullscreen" => {
                s.screen = crate::shared::ScreenMode::ALL[wrap(s.screen as i32, 3) as usize]
            }
            "Hide" => {
                use crate::shared::Hide;
                s.hide = [Hide::Nothing, Hide::Names, Hide::NamesAndAvatars]
                    [wrap(s.hide as i32, 3) as usize]
            }
            "Resolution" => {
                let list = &shared.platform.resolutions;
                if list.is_empty() {
                    shared.sounds.play(audio, "smallError");
                    return;
                }
                let cur = s.resolution.or(shared.platform.desktop);
                // The list is largest first: "right" goes to larger.
                let k = cur
                    .and_then(|c| list.iter().position(|r| *r == c))
                    .map_or(0, |k| {
                        (k as i32 - dir).rem_euclid(list.len() as i32) as usize
                    });
                s.resolution = Some(list[k]);
            }
            _ => {
                shared.sounds.play(audio, "smallError");
                return;
            }
        }
        shared.sounds.play(audio, "navigateMenu");
        self.apply_settings(shared);
    }

    fn activate(&mut self, shared: &mut Shared, audio: &mut Audio) -> MenuOutcome {
        let Some(item) = self.items().get(self.selected).cloned() else {
            return MenuOutcome::Stay;
        };
        if item.disabled {
            shared.sounds.play(audio, "smallError");
            return MenuOutcome::Stay;
        }
        let name = self.canvas.nodes[item.node].name.clone();
        match (self.sub, name.as_str()) {
            (Sub::Main, "Play") => {
                shared.sounds.play(audio, "menuWhoosh1");
                shared.sounds.play(audio, "startGame");
                return MenuOutcome::Play;
            }
            (Sub::Main, "Tutorial") => {
                shared.sounds.play(audio, "menuWhoosh1");
                shared.sounds.play(audio, "startGame");
                return MenuOutcome::Tutorial(self.last_device);
            }
            (Sub::Main, "Online") => {
                shared.sounds.play(audio, "return3");
                self.show(Sub::Online);
            }
            (Sub::Online, "online:host game") => {
                shared.sounds.play(audio, "menuWhoosh1");
                shared.sounds.play(audio, "startGame");
                return MenuOutcome::Online(crate::online::scene::OnlineIntent::Host {
                    private: false,
                });
            }
            (Sub::Online, "online:host private") => {
                shared.sounds.play(audio, "menuWhoosh1");
                shared.sounds.play(audio, "startGame");
                return MenuOutcome::Online(crate::online::scene::OnlineIntent::Host {
                    private: true,
                });
            }
            (Sub::Online, "online:join code") => {
                shared.sounds.play(audio, "accept");
                self.typing = Some(Typing::Code(String::new()));
                self.show_online_texts(shared);
            }
            (Sub::Online, "online:name") => {
                shared.sounds.play(audio, "accept");
                self.typing = Some(Typing::Name(shared.settings.player_name.clone()));
                self.show_online_texts(shared);
            }
            (Sub::Online, "online:room list") => {
                shared.sounds.play(audio, "return3");
                self.browser = Some(crate::online::browser::Browser::new(&player_name(shared)));
                self.show(Sub::Rooms);
                self.update_rooms();
            }
            (Sub::Online, _) => {
                shared.sounds.play(audio, "return");
                self.show(Sub::Main);
            }
            (Sub::Rooms, "rooms:refresh") => {
                shared.sounds.play(audio, "navigateMenu");
                if let Some(b) = self.browser.as_mut() {
                    b.refresh();
                }
            }
            (Sub::Rooms, n) if n.starts_with("rooms:") && n != "rooms:back" => {
                let k: usize = n[6..].parse().unwrap_or(0);
                let room = self
                    .browser
                    .as_ref()
                    .and_then(|b| b.page_rooms().get(k).cloned());
                match room {
                    Some(r) if r.other_version => {
                        shared.sounds.play(audio, "smallError");
                        self.error = Some(
                            "that room runs another bopl-port version: everybody needs the same version"
                                .into(),
                        );
                    }
                    Some(r) => {
                        shared.sounds.play(audio, "startGame");
                        self.browser = None;
                        return MenuOutcome::Online(crate::online::scene::OnlineIntent::Join(r.id));
                    }
                    None => {}
                }
            }
            (Sub::Rooms, _) => {
                shared.sounds.play(audio, "return");
                self.browser = None;
                self.error = None;
                self.show(Sub::Online);
            }
            (Sub::Main, "Settings") => {
                shared.sounds.play(audio, "return3");
                self.display_at_open = shared.settings.display();
                self.apply_settings(shared);
                self.show(Sub::Settings);
            }
            (Sub::Main, "Credits") => {
                shared.sounds.play(audio, "return3");
                self.show(Sub::Credits);
            }
            (Sub::Main, "LanguageSelector") => {
                shared.sounds.play(audio, "return3");
                self.show(Sub::Language);
            }
            (Sub::Main, "Exit") => return MenuOutcome::Quit,
            (Sub::Settings, "Save") => {
                shared.sounds.play(audio, "return3");
                if !shared.headless {
                    shared.settings.save();
                }
                if shared.settings.display() != self.display_at_open {
                    shared.display_changed = true;
                }
                self.show(Sub::Main);
            }
            (Sub::Settings, _) => self.change_setting(shared, audio, 1),
            (Sub::Language, code) => {
                if let Some(i) = crate::ui::LANGUAGES.iter().position(|l| l.0 == code) {
                    shared.settings.language = i;
                    self.l10n = L10n::new(&shared.data, i);
                    if !shared.headless {
                        shared.settings.save();
                    }
                }
                shared.sounds.play(audio, "accept");
                self.show(Sub::Main);
            }
            (Sub::Credits, _) => {
                shared.sounds.play(audio, "return");
                self.show(Sub::Main);
            }
            _ => {}
        }
        MenuOutcome::Stay
    }

    /// The "join code" and "name" rows: the text being typed, or the
    /// current name.
    fn show_online_texts(&mut self, shared: &Shared) {
        let items = &self.items[Sub::Online as usize];
        let (Some(code_item), Some(name_item)) =
            (items.get(ROW_CODE).cloned(), items.get(ROW_NAME).cloned())
        else {
            return;
        };
        let code = match &self.typing {
            Some(Typing::Code(c)) => {
                format!("code: {c}{}", "_".repeat(6usize.saturating_sub(c.len())))
            }
            _ => ONLINE_ITEMS[ROW_CODE].to_string(),
        };
        let name = match &self.typing {
            Some(Typing::Name(n)) => format!("name: {n}_"),
            _ if shared.settings.player_name.is_empty() => "name: (warpzone)".to_string(),
            _ => format!("name: {}", shared.settings.player_name),
        };
        for t in code_item.texts {
            self.canvas.set_text(Some(t), &code);
        }
        for t in name_item.texts {
            self.canvas.set_text(Some(t), &name);
        }
    }

    /// The room browser rows from the current page.
    fn update_rooms(&mut self) {
        let rooms: Vec<crate::online::browser::RoomRow> = self
            .browser
            .as_ref()
            .map(|b| b.page_rooms().to_vec())
            .unwrap_or_default();
        let mut shown = Vec::new();
        for (k, it) in self.room_rows.clone().into_iter().enumerate() {
            let visible = if k < crate::online::browser::PAGE {
                match rooms.get(k) {
                    Some(r) => {
                        for &t in &it.texts {
                            self.canvas.set_text(Some(t), &r.label);
                        }
                        true
                    }
                    None => false,
                }
            } else {
                true
            };
            self.canvas.nodes[it.node].active = visible;
            if visible {
                shown.push(it);
            }
        }
        let n = shown.len();
        self.items[Sub::Rooms as usize] = shown;
        self.selected = self.selected.min(n.saturating_sub(1));
    }

    /// Typing a join code (letters, digits) or a name (letters, digits,
    /// spaces) from the keyboard: Enter finishes, Backspace deletes, Esc
    /// cancels.
    fn type_text(
        &mut self,
        shared: &mut Shared,
        raw: &[crate::input::RawPress],
        audio: &mut Audio,
    ) -> Option<MenuOutcome> {
        let typing = self.typing.as_mut()?;
        let (text, max, min, spaces) = match typing {
            Typing::Code(c) => (c, 10, 4, false),
            Typing::Name(n) => (n, 12, 0, true),
        };
        let mut done = None;
        for r in raw.iter().filter(|r| r.device == DeviceId::KeyboardMouse) {
            let name = r.name.strip_prefix("Keypad ").unwrap_or(&r.name);
            match name {
                "Return" | "Enter" if text.len() >= min => done = Some(true),
                "Escape" => done = Some(false),
                "Backspace" => {
                    text.pop();
                }
                "Space" if spaces && text.len() < max && !text.is_empty() => text.push(' '),
                n if n.len() == 1
                    && n.chars().all(|c| c.is_ascii_alphanumeric())
                    && text.len() < max =>
                {
                    if spaces {
                        text.push_str(&n.to_ascii_lowercase());
                    } else {
                        text.push_str(&n.to_ascii_uppercase());
                    }
                    shared.sounds.play(audio, "navigateMenu");
                }
                _ => {}
            }
        }
        let mut out = None;
        match done {
            Some(true) => match self.typing.take() {
                Some(Typing::Code(c)) => {
                    shared.sounds.play(audio, "startGame");
                    out = Some(MenuOutcome::Online(
                        crate::online::scene::OnlineIntent::Join(c),
                    ));
                }
                Some(Typing::Name(n)) => {
                    shared.sounds.play(audio, "accept");
                    shared.settings.player_name = n.trim().to_string();
                    if !shared.headless {
                        shared.settings.save();
                    }
                }
                None => {}
            },
            Some(false) => {
                self.typing = None;
                shared.sounds.play(audio, "decline");
            }
            None => {}
        }
        self.show_online_texts(shared);
        if out.is_some() {
            return out;
        }
        // The key that ended typing is no menu input this tick.
        (self.typing.is_some() || done.is_some()).then_some(MenuOutcome::Stay)
    }

    pub fn tick(
        &mut self,
        shared: &mut Shared,
        devices: &[(DeviceId, InputState)],
        raw: &[crate::input::RawPress],
        audio: &mut Audio,
    ) -> MenuOutcome {
        self.since += 1.0 / 60.0;
        self.time += 1.0 / 60.0;
        if let Some((l, t)) = self.leaving.as_mut() {
            *t += 1.0 / 60.0;
            let n = self.items[*l as usize].len();
            if self.anims[*l as usize].out_done(n, *t) {
                let l = *l;
                for it in self.items[l as usize].clone() {
                    self.canvas.nodes[it.node].offset = Vec2::ZERO;
                }
                self.leaving = None;
                self.update_visible();
            }
        }
        shared.menu_time += 1.0 / 60.0;
        if let Some(out) = self.type_text(shared, raw, audio) {
            return out;
        }
        if self.sub == Sub::Online && self.since < 0.05 {
            self.show_online_texts(shared);
        }
        // The room browser.
        if let Some(b) = self.browser.as_mut() {
            let before = (b.rooms.clone(), b.page);
            b.tick(1.0 / 60.0);
            if (b.rooms.clone(), b.page) != before {
                self.update_rooms();
            }
            if self.sub == Sub::Rooms {
                for (_, st) in devices {
                    let p = st.pressed;
                    let dir = if p.contains(Buttons::LEFT) {
                        -1
                    } else if p.contains(Buttons::RIGHT) {
                        1
                    } else {
                        0
                    };
                    if dir != 0
                        && let Some(b) = self.browser.as_mut()
                        && b.pages() > 1
                    {
                        b.turn_page(dir);
                        shared.sounds.play(audio, "navigateMenu");
                        self.update_rooms();
                    }
                }
            }
        }
        let n = self.items().len().max(1);
        let screen = Vec2::new(self.size.0 as f32, self.size.1 as f32);
        let placed = self.canvas.layout(screen);
        for (dev, st) in devices {
            let p = st.pressed;
            if p.0 != 0 {
                self.last_device = *dev;
            }
            // Mouse: hover selects, click activates.
            if *dev == DeviceId::KeyboardMouse
                && let Some(ptr) = st.pointer
            {
                let hovered = self.items().iter().position(|it| {
                    self.canvas.hit(&placed, it.node, ptr, screen)
                        || it
                            .border
                            .is_some_and(|b| self.canvas.hit(&placed, b, ptr, screen))
                });
                if let Some(h) = hovered
                    && Some(h) != self.hover_prev
                {
                    if h != self.selected {
                        shared.sounds.play(audio, "navigateMenu");
                    }
                    self.selected = h;
                }
                self.hover_prev = hovered;
                if let Some(h) = hovered
                    && p.contains(Buttons::ABILITY0)
                {
                    // Settings rows: the arrow buttons step down / up.
                    if self.sub == Sub::Settings {
                        let node = self.items()[h].node;
                        let left = self.canvas.child(node, "BR_buttonLeft");
                        let right = self.canvas.child(node, "BR_buttonRight");
                        if left.is_some_and(|b| self.canvas.hit(&placed, b, ptr, screen)) {
                            self.change_setting(shared, audio, -1);
                            return MenuOutcome::Stay;
                        }
                        if right.is_some_and(|b| self.canvas.hit(&placed, b, ptr, screen)) {
                            self.change_setting(shared, audio, 1);
                            return MenuOutcome::Stay;
                        }
                    }
                    return self.activate(shared, audio);
                }
            }
            if p.contains(Buttons::DOWN) {
                self.selected = (self.selected + 1) % n;
                shared.sounds.play(audio, "navigateMenu");
            } else if p.contains(Buttons::UP) {
                self.selected = (self.selected + n - 1) % n;
                shared.sounds.play(audio, "navigateMenu");
            } else if self.sub == Sub::Settings
                && (p.contains(Buttons::LEFT) || p.contains(Buttons::RIGHT))
            {
                let d = if p.contains(Buttons::LEFT) { -1 } else { 1 };
                self.change_setting(shared, audio, d);
            } else if p.contains(Buttons::ACCEPT) {
                return self.activate(shared, audio);
            } else if p.contains(Buttons::BACK) && self.sub != Sub::Main && self.since > 1.0 {
                shared.sounds.play(audio, "return");
                if self.sub == Sub::Settings {
                    // Display options only change with Save.
                    let d = self.display_at_open;
                    let s = &mut shared.settings;
                    (s.screen, s.resolution, s.vsync) = (d.mode, d.resolution, d.vsync);
                    self.apply_settings(shared);
                }
                if self.sub == Sub::Rooms {
                    self.browser = None;
                    self.error = None;
                    self.show(Sub::Online);
                } else {
                    self.show(Sub::Main);
                }
            }
        }
        MenuOutcome::Stay
    }

    pub fn draw(&mut self, shared: &mut Shared, ctx: &mut DrawContext<'_>) {
        self.size = ctx.size;
        // Selection colours and the slide-in of the items.
        let sel = self.selected;
        let since = self.since;
        let items = self.items().clone();
        for (k, it) in items.iter().enumerate() {
            let on = k == sel;
            let dim = if it.disabled { 0.55 } else { 1.0 };
            if let Some(b) = it.border {
                let mut c = if on {
                    it.selected_border
                } else {
                    it.unselected_border
                };
                c.a *= dim;
                self.canvas.set_color(Some(b), c);
            }
            for &t in &it.texts {
                let mut c = if on {
                    it.selected_text
                } else {
                    it.unselected_text
                };
                c.a *= dim;
                self.canvas.set_color(Some(t), c);
            }
            // Items slide in one after another (MenuAnim).
            let x = self.anims[self.sub as usize].x_in(k, since);
            self.canvas.nodes[it.node].offset = Vec2::new(x, 0.0);
        }
        if let Some((l, t)) = self.leaving
            && l != self.sub
        {
            let items = &self.items[l as usize];
            let n = items.len();
            for (k, it) in items.iter().enumerate() {
                let y = self.anims[l as usize].y_out(k, n, t);
                self.canvas.nodes[it.node].offset = Vec2::new(0.0, y);
            }
        }
        // The error / status line: our message, or the room browser's.
        let line = self.error.clone().or_else(|| {
            (self.sub == Sub::Rooms)
                .then(|| self.browser.as_ref().map(|b| b.status.clone()))
                .flatten()
        });
        self.canvas.set_active(self.error_line, line.is_some());
        if let Some(l) = &line {
            self.canvas.set_text(self.error_line, l);
        }
        let time = shared.menu_time;
        let mut pass = ui_pass("menu", ctx.size);
        let mut d = UiDraw {
            gpu: ctx.gpu,
            renderer: ctx.renderer,
            assets: &mut shared.assets,
            fonts: &mut shared.fonts,
            l10n: &self.l10n,
            time,
        };
        self.canvas.draw(&mut d, ctx.size, &mut pass);
        pass.clear = Some(Color::BLACK);
        ctx.frame.passes.push(pass);
    }
}

/// The name shown online: the chosen one, else the OS user's (Warpzone
/// registers it for a new identity; an existing account keeps its name).
pub fn player_name(shared: &Shared) -> String {
    if !shared.settings.player_name.is_empty() {
        return shared.settings.player_name.clone();
    }
    crate::online::scene::default_name()
}

/// A copy of the main list (`main`) with our own rows (`items`, named
/// `<prefix>:<item>`); extra original rows are hidden (`<prefix>:unused`).
fn clone_list(canvas: &mut Canvas, main: usize, items: &[&str], prefix: &str) -> usize {
    let parent = canvas.nodes[main].parent.unwrap_or(0);
    let id = canvas.clone_subtree(main, parent);
    let kids = canvas.nodes[id].children.clone();
    let mut kept = 0;
    for c in kids {
        let has_border = canvas.nodes[c]
            .children
            .iter()
            .any(|&b| canvas.nodes[b].name.to_lowercase().starts_with("border"));
        if has_border && kept < items.len() {
            let item = items[kept];
            let item = item.strip_prefix(&format!("{prefix}:")).unwrap_or(item);
            canvas.nodes[c].name = format!("{prefix}:{item}");
            for t in descendants(canvas, c) {
                if canvas.nodes[t].text.is_some() {
                    canvas.set_text(Some(t), item);
                }
            }
            // Hide icons (globe, ...) that belong to the original rows; the
            // text then sits in the middle of the row.
            let mut had_icon = false;
            for t in descendants(canvas, c) {
                let n = canvas.nodes[t].name.to_lowercase();
                if canvas.nodes[t].text.is_none() && !n.starts_with("border") {
                    canvas.nodes[t].active = false;
                    had_icon = true;
                }
            }
            if had_icon {
                for t in descendants(canvas, c) {
                    if canvas.nodes[t].text.is_some() {
                        let node = &mut canvas.nodes[t];
                        node.anchor_min.x = 0.5;
                        node.anchor_max.x = 0.5;
                        node.pos.x = 0.0;
                        if let Some(tx) = node.text.as_mut() {
                            tx.h_align = 0.5;
                        }
                    }
                }
            }
            kept += 1;
        } else if has_border {
            canvas.nodes[c].active = false;
            canvas.nodes[c].name = format!("{prefix}:unused");
        }
    }
    id
}

/// Ours: a "camera zoom" on/off row (a copy of the screen-shake row)
/// before Save; the rows are spaced evenly over the authored span.
fn add_camera_zoom_row(canvas: &mut Canvas, menu: usize) {
    let Some(shake) = canvas.child(menu, "ScreenShake") else {
        return;
    };
    let row = canvas.clone_subtree(shake, menu);
    canvas.nodes[row].name = "CameraZoom".into();
    let label = canvas.child(row, "text");
    canvas.set_text(label, "camera zoom");
    // Before Save.
    let kids = &mut canvas.nodes[menu].children;
    kids.retain(|&c| c != row);
    let save = kids.len().saturating_sub(1);
    kids.insert(save, row);
    let kids = kids.clone();
    let (Some(&first), Some(&last)) = (kids.first(), kids.last()) else {
        return;
    };
    let (top, bottom) = (canvas.nodes[first].pos.y, canvas.nodes[last].pos.y);
    let n = kids.len().max(2) - 1;
    for (k, &c) in kids.iter().enumerate() {
        canvas.nodes[c].pos.y = top + (bottom - top) * k as f32 / n as f32;
        // A little smaller so that 11 rows keep a gap.
        canvas.nodes[c].scale *= 0.94;
    }
}

/// Every descendant of `n` (depth first).
pub fn descendants(c: &Canvas, n: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack: Vec<usize> = c.nodes[n].children.iter().rev().copied().collect();
    while let Some(i) = stack.pop() {
        out.push(i);
        stack.extend(c.nodes[i].children.iter().rev());
    }
    out
}

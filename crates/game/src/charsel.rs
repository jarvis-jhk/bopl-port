//! Character select (match-flow.md 4, ui.md 4) on the original's
//! `CharacterSelect` canvas: join by device (keyboard + mouse, keyboard
//! halves, controllers), team, three abilities (row cycling or the ability
//! grid, all unlocked), colour, ready, start. Up to 12 boxes: the original 4
//! in a row, more in two rows (match-flow.md 17.2).

use bopl_render::Color;
use bopl_sim::World;
use bopl_sim::player::PlayerSetup;
use glam::Vec2;

use crate::app::DrawContext;
use crate::audio::Audio;
use crate::input::{Buttons, DeviceId, InputState, RawPress};
use crate::match_scene::Source;
use crate::match_view::{MatchVisuals, eval_keys};
use crate::remap::{self, Rebind, RebindEvent};
use crate::shared::Shared;
use crate::ui::{Canvas, L10n, UiDraw, UiShade, shade_of, ui_pass};

/// A player chosen in character select.
#[derive(Clone, Debug, PartialEq)]
pub struct LobbyPlayer {
    pub device: DeviceId,
    pub color: i32,
    pub team: i32,
    /// Ability-list indices per slot (slot 0, 1, 2).
    pub abilities: [usize; 3],
}

/// The players kept between character select and matches (box order).
#[derive(Clone, Debug, Default)]
pub struct Lobby {
    pub players: Vec<LobbyPlayer>,
}

impl Lobby {
    /// Sim setups and input sources (ids 1..N in box order).
    pub fn setups(&self, n_abilities: usize) -> (Vec<PlayerSetup>, Vec<Source>) {
        let setups = self
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| PlayerSetup {
                id: i as i32 + 1,
                team: p.team,
                color: p.color,
                abilities: p
                    .abilities
                    .iter()
                    .copied()
                    .take(n_abilities.clamp(1, 3))
                    .collect(),
                uses_keyboard_and_mouse: p.device == DeviceId::KeyboardMouse,
            })
            .collect();
        let sources = self
            .players
            .iter()
            .map(|p| Source::Device(p.device))
            .collect();
        (setups, sources)
    }

    /// Takes the players' (mid-game edited) abilities back from a session.
    pub fn absorb(&mut self, w: &World) {
        for (p, wp) in self.players.iter_mut().zip(&w.players) {
            for (k, a) in wp.abilities.iter().enumerate().take(3) {
                p.abilities[k] = *a;
            }
        }
    }
}

/// A player of another machine in the online lobby.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteBox {
    pub name: String,
    pub color: i32,
    pub team: i32,
    pub abilities: [usize; 3],
    /// Its machine is ready.
    pub ready: bool,
}

/// A local box as the online lobby announces it.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalBox {
    pub device: DeviceId,
    pub color: i32,
    /// `None` until the player picked one (the host assigns it).
    pub team: Option<i32>,
    pub abilities: [usize; 3],
    pub ready: bool,
}

/// What the online lobby tells character select each tick.
#[derive(Clone, Debug, Default)]
pub struct OnlineView {
    /// Title (room code) and status line.
    pub title: String,
    pub status: String,
    /// The start button is shown (the host, everybody ready).
    pub can_start: bool,
    pub remote: Vec<RemoteBox>,
    /// Per local box (box order): colour and team the host resolved.
    pub resolved: Vec<Option<(i32, i32)>>,
    /// Ability slots (the host's setting).
    pub n_abilities: usize,
}

pub enum CharSelectOutcome {
    Stay,
    Back,
    Start(Lobby),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Join,
    Select,
    Ready,
    Grid,
}

/// Rows of the select face (match-flow.md 4.2).
const ROW_REBIND: usize = 0;
const ROW_TEAM: usize = 1;
const ROW_AB: [usize; 3] = [2, 3, 4];
const ROW_COLOR: usize = 5;
const ROW_READY: usize = 6;
const ROW_LABEL_KEYS: [&str; 7] = ["rebind keys", "team", "", "", "", "color", "ready!"];

/// Node ids of one box.
#[derive(Clone, Debug, Default)]
struct BoxUi {
    root: usize,
    join: Option<usize>,
    join_border: Option<usize>,
    select: Option<usize>,
    select_border: Option<usize>,
    label: Option<usize>,
    team: Option<usize>,
    team_color: Option<usize>,
    rows: [Option<usize>; 7],
    ab_icons: [Option<usize>; 3],
    ab_locks: [Option<usize>; 3],
    ab_dots: [Option<usize>; 3],
    idle_char: Option<usize>,
    wobble_char: Option<usize>,
    arrows: Option<usize>,
    arrow_l: Option<usize>,
    arrow_r: Option<usize>,
    rebind_icon: Option<usize>,
    /// `KeyBindButton.FakeArrows`: the arrow over the rebind button while
    /// its row is selected.
    fake_arrows: Option<usize>,
    ready_face: Option<usize>,
    ready_border: Option<usize>,
    ready_char: Option<usize>,
    /// Player name on the ready face (online boxes).
    ready_name: Option<usize>,
    grid: Option<usize>,
    grid_circle: Option<usize>,
    grid_text: Option<usize>,
    grid_demo: Option<usize>,
    grid_entries: Vec<usize>,
    /// Images tinted with the team fill / border / saturated colours, and
    /// texts tinted with the border colour.
    fill_imgs: Vec<usize>,
    border_imgs: Vec<usize>,
    texts: Vec<usize>,
    /// Authored y of the ability rows.
    row_y: [f32; 3],
    /// Key rebinding cover (`KeyBindButton.RebindKeysCover`) and its parts.
    rebind: RebindUi,
}

/// Nodes of a box's key rebinding cover (ui.md 6).
#[derive(Clone, Debug, Default)]
struct RebindUi {
    cover: Option<usize>,
    /// The step prompt (`RebindKeyText`).
    text: Option<usize>,
    /// The circle indicator of the step (`rebindImage`).
    circles: Option<usize>,
    /// Upper list (jump, a1..a3) and the lower one (directions, keyboards).
    top: Option<usize>,
    bottom: Option<usize>,
    /// Chosen controls per step (`bindingListChoices`).
    choices: [Option<usize>; 8],
    mouse_only: Option<usize>,
    /// Authored local y of the upper list (`bindListParentTopOriginalPos`).
    top_y: f32,
}

#[derive(Clone, Debug)]
struct BoxState {
    device: Option<DeviceId>,
    face: Face,
    row: usize,
    team: i32,
    color: i32,
    abilities: [usize; 3],
    /// Grid selection: ability index - 1.
    grid_sel: usize,
    /// Seconds since the colour changed (wobble), since the face changed.
    color_since: f32,
    face_since: f32,
    /// Online: a player of another machine (shown, never driven here).
    remote: bool,
    /// Online: the player picked a team (else the host assigns one).
    team_chosen: bool,
    /// Online: name on the ready face.
    name: Option<String>,
    /// Key rebinding in progress (ui.md 6).
    rebind: Option<Rebind>,
    /// Seconds since each binding entry refused a clash (wiggle), since
    /// the last step was bound (button pulse).
    rebind_wiggle: [f32; 8],
    rebind_pulse: f32,
    /// The rebinding ended this tick: the box ignores its device until the
    /// next one (the press that ended it is no menu input).
    rebind_ended: bool,
}

impl BoxState {
    fn new(i: usize) -> Self {
        const COLORS: [i32; 12] = [0, 6, 2, 3, 4, 5, 1, 7, 8, 9, 10, 11];
        Self {
            device: None,
            face: Face::Join,
            row: ROW_TEAM,
            team: (i % 12) as i32,
            color: COLORS[i % 12],
            // Slot 0 Bow, slot 1 Gust, slot 2 Dash.
            abilities: [4, 7, 2],
            grid_sel: 3,
            color_since: 10.0,
            face_since: 10.0,
            remote: false,
            team_chosen: false,
            name: None,
            rebind: None,
            rebind_wiggle: [10.0; 8],
            rebind_pulse: 10.0,
            rebind_ended: false,
        }
    }
}

/// The binding scheme of a box's device (for logs).
fn r_scheme(s: &BoxState) -> crate::input::Scheme {
    s.device
        .map_or(crate::input::Scheme::KeyboardMouse, remap::scheme_of)
}

/// The ability grid: 3 columns.
const GRID_COLS: usize = 3;
/// `arrow_Separation_YPos` x per row.
const ARROW_X: [f32; 7] = [218.0, 218.0, 202.0, 202.0, 202.0, 202.0, 202.0];
/// A refused binding entry wiggles (degrees over seconds; the entries'
/// `AnimateTransformUI` rotation curve, approximated).
const WIGGLE: [(f32, f32); 4] = [(0.0, 0.0), (0.129, -8.0), (0.259, 7.4), (0.4, 0.0)];
/// The rebind button pulses on every bound step (scale over seconds).
const PULSE: [(f32, f32); 3] = [(0.0, 1.0), (0.08, 1.15), (0.25, 1.0)];
/// Box slide-in when a face changes (AnimateInOutUI `animate01`).
const FACE_IN: [(f32, f32); 2] = [(0.0, 0.0), (0.3, 1.0)];

pub struct CharSelect {
    canvas: Canvas,
    l10n: L10n,
    visuals: std::rc::Rc<MatchVisuals>,
    data: std::rc::Rc<bopl_import::GameData>,
    template: BoxUi,
    boxes: Vec<BoxUi>,
    states: Vec<BoxState>,
    rectangles: Option<usize>,
    start: Option<usize>,
    start_since: Option<f32>,
    start_frames: u32,
    time: f32,
    size: (u32, u32),
    ability_count: usize,
    n_abilities: usize,
    /// Leaving to the main menu (seconds since).
    leaving: Option<f32>,
    starting: Option<Lobby>,
    rng: u32,
    /// Online lobby mode (`crate::online::scene`).
    online: Option<OnlineView>,
    title: Option<usize>,
    status: Option<usize>,
    /// The mouse pointer of the last tick (hover selects only when it
    /// moves, so that it does not fight the keys).
    last_pointer: Option<Vec2>,
}

/// Slot of an ability row (row 3 is slot 2 and row 4 slot 1 with 3).
fn row_slot(row: usize, n_abilities: usize) -> usize {
    match (row, n_abilities) {
        (2, _) => 0,
        (3, 3) => 2,
        (3, _) => 1,
        _ => 1,
    }
}

fn find(c: &Canvas, root: usize, name: &str) -> Option<usize> {
    c.find_in(root, name)
}

impl CharSelect {
    pub fn new(shared: &mut Shared, lobby: &Lobby) -> Self {
        let data = shared.data.clone();
        let scene = data.scene(2).ok();
        let objects = scene.as_ref().map(|s| s.objects.as_slice()).unwrap_or(&[]);
        let root = objects.iter().position(|o| o.name == "Canvas").unwrap_or(0);
        let canvas = Canvas::from_objects(&data, objects, root);
        let rectangles = canvas.find("Rectangles");
        let mut boxes = Vec::new();
        for k in 1..=4 {
            if let Some(r) = canvas.find(&format!("Rectangle{k}")) {
                boxes.push(Self::box_ui(&canvas, r));
            }
        }
        let template = boxes.first().cloned().unwrap_or_default();
        let start = canvas.find("Start");
        let mut canvas = canvas;
        let title = canvas.find("localGame_text");
        // A status line under the title (online lobby).
        let status = title.map(|t| {
            let parent = canvas.nodes[t].parent.unwrap_or(0);
            let id = canvas.clone_subtree(t, parent);
            canvas.nodes[id].pos.y -= canvas.nodes[t].size.y * 0.9;
            canvas.nodes[id].scale_mul = 0.55;
            canvas.nodes[id].active = false;
            id
        });
        let mut cs = Self {
            l10n: L10n::new(&data, shared.settings.language),
            canvas,
            visuals: shared.visuals.clone(),
            data,
            template,
            boxes,
            states: Vec::new(),
            rectangles,
            start,
            start_since: None,
            start_frames: 0,
            time: 0.0,
            size: (1920, 1080),
            ability_count: shared.levels.content.abilities.len(),
            n_abilities: shared.settings.number_of_abilities.clamp(1, 3),
            leaving: None,
            starting: None,
            rng: 0x1234_5678,
            online: None,
            title,
            status,
            last_pointer: None,
        };
        cs.states = (0..cs.boxes.len()).map(BoxState::new).collect();
        // Coming back: player i into box i, select state, choices kept.
        for (i, p) in lobby.players.iter().enumerate() {
            cs.ensure_boxes(i + 1);
            let s = &mut cs.states[i];
            s.device = Some(p.device);
            s.face = Face::Select;
            s.face_since = 0.0;
            s.row = ROW_TEAM;
            s.team = p.team;
            s.color = p.color;
            s.abilities = p.abilities;
        }
        cs.build_grids();
        cs
    }

    fn box_ui(c: &Canvas, r: usize) -> BoxUi {
        let rows_names = [
            "REBIND_KEYS",
            "TEAM",
            "ability_TOP",
            "ability_MID",
            "ability_BOT",
            "Color",
            "READY_BUTTON",
        ];
        let rows = rows_names.map(|n| find(c, r, n));
        let ab = |k: usize, n: &str| rows[ROW_AB[k]].and_then(|row| find(c, row, n));
        let dots = ["X", "B", "Y"];
        let mut b = BoxUi {
            root: r,
            join: find(c, r, "JOIN_COLOR"),
            join_border: find(c, r, "JOIN_BORDER"),
            select: find(c, r, "SELECT_COLOR"),
            select_border: find(c, r, "SELECT_BORDER"),
            label: find(c, r, "Text_leaveActive"),
            team: rows[ROW_TEAM],
            team_color: find(c, r, "team_COLOR"),
            rows,
            ab_icons: [0, 1, 2].map(|k| ab(k, "AbilityIcon")),
            ab_locks: [0, 1, 2].map(|k| ab(k, "AbilityIconLOCK")),
            ab_dots: [0, 1, 2].map(|k| ab(k, dots[k])),
            idle_char: find(c, r, "idle_char"),
            wobble_char: find(c, r, "wobble_char"),
            arrows: find(c, r, "<     >"),
            arrow_l: None,
            arrow_r: None,
            rebind_icon: find(c, r, "controller"),
            fake_arrows: find(c, r, "<     > (1)"),
            ready_face: find(c, r, "READY_COLOR_leaveACTIVE"),
            ready_border: find(c, r, "READY_BORDER_leaveACTIVE"),
            ready_char: find(c, r, "Character"),
            ready_name: find(c, r, "READY_PLAYERNAME"),
            grid: find(c, r, "AbilityGrid_leaveActive"),
            grid_circle: None,
            grid_text: None,
            grid_demo: None,
            grid_entries: Vec::new(),
            fill_imgs: Vec::new(),
            border_imgs: Vec::new(),
            texts: Vec::new(),
            row_y: [0.0; 3],
            rebind: RebindUi::default(),
        };
        if let Some(cover) = find(c, r, "RenbindKeys_COVER_leaveINACTIVE") {
            let names = [
                "bind_jmp",
                "bind_a1",
                "bind_a2",
                "bind_a3",
                "bind_up",
                "bind_left",
                "bind_down",
                "bind_right",
            ];
            let top = c.find_in(cover, "jmp,a1,a2,a3");
            b.rebind = RebindUi {
                cover: Some(cover),
                text: c.child(cover, "Text"),
                circles: c.child(cover, "3circles"),
                top,
                bottom: c.find_in(cover, "u,d,r,l"),
                choices: names.map(|n| c.find_in(cover, n)),
                mouse_only: c.find_in(cover, "mouse only"),
                top_y: top.map_or(0.0, |t| c.nodes[t].pos.y),
            };
        }
        if let Some(a) = b.arrows {
            b.arrow_l = c.child(a, "<-");
            b.arrow_r = c.child(a, "->");
        }
        if let Some(g) = b.grid {
            b.grid_circle = c.child(g, "selectionCircle");
            b.grid_text = c.child(g, "Text");
            b.grid_demo = c.child(g, "NotAvailableInDemo");
        }
        for k in 0..3 {
            b.row_y[k] = rows[ROW_AB[k]].map_or(0.0, |n| c.nodes[n].pos.y);
        }
        b.fill_imgs = ["SELECT_COLOR", "READY_COLOR_leaveACTIVE", "bg"]
            .iter()
            .filter_map(|n| find(c, r, n))
            .collect();
        b.border_imgs = [
            "team_BORDER",
            "ability_1_BORDER (1)",
            "X",
            "ability_1_BORDER (2)",
            "B",
            "ability_1_BORDER (3)",
            "Y",
            "SELECT_BORDER",
            "READY_BORDER_leaveACTIVE",
            "border",
            "selectionCircle",
        ]
        .iter()
        .filter_map(|n| find(c, r, n))
        .collect();
        if let Some(a) = b.arrows {
            b.border_imgs.extend(c.nodes[a].children.iter().copied());
        }
        if let Some(rb) = rows[ROW_READY] {
            b.border_imgs.extend(crate::menu::descendants(c, rb));
        }
        if let Some(rb) = rows[ROW_REBIND] {
            b.border_imgs.extend(c.child(rb, "border"));
        }
        // The rebinding cover follows the team colours too (TeamSelector).
        if let Some(cover) = b.rebind.cover {
            b.fill_imgs.push(cover);
            for n in crate::menu::descendants(c, cover) {
                if c.nodes[n].image.is_some() {
                    b.border_imgs.push(n);
                }
            }
        }
        b.texts = [
            b.label,
            find(c, r, "READY_COLOR_leaveACTIVE").and_then(|x| c.child(x, "Text (1)")),
        ]
        .into_iter()
        .flatten()
        .collect();
        if let Some(cover) = b.rebind.cover {
            for n in crate::menu::descendants(c, cover) {
                if c.nodes[n].text.is_some() {
                    b.texts.push(n);
                }
            }
        }
        b
    }

    /// Instantiates the grid entries (`AbilityGridEntry`) in every box.
    fn build_grids(&mut self) {
        for bi in 0..self.boxes.len() {
            self.build_grid(bi);
        }
    }

    fn build_grid(&mut self, bi: usize) {
        let Some(g) = self.boxes[bi].grid else { return };
        if !self.boxes[bi].grid_entries.is_empty() {
            return;
        }
        let Ok(p) = self.data.prefab("sharedassets2.assets:32") else {
            return;
        };
        let n = self.ability_count.saturating_sub(1);
        let rows = n.div_ceil(GRID_COLS).max(1);
        let gsize = self.canvas.nodes[g].size;
        let h = gsize.y * 0.9;
        let entry = (gsize.x * h / (GRID_COLS * rows) as f32).sqrt();
        let base = Vec2::new(126.0, -122.0);
        for k in 0..n {
            let (row, col) = (k / GRID_COLS, k % GRID_COLS);
            let id = self.canvas.graft(&self.data, &p.objects, 0, g);
            let node = &mut self.canvas.nodes[id];
            node.pos = base
                + Vec2::new(
                    col as f32 * entry - gsize.x / 2.0,
                    -(row as f32) * entry + gsize.y / 2.0,
                );
            let s = entry / node.size.x.max(1.0);
            node.scale = glam::Vec3::new(0.9 * s, 0.9 * entry / node.size.y.max(1.0), 1.0);
            let icon = self.canvas.find_in(id, "AbilityIcon");
            let lock = self.canvas.find_in(id, "LOCK");
            self.canvas
                .set_sprite(icon, self.visuals.ability_icon(k + 1));
            self.canvas.set_active(lock, false);
            self.boxes[bi].grid_entries.push(id);
        }
        // The selection circle draws above the entries.
        if let Some(c) = self.boxes[bi].grid_circle {
            let kids = &mut self.canvas.nodes[g].children;
            kids.retain(|&x| x != c);
            kids.push(c);
        }
        if let Some(b) = self.canvas.child(g, "border") {
            let kids = &mut self.canvas.nodes[g].children;
            kids.retain(|&x| x != b);
            kids.insert(1.min(kids.len()), b);
        }
    }

    /// Makes sure at least `n` boxes exist (clones of the first, up to 12).
    fn ensure_boxes(&mut self, n: usize) {
        let n = n.min(12);
        while self.boxes.len() < n {
            let (Some(parent), true) = (self.rectangles, !self.boxes.is_empty()) else {
                return;
            };
            let id = self.canvas.clone_subtree(self.template.root, parent);
            let ui = Self::box_ui(&self.canvas, id);
            let i = self.boxes.len();
            self.boxes.push(ui);
            self.states.push(BoxState::new(i));
            self.build_grid(i);
        }
    }

    /// Online lobby: remote boxes, resolved colours/teams, title, start.
    pub fn set_online(&mut self, view: OnlineView) {
        self.n_abilities = view.n_abilities.clamp(1, 3);
        // Remote boxes: reuse existing ones, then free boxes, then new.
        let want = view.remote.len();
        let mut have: Vec<usize> = (0..self.states.len())
            .filter(|&i| self.states[i].remote)
            .collect();
        while have.len() > want {
            let i = have.pop().unwrap_or(0);
            let st = &mut self.states[i];
            st.remote = false;
            st.name = None;
            st.face = Face::Join;
        }
        while have.len() < want {
            let free = (0..self.states.len())
                .find(|&i| !self.states[i].remote && self.states[i].device.is_none());
            let i = match free {
                Some(i) => i,
                None if self.boxes.len() < 12 => {
                    self.ensure_boxes(self.boxes.len() + 1);
                    self.boxes.len() - 1
                }
                None => break,
            };
            self.states[i].remote = true;
            self.states[i].face_since = 0.0;
            have.push(i);
        }
        for (&i, r) in have.iter().zip(&view.remote) {
            let st = &mut self.states[i];
            let face = if r.ready { Face::Ready } else { Face::Select };
            if st.face != face {
                st.face = face;
                st.face_since = 0.0;
            }
            if st.color != r.color {
                st.color_since = 0.0;
            }
            st.color = r.color;
            st.team = r.team;
            st.abilities = r.abilities;
            st.name = Some(r.name.clone());
            st.row = ROW_READY;
        }
        // Local boxes: the host's resolution once ready.
        let locals: Vec<usize> = (0..self.states.len())
            .filter(|&i| self.states[i].device.is_some() && !self.states[i].remote)
            .collect();
        for (k, &i) in locals.iter().enumerate() {
            if let Some(Some((c, t))) = view.resolved.get(k) {
                let st = &mut self.states[i];
                if st.face == Face::Ready {
                    st.color = *c;
                }
                if !st.team_chosen {
                    st.team = *t;
                }
            }
        }
        // At least one free box to join.
        let used = self
            .states
            .iter()
            .filter(|s| s.device.is_some() || s.remote)
            .count();
        if used >= self.boxes.len() && used < 12 {
            self.ensure_boxes(used + 1);
        }
        self.online = Some(view);
    }

    /// Online: the local boxes (box order) with their choices.
    pub fn local_boxes(&self) -> Vec<LocalBox> {
        self.states
            .iter()
            .filter(|s| !s.remote && s.face != Face::Join)
            .filter_map(|s| {
                Some(LocalBox {
                    device: s.device?,
                    color: s.color,
                    team: s.team_chosen.then_some(s.team),
                    abilities: s.abilities,
                    ready: s.face == Face::Ready,
                })
            })
            .collect()
    }

    /// Online: back from a match: local boxes return to the select face
    /// (ready again, like the original) with their mid-game abilities.
    pub fn back_from_match(&mut self, abilities: &[[usize; 3]]) {
        self.starting = None;
        let locals: Vec<usize> = (0..self.states.len())
            .filter(|&i| self.states[i].device.is_some() && !self.states[i].remote)
            .collect();
        for (k, &i) in locals.iter().enumerate() {
            if let Some(a) = abilities.get(k) {
                self.states[i].abilities = *a;
            }
            self.set_face(i, Face::Select);
            self.states[i].row = ROW_TEAM;
        }
    }

    fn rand(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    fn occupied(&self, d: DeviceId) -> bool {
        self.states.iter().any(|s| s.device == Some(d))
    }

    /// The keyboard and mouse player shares keys with the keyboard halves.
    fn device_free(&self, d: DeviceId) -> bool {
        if self.occupied(d) {
            return false;
        }
        match d {
            DeviceId::KeyboardMouse => {
                !self.occupied(DeviceId::KeyboardLeft) && !self.occupied(DeviceId::KeyboardRight)
            }
            DeviceId::KeyboardLeft | DeviceId::KeyboardRight => {
                !self.occupied(DeviceId::KeyboardMouse)
            }
            _ => true,
        }
    }

    /// Visible rows for the number of abilities.
    fn row_visible(&self, row: usize) -> bool {
        match row {
            3 => self.n_abilities >= 2,
            4 => self.n_abilities >= 3,
            _ => true,
        }
    }

    /// Slot of an ability row (row 3 is slot 2 and row 4 slot 1 with 3).
    fn row_slot(&self, row: usize) -> usize {
        row_slot(row, self.n_abilities)
    }

    fn color_taken_by_ready(&self, me: usize, color: i32) -> bool {
        self.states
            .iter()
            .enumerate()
            .any(|(i, s)| i != me && s.face == Face::Ready && s.color == color)
    }

    fn next_color(&self, me: usize, from: i32, dir: i32) -> i32 {
        let mut c = from;
        for _ in 0..12 {
            c = (c + dir).rem_euclid(12);
            if !self.color_taken_by_ready(me, c) {
                return c;
            }
        }
        from
    }

    fn cycle_ability(&self, a: usize, dir: i32) -> usize {
        let n = self.ability_count.max(2) as i32;
        let mut x = (a as i32 + dir).rem_euclid(n);
        if x == 0 {
            x = if dir > 0 { 1 } else { n - 1 };
        }
        x as usize
    }

    fn set_face(&mut self, i: usize, face: Face) {
        self.states[i].face = face;
        self.states[i].face_since = 0.0;
    }

    fn can_start(&self) -> bool {
        let local = |s: &&BoxState| !s.remote;
        let busy = self
            .states
            .iter()
            .filter(local)
            .any(|s| matches!(s.face, Face::Select | Face::Grid));
        if let Some(o) = &self.online {
            return o.can_start && !busy;
        }
        let ready: Vec<&BoxState> = self
            .states
            .iter()
            .filter(|s| s.face == Face::Ready)
            .collect();
        if busy || ready.is_empty() {
            return false;
        }
        ready.len() == 1 || ready.iter().any(|s| s.team != ready[0].team)
    }

    fn lobby(&self) -> Lobby {
        Lobby {
            players: self
                .states
                .iter()
                .filter(|s| s.face == Face::Ready && !s.remote)
                .filter_map(|s| {
                    Some(LobbyPlayer {
                        device: s.device?,
                        color: s.color,
                        team: s.team,
                        abilities: s.abilities,
                    })
                })
                .collect(),
        }
    }

    pub fn tick(
        &mut self,
        shared: &mut Shared,
        devices: &[(DeviceId, InputState)],
        raw: &[RawPress],
        audio: &mut Audio,
    ) -> CharSelectOutcome {
        let dt = 1.0 / 60.0;
        self.time += dt;
        shared.menu_time += dt;
        for s in &mut self.states {
            s.color_since += dt;
            s.face_since += dt;
            s.rebind_pulse += dt;
            for w in &mut s.rebind_wiggle {
                *w += dt;
            }
            s.rebind_ended = false;
        }
        self.rebind_presses(shared, raw, audio);
        if let Some(t) = &mut self.leaving {
            *t += dt;
            if *t > 0.5 {
                return CharSelectOutcome::Back;
            }
            return CharSelectOutcome::Stay;
        }
        if let Some(l) = &self.starting {
            let l = l.clone();
            if self.online.is_some() {
                // The lobby starts the match; we stay on screen meanwhile.
                self.starting = None;
            }
            return CharSelectOutcome::Start(l);
        }
        let snd = |shared: &mut Shared, audio: &mut Audio, n: &str| {
            shared.sounds.play(audio, n);
        };
        // Colour uniqueness against ready boxes (every frame).
        for i in 0..self.states.len() {
            if self.states[i].face != Face::Ready
                && self.color_taken_by_ready(i, self.states[i].color)
            {
                let c = self.next_color(i, self.states[i].color, 1);
                self.states[i].color = c;
            }
        }
        // Gamepads that disappeared free their box.
        for s in &mut self.states {
            if let Some(DeviceId::Controller(id)) = s.device
                && !devices.iter().any(|(d, _)| *d == DeviceId::Controller(id))
            {
                s.device = None;
                s.face = Face::Join;
                s.rebind = None;
            }
        }
        let start_ok = self.can_start();
        self.start_frames = if start_ok { self.start_frames + 1 } else { 0 };
        if start_ok && self.start_since.is_none() {
            self.start_since = Some(0.0);
        } else if !start_ok {
            self.start_since = None;
        }
        if let Some(t) = &mut self.start_since {
            *t += dt;
        }
        let screen = Vec2::new(self.size.0 as f32, self.size.1 as f32);
        let placed = self.canvas.layout(screen);
        for (dev, st) in devices {
            let p = st.pressed;
            let dev = *dev;
            if p.0 != 0 {
                log::debug!("charsel: {dev:?} pressed {:#x}", p.0);
            }
            let bi = self.states.iter().position(|s| s.device == Some(dev));
            let Some(i) = bi else {
                // Not in a box: join, or leave the screen.
                let back = match dev {
                    DeviceId::KeyboardMouse => p.contains(Buttons::BACK),
                    DeviceId::Controller(_) => p.contains(Buttons::BACK),
                    _ => false,
                };
                let keyboard_box = [
                    DeviceId::KeyboardMouse,
                    DeviceId::KeyboardLeft,
                    DeviceId::KeyboardRight,
                ]
                .iter()
                .any(|&d| self.occupied(d));
                if back && (dev != DeviceId::KeyboardMouse || !keyboard_box) {
                    snd(shared, audio, "return");
                    snd(shared, audio, "menuWhoosh2");
                    self.leaving = Some(0.0);
                    return CharSelectOutcome::Stay;
                }
                let join = match dev {
                    DeviceId::KeyboardMouse => p.contains(Buttons::ACCEPT),
                    DeviceId::KeyboardLeft | DeviceId::KeyboardRight => p.contains(Buttons::START),
                    DeviceId::Controller(_) => {
                        p.contains(Buttons::ACCEPT) || p.contains(Buttons::START)
                    }
                };
                // Mouse: clicking a free box's join panel joins that box.
                let mut target = None;
                if dev == DeviceId::KeyboardMouse
                    && p.contains(Buttons::ABILITY0)
                    && let Some(ptr) = st.pointer
                {
                    target = (0..self.boxes.len()).find(|&b| {
                        self.states[b].device.is_none()
                            && !self.states[b].remote
                            && self.boxes[b]
                                .join
                                .is_some_and(|j| self.canvas.hit(&placed, j, ptr, screen))
                    });
                }
                if (join || target.is_some()) && self.device_free(dev) {
                    let free = target.or_else(|| {
                        self.states
                            .iter()
                            .position(|s| s.device.is_none() && !s.remote)
                    });
                    let free = match free {
                        Some(f) => Some(f),
                        None if self.boxes.len() < 12 => {
                            self.ensure_boxes(self.boxes.len() + 1);
                            Some(self.boxes.len() - 1)
                        }
                        None => None,
                    };
                    if let Some(f) = free {
                        let s = &mut self.states[f];
                        s.device = Some(dev);
                        s.row = ROW_TEAM;
                        self.set_face(f, Face::Select);
                        snd(shared, audio, "accept");
                        // Keep one free box beyond the occupied ones (up to 12).
                        let used = self
                            .states
                            .iter()
                            .filter(|s| s.device.is_some() || s.remote)
                            .count();
                        if used >= 4 {
                            self.ensure_boxes(used + 1);
                        }
                    }
                }
                continue;
            };
            if self.states[i].rebind.is_some() || self.states[i].rebind_ended {
                // The box's inputs are off while rebinding (ui.md 6).
                continue;
            }
            let face = self.states[i].face;
            let mut accept = p.contains(Buttons::ACCEPT);
            let back = p.contains(Buttons::BACK);
            let start = p.contains(Buttons::START);
            // Mouse (keyboard + mouse box): hover selects rows and grid
            // icons, clicks on the side arrows step, clicks on an ability
            // row open the grid, on an icon pick it, on ready ready up.
            let ptr = (dev == DeviceId::KeyboardMouse)
                .then_some(st.pointer)
                .flatten();
            let moved = ptr.is_some() && ptr != self.last_pointer;
            if dev == DeviceId::KeyboardMouse {
                self.last_pointer = ptr;
            }
            let clicked = ptr.is_some() && p.contains(Buttons::ABILITY0);
            let hit = |n: Option<usize>| {
                ptr.is_some_and(|q| n.is_some_and(|n| self.canvas.hit(&placed, n, q, screen)))
            };
            let mut mouse_h = 0;
            if face == Face::Select && ptr.is_some() {
                let (on_l, on_r) = (hit(self.boxes[i].arrow_l), hit(self.boxes[i].arrow_r));
                let hover = (0..7).find(|&r| self.row_visible(r) && hit(self.boxes[i].rows[r]));
                if moved
                    && !on_l
                    && !on_r
                    && let Some(r) = hover
                    && r != self.states[i].row
                {
                    self.states[i].row = r;
                    snd(shared, audio, "navigateMenu");
                }
                if clicked {
                    if on_l {
                        mouse_h = -1;
                    } else if on_r {
                        mouse_h = 1;
                    } else if hover.is_some_and(|r| (2..=4).contains(&r) || r == ROW_READY) {
                        accept = true;
                    }
                }
            }
            if face == Face::Grid && ptr.is_some() {
                let n = self.ability_count.saturating_sub(1).max(1);
                let over = self.boxes[i].grid_entries.iter().take(n).position(|&e| {
                    hit(Some(e))
                        || crate::menu::descendants(&self.canvas, e)
                            .into_iter()
                            .any(|c| hit(Some(c)))
                });
                if let Some(k) = over {
                    if moved && k != self.states[i].grid_sel {
                        self.states[i].grid_sel = k;
                        snd(shared, audio, "navigateMenu");
                    }
                    if clicked {
                        self.states[i].grid_sel = k;
                        accept = true;
                    }
                }
            }
            match face {
                Face::Join => {}
                Face::Select => {
                    if back {
                        snd(shared, audio, "decline");
                        self.states[i].device = None;
                        self.set_face(i, Face::Join);
                        continue;
                    }
                    if start && matches!(dev, DeviceId::Controller(_)) {
                        snd(shared, audio, "accept");
                        self.set_face(i, Face::Ready);
                        continue;
                    }
                    let dir_v = if p.contains(Buttons::DOWN) {
                        1
                    } else if p.contains(Buttons::UP) {
                        -1
                    } else {
                        0
                    };
                    if dir_v != 0 {
                        let mut r = self.states[i].row as i32;
                        for _ in 0..7 {
                            r = (r + dir_v).rem_euclid(7);
                            if self.row_visible(r as usize) {
                                break;
                            }
                        }
                        self.states[i].row = r as usize;
                        snd(shared, audio, "navigateMenu");
                    }
                    let dir_h = if p.contains(Buttons::RIGHT) {
                        1
                    } else if p.contains(Buttons::LEFT) {
                        -1
                    } else {
                        mouse_h
                    };
                    let row = self.states[i].row;
                    if dir_h != 0 {
                        match row {
                            ROW_TEAM => {
                                let teams = if self.boxes.len() > 4 || self.online.is_some() {
                                    12
                                } else {
                                    4
                                };
                                self.states[i].team_chosen = true;
                                self.states[i].team =
                                    (self.states[i].team + dir_h).rem_euclid(teams);
                                snd(shared, audio, "navigateMenu2");
                            }
                            2..=4 => {
                                let slot = self.row_slot(row);
                                let a = self.states[i].abilities[slot];
                                self.states[i].abilities[slot] = self.cycle_ability(a, dir_h);
                                snd(shared, audio, "navigateMenu2");
                            }
                            ROW_COLOR => {
                                let c = self.next_color(i, self.states[i].color, dir_h);
                                self.states[i].color = c;
                                self.states[i].color_since = 0.0;
                                let n = 1 + self.rand() % 3;
                                snd(shared, audio, &format!("transform{n}"));
                            }
                            _ => {}
                        }
                    }
                    let click_rebind = dev == DeviceId::KeyboardMouse
                        && p.contains(Buttons::ABILITY0)
                        && st.pointer.is_some_and(|ptr| {
                            self.boxes[i].rows[ROW_REBIND]
                                .is_some_and(|r| self.canvas.hit(&placed, r, ptr, screen))
                        });
                    if click_rebind || (accept && row == ROW_REBIND) {
                        self.states[i].row = ROW_REBIND;
                        let key = crate::input::device_key(dev, &shared.platform.controllers);
                        self.states[i].rebind = Some(Rebind::new(dev, key));
                        snd(shared, audio, "accept");
                        continue;
                    }
                    if accept {
                        match row {
                            ROW_REBIND => {}
                            ROW_TEAM | ROW_COLOR => {
                                self.states[i].row = row + 1;
                                while !self.row_visible(self.states[i].row) {
                                    self.states[i].row += 1;
                                }
                                snd(shared, audio, "navigateMenu");
                            }
                            2..=4 => {
                                let slot = self.row_slot(row);
                                self.states[i].grid_sel =
                                    self.states[i].abilities[slot].saturating_sub(1);
                                self.set_face(i, Face::Grid);
                                snd(shared, audio, "accept");
                            }
                            _ => {
                                snd(shared, audio, "accept");
                                self.set_face(i, Face::Ready);
                            }
                        }
                    }
                }
                Face::Grid => {
                    if back {
                        snd(shared, audio, "decline");
                        self.set_face(i, Face::Select);
                        continue;
                    }
                    let n = self.ability_count.saturating_sub(1).max(1);
                    let rows = n.div_ceil(GRID_COLS);
                    let s = &mut self.states[i];
                    let (mut r, mut c) = (s.grid_sel / GRID_COLS, s.grid_sel % GRID_COLS);
                    let mut moved = false;
                    if p.contains(Buttons::RIGHT) {
                        c = (c + 1) % GRID_COLS;
                        moved = true;
                    } else if p.contains(Buttons::LEFT) {
                        c = (c + GRID_COLS - 1) % GRID_COLS;
                        moved = true;
                    } else if p.contains(Buttons::DOWN) {
                        r = (r + 1) % rows;
                        moved = true;
                    } else if p.contains(Buttons::UP) {
                        r = (r + rows - 1) % rows;
                        moved = true;
                    }
                    if moved {
                        s.grid_sel = (r * GRID_COLS + c).min(n - 1);
                        snd(shared, audio, "navigateMenu");
                    }
                    if accept || start {
                        let row = self.states[i].row;
                        let slot = self.row_slot(row);
                        self.states[i].abilities[slot] = self.states[i].grid_sel + 1;
                        self.set_face(i, Face::Select);
                        snd(shared, audio, "accept");
                    }
                }
                Face::Ready => {
                    if back {
                        snd(shared, audio, "decline");
                        self.set_face(i, Face::Select);
                        continue;
                    }
                    let click_start = dev == DeviceId::KeyboardMouse
                        && p.contains(Buttons::ABILITY0)
                        && st.pointer.is_some_and(|ptr| {
                            self.start
                                .is_some_and(|s| self.canvas.hit(&placed, s, ptr, screen))
                        });
                    if (accept || start || click_start) && start_ok && self.start_frames > 1 {
                        snd(shared, audio, "startGame");
                        self.starting = Some(self.lobby());
                        return CharSelectOutcome::Stay;
                    }
                }
            }
        }
        for s in &mut self.states {
            if let Some(r) = s.rebind.as_mut() {
                r.arm();
            }
        }
        CharSelectOutcome::Stay
    }

    /// Feeds this tick's raw key / button presses to the boxes that are
    /// rebinding (ui.md 6).
    fn rebind_presses(&mut self, shared: &mut Shared, raw: &[RawPress], audio: &mut Audio) {
        for i in 0..self.states.len() {
            if self.states[i].rebind.is_none() {
                continue;
            }
            // A controller that disappeared ends its rebinding.
            for p in raw {
                let Some(r) = self.states[i].rebind.as_mut() else {
                    break;
                };
                let Some(ev) = r.feed(p, &shared.settings.bindings) else {
                    continue;
                };
                let key = r.key.clone();
                let s = &mut self.states[i];
                match ev {
                    RebindEvent::Bound(_) => {
                        s.rebind_pulse = 0.0;
                        shared.sounds.play(audio, "navigateMenu");
                    }
                    RebindEvent::Clash(k) => {
                        if let Some(w) = s.rebind_wiggle.get_mut(k) {
                            *w = 0.0;
                        }
                        shared.sounds.play(audio, "smallError");
                    }
                    RebindEvent::Refused => {
                        shared.sounds.play(audio, "smallError");
                    }
                    RebindEvent::Done(b) => {
                        log::info!(
                            "rebound {:?} ({}): {:?}",
                            r_scheme(s),
                            key,
                            b.devices.get(&key)
                        );
                        s.rebind = None;
                        s.rebind_ended = true;
                        s.rebind_pulse = 0.0;
                        shared.sounds.play(audio, "accept2");
                        shared.set_bindings(b);
                        break;
                    }
                    RebindEvent::Cancelled => {
                        s.rebind = None;
                        s.rebind_ended = true;
                        shared.sounds.play(audio, "decline2");
                        break;
                    }
                }
            }
        }
    }

    /// Positions the boxes: the original row of 4, or two rows for more.
    fn layout_boxes(&mut self) {
        let n = self.boxes.len();
        // Online: this machine's boxes first, then the remote players.
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&i| (self.states.get(i).is_some_and(|s| s.remote), i));
        for (rank, &bi) in order.iter().enumerate() {
            let i = rank;
            let b = &self.boxes[bi];
            let node = &mut self.canvas.nodes[b.root];
            if n <= 4 {
                node.pos = Vec2::new(800.0 * i as f32, -51.0);
                node.scale_mul = 1.0;
            } else {
                let cols = n.div_ceil(2);
                let (row, col) = (i / cols, i % cols);
                let spacing = (3600.0 / cols as f32).min(800.0);
                let k = (spacing / 800.0).min(0.47);
                node.pos = Vec2::new(
                    1200.0 + (col as f32 - (cols as f32 - 1.0) * 0.5) * spacing,
                    if row == 0 { 520.0 } else { -640.0 },
                );
                node.scale_mul = k;
            }
        }
    }

    #[allow(clippy::needless_range_loop)] // rows by index
    fn apply(&mut self) {
        self.layout_boxes();
        let v = self.visuals.clone();
        for i in 0..self.boxes.len() {
            let b = self.boxes[i].clone();
            let s = self.states[i].clone();
            let c = &mut self.canvas;
            let slide = |since: f32| (1.0 - eval_keys(&FACE_IN, since)).max(0.0);
            let k = slide(s.face_since);
            let join = s.face == Face::Join;
            let ready = s.face == Face::Ready;
            let selecting = matches!(s.face, Face::Select | Face::Grid);
            for n in [b.join, b.join_border] {
                c.set_active(n, join);
                if let Some(n) = n {
                    c.nodes[n].offset.y = if join { -600.0 * k } else { 0.0 };
                }
            }
            for n in [b.select, b.select_border] {
                c.set_active(n, selecting);
                if let Some(n) = n {
                    c.nodes[n].offset.y = if s.face == Face::Select {
                        2000.0 * k * k
                    } else {
                        0.0
                    };
                }
            }
            for n in [b.ready_face, b.ready_border] {
                c.set_active(n, ready);
            }
            c.set_active(b.grid, s.face == Face::Grid);
            c.set_active(b.ready_name, ready && s.name.is_some());
            if let Some(name) = &s.name {
                c.set_text(b.ready_name, name);
            }
            // Team colours.
            let team = v.team(s.team);
            for &n in &b.fill_imgs {
                c.set_color(Some(n), team.fill);
            }
            for &n in &b.border_imgs {
                c.set_color(Some(n), team.border);
            }
            c.set_color(b.team_color, team.saturated);
            for &n in &b.texts {
                c.set_color(Some(n), team.border);
            }
            // Rows: abilities, visibility, arrows.
            for k in 0..3 {
                let row = ROW_AB[k];
                let visible = match row {
                    3 => self.n_abilities >= 2,
                    4 => self.n_abilities >= 3,
                    _ => true,
                };
                c.set_active(b.rows[row], visible);
                let slot = match (row, self.n_abilities) {
                    (2, _) => 0,
                    (3, 3) => 2,
                    (3, _) => 1,
                    _ => 1,
                };
                c.set_sprite(b.ab_icons[k], v.ability_icon(s.abilities[slot]));
                c.set_active(b.ab_locks[k], false);
                if let Some(rn) = b.rows[row] {
                    c.nodes[rn].pos.y = match self.n_abilities {
                        1 => 18.3,
                        2 if row == 2 => 220.0,
                        2 => -230.0,
                        _ => b.row_y[k],
                    };
                }
                // The selected row's button dots give way to the arrows.
                if let Some(d) = b.ab_dots[k]
                    && let Some(img) = c.nodes[d].image.as_mut()
                {
                    img.enabled = s.row != row;
                }
            }
            if let Some(a) = b.arrows {
                let y = match s.row {
                    ROW_REBIND | ROW_READY => -1024.0,
                    r => b.rows[r].map_or(0.0, |n| c.nodes[n].pos.y),
                };
                c.nodes[a].pos.y = y;
                c.nodes[a].active = selecting;
            }
            if let Some(l) = b.arrow_l {
                c.nodes[l].pos.x = -ARROW_X[s.row];
            }
            if let Some(r) = b.arrow_r {
                c.nodes[r].pos.x = ARROW_X[s.row];
            }
            // Label: the selected row (ability name on ability rows).
            let label = match s.row {
                2..=4 => {
                    let slot = row_slot(s.row, self.n_abilities);
                    v.content
                        .abilities
                        .get(s.abilities[slot])
                        .map(|a| self.l10n.get(&a.name))
                        .unwrap_or_default()
                }
                r => self.l10n.get(ROW_LABEL_KEYS[r]),
            };
            c.set_text(b.label, &label);
            // Player colour on the slime images.
            let mat = v.ui_materials.get(s.color as usize).cloned();
            let shade = shade_of(&self.data, mat.as_deref());
            for n in [b.idle_char, b.wobble_char, b.ready_char]
                .into_iter()
                .flatten()
            {
                if let Some(img) = c.nodes[n].image.as_mut()
                    && matches!(img.shade, UiShade::Channel { .. })
                {
                    img.shade = shade.clone();
                }
            }
            // Wobble on a colour change (WobbleAnimation).
            if let Some(wc) = b.idle_char {
                let t = s.color_since;
                let w = if t < 0.6 {
                    (t * 30.0).sin() * (0.6 - t) * 0.25
                } else {
                    0.0
                };
                c.nodes[wc].scale_mul = 1.0 + w;
            }
            // Rebind icon: keyboard or controller.
            let kb = matches!(s.device, Some(d) if d.is_keyboard());
            let sprite = if kb {
                "sharedassets2.assets:15"
            } else {
                "sharedassets2.assets:5"
            };
            c.set_sprite(b.rebind_icon, Some(sprite));
            // Key rebinding cover (ui.md 6): it hides the box's rows, the
            // label and the ready button.
            let rb = &b.rebind;
            let rebinding = s.rebind.is_some() && selecting;
            c.set_active(rb.cover, rebinding);
            if rebinding {
                c.set_active(b.label, false);
                c.set_active(b.rows[ROW_READY], false);
                c.set_active(b.arrows, false);
            } else if selecting {
                c.set_active(b.label, true);
                c.set_active(b.rows[ROW_READY], true);
            }
            if let Some(r) = &s.rebind {
                let step = r.step.min(7);
                let kb = r.scheme != crate::input::Scheme::Controller;
                let prompt = self
                    .l10n
                    .get(remap::STEP_KEYS[step])
                    .replace(' ', "\n")
                    .replace('_', " ");
                c.set_text(rb.text, &prompt);
                c.set_sprite(rb.circles, Some(remap::STEP_SPRITES[step]));
                if let Some(n) = rb.circles {
                    c.nodes[n].rotation = if kb {
                        remap::STEP_ROTATIONS[step].to_radians()
                    } else {
                        0.0
                    };
                }
                c.set_active(rb.bottom, kb);
                if let Some(t) = rb.top {
                    c.nodes[t].pos.y = rb.top_y;
                }
                for k in 0..8 {
                    let name = r
                        .chosen
                        .get(k)
                        .cloned()
                        .flatten()
                        .map(|n| remap::display_name(&n))
                        .unwrap_or_default();
                    c.set_text(rb.choices[k], &name);
                    c.set_active(rb.choices[k], k < 4 || kb);
                    if let Some(n) = rb.choices[k] {
                        c.nodes[n].rotation = eval_keys(&WIGGLE, s.rebind_wiggle[k]).to_radians();
                    }
                }
                let mouse =
                    r.scheme == crate::input::Scheme::KeyboardMouse && remap::MOUSE_ONLY[step];
                c.set_active(rb.mouse_only, mouse);
                c.set_text(rb.mouse_only, &self.l10n.get("mouse only"));
            }
            c.set_active(b.fake_arrows, s.row == ROW_REBIND && s.face == Face::Select);
            // The rebind button: hover scale while selected, a pulse per
            // bound step (`HoverScaleAnim`, `animateOnButtonBinded`).
            if let Some(n) = b.rows[ROW_REBIND] {
                let hover = if s.row == ROW_REBIND && selecting {
                    1.11
                } else {
                    1.0
                };
                c.nodes[n].scale_mul = hover * eval_keys(&PULSE, s.rebind_pulse);
            }
            // Grid: selection.
            if s.face == Face::Grid {
                for (k, &e) in b.grid_entries.iter().enumerate() {
                    let sel = k == s.grid_sel;
                    c.nodes[e].scale_mul = if sel { 1.2 / 0.9 } else { 1.0 };
                }
                if let (Some(circle), Some(&e)) = (b.grid_circle, b.grid_entries.get(s.grid_sel)) {
                    c.nodes[circle].pos = c.nodes[e].pos;
                }
                let name = v
                    .content
                    .abilities
                    .get(s.grid_sel + 1)
                    .map(|a| self.l10n.get(&a.name))
                    .unwrap_or_default();
                c.set_text(b.grid_text, &name);
                c.set_active(b.grid_demo, false);
            }
            let _ = b.team;
        }
        // Online: title (room code) and status line.
        if let Some(o) = &self.online {
            self.canvas.set_text(self.title, &o.title);
            self.canvas.set_active(self.status, !o.status.is_empty());
            self.canvas.set_text(self.status, &o.status);
        }
        // Start button: shown (sliding in) while the start condition holds.
        if let Some(st) = self.start {
            let on = self.start_since.is_some();
            self.canvas.nodes[st].active = on;
            let t = self.start_since.unwrap_or(0.0);
            self.canvas.nodes[st].offset.y =
                400.0 * (1.0 - eval_keys(&[(0.0, 0.0), (0.35, 1.0)], t));
        }
        // Leaving: boxes fly out.
        if let (Some(t), Some(r)) = (self.leaving, self.rectangles) {
            self.canvas.nodes[r].offset.y = -3000.0 * eval_keys(&[(0.0, 0.0), (0.5, 1.0)], t);
        }
    }

    pub fn draw(&mut self, shared: &mut Shared, ctx: &mut DrawContext<'_>) {
        self.size = ctx.size;
        self.apply();
        let time = shared.menu_time;
        let mut pass = ui_pass("charsel", ctx.size);
        pass.clear = Some(Color::BLACK);
        let mut d = UiDraw {
            gpu: ctx.gpu,
            renderer: ctx.renderer,
            assets: &mut shared.assets,
            fonts: &mut shared.fonts,
            l10n: &self.l10n,
            time,
        };
        self.canvas.draw(&mut d, ctx.size, &mut pass);
        ctx.frame.passes.push(pass);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_setups_follow_box_order() {
        let lobby = Lobby {
            players: vec![
                LobbyPlayer {
                    device: DeviceId::KeyboardMouse,
                    color: 0,
                    team: 1,
                    abilities: [4, 7, 2],
                },
                LobbyPlayer {
                    device: DeviceId::Controller(3),
                    color: 6,
                    team: 0,
                    abilities: [1, 1, 1],
                },
            ],
        };
        let (s, src) = lobby.setups(2);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].id, s[1].id), (1, 2));
        assert!(s[0].uses_keyboard_and_mouse && !s[1].uses_keyboard_and_mouse);
        assert_eq!(s[0].abilities, vec![4, 7]);
        assert!(matches!(src[1], Source::Device(DeviceId::Controller(3))));
    }
}

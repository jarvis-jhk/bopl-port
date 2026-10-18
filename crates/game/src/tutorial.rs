//! The tutorial (docs/spec/tutorial.md, main menu → Tutorial): the sim runs
//! the tutorial scene (`bopl_sim::tutorial`); this screen feeds player 1
//! the last used device (keyboard + mouse or a controller, switching like
//! the original), draws the tutorial's objects (arrows, pickups, dummies)
//! and the hint set of the current device over the level presentation, and
//! returns to the main menu when the sim says the tutorial is over.

use anyhow::{Context, Result};
use bopl_import::GameData;
use bopl_import::format::{Level, Object};
use bopl_render::{Color, Frame, PassTarget, Text};
use bopl_sim::World;
use bopl_sim::input::PlayerInput;
use bopl_sim::player::PlayerSetup;
use bopl_sim::view::View;
use bopl_sim::world::MatchSettings;
use glam::{Mat4, Quat, Vec2, Vec3};

use crate::app::DrawContext;
use crate::audio::Audio;
use crate::input::{Buttons, DeviceId, InputState, RawPress};
use crate::level_view::LevelView;
use crate::match_scene::device_input;
use crate::match_view::{MatchPresenter, RendererInfo};
use crate::present::{SpriteDraw, WorldView};
use crate::shared::Shared;
use crate::ui::{Canvas, L10n, UiDraw, ui_pass};

/// Build index of the tutorial scene.
pub const SCENE: usize = bopl_import::load::TUTORIAL;

pub enum TutorialOutcome {
    Stay,
    /// Done (dash, or select): back to the main menu.
    Leave,
}

/// A sprite renderer of a scene object, relative to its tutorial root.
#[derive(Clone, Debug)]
struct Piece {
    info: RendererInfo,
    sprite: String,
    /// Local matrix relative to the root (the root's own scale included).
    local: Mat4,
    /// A child of the root (an arrow's bobbing sprite).
    child: bool,
}

/// One hint set (`TutorialSceneReferences`).
#[derive(Clone, Debug, Default)]
struct HintSet {
    /// Movement hints in order (`wasd`): object indices.
    wasd: Vec<usize>,
    indicator: Option<usize>,
    hold_to_shoot: Option<usize>,
    hold_to_grenade: Option<usize>,
    click_to_dash: Option<usize>,
    /// The world-space canvas and the node ids of the three texts in it.
    canvas: Option<Canvas>,
    canvas_world: Mat4,
    nodes: [Option<usize>; 3],
}

pub struct TutorialScene {
    world: World,
    pres: MatchPresenter,
    objects: Vec<Object>,
    /// World matrices of the scene objects (authored poses).
    world_m: Vec<Mat4>,
    /// Draw pieces of the arrows, pickups and dummies (by sim object order
    /// in `World::tutorial`).
    arrow_pieces: Vec<Vec<Piece>>,
    pickup_pieces: Vec<Vec<Piece>>,
    dummy_pieces: Vec<Vec<Piece>>,
    keyboard: HintSet,
    gamepad: HintSet,
    l10n: L10n,
    view: View,
    last_world: WorldView,
    size: (u32, u32),
    /// The player's device (keyboard + mouse or a controller).
    pub device: DeviceId,
    time: f32,
    ticks: u64,
    /// Last logged progress (absorbed, unlocked slots).
    logged: (i32, [bool; 3]),
}

/// Composed local TRS of a scene object.
fn local_matrix(o: &Object) -> Mat4 {
    let [x, y, z, w] = o.local.rotation;
    Mat4::from_scale_rotation_translation(
        Vec3::from(o.local.scale),
        Quat::from_xyzw(x, y, z, w),
        Vec3::from(o.local.position),
    )
}

fn world_matrices(objs: &[Object]) -> Vec<Mat4> {
    let mut out = vec![Mat4::IDENTITY; objs.len()];
    // Parents come before children in the scene list.
    for (i, o) in objs.iter().enumerate() {
        let l = local_matrix(o);
        out[i] = match o.parent {
            Some(p) => out[p] * l,
            None => l,
        };
    }
    out
}

fn subtree(objs: &[Object], root: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        out.push(i);
        stack.extend(objs[i].children.iter().rev().copied());
    }
    out
}

/// The renderers of `root`'s subtree relative to the root's position and
/// rotation (scales kept).
fn pieces(objs: &[Object], world_m: &[Mat4], root: usize) -> Vec<Piece> {
    let (_, rot, pos) = world_m[root].to_scale_rotation_translation();
    let base = Mat4::from_rotation_translation(rot, pos).inverse();
    subtree(objs, root)
        .into_iter()
        .filter_map(|i| {
            let o = &objs[i];
            if i != root && !o.active {
                return None;
            }
            let info = RendererInfo::of(o)?;
            if !info.enabled {
                return None;
            }
            Some(Piece {
                sprite: info.sprite.clone()?,
                info,
                local: base * world_m[i],
                child: i != root,
            })
        })
        .collect()
}

/// World sprites with a UI slime material (`Custom/UI_CustomColor`, e.g.
/// the arrows' `blackSlimeUI`): the in-game slime material with the same
/// colours draws them the same way in the level presentation.
fn world_material(data: &GameData, id: &str) -> Option<String> {
    let m = data.materials.get(id)?;
    if m.shader != "Custom/UI_CustomColor" {
        return None;
    }
    let key = |m: &bopl_import::format::Material| {
        (
            m.colors.get("_MainColor").copied(),
            m.colors.get("_ShadowColor").copied(),
        )
    };
    let want = key(m);
    data.materials
        .iter()
        .find(|(_, x)| x.shader == "Custom/CustomColourShader" && key(x) == want)
        .map(|(k, _)| k.clone())
}

/// Draws pieces with the root at `pos` (world) rotated by `rot`.
fn draw_pieces(out: &mut Vec<SpriteDraw>, ps: &[Piece], pos: Vec3, rot: f32, bob: f32) {
    let root = Mat4::from_rotation_translation(Quat::from_rotation_z(rot), pos);
    for p in ps {
        let local = if p.child {
            Mat4::from_translation(Vec3::new(0.0, bob, 0.0)) * p.local
        } else {
            p.local
        };
        out.push(p.info.draw(&p.sprite, root * local));
    }
}

/// `{"$ref": id}` → the object owning that component (or the object).
fn ref_index(objs: &[Object], j: &serde_json::Value) -> Option<usize> {
    let id = j["$ref"].as_str()?;
    objs.iter()
        .position(|o| o.id == id || o.components.iter().any(|c| c.id == id))
}

impl TutorialScene {
    /// Starts the tutorial; `device` = the device that chose it in the menu
    /// (a controller takes the player at once, ui.md 10).
    pub fn new(shared: &mut Shared, device: DeviceId, audio: &mut Audio) -> Result<Self> {
        let data = shared.data.clone();
        let device = match device {
            DeviceId::Controller(_) => device,
            _ => DeviceId::KeyboardMouse,
        };
        let setup = PlayerSetup {
            id: 1,
            team: 1,
            color: 0,
            abilities: vec![4, 3, 2],
            uses_keyboard_and_mouse: device == DeviceId::KeyboardMouse,
        };
        let mut world = World::new(
            shared.levels.content.clone(),
            &[setup],
            MatchSettings::default(),
        );
        let lv = shared.levels.get(SCENE).context("the tutorial scene")?;
        bopl_sim::session::load_level(&mut world, lv);
        let st = world.tutorial.as_ref().context("tutorial state")?;
        let level: Level = data.level(SCENE)?;
        let objects = level.objects.clone();
        let world_m = world_matrices(&objects);
        let def = st.def.clone();
        let arrow_pieces = def
            .arrows
            .iter()
            .map(|a| pieces(&objects, &world_m, a.object))
            .collect();
        let pickup_pieces = def
            .pickups
            .iter()
            .map(|p| pieces(&objects, &world_m, p.obj.object))
            .collect();
        let dummy_pieces: Vec<Vec<Piece>> = def
            .dummies
            .iter()
            .map(|d| pieces(&objects, &world_m, d.obj.object))
            .collect();
        let mut arrow_pieces: Vec<Vec<Piece>> = arrow_pieces;
        for p in arrow_pieces.iter_mut().flatten() {
            if let Some(m) = p
                .info
                .material
                .as_deref()
                .and_then(|m| world_material(&data, m))
            {
                p.info.material = Some(m);
            }
        }
        let refs = |key: &str| -> HintSet {
            let r = &level.session[key];
            let one = |k: &str| ref_index(&objects, &r[k]);
            let mut h = HintSet {
                wasd: r["wasd"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|j| ref_index(&objects, j)).collect())
                    .unwrap_or_default(),
                indicator: one("keyboardIndicator"),
                hold_to_shoot: one("holdToShootText"),
                hold_to_grenade: one("holdToGrenadeText"),
                click_to_dash: one("clickToDashText"),
                ..HintSet::default()
            };
            // The world-space canvas holding the three texts.
            let canvas_obj = h
                .hold_to_shoot
                .and_then(|t| objects[t].parent)
                .filter(|&c| objects[c].component("Canvas").is_some());
            if let Some(ci) = canvas_obj {
                let mut c = Canvas::from_objects(&data, &objects, ci);
                for n in &mut c.nodes {
                    n.active = true;
                }
                let names = [h.hold_to_shoot, h.hold_to_grenade, h.click_to_dash];
                let mut nodes = [None; 3];
                for (k, t) in names.iter().enumerate() {
                    if let Some(t) = t {
                        nodes[k] = c.find_in(0, &objects[*t].name);
                    }
                }
                // A world canvas sits at its anchored position under its
                // parent (no parent rect), scaled like its transform.
                let o = &objects[ci];
                let parent = o.parent.map_or(Mat4::IDENTITY, |p| world_m[p]);
                let ap = o.rect.as_ref().map_or([0.0, 0.0], |r| r.anchored_position);
                let local = Mat4::from_scale_rotation_translation(
                    Vec3::from(o.local.scale),
                    Quat::IDENTITY,
                    Vec3::new(ap[0], ap[1], o.local.position[2]),
                );
                h.canvas_world = parent * local;
                h.nodes = nodes;
                h.canvas = Some(c);
            }
            h
        };
        let keyboard = refs("textReferences_keyboard");
        let gamepad = refs("textReferences_gamePad");
        // The level presentation without the objects drawn here.
        let mut hidden: Vec<usize> = Vec::new();
        for i in def
            .arrows
            .iter()
            .map(|a| a.object)
            .chain(def.pickups.iter().map(|p| p.obj.object))
            .chain(def.dummies.iter().map(|d| d.obj.object))
        {
            hidden.extend(subtree(&objects, i));
        }
        for key in ["textReferences_keyboard", "textReferences_gamePad"] {
            if let Some(p) = ref_index(&objects, &level.session[key]["parent"]) {
                hidden.extend(subtree(&objects, p));
            }
        }
        // The pre-placed player (the sim spawns it; the presenter draws it)
        // with its indicators, and the player's ability objects.
        for (i, o) in objects.iter().enumerate() {
            if o.component("SlimeController").is_some() || o.name == "AbilityList" {
                let root = o
                    .parent
                    .filter(|_| o.component("SlimeController").is_some());
                hidden.extend(subtree(&objects, root.unwrap_or(i)));
            }
        }
        let mut shown = level.clone();
        for i in hidden {
            shown.objects[i].active = false;
            shown.objects[i].active_in_hierarchy = false;
        }
        let mut pres = MatchPresenter::new(shared.visuals.clone(), SCENE, 1)?;
        pres.level_view = LevelView::new(&data, shown);
        pres.screen_shake = shared.settings.screen_shake;
        pres.camera_zoom = shared.settings.camera_zoom;
        // The tutorial's `PlayerAverageCamera` is disabled: the camera stays
        // centred (tutorial.md 1).
        pres.camera_follow = false;
        // The dummies die in their own slime material (orange): the player
        // colour with that material.
        let base = |n: &str| n.trim_end_matches(" 1").to_string();
        pres.orphan_effect_color = dummy_pieces
            .iter()
            .flatten()
            .find_map(|p| p.info.material_name.clone())
            .and_then(|n: String| {
                shared.visuals.player_materials.iter().position(|id| {
                    data.materials
                        .get(id)
                        .is_some_and(|m| base(&m.name) == base(&n))
                })
            })
            .map_or(-1, |i| i as i32);
        let view = world.view();
        shared.sounds.match_started(audio);
        Ok(Self {
            world,
            pres,
            l10n: L10n::new(&data, shared.settings.language),
            objects,
            world_m,
            arrow_pieces,
            pickup_pieces,
            dummy_pieces,
            keyboard,
            gamepad,
            view,
            last_world: WorldView::default(),
            size: (1920, 1080),
            device,
            time: 0.0,
            ticks: 0,
            logged: (0, [false; 3]),
        })
    }

    /// Device switching (spec section 8): a controller pressing a face
    /// button, Start or Select takes the player; on a controller, any key
    /// or mouse button gives it back to the keyboard.
    fn switch_device(&mut self, devices: &[(DeviceId, InputState)], raw: &[RawPress]) {
        let pad_buttons = Buttons::JUMP
            | Buttons::ABILITY0
            | Buttons::ABILITY1
            | Buttons::ABILITY2
            | Buttons::START
            | Buttons::SELECT
            | Buttons::ACCEPT
            | Buttons::BACK;
        for (d, st) in devices {
            if matches!(d, DeviceId::Controller(_))
                && *d != self.device
                && st.held.0 & pad_buttons.0 != 0
            {
                self.device = *d;
                return;
            }
        }
        if matches!(self.device, DeviceId::Controller(_)) {
            let keys = raw.iter().any(|r| r.device == DeviceId::KeyboardMouse)
                || devices
                    .iter()
                    .any(|(d, st)| *d == DeviceId::KeyboardMouse && st.held.0 != 0);
            if keys {
                self.device = DeviceId::KeyboardMouse;
            }
        }
    }

    pub fn tick(
        &mut self,
        shared: &mut Shared,
        devices: &[(DeviceId, InputState)],
        raw: &[RawPress],
        audio: &mut Audio,
    ) -> TutorialOutcome {
        self.time += 1.0 / 60.0;
        self.ticks += 1;
        self.switch_device(devices, raw);
        let mouse = self.device == DeviceId::KeyboardMouse;
        let input = match devices.iter().find(|(d, _)| *d == self.device) {
            Some((_, st)) => {
                let aim = if mouse {
                    st.pointer.map(|px| {
                        let w = self.pres.screen_to_world(&self.last_world, px, self.size);
                        let p = self.view.players.first().map(|p| p.position);
                        w - p.map_or(Vec2::ZERO, |p| Vec2::new(p.x.to_f32(), p.y.to_f32()))
                    })
                } else {
                    None
                };
                device_input(st, mouse, aim)
            }
            None => PlayerInput::default(),
        };
        if let Some(p) = self.world.players.first_mut() {
            p.uses_mouse = mouse;
        }
        self.world.step(&[input]);
        self.view = self.world.view();
        // Locked slots show no indicator until their pickup unlocks them.
        if let Some(t) = self.world.tutorial.as_deref() {
            self.pres.locked_slots = t.unlocked.map(|u| !u);
        }
        let events = self.view.events.clone();
        self.pres.observe_events(
            &self.world,
            &self.view,
            &events,
            true,
            &mut shared.sounds,
            audio,
        );
        if self.ticks.is_multiple_of(10)
            && let Some(p) = self.view.players.first()
        {
            log::debug!(
                "tutorial: tick {} player at ({:.1}, {:.1}) device {:?}",
                self.ticks,
                p.position.x.to_f32(),
                p.position.y.to_f32(),
                self.device
            );
        }
        if let Some(t) = self.world.tutorial.as_deref() {
            let now = (t.absorbed, t.unlocked);
            if now != self.logged {
                log::info!(
                    "tutorial: tick {} progress {} unlocked {:?}",
                    self.ticks,
                    now.0,
                    now.1
                );
                self.logged = now;
            }
        }
        if self.world.tutorial.as_ref().is_some_and(|t| t.done) {
            return TutorialOutcome::Leave;
        }
        TutorialOutcome::Stay
    }

    fn hint_set(&self) -> &HintSet {
        if self.device == DeviceId::KeyboardMouse {
            &self.keyboard
        } else {
            &self.gamepad
        }
    }

    /// Sprites of the tutorial objects and the current hint set.
    fn sprites(&self, out: &mut Vec<SpriteDraw>, texts: &mut Vec<(Vec2, String)>) {
        let Some(st) = self.world.tutorial.as_deref() else {
            return;
        };
        // Arrow sprites bob up once after the level loads (their authored
        // curve, clamped after its last key).
        let bob = crate::match_view::eval_keys(&[(-0.022, 0.076), (0.529, 0.386)], self.time);
        let groups: [(&[bopl_sim::object::ObjId], &Vec<Vec<Piece>>); 3] = [
            (&st.arrows, &self.arrow_pieces),
            (&st.pickups, &self.pickup_pieces),
            (&st.dummies, &self.dummy_pieces),
        ];
        for (gi, (ids, ps)) in groups.into_iter().enumerate() {
            for (k, &id) in ids.iter().enumerate() {
                let Some(o) = self.world.objects.get(id) else {
                    continue;
                };
                if !o.active || o.destroyed {
                    continue;
                }
                let Some(p) = ps.get(k) else { continue };
                let z = p.first().map_or(0.0, |x| x.local.w_axis.z);
                let pos = Vec3::new(o.tf.position.x.to_f32(), o.tf.position.y.to_f32(), z);
                let rot = o.tf.rotation.to_f32();
                draw_pieces(out, p, pos, rot, if gi == 0 { bob } else { 0.0 });
            }
        }
        // The hint set: movement hint M, the device indicator.
        let set = self.hint_set();
        let mut show: Vec<usize> = Vec::new();
        if let Some(m) = st.movement_hint()
            && let Some(&h) = set.wasd.get(m)
        {
            show.push(h);
        }
        if st.hints.device_indicator
            && st.absorbed < 4
            && let Some(i) = set.indicator
        {
            show.push(i);
        }
        for root in show {
            for i in subtree(&self.objects, root) {
                let o = &self.objects[i];
                if i != root && !o.active {
                    continue;
                }
                if let Some(tm) = o.component("TextMesh") {
                    let t = tm.fields["m_Text"].as_str().unwrap_or("").to_string();
                    let p = self.world_m[i].w_axis;
                    texts.push((Vec2::new(p.x, p.y), t));
                }
                if let Some(info) = RendererInfo::of(o)
                    && info.enabled
                    && let Some(s) = info.sprite.clone()
                {
                    out.push(info.draw(&s, self.world_m[i]));
                }
            }
        }
    }

    pub fn draw(&mut self, shared: &mut Shared, ctx: &mut DrawContext<'_>) {
        self.size = ctx.size;
        self.pres.aspect = ctx.size.0 as f32 / ctx.size.1.max(1) as f32;
        let mut world = self.pres.world_view(
            &self.view,
            &mut crate::fx::Gfx {
                gpu: ctx.gpu,
                renderer: ctx.renderer,
                assets: &mut shared.assets,
            },
        );
        let mut texts = Vec::new();
        self.sprites(&mut world.sprites, &mut texts);
        let (frame, _targets) =
            self.pres
                .level_view
                .frame(&world, ctx.size, ctx.gpu, ctx.renderer, &mut shared.assets);
        let mut frame: Frame = frame;
        let quads =
            self.pres
                .indicator_quads(&self.view, ctx.gpu, ctx.renderer, &mut shared.assets);
        if let Some(main) = frame
            .passes
            .iter_mut()
            .rev()
            .find(|p| p.target == PassTarget::Output)
        {
            for q in quads {
                main.push(q);
            }
        }
        // Hints in screen space: the world letters and the world canvas.
        let mut pass = ui_pass("tutorial hints", ctx.size);
        let (w, h) = (ctx.size.0 as f32, ctx.size.1 as f32);
        let to_screen = |p: Vec2| {
            let s = self.pres.world_to_screen(&world, p, ctx.size);
            Vec2::new(s.x, h - s.y)
        };
        let px_per_unit = (to_screen(Vec2::new(0.0, 1.0)) - to_screen(Vec2::ZERO)).length();
        if let Some(font) = shared.fonts.get(
            ctx.gpu,
            ctx.renderer,
            &mut shared.assets,
            crate::level_scene::UI_FONT,
        ) {
            for (p, t) in &texts {
                pass.push(Text {
                    text: t.clone(),
                    font,
                    // TextMesh: 51 pt at character size 1 ≈ 4.2 units per em.
                    size: px_per_unit * 4.2,
                    position: to_screen(*p),
                    rotation: 0.0,
                    anchor: Vec2::new(0.5, 0.5),
                    color: Color::rgb(0.34, 0.34, 0.34),
                    outline: 0.0,
                    outline_color: Color::BLACK,
                    dilate: 0.0,
                    spacing: 0.0,
                    stencil: bopl_render::Stencil::None,
                });
            }
        }
        let st_hints = self.world.tutorial.as_ref().map(|t| t.hints);
        let ws = {
            // World → output pixels (y up) as an affine map.
            let o = to_screen(Vec2::ZERO);
            let ex = to_screen(Vec2::X) - o;
            let ey = to_screen(Vec2::Y) - o;
            Mat4::from_cols(
                ex.extend(0.0).extend(0.0),
                ey.extend(0.0).extend(0.0),
                Vec3::Z.extend(0.0),
                o.extend(0.0).extend(1.0),
            )
        };
        let device_kb = self.device == DeviceId::KeyboardMouse;
        let set = if device_kb {
            &mut self.keyboard
        } else {
            &mut self.gamepad
        };
        if let (Some(c), Some(hints)) = (set.canvas.as_mut(), st_hints) {
            let on = [
                hints.hold_to_shoot,
                hints.hold_to_grenade,
                hints.click_to_dash,
            ];
            for (k, n) in set.nodes.iter().enumerate() {
                c.set_active(*n, on[k]);
            }
            c.world_root = Some(ws * set.canvas_world);
            let mut d = UiDraw {
                gpu: ctx.gpu,
                renderer: ctx.renderer,
                assets: &mut shared.assets,
                fonts: &mut shared.fonts,
                l10n: &self.l10n,
                time: shared.menu_time,
            };
            c.draw(&mut d, ctx.size, &mut pass);
        }
        let _ = w;
        frame.passes.push(pass);
        self.last_world = world;
        *ctx.frame = frame;
    }
}

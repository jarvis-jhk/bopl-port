//! uGUI canvases from the import: RectTransform layout, `Image` and
//! `TextMeshProUGUI` drawing, `Mask` clipping, the canvas scaler, hit tests
//! and localisation. Screens (menus, character select, winner screen) load
//! the original's canvases and drive them by node name; the behaviour
//! scripts of the original are reimplemented per screen.
//!
//! Coordinates: a canvas renders in a pass whose world units are output
//! pixels with y up (origin bottom-left), so rotations, scales and the
//! sprites' texture orientation work as in the level passes.

use std::collections::HashMap;

use bopl_import::GameData;
use bopl_import::format::Object;
use bopl_render::{
    Blend, Camera, Color, FontId, Gpu, Pass, PassTarget, Projection, Quad, Renderer, Shading,
    Stencil, Text, UvRect, View,
};

use glam::{Mat4, Quat, Vec2, Vec3};
use serde_json::Value as J;

use crate::assets::Assets;

/// How an image is shaded.
#[derive(Clone, Debug, PartialEq)]
pub enum UiShade {
    Sprite,
    /// Custom/UI_CustomColor, Custom/CustomColourShader (slime colours).
    Channel {
        main: Color,
        shadow: Color,
    },
    Additive,
    /// Custom/repeater: the menus' moving circle pattern.
    Repeater {
        frequency: f32,
        speed: Vec2,
    },
    /// Not drawn (Steam frames).
    Hidden,
}

#[derive(Clone, Debug)]
pub struct UiImage {
    pub enabled: bool,
    pub sprite: Option<String>,
    pub color: Color,
    pub shade: UiShade,
    pub preserve_aspect: bool,
}

#[derive(Clone, Debug)]
pub struct UiText {
    pub enabled: bool,
    /// Authored text (the localisation key when `localized`).
    pub source: String,
    /// Text drawn (after localisation / controller changes).
    pub text: String,
    pub localized: bool,
    pub stroke: bool,
    /// Font asset name (`fonts.json`).
    pub font: Option<String>,
    pub size: f32,
    pub auto_size: Option<(f32, f32)>,
    pub color: Color,
    /// 0 left, 0.5 centre, 1 right.
    pub h_align: f32,
    /// 0 top, 0.5 middle, 1 bottom.
    pub v_align: f32,
    pub bold: bool,
    pub wrap: bool,
    pub outline: f32,
    pub outline_color: Color,
    /// The text has its own material (`m_sharedMaterial`); otherwise the
    /// font asset's material applies (outline of the stroke fonts).
    pub own_material: bool,
    /// TMP `m_characterSpacing` (em / 100).
    pub char_spacing: f32,
    /// TMP margins (left, top, right, bottom): positive shrink, negative
    /// grow the text area.
    pub margin: [f32; 4],
}

/// One RectTransform node.
#[derive(Clone, Debug)]
pub struct UiNode {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub active: bool,
    pub anchor_min: Vec2,
    pub anchor_max: Vec2,
    pub pos: Vec2,
    pub size: Vec2,
    pub pivot: Vec2,
    pub scale: Vec3,
    /// Radians around z.
    pub rotation: f32,
    pub image: Option<UiImage>,
    pub text: Option<UiText>,
    pub mask: bool,
    /// Controller overrides: alpha multiplier (inherited by children),
    /// extra scale and offset (animations).
    pub alpha: f32,
    pub scale_mul: f32,
    pub offset: Vec2,
}

/// A computed node: local → output-pixel matrix (y up) and the local rect.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub m: Mat4,
    pub min: Vec2,
    pub size: Vec2,
    pub visible: bool,
    pub alpha: f32,
}

/// A canvas: the subtree of a `Canvas` object.
#[derive(Clone, Debug)]
pub struct Canvas {
    pub nodes: Vec<UiNode>,
    pub reference: Vec2,
    /// `m_ScreenMatchMode`: 0 match width/height, 1 expand, 2 shrink.
    pub match_mode: i64,
    pub match_wh: f32,
    /// Constant pixel size instead of scaling (unused by the game).
    pub constant: bool,
    /// World-space canvases: the matrix from the root's local units to
    /// output pixels (y up); the root then keeps its own rect size.
    pub world_root: Option<Mat4>,
}

fn f(j: &J, k: &str, d: f32) -> f32 {
    j[k].as_f64().map_or(d, |v| v as f32)
}

fn color(j: &J) -> Color {
    Color::rgba(
        f(j, "r", 1.0),
        f(j, "g", 1.0),
        f(j, "b", 1.0),
        f(j, "a", 1.0),
    )
}

pub fn shade_of(data: &GameData, material: Option<&str>) -> UiShade {
    let Some(m) = material.and_then(|id| data.materials.get(id)) else {
        return UiShade::Sprite;
    };
    let c = |k: &str| {
        m.colors
            .get(k)
            .map_or(Color::WHITE, |c| Color::from_array(*c))
    };
    match m.shader.as_str() {
        "Custom/UI_CustomColor" | "Custom/CustomColourShader" => UiShade::Channel {
            main: c("_MainColor"),
            shadow: c("_ShadowColor"),
        },
        "Custom/AdditiveColoredA8" => UiShade::Additive,
        "Custom/repeater" => UiShade::Repeater {
            frequency: m.floats.get("_Frequency").copied().unwrap_or(13.0),
            speed: Vec2::new(
                m.floats.get("_speedX").copied().unwrap_or(0.15),
                m.floats.get("_speedY").copied().unwrap_or(0.15),
            ),
        },
        "Custom/SteamFrame" => UiShade::Hidden,
        _ => UiShade::Sprite,
    }
}

/// A node built from an imported object.
pub fn node_of(data: &GameData, o: &Object) -> UiNode {
    let r = o.rect.unwrap_or_default();
    let [qx, qy, qz, qw] = o.local.rotation;
    let q = Quat::from_xyzw(qx, qy, qz, qw);
    let rotation = if q.length_squared() > 1e-12 {
        let (axis, a) = q.normalize().to_axis_angle();
        if axis.z < 0.0 { -a } else { a }
    } else {
        0.0
    };
    let mut image = None;
    let mut text = None;
    let mut mask = false;
    let localized = o.component("LocalizedText");
    for c in &o.components {
        let fl = &c.fields;
        match c.script.as_deref() {
            Some("UnityEngine.UI.Image") => {
                image = Some(UiImage {
                    enabled: c.enabled != Some(false),
                    sprite: fl["m_Sprite"]["$ref"].as_str().map(str::to_string),
                    color: color(&fl["m_Color"]),
                    shade: shade_of(data, fl["m_Material"]["$ref"].as_str()),
                    preserve_aspect: fl["m_PreserveAspect"].as_i64() == Some(1),
                });
            }
            Some("UnityEngine.UI.Mask") => mask = c.enabled != Some(false),
            Some("TMPro.TextMeshProUGUI") => {
                let font_id = fl["m_fontAsset"]["$ref"].as_str();
                let font = font_id
                    .and_then(|id| data.fonts.iter().find(|x| x.id == id))
                    .map(|x| x.name.clone());
                let mat = fl["m_sharedMaterial"]["$ref"]
                    .as_str()
                    .and_then(|id| data.materials.get(id));
                let outline = mat
                    .and_then(|m| m.floats.get("_OutlineWidth").copied())
                    .unwrap_or(0.0);
                let outline_color = mat
                    .and_then(|m| m.colors.get("_OutlineColor"))
                    .map_or(Color::BLACK, |c| Color::from_array(*c));
                // TMP converts a legacy `m_textAlignment` (not yet
                // `Converted` = 65535) into the split fields when it loads.
                let (h, v) = match fl["m_textAlignment"].as_i64() {
                    Some(a) if a != 65535 => (a & 0xff, a & 0xff00),
                    _ => (
                        fl["m_HorizontalAlignment"].as_i64().unwrap_or(1),
                        fl["m_VerticalAlignment"].as_i64().unwrap_or(256),
                    ),
                };
                let src = fl["m_text"].as_str().unwrap_or("").to_string();
                let stroke = localized
                    .is_some_and(|l| l.fields["useFontWithStroke"].as_i64() == Some(1))
                    || font.as_deref().is_some_and(|n| n.starts_with("stroke"));
                text = Some(UiText {
                    enabled: c.enabled != Some(false),
                    text: src.clone(),
                    source: src,
                    localized: localized.is_some()
                        && localized.is_some_and(|l| l.enabled != Some(false)),
                    stroke,
                    font,
                    size: f(fl, "m_fontSize", 36.0),
                    auto_size: (fl["m_enableAutoSizing"].as_i64() == Some(1))
                        .then(|| (f(fl, "m_fontSizeMin", 18.0), f(fl, "m_fontSizeMax", 72.0))),
                    color: color(&fl["m_fontColor"]),
                    h_align: match h {
                        1 => 0.0,
                        4 => 1.0,
                        _ => 0.5,
                    },
                    v_align: match v {
                        256 => 0.0,
                        1024 | 2048 => 1.0,
                        _ => 0.5,
                    },
                    bold: fl["m_fontStyle"].as_i64().is_some_and(|s| s & 1 != 0),
                    wrap: fl["m_enableWordWrapping"].as_i64() == Some(1),
                    outline,
                    outline_color,
                    own_material: mat.is_some(),
                    char_spacing: f(fl, "m_characterSpacing", 0.0),
                    margin: [
                        f(&fl["m_margin"], "x", 0.0),
                        f(&fl["m_margin"], "y", 0.0),
                        f(&fl["m_margin"], "z", 0.0),
                        f(&fl["m_margin"], "w", 0.0),
                    ],
                });
            }
            _ => {}
        }
    }
    UiNode {
        name: o.name.clone(),
        parent: None,
        children: Vec::new(),
        active: o.active,
        anchor_min: Vec2::from(r.anchor_min),
        anchor_max: Vec2::from(r.anchor_max),
        pos: Vec2::from(r.anchored_position),
        size: Vec2::from(r.size_delta),
        pivot: Vec2::from(r.pivot),
        scale: Vec3::from(o.local.scale),
        rotation,
        image,
        text,
        mask,
        alpha: 1.0,
        scale_mul: 1.0,
        offset: Vec2::ZERO,
    }
}

impl Canvas {
    /// The canvas rooted at `objects[root]` (a `Canvas` object) or any
    /// subtree (then laid out in a reference-sized root).
    pub fn from_objects(data: &GameData, objects: &[Object], root: usize) -> Canvas {
        let mut nodes: Vec<UiNode> = Vec::new();
        let mut map: HashMap<usize, usize> = HashMap::new();
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            let o = &objects[i];
            let mut n = node_of(data, o);
            n.parent = o.parent.and_then(|p| map.get(&p).copied());
            let id = nodes.len();
            if let Some(p) = n.parent {
                nodes[p].children.push(id);
            }
            nodes.push(n);
            map.insert(i, id);
            for &c in o.children.iter().rev() {
                stack.push(c);
            }
        }
        // Children were pushed in reverse; the DFS above visits them in
        // order, so `children` lists are already in sibling order.
        let scaler = objects[root].component("UnityEngine.UI.CanvasScaler");
        let (reference, match_mode, match_wh, constant) = match scaler {
            Some(c) => {
                let fl = &c.fields;
                (
                    Vec2::new(
                        f(&fl["m_ReferenceResolution"], "x", 4096.0),
                        f(&fl["m_ReferenceResolution"], "y", 2160.0),
                    ),
                    fl["m_ScreenMatchMode"].as_i64().unwrap_or(1),
                    f(fl, "m_MatchWidthOrHeight", 0.0),
                    fl["m_UiScaleMode"].as_i64() == Some(0),
                )
            }
            None => (Vec2::new(4096.0, 2160.0), 1, 0.0, false),
        };
        // The root behaves like a screen-filling canvas.
        if let Some(r) = nodes.first_mut() {
            r.anchor_min = Vec2::ZERO;
            r.anchor_max = Vec2::ZERO;
            r.pos = Vec2::ZERO;
            r.pivot = Vec2::splat(0.5);
            r.scale = Vec3::ONE;
            r.rotation = 0.0;
            r.active = true;
        }
        Canvas {
            nodes,
            reference,
            match_mode,
            match_wh,
            constant,
            world_root: None,
        }
    }

    /// A canvas from a prefab (its root object laid out under a screen
    /// root of the given reference size).
    pub fn from_prefab(data: &GameData, prefab: &str, reference: Vec2) -> Option<Canvas> {
        let p = data.prefab(prefab).ok()?;
        let mut c = Canvas {
            nodes: vec![UiNode {
                name: "root".into(),
                parent: None,
                children: Vec::new(),
                active: true,
                anchor_min: Vec2::ZERO,
                anchor_max: Vec2::ZERO,
                pos: Vec2::ZERO,
                size: Vec2::ZERO,
                pivot: Vec2::splat(0.5),
                scale: Vec3::ONE,
                rotation: 0.0,
                image: None,
                text: None,
                mask: false,
                alpha: 1.0,
                scale_mul: 1.0,
                offset: Vec2::ZERO,
            }],
            reference,
            match_mode: 1,
            match_wh: 0.0,
            constant: false,
            world_root: None,
        };
        c.graft(data, &p.objects, 0, 0);
        Some(c)
    }

    /// Adds the subtree `objects[i]` under node `parent`; returns its id.
    pub fn graft(&mut self, data: &GameData, objects: &[Object], i: usize, parent: usize) -> usize {
        let mut n = node_of(data, &objects[i]);
        n.parent = Some(parent);
        let id = self.nodes.len();
        self.nodes.push(n);
        self.nodes[parent].children.push(id);
        for &c in &objects[i].children {
            self.graft(data, objects, c, id);
        }
        id
    }

    /// Copies node `src` (with its subtree) under `parent`.
    pub fn clone_subtree(&mut self, src: usize, parent: usize) -> usize {
        let mut n = self.nodes[src].clone();
        n.parent = Some(parent);
        n.children = Vec::new();
        let id = self.nodes.len();
        self.nodes.push(n);
        self.nodes[parent].children.push(id);
        let kids = self.nodes[src].children.clone();
        for c in kids {
            self.clone_subtree(c, id);
        }
        id
    }

    /// First node named `name` in the subtree of `from` (depth first).
    pub fn find_in(&self, from: usize, name: &str) -> Option<usize> {
        let mut stack = vec![from];
        while let Some(i) = stack.pop() {
            if self.nodes[i].name == name && i != from {
                return Some(i);
            }
            for &c in self.nodes[i].children.iter().rev() {
                stack.push(c);
            }
        }
        None
    }

    /// Node by a `/`-separated path of names from the root (each step a
    /// descendant search).
    pub fn find(&self, path: &str) -> Option<usize> {
        let mut at = 0;
        for part in path.split('/') {
            at = self.find_in(at, part)?;
        }
        Some(at)
    }

    /// Direct child by name.
    pub fn child(&self, parent: usize, name: &str) -> Option<usize> {
        self.nodes[parent]
            .children
            .iter()
            .copied()
            .find(|&c| self.nodes[c].name == name)
    }

    pub fn set_active(&mut self, n: Option<usize>, on: bool) {
        if let Some(n) = n {
            self.nodes[n].active = on;
        }
    }

    pub fn set_text(&mut self, n: Option<usize>, text: &str) {
        if let Some(t) = n.and_then(|n| self.nodes[n].text.as_mut()) {
            t.text = text.to_string();
            t.localized = false;
        }
    }

    pub fn set_sprite(&mut self, n: Option<usize>, sprite: Option<&str>) {
        if let Some(i) = n.and_then(|n| self.nodes[n].image.as_mut()) {
            i.sprite = sprite.map(str::to_string);
        }
    }

    pub fn set_color(&mut self, n: Option<usize>, c: Color) {
        if let Some(n) = n {
            if let Some(i) = self.nodes[n].image.as_mut() {
                i.color = c;
            } else if let Some(t) = self.nodes[n].text.as_mut() {
                t.color = c;
            }
        }
    }

    /// Canvas scale factor for an output of `screen` pixels.
    pub fn scale_factor(&self, screen: Vec2) -> f32 {
        if self.constant {
            return 1.0;
        }
        let (sx, sy) = (screen.x / self.reference.x, screen.y / self.reference.y);
        match self.match_mode {
            0 => (sx.log2() + (sy.log2() - sx.log2()) * self.match_wh).exp2(),
            2 => sx.max(sy),
            _ => sx.min(sy),
        }
    }

    /// Lays out every node for an output of `screen` pixels.
    pub fn layout(&self, screen: Vec2) -> Vec<Placed> {
        let s = self.scale_factor(screen);
        let mut out = vec![
            Placed {
                m: Mat4::IDENTITY,
                min: Vec2::ZERO,
                size: Vec2::ZERO,
                visible: false,
                alpha: 1.0,
            };
            self.nodes.len()
        ];
        // Nodes are stored parents first.
        for (i, n) in self.nodes.iter().enumerate() {
            match n.parent {
                None => {
                    if let Some(m) = self.world_root {
                        out[i] = Placed {
                            m,
                            min: -n.size * 0.5,
                            size: n.size,
                            visible: n.active,
                            alpha: n.alpha,
                        };
                        continue;
                    }
                    let size = screen / s;
                    out[i] = Placed {
                        m: Mat4::from_translation((screen * 0.5).extend(0.0))
                            * Mat4::from_scale(Vec3::new(s, s, 1.0)),
                        min: -size * 0.5,
                        size,
                        visible: n.active,
                        alpha: n.alpha,
                    };
                }
                Some(p) => {
                    let pp = out[p];
                    let amin = pp.min + pp.size * n.anchor_min;
                    let amax = pp.min + pp.size * n.anchor_max;
                    let reference = amin + (amax - amin) * n.pivot;
                    let local = reference + n.pos + n.offset;
                    let size = (amax - amin) + n.size;
                    let sc = n.scale * n.scale_mul;
                    let m = pp.m
                        * Mat4::from_scale_rotation_translation(
                            Vec3::new(sc.x, sc.y, 1.0),
                            Quat::from_rotation_z(n.rotation),
                            local.extend(0.0),
                        );
                    out[i] = Placed {
                        m,
                        min: -size * n.pivot,
                        size,
                        visible: pp.visible && n.active,
                        alpha: pp.alpha * n.alpha,
                    };
                }
            }
        }
        out
    }

    /// Whether the output pixel `p` (top-left origin) is inside node `n`.
    pub fn hit(&self, placed: &[Placed], n: usize, p: Vec2, screen: Vec2) -> bool {
        let pl = placed[n];
        if !pl.visible {
            return false;
        }
        let q =
            pl.m.inverse()
                .transform_point3(Vec3::new(p.x, screen.y - p.y, 0.0))
                .truncate();
        q.x >= pl.min.x
            && q.y >= pl.min.y
            && q.x <= pl.min.x + pl.size.x
            && q.y <= pl.min.y + pl.size.y
    }

    /// Centre of node `n` in output pixels (top-left origin).
    #[allow(dead_code)]
    pub fn center(&self, placed: &[Placed], n: usize, screen: Vec2) -> Vec2 {
        let pl = placed[n];
        let c = pl.m.transform_point3((pl.min + pl.size * 0.5).extend(0.0));
        Vec2::new(c.x, screen.y - c.y)
    }
}

/// Draw context for canvases.
pub struct UiDraw<'a> {
    pub gpu: &'a Gpu,
    pub renderer: &'a mut Renderer,
    pub assets: &'a mut Assets,
    pub fonts: &'a mut crate::shared::FontCache,
    pub l10n: &'a L10n,
    /// Seconds (animated shaders).
    pub time: f32,
}

/// A pass for canvases: output pixels, y up.
pub fn ui_pass(label: &str, screen: (u32, u32)) -> Pass {
    let (w, h) = (screen.0 as f32, screen.1 as f32);
    Pass::new(
        label,
        PassTarget::Output,
        View::Camera(Camera {
            position: Vec3::new(w * 0.5, h * 0.5, -100.0),
            rotation: Quat::IDENTITY,
            projection: Projection::Orthographic { size: h * 0.5 },
            near: 0.1,
            far: 1000.0,
            aspect: None,
        }),
    )
}

/// Word-wraps `text` to `width` at `size`.
fn wrap_text(renderer: &mut Renderer, font: FontId, text: &str, size: f32, width: f32) -> String {
    let mut out = String::new();
    for (li, para) in text.split('\n').enumerate() {
        if li > 0 {
            out.push('\n');
        }
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && renderer.measure_text(font, &candidate, size).x > width {
                out.push_str(&line);
                out.push('\n');
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        out.push_str(&line);
    }
    out
}

impl Canvas {
    /// Draws the canvas into `pass` (built with [`ui_pass`]).
    pub fn draw(&self, d: &mut UiDraw<'_>, screen: (u32, u32), pass: &mut Pass) {
        let placed = self.layout(Vec2::new(screen.0 as f32, screen.1 as f32));
        self.draw_node(d, &placed, 0, pass, screen);
    }

    fn draw_node(
        &self,
        d: &mut UiDraw<'_>,
        placed: &[Placed],
        i: usize,
        pass: &mut Pass,
        screen: (u32, u32),
    ) {
        let n = &self.nodes[i];
        let pl = placed[i];
        if !pl.visible || pl.alpha <= 0.001 {
            return;
        }
        let mut mask_quad = None;
        if let Some(img) = &n.image
            && img.enabled
            && let Some(q) = self.image_quads(d, n, img, &pl, pass, screen)
            && n.mask
        {
            mask_quad = Some(q);
        }
        if let Some(t) = &n.text
            && t.enabled
        {
            self.draw_text(d, t, &pl, pass);
        }
        if let Some(q) = mask_quad {
            pass.push(Quad {
                shading: Shading::Mask { cutoff: 0.01 },
                stencil: Stencil::MaskAdd,
                ..q
            });
        }
        let start = pass.items.len();
        for &c in &n.children {
            self.draw_node(d, placed, c, pass, screen);
        }
        if let Some(q) = mask_quad {
            for it in &mut pass.items[start..] {
                match it {
                    bopl_render::Item::Quad(q) if q.stencil == Stencil::None => {
                        q.stencil = Stencil::VisibleInside
                    }
                    bopl_render::Item::Text(t) if t.stencil == Stencil::None => {
                        t.stencil = Stencil::VisibleInside
                    }
                    _ => {}
                }
            }
            pass.push(Quad {
                shading: Shading::Mask { cutoff: 0.01 },
                stencil: Stencil::MaskRemove,
                ..q
            });
        }
    }

    /// Draws an image; returns its quad (for masks).
    fn image_quads(
        &self,
        d: &mut UiDraw<'_>,
        _n: &UiNode,
        img: &UiImage,
        pl: &Placed,
        pass: &mut Pass,
        screen: (u32, u32),
    ) -> Option<Quad> {
        let mut color = img.color;
        color.a *= pl.alpha;
        let mut min = pl.min;
        let mut size = pl.size;
        let geom = img.sprite.as_deref().and_then(|s| {
            Some((
                d.assets.sprite(d.gpu, d.renderer, s)?,
                d.assets.data.sprites.get(s)?.clone(),
            ))
        });
        if let UiShade::Repeater { frequency, speed } = &img.shade {
            // Square cells `height / frequency` big, scrolling diagonally.
            let (g, _) = geom?;
            let world_h = pl.m.y_axis.truncate().length() * pl.size.y;
            let cell = (world_h / frequency).max(4.0);
            let off = *speed * d.time * cell;
            let (w, h) = (screen.0 as f32, screen.1 as f32);
            let x0 = (off.x % cell) - cell;
            let y0 = (off.y % cell) - cell;
            let circle = cell * (g.size.x / g.rect_size.x.max(1e-6));
            let mut y = y0;
            while y < h + cell {
                let mut x = x0;
                while x < w + cell {
                    pass.push(Quad {
                        texture: g.texture,
                        uv: g.uv,
                        transform: Mat4::from_translation(Vec3::new(
                            x + cell * 0.5,
                            y + cell * 0.5,
                            0.0,
                        )),
                        size: Vec2::splat(circle),
                        pivot: Vec2::splat(0.5),
                        color,
                        ..Default::default()
                    });
                    x += cell;
                }
                y += cell;
            }
            return None;
        }
        if img.shade == UiShade::Hidden {
            return None;
        }
        let (texture, uv) = match &geom {
            Some((g, sp)) => {
                // Simple image: the sprite's full rect fills the node rect;
                // the trimmed texture rect sits inside it.
                let rect = Vec2::new(sp.rect[2], sp.rect[3]).max(Vec2::splat(1e-3));
                if img.preserve_aspect {
                    let a = rect.x / rect.y;
                    let fit = if size.x / size.y.max(1e-6) > a {
                        Vec2::new(size.y * a, size.y)
                    } else {
                        Vec2::new(size.x, size.x / a)
                    };
                    min += (size - fit) * _n.pivot;
                    size = fit;
                }
                let off = Vec2::new(sp.texture_rect_offset[0], sp.texture_rect_offset[1]) / rect;
                let tr = Vec2::new(sp.texture_rect[2], sp.texture_rect[3]) / rect;
                min += size * off;
                size *= tr;
                (g.texture, g.uv)
            }
            None if img.sprite.is_none() => (bopl_render::TextureId::WHITE, UvRect::FULL),
            None => return None,
        };
        let mut q = Quad {
            texture,
            uv,
            transform: pl.m * Mat4::from_translation(min.extend(0.0)),
            size,
            pivot: Vec2::ZERO,
            color,
            ..Default::default()
        };
        match &img.shade {
            UiShade::Channel { main, shadow } => {
                q.shading = Shading::ChannelColour {
                    main: *main,
                    shadow: *shadow,
                };
            }
            UiShade::Additive => {
                q.shading = Shading::AlphaTint;
                q.blend = Blend::Additive;
            }
            _ => {}
        }
        if color.a > 0.001 {
            pass.push(q);
        }
        Some(q)
    }

    fn draw_text(&self, d: &mut UiDraw<'_>, t: &UiText, pl: &Placed, pass: &mut Pass) {
        let text = if t.localized {
            d.l10n.get(&t.source)
        } else {
            t.text.clone()
        };
        if text.trim().is_empty() {
            return;
        }
        let font_name = d.l10n.font_for(t.font.as_deref(), t.stroke);
        let Some(font) = d.fonts.get(d.gpu, d.renderer, d.assets, &font_name) else {
            return;
        };
        // The font asset's spacing (TMP: normal spacing offset, bold
        // spacing) and, without a material of the text's own, its material.
        let asset = d.assets.data.fonts.iter().find(|x| x.name == font_name);
        let fnum = |k: &str| asset.and_then(|a| a.fields[k].as_f64()).unwrap_or(0.0) as f32;
        let spacing = (fnum("normalSpacingOffset")
            + t.char_spacing
            + if t.bold { fnum("boldSpacing") } else { 0.0 })
            / 100.0;
        let (outline, outline_color) = if t.own_material {
            (t.outline, t.outline_color)
        } else {
            let mat = asset
                .and_then(|a| a.fields["material"]["$ref"].as_str())
                .and_then(|id| d.assets.data.materials.get(id));
            (
                mat.and_then(|m| m.floats.get("_OutlineWidth").copied())
                    .unwrap_or(t.outline),
                mat.and_then(|m| m.colors.get("_OutlineColor"))
                    .map_or(t.outline_color, |c| Color::from_array(*c)),
            )
        };
        let scale = pl.m.x_axis.truncate().length();
        let rot = pl.m.x_axis.y.atan2(pl.m.x_axis.x);
        // The text area: the rect with TMP's margins.
        let [ml, mt, mr, mb] = t.margin;
        let pl = &Placed {
            min: pl.min + Vec2::new(ml, mb),
            size: pl.size - Vec2::new(ml + mr, mt + mb),
            ..*pl
        };
        let flip_y = pl.m.y_axis.y * pl.m.x_axis.x < 0.0;
        let mut size = t.size;
        let mut s = text.clone();
        let fits = |r: &mut Renderer, s: &str, size: f32| {
            let m = r.measure_text_spaced(font, s, size, spacing);
            m.x <= pl.size.x * 1.02 && m.y <= pl.size.y * 1.15
        };
        if let Some((lo, hi)) = t.auto_size {
            let (mut lo, mut hi) = (lo, hi);
            size = hi;
            let wrapped = |r: &mut Renderer, sz: f32| {
                if t.wrap {
                    wrap_text(r, font, &text, sz, pl.size.x)
                } else {
                    text.clone()
                }
            };
            let w = wrapped(d.renderer, hi);
            if !fits(d.renderer, &w, hi) {
                for _ in 0..12 {
                    let mid = (lo + hi) * 0.5;
                    let w = wrapped(d.renderer, mid);
                    if fits(d.renderer, &w, mid) {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                size = lo;
            }
            s = wrapped(d.renderer, size);
        } else if t.wrap && pl.size.x > 1.0 {
            s = wrap_text(d.renderer, font, &text, size, pl.size.x);
        }
        let anchor = Vec2::new(t.h_align, t.v_align);
        // Point of the rect the anchor sits on (local, y up).
        let p = Vec2::new(
            pl.min.x + pl.size.x * t.h_align,
            pl.min.y + pl.size.y * (1.0 - t.v_align),
        );
        let wp = pl.m.transform_point3(p.extend(0.0)).truncate();
        let mut color = t.color;
        color.a *= pl.alpha;
        pass.push(Text {
            text: s,
            font,
            size: size * scale,
            position: wp,
            rotation: if flip_y {
                rot + std::f32::consts::PI
            } else {
                rot
            },
            anchor,
            color,
            outline,
            outline_color,
            dilate: if t.bold { 0.19 } else { 0.0 },
            spacing,
            stencil: Stencil::None,
        });
    }
}

/// Localisation (ui.md 2): the table and per-language fonts.
#[derive(Clone, Debug, Default)]
pub struct L10n {
    /// Language code of the table (`en`, `de`, ...).
    pub lang: String,
    en: Vec<String>,
    target: Vec<String>,
    /// Font asset names: (normal, stroke) for the current language.
    fonts: (String, String),
}

/// The 14 languages in the original's menu order and their table codes.
pub const LANGUAGES: [(&str, &str); 14] = [
    ("en", "en"),
    ("de", "de"),
    ("es", "es"),
    ("fr", "fr"),
    ("it", "it"),
    ("ja", "jp"),
    ("ko", "ko"),
    ("pl", "pl"),
    ("pt-BR", "ptbr"),
    ("ru", "ru"),
    ("se", "se"),
    ("tr", "tr"),
    ("zh-CN", "zhcn"),
    ("zh-TW", "zhtw"),
];

impl L10n {
    pub fn new(data: &GameData, language: usize) -> Self {
        let code = LANGUAGES.get(language).map_or("en", |l| l.1).to_string();
        let loc = &data.localization;
        let en = loc.strings.get("en").cloned().unwrap_or_default();
        let target = loc.strings.get(&code).cloned().unwrap_or_default();
        let font = |key: &str| {
            loc.fonts
                .get(key)
                .and_then(|id| id.as_ref())
                .and_then(|id| data.fonts.iter().find(|f| &f.id == id))
                .map(|f| f.name.clone())
        };
        let (normal, stroke) = match code.as_str() {
            "jp" => ("jpFontAsset", "jpFontAsset_stroke"),
            "ko" | "zhcn" | "zhtw" => ("chinese_koreanAsset", "chinese_koreanAsset_stroke"),
            "ru" => ("russian_Asset", "russian_Asset_stroke"),
            "pl" | "tr" | "ptbr" => ("plFontAsset", "plFontAsset_stroke"),
            _ => ("enFontAsset", "enFontAsset_stroke"),
        };
        let fonts = (
            font(normal).unwrap_or_else(|| crate::level_scene::UI_FONT.into()),
            font(stroke).unwrap_or_else(|| "stroke_en_se_de_fr_it_es".into()),
        );
        Self {
            lang: code,
            en,
            target,
            fonts,
        }
    }

    /// The text for an English key (any case); English shows the key in
    /// lower case (the fonts draw capitals).
    pub fn get(&self, key: &str) -> String {
        let k = key.to_lowercase();
        if self.lang == "en" {
            return k;
        }
        self.en
            .iter()
            .position(|s| s.to_lowercase() == k)
            .and_then(|i| self.target.get(i))
            .cloned()
            .unwrap_or(k)
    }

    /// The font for a text: the language's (stroke) font when the text uses
    /// the English font family, else its own.
    pub fn font_for(&self, own: Option<&str>, stroke: bool) -> String {
        match own {
            Some(n) if !(n.starts_with("en_") || n.starts_with("stroke_en") || n.is_empty()) => {
                n.to_string()
            }
            _ => {
                if stroke {
                    self.fonts.1.clone()
                } else {
                    self.fonts.0.clone()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(
        name: &str,
        amin: [f32; 2],
        amax: [f32; 2],
        pos: [f32; 2],
        size: [f32; 2],
        pivot: [f32; 2],
    ) -> UiNode {
        UiNode {
            name: name.into(),
            parent: None,
            children: vec![],
            active: true,
            anchor_min: amin.into(),
            anchor_max: amax.into(),
            pos: pos.into(),
            size: size.into(),
            pivot: pivot.into(),
            scale: Vec3::ONE,
            rotation: 0.0,
            image: None,
            text: None,
            mask: false,
            alpha: 1.0,
            scale_mul: 1.0,
            offset: Vec2::ZERO,
        }
    }

    #[test]
    fn rect_transform_layout() {
        let mut c = Canvas {
            nodes: vec![node(
                "root", [0.0; 2], [0.0; 2], [0.0; 2], [0.0; 2], [0.5; 2],
            )],
            reference: Vec2::new(4096.0, 2160.0),
            match_mode: 1,
            match_wh: 0.0,
            constant: false,
            world_root: None,
        };
        // Stretched full-screen child and a bottom-right anchored box.
        let mut a = node(
            "full",
            [0.0, 0.0],
            [1.0, 1.0],
            [0.0, 0.0],
            [0.0, 0.0],
            [0.5, 0.5],
        );
        a.parent = Some(0);
        let mut b = node(
            "br",
            [1.0, 0.0],
            [1.0, 0.0],
            [-100.0, 50.0],
            [200.0, 100.0],
            [1.0, 0.0],
        );
        b.parent = Some(0);
        c.nodes.push(a);
        c.nodes.push(b);
        c.nodes[0].children = vec![1, 2];
        let screen = Vec2::new(2048.0, 1080.0);
        let p = c.layout(screen);
        // Expand: scale = min(0.5, 0.5) = 0.5.
        assert!((c.scale_factor(screen) - 0.5).abs() < 1e-6);
        let corner = |i: usize, f: Vec2| {
            p[i].m
                .transform_point3((p[i].min + p[i].size * f).extend(0.0))
                .truncate()
        };
        assert!((corner(1, Vec2::ZERO) - Vec2::ZERO).length() < 1e-3);
        assert!((corner(1, Vec2::ONE) - screen).length() < 1e-3);
        // br: pivot (bottom-right) at (W - 50, 25) in pixels.
        assert!((corner(2, Vec2::new(1.0, 0.0)) - Vec2::new(2048.0 - 50.0, 25.0)).length() < 1e-3);
        assert!(c.hit(&p, 2, Vec2::new(2048.0 - 60.0, 1080.0 - 30.0), screen));
        assert!(!c.hit(&p, 2, Vec2::new(10.0, 10.0), screen));
    }
}

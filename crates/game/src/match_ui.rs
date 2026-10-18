//! In-match overlays on the original's canvases (ui.md 5, 7, 8;
//! match-flow.md 11): the winner triangle with its "next level" / "ability
//! select" prompts, the ability-select circles with the mid-game wheel
//! (state from the simulation's flow), and the pause prompt.

use bopl_import::GameData;
use bopl_render::{Color, Frame, Gpu, Renderer};
use bopl_sim::World;
use bopl_sim::flow::{Circle, row_slot};
use bopl_sim::player::CauseOfDeath;
use bopl_sim::view::{Cue, View};
use glam::Vec2;

use crate::audio::Audio;
use crate::match_scene::Pause;
use crate::match_view::{MatchVisuals, eval_keys};
use crate::shared::Shared;
use crate::sound::Sounds;
use crate::ui::{Canvas, L10n, UiDraw, UiShade, shade_of, ui_pass};

/// A Mecanim clip curve (`animations.json`): cubic segments
/// `[time, a, b, c, d]`, value `a·dt³ + b·dt² + c·dt + d`.
#[derive(Clone, Debug, Default)]
pub struct ClipCurve {
    segments: Vec<[f32; 5]>,
}

impl ClipCurve {
    pub fn load(data: &GameData, clip: &str, property: &str) -> Self {
        let mut segments = Vec::new();
        if let Some(c) = data.animations.clips.values().find(|c| c.name == clip)
            && let Some(cv) = c.curves.iter().find(|cv| cv.property == property)
        {
            segments = cv.segments.clone();
        }
        Self { segments }
    }

    /// The time of the last key.
    pub fn end(&self) -> Option<f32> {
        self.segments.last().map(|s| s[0])
    }

    pub fn eval(&self, t: f32) -> Option<f32> {
        let first = self.segments.first()?;
        if t <= first[0] {
            return Some(first[4]);
        }
        let mut seg = first;
        for s in &self.segments {
            if s[0] <= t {
                seg = s;
            }
        }
        let dt = t - seg[0];
        Some(seg[1] * dt * dt * dt + seg[2] * dt * dt + seg[3] * dt + seg[4])
    }
}

/// The winner screen's state when it began to leave: the layout, each
/// prompt's time in its `nextLevel` clip (`None` = not shown yet; ability
/// select, next level) and the prompts' lift.
#[derive(Clone, Copy, Debug)]
struct ExitFrom {
    layout: usize,
    prompt_t: [Option<f32>; 2],
    lift: f32,
}

/// `CharacterStatsList.Transition`: the prompts are hidden `k ·
/// DelaySpacing / transitionSpeed` s after the press (ability select k = 1,
/// next level k = 2), each `nextLevel` clip playing back at speed 2; the
/// triangle plays `TriangleOut` at once.
const EXIT_PROMPT_DELAY: f32 = 0.3 / 3.0;

/// One ability-select circle (prefab `abSelCombined`).
#[derive(Clone, Debug, Default)]
struct CircleUi {
    root: usize,
    winner: Option<usize>,
    winner_circle: Option<usize>,
    winner_char: Option<usize>,
    winner_text: Option<usize>,
    loser: Option<usize>,
    wheel: Option<usize>,
    wheel_rows: [Option<usize>; 3],
    wheel_icons: [Option<usize>; 3],
    wheel_row_y: [f32; 3],
    arrow_l: Option<usize>,
    arrow_r: Option<usize>,
    up_arrow: Option<usize>,
    up_arrow_y: f32,
    loser_circle: Option<usize>,
    loser_char: Option<usize>,
    loser_text: Option<usize>,
    dots: Option<usize>,
}

pub struct MatchUi {
    data: std::rc::Rc<GameData>,
    visuals: std::rc::Rc<MatchVisuals>,
    l10n: L10n,
    winner: Canvas,
    triangles: [Option<usize>; 4],
    winner_text: [Option<usize>; 4],
    single_img: Option<usize>,
    draw_imgs: [Option<usize>; 4],
    team2_imgs: [Option<usize>; 2],
    team3_imgs: [Option<usize>; 3],
    /// Per layout: the image nodes in use (authored ones first, then
    /// copies for more winners than the original layout has) with their
    /// authored positions.
    layout_imgs: [Vec<(usize, Vec2)>; 4],
    next_prompt: Option<usize>,
    select_prompt: Option<usize>,
    stats_row: Option<usize>,
    circles: Canvas,
    circle_uis: Vec<CircleUi>,
    pause: Option<Canvas>,
    pause_v: Option<usize>,
    pause_x: Option<usize>,
    tri_in: ClipCurve,
    tri_out: ClipCurve,
    prompt_in: ClipCurve,
    /// The winner screen as last drawn and when (for its exit: the flow
    /// clears the screen a tick before the next level loads).
    last_shown: Option<(ExitFrom, f32)>,
    /// The winner screen's exit on "next level" (ui.md 8): it stays over
    /// the new level while it animates out.
    exit: Option<(ExitFrom, f32)>,
    /// Per player: the circle's x while shown, and since when it slides
    /// down after a level load (`AbilitySelectCircle.Disable`).
    circle_shown_x: Vec<Option<f32>>,
    circle_down: Vec<Option<(f32, f32)>>,
    /// Sprites of the portraits.
    loser_chars: Vec<String>,
    winner_chars: Vec<String>,
    special_chars: Vec<String>,
    time: f32,
    /// Per player: plays on another machine (online). Their open wheel is
    /// shown as three bouncing "busy" dots on the loser circle (ui.md 5),
    /// like the original, not as the wheel itself.
    pub remote: Vec<bool>,
}

fn refs(j: &serde_json::Value) -> Vec<String> {
    j.as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["$ref"].as_str().map(str::to_string))
        .collect()
}

impl MatchUi {
    pub fn new(shared: &mut Shared) -> Self {
        let data = shared.data.clone();
        let scene = data.scene(4).ok();
        let objects = scene.as_ref().map(|s| s.objects.as_slice()).unwrap_or(&[]);
        let winner_root = objects
            .iter()
            .position(|o| o.name.starts_with("WinnerTriangleCanvas"))
            .unwrap_or(0);
        let winner = Canvas::from_objects(&data, objects, winner_root);
        let tri_names = [
            "WinnerTriangle",
            "UndecidedWinnerTriangle",
            "2v2WinnerTriangle",
            "3v1WinnerTriangle",
        ];
        let triangles = tri_names.map(|n| winner.find(n));
        let winner_text = triangles.map(|t| {
            t.and_then(|t| {
                winner.nodes[t]
                    .children
                    .iter()
                    .copied()
                    .find(|&c| winner.nodes[c].name.starts_with("WinnerText"))
            })
        });
        let single_img = triangles[0].and_then(|t| winner.child(t, "Winner"));
        let draw_imgs = ["Winner1", "Winner2", "Winner3", "Winner4"]
            .map(|n| triangles[1].and_then(|t| winner.child(t, n)));
        let team2_imgs =
            ["Winner1", "Winner2"].map(|n| triangles[2].and_then(|t| winner.child(t, n)));
        let team3_imgs = ["Winner1", "Winner2", "Winner3"]
            .map(|n| triangles[3].and_then(|t| winner.child(t, n)));
        let next_prompt = winner.find("nextLevel");
        let select_prompt = winner.find("abilitySelect");
        let stats_row = winner
            .find("CharacterList")
            .and_then(|c| winner.child(c, "winner"));
        // The ability-select canvas and its circle prefab.
        let asc = objects.iter().position(|o| o.name == "AbilitySelectCanvas");
        let (circles, loser_chars, winner_chars, special_chars) = match asc {
            Some(i) => {
                let c = Canvas::from_objects(&data, objects, i);
                let h = objects[i]
                    .component("HandleAbilitySelectUI")
                    .map(|c| c.fields.clone());
                let get = |k: &str| h.as_ref().map(|h| refs(&h[k])).unwrap_or_default();
                (
                    c,
                    get("LoserCharacters"),
                    get("WinnerCharacters"),
                    get("SpecialCharacters"),
                )
            }
            None => (
                Canvas::from_prefab(&data, "", Vec2::new(2560.0, 1440.0)).unwrap_or_else(|| {
                    Canvas {
                        nodes: Vec::new(),
                        reference: Vec2::new(2560.0, 1440.0),
                        match_mode: 1,
                        match_wh: 0.0,
                        constant: false,
                        world_root: None,
                    }
                }),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        };
        // The pause prompt from the first level's ExitGameMenu.
        let mut pause = None;
        let (mut pause_v, mut pause_x) = (None, None);
        if let Some(&bi) = data.level_indices().first()
            && let Ok(l) = data.level(bi)
            && let Some(i) = l
                .objects
                .iter()
                .position(|o| o.name.starts_with("Canvas_leave"))
        {
            let c = Canvas::from_objects(&data, &l.objects, i);
            pause_v = c.find("V");
            pause_x = c.find("X");
            pause = Some(c);
        }
        Self {
            l10n: L10n::new(&data, shared.settings.language),
            tri_in: ClipCurve::load(&data, "WinnerTriangle", "m_AnchoredPosition.x"),
            tri_out: ClipCurve::load(&data, "TriangleOut", "m_AnchoredPosition.x"),
            prompt_in: ClipCurve::load(&data, "nextLevel", "m_AnchoredPosition.y"),
            last_shown: None,
            exit: None,
            circle_shown_x: Vec::new(),
            circle_down: Vec::new(),
            visuals: shared.visuals.clone(),
            data,
            winner,
            triangles,
            winner_text,
            single_img,
            draw_imgs,
            team2_imgs,
            team3_imgs,
            layout_imgs: Default::default(),
            next_prompt,
            select_prompt,
            stats_row,
            circles,
            circle_uis: Vec::new(),
            pause,
            pause_v,
            pause_x,
            loser_chars,
            winner_chars,
            special_chars,
            time: 0.0,
            remote: Vec::new(),
        }
    }

    /// A new level loaded: a winner screen that was up ("next level")
    /// animates out over it.
    pub fn level_started(&mut self) {
        let now = self.time;
        self.exit = self
            .last_shown
            .take()
            .filter(|&(_, at)| now - at < 0.5)
            .map(|(f, _)| (f, now));
    }

    /// The winner screen leaving (`TriangleOut`, prompts hiding); true
    /// while it is still to be drawn.
    fn apply_exit(&mut self) -> bool {
        let Some((from, start)) = self.exit else {
            return false;
        };
        let t = self.time - start;
        let tri_end = self.tri_out.end().unwrap_or(0.0);
        // The prompts' reverse clips (speed 2) end at most 0.5 s after
        // their delay.
        let prompts_end = 2.0 * EXIT_PROMPT_DELAY + 0.5;
        if t > tri_end.max(prompts_end) {
            self.exit = None;
            return false;
        }
        for (k, tri) in self.triangles.iter().enumerate() {
            self.winner.set_active(*tri, k == from.layout);
        }
        if let Some(tri) = self.triangles[from.layout]
            && let Some(x) = self.tri_out.eval(t)
        {
            self.winner.nodes[tri].pos.x = x;
        }
        for (k, node) in [self.select_prompt, self.next_prompt]
            .into_iter()
            .enumerate()
        {
            let Some(t_in) = from.prompt_t[k] else {
                self.winner.set_active(node, false);
                continue;
            };
            let back = (t - EXIT_PROMPT_DELAY * (k + 1) as f32).max(0.0);
            let ct = t_in.min(1.0) - 2.0 * back;
            self.winner.set_active(node, ct > 0.0);
            if let Some(nd) = node
                && let Some(y) = self.prompt_in.eval(ct.max(0.0))
            {
                self.winner.nodes[nd].pos.y = y + from.lift;
            }
        }
        // The banner lives under the celebration node: it stays active.
        let celebration = self.winner.child(0, "WinnerCelebration_leaveDisabled");
        self.winner.set_active(celebration, true);
        self.winner.set_active(self.stats_row, false);
        true
    }

    /// Which winner-screen prompt the output pixel `p` hits: `Some(true)`
    /// "next level", `Some(false)` "ability select".
    pub fn prompt_hit(&self, p: Vec2, size: (u32, u32)) -> Option<bool> {
        let screen = Vec2::new(size.0 as f32, size.1 as f32);
        let placed = self.winner.layout(screen);
        for (node, next) in [(self.next_prompt, true), (self.select_prompt, false)] {
            let Some(n) = node else { continue };
            let hit = crate::menu::descendants(&self.winner, n)
                .into_iter()
                .any(|c| self.winner.hit(&placed, c, p, screen));
            if hit {
                return Some(next);
            }
        }
        None
    }

    /// Which part of player `player`'s ability wheel the output pixel `p`
    /// hits (mouse): `Some(-1)` / `Some(1)` the left / right arrow,
    /// `Some(0)` the up arrow (the wheel handle).
    pub fn wheel_hit(&self, player: usize, p: Vec2, size: (u32, u32)) -> Option<i32> {
        let ui = self.circle_uis.get(player)?;
        let screen = Vec2::new(size.0 as f32, size.1 as f32);
        let placed = self.circles.layout(screen);
        let visible = |n: Option<usize>| -> bool {
            // Active up the hierarchy.
            let mut at = n;
            while let Some(i) = at {
                if !self.circles.nodes[i].active {
                    return false;
                }
                at = self.circles.nodes[i].parent;
            }
            n.is_some()
        };
        let hit = |n: Option<usize>| {
            visible(n)
                && n.is_some_and(|n| {
                    std::iter::once(n)
                        .chain(crate::menu::descendants(&self.circles, n))
                        .any(|c| self.circles.hit(&placed, c, p, screen))
                })
        };
        if hit(ui.arrow_l) {
            Some(-1)
        } else if hit(ui.arrow_r) {
            Some(1)
        } else if hit(ui.up_arrow) {
            Some(0)
        } else {
            None
        }
    }

    /// The wheel row (`circle`, `circle (1)`, `circle (2)`) of player
    /// `player` under the output pixel `p` (`CallOnHover.onHover` →
    /// `MidGameAbilitySelect.HoverCircle(row)`).
    pub fn wheel_row_hit(&self, player: usize, p: Vec2, size: (u32, u32)) -> Option<usize> {
        let ui = self.circle_uis.get(player)?;
        let screen = Vec2::new(size.0 as f32, size.1 as f32);
        let placed = self.circles.layout(screen);
        let active = |n: usize| {
            let mut at = Some(n);
            while let Some(i) = at {
                if !self.circles.nodes[i].active {
                    return false;
                }
                at = self.circles.nodes[i].parent;
            }
            true
        };
        (0..3).find(|&k| {
            ui.wheel_rows[k].is_some_and(|r| active(r) && self.circles.hit(&placed, r, p, screen))
        })
    }

    pub fn observe(&mut self, v: &View, sounds: &mut Sounds, audio: &mut Audio) {
        self.time += 1.0 / 60.0;
        for e in &v.events {
            if let Cue::WinnerScreen = e.cue {
                let _ = (&sounds, &audio);
            }
        }
    }

    /// Instantiates one circle per player.
    fn ensure_circles(&mut self, n: usize) {
        if self.circle_uis.len() >= n || self.circles.nodes.is_empty() {
            return;
        }
        let Ok(p) = self.data.prefab("sharedassets4.assets:44") else {
            return;
        };
        while self.circle_uis.len() < n {
            let c = &mut self.circles;
            let root = c.graft(&self.data, &p.objects, 0, 0);
            let f = |name: &str| c.find_in(root, name);
            let winner = f("Winner");
            let loser = f("Loser");
            let wheel = loser.and_then(|l| c.find_in(l, "AbilitySelect"));
            let character = loser.and_then(|l| c.find_in(l, "character"));
            let rows =
                ["circle", "circle (1)", "circle (2)"].map(|n| wheel.and_then(|w| c.child(w, n)));
            let icons = [
                ("circle", "ab1"),
                ("circle (1)", "ab2"),
                ("circle (2)", "ab3"),
            ]
            .map(|(r, i)| {
                wheel
                    .and_then(|w| c.child(w, r))
                    .and_then(|r| c.child(r, i))
            });
            let mut ui = CircleUi {
                root,
                winner,
                winner_circle: winner.and_then(|w| c.child(w, "circle")),
                winner_char: winner.and_then(|w| c.child(w, "char")),
                winner_text: winner.and_then(|w| c.child(w, "Text")),
                loser,
                wheel,
                wheel_rows: rows,
                wheel_icons: icons,
                wheel_row_y: rows.map(|r| r.map_or(0.0, |r| c.nodes[r].pos.y)),
                arrow_l: wheel.and_then(|w| c.child(w, "arrowL")),
                arrow_r: wheel.and_then(|w| c.child(w, "arrowR")),
                up_arrow: character.and_then(|ch| c.child(ch, "arrow")),
                up_arrow_y: 0.0,
                loser_circle: character.and_then(|ch| c.child(ch, "circle")),
                loser_char: character.and_then(|ch| c.child(ch, "char")),
                loser_text: character.and_then(|ch| c.child(ch, "Text")),
                dots: loser.and_then(|l| c.find_in(l, "dots")),
            };
            ui.up_arrow_y = ui.up_arrow.map_or(0.0, |a| c.nodes[a].pos.y);
            c.set_active(ui.dots, false);
            self.circle_uis.push(ui);
        }
    }

    fn player_shade(&self, color: i32) -> UiShade {
        let mat = self
            .visuals
            .ui_materials
            .get(color.max(0) as usize)
            .cloned();
        shade_of(&self.data, mat.as_deref())
    }

    fn set_shade(c: &mut Canvas, n: Option<usize>, shade: &UiShade) {
        if let Some(img) = n.and_then(|n| c.nodes[n].image.as_mut())
            && matches!(img.shade, UiShade::Channel { .. })
        {
            img.shade = shade.clone();
        }
    }

    /// Winner screen layout (match-flow.md 11.1).
    fn apply_winner(&mut self, v: &View) {
        let celebration = self.winner.child(0, "WinnerCelebration_leaveDisabled");
        self.winner
            .set_active(celebration, v.flow.winner_screen_at.is_some());
        let Some(at) = v.flow.winner_screen_at else {
            if self.apply_exit() {
                return;
            }
            for t in self.triangles {
                self.winner.set_active(t, false);
            }
            self.winner.set_active(self.next_prompt, false);
            self.winner.set_active(self.select_prompt, false);
            return;
        };
        let t = (v.tick - at) as f32 / 60.0;
        let n = v.players.len() as f32;
        // Winners first, then by score (stable here).
        let mut order: Vec<usize> = (0..v.players.len()).collect();
        order.sort_by_key(|&i| {
            let p = &v.players[i];
            (!p.is_most_recent_winner, -p.games_won)
        });
        let winners: Vec<usize> = order
            .iter()
            .copied()
            .filter(|&i| v.players[i].is_most_recent_winner)
            .collect();
        let teams: Vec<i32> = {
            let mut t: Vec<i32> = winners.iter().map(|&i| v.players[i].team).collect();
            t.dedup();
            t.sort();
            t.dedup();
            t
        };
        let layout = match winners.len() {
            0 | 1 => 0,
            _ if teams.len() >= 2 => 1,
            2 => 2,
            _ => 3,
        };
        let key = match layout {
            0 => "winner!!",
            1 => "draw!",
            _ => "winners!!",
        };
        for (k, tri) in self.triangles.iter().enumerate() {
            self.winner.set_active(*tri, k == layout);
            if let Some(tn) = self.winner_text[k]
                && let Some(tx) = self.winner.nodes[tn].text.as_mut()
            {
                tx.source = key.into();
                tx.localized = true;
            }
        }
        if let Some(tri) = self.triangles[layout]
            && let Some(x) = self.tri_in.eval(t)
        {
            self.winner.nodes[tri].pos.x = x;
        }
        let shown = if layout == 0 { 1 } else { winners.len() };
        self.winner_images(layout, shown);
        for (k, &(img, _)) in self.layout_imgs[layout].iter().enumerate() {
            let on = k < shown;
            self.winner.set_active(Some(img), on);
            let p = winners.get(k).or(order.get(k)).copied();
            if let (Some(p), true) = (p, on) {
                let shade = self.player_shade(v.players[p].color);
                Self::set_shade(&mut self.winner, Some(img), &shade);
            }
        }
        // Prompts after 1.0 + 0.3·N + 0.3 and one spacing later (N capped
        // at the original's 4 players: 12 would keep them away for 5 s).
        let n = n.min(4.0);
        let delays = [1.0 + 0.3 * n + 0.3, 1.0 + 0.3 * (n + 1.0) + 0.3];
        let lift = if v.players.len() > 5 { 470.0 } else { 0.0 };
        self.exit = None;
        self.last_shown = Some((
            ExitFrom {
                layout,
                prompt_t: delays.map(|d| (t > d).then_some(t - d)),
                lift,
            },
            self.time,
        ));
        // "ability select" first, then "next level" (CharacterStatsList).
        for (k, node) in [self.select_prompt, self.next_prompt]
            .into_iter()
            .enumerate()
        {
            let on = t > delays[k];
            self.winner.set_active(node, on);
            if let (Some(nd), true) = (node, on)
                && let Some(y) = self.prompt_in.eval(t - delays[k])
            {
                // More than 5 circles reach under the prompts: lift them
                // above the circle row.
                self.winner.nodes[nd].pos.y = y + lift;
            }
        }
        self.winner.set_active(self.stats_row, false);
    }

    /// The image nodes of winner layout `layout` for `n` winners: the
    /// authored images, or for more winners than the original layout has
    /// (draw > 4, one team > 3; match-flow.md 17.2) copies of them
    /// arranged in up to two rows over the authored area, scaled to fit.
    fn winner_images(&mut self, layout: usize, n: usize) {
        if self.layout_imgs[layout].is_empty() {
            let authored: Vec<Option<usize>> = match layout {
                0 => vec![self.single_img],
                1 => self.draw_imgs.to_vec(),
                2 => self.team2_imgs.to_vec(),
                _ => self.team3_imgs.to_vec(),
            };
            self.layout_imgs[layout] = authored
                .into_iter()
                .flatten()
                .map(|i| (i, self.winner.nodes[i].pos))
                .collect();
        }
        let authored = match layout {
            0 => 1,
            1 => 4,
            2 => 2,
            _ => 3,
        }
        .min(self.layout_imgs[layout].len());
        if authored == 0 {
            return;
        }
        // Copies (cycling through the authored poses) up to `n`.
        while self.layout_imgs[layout].len() < n.min(12) {
            let k = self.layout_imgs[layout].len();
            let (src, pos) = self.layout_imgs[layout][k % authored];
            let parent = self.winner.nodes[src].parent.unwrap_or(0);
            let id = self.winner.clone_subtree(src, parent);
            // Behind the banner like the authored images: before the
            // triangle in draw order.
            let kids = &mut self.winner.nodes[parent].children;
            kids.retain(|&c| c != id);
            let at = kids.iter().position(|&c| c == src).unwrap_or(0);
            kids.insert(at, id);
            self.layout_imgs[layout].push((id, pos));
        }
        let imgs = self.layout_imgs[layout].clone();
        if n <= authored {
            for &(id, pos) in &imgs {
                self.winner.nodes[id].pos = pos;
                self.winner.nodes[id].scale_mul = 1.0;
            }
            return;
        }
        // Along the banner: the images stand on its upper edge, from the
        // lower left to the upper right, in two staggered rows for more
        // than 6. Worked out in 1920x1080 output pixels of the 4096x2160
        // canvas at rest (the triangle node's own slide moves them along).
        let rows = if n > 6 { 2 } else { 1 };
        let per_row = n.div_ceil(rows);
        let (x0, x1) = (380.0f32, 1460.0f32);
        let step = (x1 - x0) / (per_row.max(2) - 1) as f32;
        let scale = (step / 430.0).clamp(0.3, 0.8);
        let banner = |sx: f32| 1000.0 - (sx - 400.0) * 0.507;
        const K: f32 = 4096.0 / 1920.0;
        let mut order = Vec::with_capacity(n);
        for row in 0..rows {
            // Back row (row 0 of two) first in draw order.
            let in_row = if row + 1 == rows {
                n - row * per_row
            } else {
                per_row
            };
            for col in 0..in_row {
                let k = row * per_row + col;
                let back = rows == 2 && row == 0;
                let sx = x0 + step * col as f32 - if back { step * 0.4 } else { 0.0 };
                let sy = banner(sx) - 620.0 * scale - if back { 290.0 * scale } else { 0.0 };
                let (id, _) = imgs[k];
                let node = &mut self.winner.nodes[id];
                node.pos = Vec2::new(sx * K - 3720.0, 858.0 + (540.0 - sy) * K);
                node.scale_mul = scale;
                order.push(id);
            }
        }
        // Draw order: back row, then front row, all before the banner.
        if let Some(parent) = imgs
            .first()
            .and_then(|&(id, _)| self.winner.nodes[id].parent)
        {
            let all: Vec<usize> = imgs.iter().map(|&(id, _)| id).collect();
            let kids = &mut self.winner.nodes[parent].children;
            kids.retain(|c| !all.contains(c));
            for (i, id) in order.into_iter().enumerate() {
                kids.insert(i, id);
            }
            let hidden: Vec<usize> = all.iter().copied().filter(|c| !kids.contains(c)).collect();
            for (i, id) in hidden.into_iter().enumerate() {
                kids.insert(i, id);
            }
        }
    }

    /// Ability-select circles and wheels from the flow state.
    #[allow(clippy::needless_range_loop)] // rows by index
    fn apply_circles(&mut self, v: &View) {
        let n = v.players.len();
        self.ensure_circles(n);
        let spacing = (2400.0 / n.max(1) as f32).min(230.0);
        let rows_n = v.number_of_abilities.clamp(1, 3);
        // Online: players who left keep their score on the winner screen,
        // greyed, in the slots nobody else took (docs/net.md, leave rule).
        let gone = |i: usize| v.flow.gone.get(i).copied().unwrap_or(false);
        let mut left_slots: Vec<(usize, usize)> = Vec::new();
        if v.flow.winner_screen_at.is_some() {
            let used: Vec<usize> = v
                .flow
                .players
                .iter()
                .filter_map(|p| p.circle.slot())
                .collect();
            let mut free = (0..n).filter(|s| !used.contains(s));
            for i in 0..n {
                let hidden = v
                    .flow
                    .players
                    .get(i)
                    .is_none_or(|p| p.circle == Circle::Hidden);
                if gone(i)
                    && hidden
                    && let Some(s) = free.next()
                {
                    left_slots.push((i, s));
                }
            }
        }
        self.circle_shown_x.resize(n, None);
        self.circle_down.resize(n, None);
        for (i, p) in v.players.iter().enumerate() {
            let Some(ui) = self.circle_uis.get(i).cloned() else {
                continue;
            };
            let mut fp = v.flow.players.get(i).cloned().unwrap_or_default();
            if let Some(&(_, slot)) = left_slots.iter().find(|(k, _)| *k == i) {
                fp.circle = Circle::Loser {
                    since: v.flow.winner_screen_at.unwrap_or(v.tick),
                    slot,
                };
            }
            let c = &mut self.circles;
            let (since, slot, winner) = match fp.circle {
                Circle::Hidden => {
                    // A shown circle hidden by a level load slides down:
                    // `timeSinceActivated = 1 − x / DelayDifferenceDenominator`
                    // running backwards along `moveUpAnim` (the rightmost
                    // at once, the others up to 1 − x/1000 s later; the
                    // drop is (elapsed)² × 500 units, measured equal).
                    if let Some(x) = self.circle_shown_x[i].take() {
                        self.circle_down[i] = Some((self.time, x));
                    }
                    if let Some((start, x)) = self.circle_down[i] {
                        let s = (1.0 - x / 1000.0) - (self.time - start);
                        if s > 0.0 {
                            let a = s.min(1.0);
                            let up = 2.0 * a - a * a;
                            let node = &mut c.nodes[ui.root];
                            node.active = true;
                            node.pos = Vec2::new(x, -500.0 * (1.0 - up));
                            continue;
                        }
                        self.circle_down[i] = None;
                    }
                    c.nodes[ui.root].active = false;
                    continue;
                }
                Circle::Loser { since, slot } => (since, slot, false),
                Circle::Winner { since, slot } => (since, slot, true),
            };
            let node = &mut c.nodes[ui.root];
            node.active = true;
            node.alpha = if gone(i) { 0.4 } else { 1.0 };
            let x = -spacing * (n as f32 - 1.0) + spacing * slot as f32;
            self.circle_shown_x[i] = Some(x);
            self.circle_down[i] = None;
            // Slide up from -500 after `ShowUpDelay − x / DelayDifferenceDenominator`
            // = 2 s − x/1000 (farther left = later; match-flow.md 11.2), along
            // `moveUpAnim` (Hermite (0,0) slope 2 → (1,1) slope 0 = 2t − t²).
            let delay = 2.0 - x / 1000.0;
            let age = ((v.tick - since) as f32 / 60.0 - delay).clamp(0.0, 1.0);
            let up = 2.0 * age - age * age;
            node.pos = Vec2::new(x, -500.0 * (1.0 - up));
            // More than the original's 4 players: the row keeps the
            // original spacing up to 2400 units, then shrinks (circles
            // shrink with it so that neighbours do not touch).
            node.scale_mul = (spacing / 230.0).powf(1.5).max(0.7);
            if let Some(at) = fp.wiggle_at {
                let w = (v.tick - at) as f32 / 60.0;
                if w < 0.35 {
                    node.pos.x += (w * 60.0).sin() * 8.0 * (0.35 - w) / 0.35;
                }
            }
            let team = self.visuals.team(p.team);
            let shade = {
                let mat = self
                    .visuals
                    .ui_materials
                    .get(p.color.max(0) as usize)
                    .cloned();
                shade_of(&self.data, mat.as_deref())
            };
            c.set_active(ui.winner, winner);
            c.set_active(ui.loser, !winner);
            let score = p.games_won.to_string();
            if winner {
                c.set_color(ui.winner_circle, team.fill);
                c.set_text(ui.winner_text, &score);
                let kills = (p.kills - p.kills_at_start_of_round).clamp(0, 2) as usize;
                let alive = p.alive || p.cause_of_death == CauseOfDeath::Other;
                let sprite = if alive && p.eaten_this_round > 0 {
                    self.special_chars.first()
                } else if alive {
                    self.winner_chars.get(kills)
                } else {
                    self.loser_chars.get(p.cause_of_death as usize)
                };
                c.set_sprite(ui.winner_char, sprite.map(String::as_str));
                Self::set_shade(c, ui.winner_char, &shade);
            } else {
                c.set_color(ui.loser_circle, team.fill);
                c.set_text(ui.loser_text, &score);
                let sprite = self.loser_chars.get((p.cause_of_death as usize).min(4));
                c.set_sprite(ui.loser_char, sprite.map(String::as_str));
                Self::set_shade(c, ui.loser_char, &shade);
                let open = fp.wheel.open;
                if let Some(ch) = ui.loser_char
                    && let Some(img) = c.nodes[ch].image.as_mut()
                {
                    img.color.a = if open { 1.0 } else { 0.7 };
                }
                // The up arrow rises while open; the wheel stack shows.
                c.set_active(ui.up_arrow, !gone(i));
                if let Some(a) = ui.up_arrow {
                    c.nodes[a].pos.y = ui.up_arrow_y
                        + if open {
                            10.0 + 70.0 * rows_n as f32
                        } else {
                            0.0
                        };
                    // Not openable yet.
                    let ready = v.tick - since > bopl_sim::flow::WHEEL_DELAY_TICKS;
                    if let Some(img) = c.nodes[a].image.as_mut() {
                        img.color.a = if ready { 1.0 } else { 0.4 };
                    }
                }
                let remote = self.remote.get(i).copied().unwrap_or(false);
                c.set_active(ui.wheel, open && !remote);
                c.set_active(ui.dots, open && remote);
                if let Some(d) = ui.dots {
                    let t = v.tick as f32 / 60.0;
                    for (k, &dot) in c.nodes[d].children.clone().iter().enumerate() {
                        let ph = t * std::f32::consts::TAU * 1.5 - k as f32 * 0.9;
                        c.nodes[dot].offset.y = ph.sin().max(0.0) * 14.0;
                    }
                }
                let missing = 3 - rows_n;
                for k in 0..3 {
                    let visible = k < rows_n;
                    c.set_active(ui.wheel_rows[k], visible);
                    if let Some(r) = ui.wheel_rows[k] {
                        c.nodes[r].pos.y = ui.wheel_row_y[k] - 71.7 * missing as f32;
                        let sel = k == fp.wheel.row as usize;
                        let age = (v.tick - fp.wheel.changed_at) as f32 / 60.0;
                        c.nodes[r].scale_mul = if sel && age < 0.27 {
                            eval_keys(
                                &[(0.0, 1.0), (0.055, 0.631), (0.16, 1.133), (0.267, 1.0)],
                                age,
                            )
                        } else {
                            1.0
                        };
                        c.set_color(Some(r), team.fill);
                    }
                    if visible {
                        let slot = row_slot(rows_n, k);
                        let ab = p.chosen.get(slot).copied().unwrap_or(1);
                        c.set_sprite(ui.wheel_icons[k], self.visuals.ability_icon(ab));
                    }
                }
                let sel_y =
                    ui.wheel_rows[fp.wheel.row as usize % 3].map_or(0.0, |r| c.nodes[r].pos.y);
                let age = (v.tick - fp.wheel.changed_at) as f32 / 60.0;
                let jump = eval_keys(&[(0.0, 0.05), (0.083, 7.1), (0.173, 0.08)], age) * 1.5;
                if let Some(l) = ui.arrow_l {
                    c.nodes[l].pos = Vec2::new(
                        -50.0 - if fp.wheel.change_dir > 0 { jump } else { 0.0 },
                        sel_y,
                    );
                }
                if let Some(r) = ui.arrow_r {
                    c.nodes[r].pos = Vec2::new(
                        50.0 + if fp.wheel.change_dir < 0 { jump } else { 0.0 },
                        sel_y,
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        shared: &mut Shared,
        v: &View,
        pause: Option<Pause>,
        _w: &World,
        gpu: &Gpu,
        renderer: &mut Renderer,
        size: (u32, u32),
        frame: &mut Frame,
    ) {
        self.apply_winner(v);
        self.apply_circles(v);
        let mut pass = ui_pass("match-ui", size);
        let mut d = UiDraw {
            gpu,
            renderer,
            assets: &mut shared.assets,
            fonts: &mut shared.fonts,
            l10n: &self.l10n,
            time: self.time,
        };
        if v.flow.winner_screen_at.is_some() || self.exit.is_some() {
            self.winner.draw(&mut d, size, &mut pass);
        }
        self.circles.draw(&mut d, size, &mut pass);
        if let (Some(p), Some(c)) = (pause, self.pause.as_mut()) {
            let pulse = |on: bool, t: f32| {
                if on {
                    1.0 + 0.047 * (t * std::f32::consts::TAU / 0.7).sin().abs()
                } else {
                    1.0
                }
            };
            if let Some(vn) = self.pause_v {
                c.nodes[vn].scale_mul = pulse(p.leave, p.opened);
            }
            if let Some(xn) = self.pause_x {
                c.nodes[xn].scale_mul = pulse(!p.leave, p.opened);
            }
            for (n, on) in [(self.pause_v, p.leave), (self.pause_x, !p.leave)] {
                if let Some(n) = n {
                    c.nodes[n].alpha = if on { 1.0 } else { 0.6 };
                }
            }
            c.draw(&mut d, size, &mut pass);
        }
        if !pass.items.is_empty() {
            frame.passes.push(pass);
        }
        let _ = Color::WHITE;
    }
}

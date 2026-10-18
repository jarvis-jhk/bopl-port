//! Platform (`docs/spec/abilities/platform.md`): `PlatformTransform` on the
//! ability object and the `ResizablePlatform` it creates (a real dynamic
//! platform: `StickyRoundedRectangle`, `AnimateVelocity`,
//! `ShakablePlatform`, `ResizablePlatform`, `BoplBody`, rounded rect).
//!
//! Update order inside the ability object: Ability 1 (framework),
//! PlatformTransform 2, SpriteAnimator 3, PlayerCollision 4,
//! DestroyIfOutsideSceneBounds 5, PlayerBody 6, DPhysicsCircle 7 (no
//! PlayerPhysics). The platform: Sticky 1, AnimateVelocity 2, Shakable 3,
//! ResizablePlatform 4, BoplBody 5, DPhysicsRoundedRect 6 (manual init).
//!
//! A held grappling rope is handed over to the new platform at
//! CreatePlatform (a stand-alone `RopeAttachment`).

use std::sync::Arc;

use super::ability_body::{self, AbilityBody};
use super::prefab;
use super::prelude::*;
use crate::curve::Curve;
use crate::data::{AnimRef, PlatformDef, SpringDef, TransformDef};
use crate::platform::{Platform as LevelPlatform, update_bounds};
use crate::view::Visual;
use crate::world::Kind;

const CB_CREATE_PLATFORM: u16 = 1;
const CB_ENTER_SIZE: u16 = 2;
const CB_BEGIN_EJECTION: u16 = 3;

/// `(Fix)0.2f`, `(Fix)4.5f`, `(Fix)0.25f`, `(Fix)0.5f`, `(Fix)0.1f`.
const F_0_2F: Fix = Fix(858_993_472);
const F_4_5F: Fix = Fix(19_327_352_832);
const F_0_1F: Fix = Fix(429_496_736);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum State {
    #[default]
    Anticipation,
    Rotation,
    Size,
    Exit,
}

/// `ResizablePlatform` component state (stored on the platform payload).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResizeState {
    pub mass_per_area: Fix,
    pub remove_animation: Arc<Curve>,
    pub being_removed: bool,
    pub time_passed: Fix,
    pub o_radius: Fix,
    pub o_height: Fix,
    pub o_width: Fix,
}

/// `PlatformArea(w, h, r)` (physics.md §12.6).
pub fn platform_area(w: Fix, h: Fix, r: Fix) -> Fix {
    let a = Fix::TWO * h * r;
    let b = (Fix::TWO * h + Fix::TWO * r) * Fix::TWO * w;
    let c = Fix::PI * r * r;
    a * Fix::TWO + b + c
}

/// `ApproximateMOI(w, h, r)`.
pub fn approximate_moi(w: Fix, h: Fix, r: Fix) -> Fix {
    let w = w + r;
    let h = h + r;
    let m = w.max(h);
    w.min(h) * m * m * F_0_2F + Fix::from_int(40)
}

/// Current half extents of a registered rounded rect (`CalcExtents`).
fn calc_extents(w: &World, plat: ObjId) -> Vec2 {
    match w.physics.rr(plat) {
        Some(r) => Vec2::new(Fix::ONE / r.b.ie.x, Fix::ONE / r.b.ie.y),
        None => w
            .objects
            .get(plat)
            .and_then(|o| o.collider)
            .map_or(Vec2::ZERO, |c| c.start_extents),
    }
}

fn rr_radius(w: &World, plat: ObjId) -> Fix {
    match w.physics.rr(plat) {
        Some(r) => r.radius,
        None => w
            .objects
            .get(plat)
            .and_then(|o| o.collider)
            .map_or(Fix::ZERO, |c| c.start_radius),
    }
}

/// `ResizablePlatform.ResizePlatform(rHeight, rWidth, bevelRadius)` on a
/// platform payload `p` of object `plat` (the payload may be taken out).
fn resize(w: &mut World, plat: ObjId, p: &mut LevelPlatform, h: Fix, wd: Fix, r: Fix) {
    let mpa = p.resize.as_ref().map_or(Fix(214_748_368), |s| s.mass_per_area);
    // `PlatformArea` receives the height as its width argument (literal).
    let im = Fix::ONE / (Fix::from_int(10) + mpa * platform_area(h, wd, r));
    let imoi = Fix::ONE / approximate_moi(wd, h, r);
    if let Some(b) = w.physics.body_mut(plat) {
        b.inverse_mass = im;
        b.inverse_moi = imoi;
    }
    if let Some(rr) = w.physics.rr_mut(plat) {
        rr.radius = r;
        rr.b.update_extents(Vec2::new(wd, h));
    }
    let q = (wd + r) / (h + r);
    if let Some(s) = &mut p.spring {
        s.def.rotation_resistence = (q.max(Fix::ONE / q) * F_4_5F - Fix::from_int(4)).max(Fix::ONE);
    }
    // UpdateBounds (the payload may be taken out of the world).
    let ext = calc_extents(w, plat);
    let radius = rr_radius(w, plat);
    let s = &mut p.sticky;
    s.extents = ext;
    let ex = ext.x * Fix::TWO;
    let ey = ext.y * Fix::TWO;
    let cr = radius * Fix::PI_OVER_2;
    s.c = [ex, cr, ey, cr, ex, cr, ey, cr];
    s.total = s.c[0] * Fix::TWO + s.c[1] * Fix::from_int(4) + s.c[2] * Fix::TWO;
}

/// Resize through the world (the platform payload is in place).
fn resize_in_world(w: &mut World, plat: ObjId, h: Fix, wd: Fix, r: Fix) {
    w.with_kind(plat, |w, k| {
        if let Kind::Platform(p) = k {
            resize(w, plat, p, h, wd, r);
        }
    });
}

/// `ResizablePlatform.UpdateSim` (called from the platform's dispatch).
pub fn resizable_update(p: &mut LevelPlatform, w: &mut World, me: ObjId) {
    let Some(s) = p.resize.clone() else {
        return;
    };
    if s.being_removed {
        let t = s.remove_animation.eval(s.time_passed);
        if s.time_passed >= s.remove_animation.last_time() {
            w.destroy(me);
            return;
        }
        if let Some(st) = &mut p.resize {
            st.time_passed += FIXED_DT * w.time.player_time_scale;
        }
        let (h, wd, r) = (
            F_0_1F.lerp(s.o_height, t),
            F_0_1F.lerp(s.o_width, t),
            F_0_1F.lerp(s.o_radius, t),
        );
        resize(w, me, p, h, wd, r);
    } else {
        let e = calc_extents(w, me);
        let r = rr_radius(w, me);
        resize(w, me, p, e.y, e.x, r);
    }
}

/// `RemovePlatform()`.
fn remove_platform(w: &mut World, plat: ObjId) {
    let e = calc_extents(w, plat);
    let r = rr_radius(w, plat);
    if let Some(Kind::Platform(p)) = w.objects.get_mut(plat).map(|o| &mut o.kind)
        && let Some(s) = &mut p.resize
    {
        s.being_removed = true;
        s.o_radius = r;
        s.o_height = e.y;
        s.o_width = e.x;
    }
}

/// The ResizablePlatform prefab as a platform definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlatformPrefab {
    pub prefab: usize,
    pub def: PlatformDef,
    pub hier_resizable: i32,
    pub state: ResizeState,
    pub updatables: i32,
}

impl PlatformPrefab {
    fn of(w: &World, prefab: usize) -> Option<PlatformPrefab> {
        let pf = w.content.prefabs.get(prefab)?;
        let o = pf.objects.first()?;
        let comp = |c: &str| o.component(c);
        let sticky = comp("StickyRoundedRectangle")?;
        let av = comp("AnimateVelocity").map(|c| (c.hierarchy.unwrap_or(0), &c.fields));
        let shake = comp("ShakablePlatform");
        let rp = comp("ResizablePlatform")?;
        let bb = comp("BoplBody")?;
        let bf = &bb.fields;
        let mut collider = o.collider?;
        collider.body = None;
        let spring = av.map(|(h, f)| SpringDef {
            hier: h,
            enabled: true,
            home_is_movable: f.boolean("HomeIsMovable").unwrap_or(false),
            speed: f.fix_or("speed", Fix::from_int(10)),
            mu: f.fix_or("mu", Fix::THREE),
            rotation_speed: f.fix_or("rotationSpeed", Fix::from_int(60)),
            rotation_mu: f.fix_or("rotationMu", Fix(715_828_032)),
            interpolate_rotation_home: f.boolean("interpolateRotationHome").unwrap_or(true),
            rotation_resistence: f.fix_or("rotationResistence", Fix::ONE),
            sudden_death_curve: f.curve("suddenDeathYSpeed").unwrap_or_default(),
            sudden_death_speed: f.fix_or("suddenDeathSpeed", Fix::ONE),
            sudden_death_acceleration: f.fix_or("SuddenDeathAcceleration", F_0_1F),
            max_random_sudden_death_delay: f.fix_or("MaxRandomSuddenDeathDelay", Fix::ONE),
            home_rotation: f.fix_or("homeRotation", Fix::ZERO),
        });
        let def = PlatformDef {
            name: o.name.clone(),
            tag: o.tag.clone(),
            layer: o.layer,
            transform: o.fix.unwrap_or(TransformDef {
                right: Vec2::RIGHT,
                up: Vec2::UP,
                ..TransformDef::default()
            }),
            collider,
            hier_collider: o.collider_hier.unwrap_or(0),
            body: crate::physics::BodyDef {
                mass: bf.fix_or("mass", Fix::ONE),
                bounciness: bf.fix_or("bounciness", Fix::ZERO),
                friction: bf.fix_or("friction", Fix::ZERO),
                dynamic_friction: bf.fix_or("dynamicFriction", Fix::ZERO),
                moment_of_inertia: bf.fix_or("momentOfInertia", Fix::ONE),
                gravity_scale: bf.fix_or("gravityScale", Fix::ZERO),
                air_friction01: bf.fix_or("airFriction01", Fix::ONE),
                rotational_drag01: bf.fix_or("rotationalDrag01", Fix::ONE),
                start_velocity: bf.vec2("StartVelocity").unwrap_or_default(),
                start_angular_velocity: bf.fix_or("StartAngularVelocity", Fix::ZERO),
            },
            hier_body: bb.hierarchy.unwrap_or(0),
            hier_sticky: sticky.hierarchy.unwrap_or(0),
            platform_type: sticky.fields.int("platformType").unwrap_or(0) as i32,
            spring,
            hier_shake: shake.and_then(|c| c.hierarchy),
            shake_strength: shake.map_or(Fix::ONE, |c| c.fields.fix_or("basicShakeStr", Fix::ONE)),
            grow_on_start: None,
            orbit: None,
            path: None,
            visual: o.visual.clone(),
        };
        Some(PlatformPrefab {
            prefab,
            def,
            hier_resizable: rp.hierarchy.unwrap_or(0),
            state: ResizeState {
                mass_per_area: rp.fields.fix_or("MassPerArea", Fix(214_748_368)),
                remove_animation: rp.fields.curve("RemoveAnimation").unwrap_or_default(),
                being_removed: false,
                time_passed: Fix::ZERO,
                o_radius: Fix::ZERO,
                o_height: Fix::ZERO,
                o_width: Fix::ZERO,
            },
            updatables: pf.updatable_count(),
        })
    }

    /// `InstantiateFixed(platformPrefab, pos)`.
    fn spawn(&self, w: &mut World, pos: Vec2) -> ObjId {
        w.open_bracket();
        let id = LevelPlatform::create(w, &self.def, 0);
        if let Some(o) = w.objects.get_mut(id) {
            o.visual = Visual::Prefab {
                prefab: self.prefab as u32,
                object: 0,
            };
            o.add_part(PartKind::Resizable, self.hier_resizable);
            if let Some(c) = o.collider.as_mut() {
                c.body = Some(self.def.body);
            }
            // `isResizable`: the prefab root is tagged `ResizablePlatform`.
            o.bb.resizable = true;
            if let Kind::Platform(p) = &mut o.kind {
                p.resize = Some(self.state.clone());
            }
        }
        let n = w.objects.get(id).map_or(0, |o| o.n_parts as i32);
        w.register_all(id);
        w.close_bracket();
        w.consume_hierarchy((self.updatables - n).max(0));
        if let Some(o) = w.objects.get_mut(id) {
            o.tf.position = pos;
            o.tf.reset_rotation();
        }
        id
    }
}

/// The Platform ability (`PlatformTransform`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Platform {
    pub b: AbilityBody,
    pub anim: SpriteAnimator,
    /// Shared, immutable prefab data (cheap rollback clones).
    pub platform: Option<Arc<PlatformPrefab>>,
    pub cursor_prefab: Option<usize>,
    /// `ropeAttachmentPrefab` (rope hand-over at CreatePlatform).
    pub rope_attachment_prefab: Option<usize>,
    pub time_in_exit: Fix,
    pub size_change_speed: Fix,
    pub min_size: Fix,
    pub rotation_speed: Fix,
    pub max_area: Fix,
    pub max_size_time: Fix,
    pub face_height: Fix,
    pub ejection_offset: Fix,
    pub cursor_sensitivity: Fix,
    pub enter_scale: Arc<Curve>,
    pub ejection: Arc<Curve>,
    pub a_anticipation: AnimRef,
    pub a_enter: AnimRef,
    pub a_width_loop: AnimRef,
    pub a_height_loop: AnimRef,
    pub a_to_exit: AnimRef,
    pub a_exit: AnimRef,
    // state
    pub state: State,
    pub input: Vec2,
    pub button_was_down: bool,
    pub time_passed: Fix,
    pub exit_time: Fix,
    pub has_released: bool,
    pub expanding_width: bool,
    pub active_corner: Vec2,
    /// The slot's live platform (`srrBuffer`, one entry).
    pub buffer: Option<ObjId>,
    pub current: Option<ObjId>,
    pub cursor: Option<ObjId>,
    pub cursor_visible: bool,
    pub sprite_visible: bool,
}

/// Objects this module spawns (the platform is a `Kind::Platform`).
pub type Spawned = NoSpawn;

impl AbilityImpl for Platform {
    fn exit_component(&mut self, cx: &mut Cx<'_>, r: AbilityExitInfo) {
        self.component_exit(cx, r);
    }

    fn new(w: &World, def: &AbilityDef) -> Self {
        let f = prefab::component_fields(w, def, "PlatformTransform");
        let data = f
            .reference("animData")
            .and_then(|r| prefab::anim_data_index(w, r))
            .unwrap_or(0);
        let a = |n: &str| prefab::anim(w, data, n);
        let mut b = AbilityBody::new(w, def);
        b.body.simulated = true;
        Platform {
            b,
            anim: ability_body::animator(prefab::ability_prefab(w, def).is_some_and(prefab::root_always_stop)),
            platform: f
                .reference("platformPrefab")
                .and_then(|r| prefab::prefab_index(w, r))
                .and_then(|p| PlatformPrefab::of(w, p))
                .map(Arc::new),
            cursor_prefab: f
                .reference("CursorPrefab")
                .and_then(|r| prefab::prefab_index(w, r)),
            rope_attachment_prefab: f
                .reference("ropeAttachmentPrefab")
                .and_then(|r| prefab::prefab_index(w, r)),
            time_in_exit: f.fix_or("TimeInExit", Fix(1_288_490_240)),
            size_change_speed: f.fix_or("SizeChangeSpeed", Fix::ONE),
            min_size: f.fix_or("MinSize", Fix::ONE),
            rotation_speed: f.fix_or("rotationSpeed", Fix::from_int(10)),
            max_area: f.fix_or("maxArea", Fix::from_int(20)),
            max_size_time: f.fix_or("maxSizeTime", Fix::ZERO),
            face_height: f.fix_or("faceHeight", Fix(1_288_490_240)),
            ejection_offset: f.fix_or("EjectionOffset", Fix::TWO),
            cursor_sensitivity: Fix::from_int(25),
            enter_scale: f.curve("EnterScaleAnimation").unwrap_or_default(),
            ejection: f.curve("EjectionCurve01").unwrap_or_default(),
            a_anticipation: a("Anticipation"),
            a_enter: a("Enter"),
            a_width_loop: a("WidthLoop"),
            a_height_loop: a("HeightLoop"),
            a_to_exit: a("ToExit"),
            a_exit: a("Exit"),
            state: State::Anticipation,
            input: Vec2::UP,
            button_was_down: false,
            time_passed: Fix::ZERO,
            exit_time: Fix::ZERO,
            has_released: false,
            expanding_width: true,
            active_corner: Vec2::new(Fix::ONE, Fix::ONE),
            buffer: None,
            current: None,
            cursor: None,
            cursor_visible: false,
            sprite_visible: true,
        }
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        let info = cx.info();
        let me = cx.me;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        self.button_was_down = true;
        AbilityBody::set_pos(cx.w, me, info.position);
        AbilityBody::update_collider(cx.w, me);
        self.state = State::Anticipation;
        self.time_passed = Fix::ZERO;
        self.has_released = false;
        self.expanding_width = true;
        cx.w.set_collider_enabled(me, true);
        self.active_corner = Vec2::new(Fix::ONE, Fix::ONE);
        let a = self.a_anticipation;
        ability_body::play_then(cx.w, &mut self.anim, a, CB_CREATE_PLATFORM);
        cx.w.emit(Cue::Sound("transform1"));
        if let Some(p) = cx.w.player_mut(cx.fw.owner) {
            p.in_platform_transform = true;
            p.custom_aim_ability_position = info.position;
        }
    }

    fn update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let owner = cx.fw.owner;
        // PlatformTransform 2.
        self.transform_update(cx, dt);
        if !cx.w.objects.get(me).is_some_and(|o| o.active) {
            return;
        }
        // SpriteAnimator 3.
        match ability_body::advance_code(cx.w, owner, &mut self.anim) {
            Some(CB_CREATE_PLATFORM) => self.create_platform(cx),
            Some(CB_ENTER_SIZE) => {
                self.cursor_visible = true;
                self.state = State::Size;
                let a = self.a_width_loop;
                ability_body::play(cx.w, &mut self.anim, a);
            }
            Some(CB_BEGIN_EJECTION) => {
                let a = self.a_exit;
                ability_body::play(cx.w, &mut self.anim, a);
            }
            _ => {}
        }
        // PlayerCollision 4, bounds 5, PlayerBody 6.
        AbilityBody::collision_update(cx.fw);
        if let Some(r) = self.b.bounds_update(cx.w, me) {
            self.exit_component(cx, r);
            return;
        }
        self.b.body_update(cx.w, me, dt);
    }

    fn late_update(&mut self, cx: &mut Cx<'_>, _dt: Fix) {
        self.b.body_late_update(cx.w, cx.me);
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        ability_body::hurt_contact(self, cx, c);
    }


    /// The transforming body (hidden while it is the platform) and the
    /// cursor while shown.
    fn look(&self, _w: &World, _me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        l.frame(&self.anim);
        l.hidden = !self.sprite_visible;
        if !self.cursor_visible
            && let Some(c) = self.cursor
        {
            l.hide.push(c);
        }
    }

    fn sprite(&self) -> crate::data::SpriteId {
        if self.sprite_visible {
            self.anim.sprite
        } else {
            crate::data::SpriteId::NONE
        }
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        Some(&mut self.b)
    }
}

impl Platform {
    fn create_platform(&mut self, cx: &mut Cx<'_>) {
        let me = cx.me;
        cx.w.emit(Cue::Sound("transform2"));
        cx.w.set_collider_enabled(me, false);
        self.state = State::Rotation;
        self.b.squishable = false;
        let pos = AbilityBody::pos(cx.w, me);
        if self.cursor.is_none_or(|c| !cx.w.objects.exists(c))
            && let Some(cp) = self.cursor_prefab
        {
            self.cursor = Some(ability_body::spawn_prefab(cx.w, cp, Kind::Empty, &[], pos, None));
        }
        self.cursor_visible = false;
        let Some(pp) = self.platform.clone() else {
            return;
        };
        let plat = pp.spawn(cx.w, pos);
        let scale = cx.w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale);
        cx.w.set_collider_scale(plat, scale);
        self.add_to_buffer(cx.w, plat);
        let hier = cx
            .w
            .objects
            .get(plat)
            .and_then(|o| o.part(PartKind::Collider))
            .map_or(0, |p| p.hier);
        cx.w.physics.register(&mut cx.w.objects, plat, hier);
        update_bounds(cx.w, plat);
        self.current = Some(plat);
        let a = self.a_enter;
        ability_body::play_then(cx.w, &mut self.anim, a, CB_ENTER_SIZE);
        let ppos = AbilityBody::pos(cx.w, plat);
        AbilityBody::set_pos(cx.w, me, ppos);
        let e = calc_extents(cx.w, plat);
        let r = rr_radius(cx.w, plat);
        resize_in_world(cx.w, plat, e.y, e.x, r);
        if cx.w.round.sudden_death {
            let stamp = cx.w.round.sudden_death_stamp;
            let delay_max = match cx.w.objects.get(plat).map(|o| &o.kind) {
                Some(Kind::Platform(p)) => p
                    .spring
                    .as_ref()
                    .map(|s| s.def.max_random_sudden_death_delay),
                _ => None,
            };
            if let Some(m) = delay_max {
                let d = cx.w.rng.random_fix(Fix::ZERO, m);
                if let Some(Kind::Platform(p)) = cx.w.objects.get_mut(plat).map(|o| &mut o.kind)
                    && let Some(s) = &mut p.spring
                {
                    s.def.home_is_movable = true;
                    s.in_sudden_death = true;
                    s.sudden_death_stamp = stamp;
                    s.random_sudden_death_delay = d;
                }
            }
        }
        let r = rr_radius(cx.w, plat);
        let body_pos = AbilityBody::pos(cx.w, me);
        if let Some(c) = self.cursor {
            AbilityBody::set_pos(cx.w, c, body_pos + Vec2::UP * r);
        }
        // The rope moves to the platform: an anchor at the platform's
        // closest point to (centre − up · 10); the body forgets it
        // (`EnterAbilityClearRope`).
        if let Some(l) = self.b.body.rope
            && cx.w.physics.ropes.enabled(l.rope as usize).is_some()
            && let Some(ap) = self.rope_attachment_prefab
            && let Some(s) = crate::surface::Surface::of(cx.w, plat)
        {
            let t = AbilityBody::tf(cx.w, plat);
            let at = s.closest_point(t.position - t.up * Fix::from_int(10));
            super::grappling_hook::spawn_platform_anchor(cx.w, ap, at, plat, l.rope, l.top);
            self.b.clear_rope();
        }
    }

    /// `AddPlatformToBuffer` (one slot): a live older platform starts its
    /// removal animation and is replaced.
    fn add_to_buffer(&mut self, w: &mut World, plat: ObjId) {
        match self.buffer.filter(|b| w.objects.exists(*b)) {
            None => self.buffer = Some(plat),
            Some(old) => {
                remove_platform(w, old);
                self.buffer = Some(plat);
            }
        }
    }

    fn transform_update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        let owner = cx.fw.owner;
        let pdt = cx.w.fixed_delta_time(owner, dt);
        let aim = cx
            .w
            .player(owner)
            .map_or(Vec2::ZERO, |p| p.inputs.aim_vector());
        self.input = aim;
        let custom = match self.cursor.filter(|c| cx.w.objects.exists(*c)) {
            Some(c) => AbilityBody::pos(cx.w, c),
            None => AbilityBody::pos(cx.w, me),
        };
        if let Some(p) = cx.w.player_mut(owner) {
            p.custom_aim_ability_position = custom;
        }
        if self.state == State::Anticipation {
            self.button_was_down = cx.button_down();
            if !self.button_was_down && !self.has_released {
                self.has_released = true;
            }
            return;
        }
        let Some(plat) = self.current.filter(|p| cx.w.objects.exists(*p)) else {
            AbilityImpl::kill(self, cx, ENVIRONMENT_ID, true, true, CauseOfDeath::Accident);
            return;
        };
        let plat_active = cx.w.objects.active_in_hierarchy(plat)
            && cx
                .w
                .objects
                .get(plat)
                .is_some_and(|o| o.part_enabled(PartKind::Sticky));
        if !plat_active {
            self.sprite_visible = false;
            return;
        }
        self.sprite_visible = true;
        self.time_passed += pdt;
        let down = cx.button_down();
        if !down && !self.has_released {
            self.has_released = true;
        }
        let e = calc_extents(cx.w, plat);
        let (x, y) = (e.x, e.y);
        let radius = rr_radius(cx.w, plat);
        let plat_scale = cx.w.objects.get(plat).map_or(Fix::ONE, |o| o.tf.scale);
        let amax = self.max_area * plat_scale;
        match self.state {
            State::Rotation => {
                let k = self.enter_scale.eval(self.time_passed);
                resize_in_world(cx.w, plat, y, x, plat_scale * k);
                if aim.magnitude() >= DEADZONE {
                    let pup = cx.w.objects.get(plat).map_or(Vec2::UP, |o| o.tf.up);
                    let up = pup.slerp(aim, dt * self.rotation_speed);
                    if let Some(o) = cx.w.objects.get_mut(me) {
                        o.tf.set_up(up);
                    }
                    let rot = cx.w.objects.get(me).map_or(Fix::ZERO, |o| o.tf.rotation);
                    if let Some(Kind::Platform(p)) =
                        cx.w.objects.get_mut(plat).map(|o| &mut o.kind)
                        && let Some(s) = &mut p.spring
                    {
                        s.home_rotation = rot;
                    }
                    if let Some(o) = cx.w.objects.get_mut(plat) {
                        o.tf.set_up(up);
                    }
                }
                let ppos = AbilityBody::pos(cx.w, plat);
                AbilityBody::set_pos(cx.w, me, ppos);
            }
            State::Size => self.size_update(cx, plat, x, y, radius, amax, pdt, dt),
            State::Exit => {
                let h = radius + y;
                let t = (self.time_passed - self.exit_time) / self.time_in_exit;
                let e = self.ejection.eval(t) * h;
                let (ppos, pup) = cx
                    .w
                    .objects
                    .get(plat)
                    .map_or((Vec2::ZERO, Vec2::UP), |o| (o.tf.position, o.tf.up));
                AbilityBody::set_pos(cx.w, me, ppos + pup * e);
                if e + self.face_height >= radius + y {
                    let p = AbilityBody::pos(cx.w, me) + pup * self.ejection_offset;
                    AbilityBody::set_pos(cx.w, me, p);
                    self.exit_component(cx, AbilityExitInfo::default());
                }
            }
            State::Anticipation => {}
        }
        self.button_was_down = down;
    }

    /// `GetNewSize(w, h)`: cap the area at `amax` by shrinking the height,
    /// then (at minimum height) the width.
    fn new_size(&self, mut wd: Fix, mut h: Fix, r: Fix, amax: Fix) -> (Fix, Fix) {
        if platform_area(wd, h, r) > amax {
            let four = Fix::from_int(4);
            h = (amax - four * r * wd - Fix::PI * r * r) / (four * r + four * wd);
            if h < self.min_size {
                h = self.min_size;
                wd = (amax - four * h * r - Fix::PI * r * r) / (four * h + four * r);
            }
        }
        (wd, h)
    }

    #[allow(clippy::too_many_arguments)]
    fn size_update(
        &mut self,
        cx: &mut Cx<'_>,
        plat: ObjId,
        x: Fix,
        y: Fix,
        radius: Fix,
        amax: Fix,
        pdt: Fix,
        dt: Fix,
    ) {
        let me = cx.me;
        let Some(t) = cx.w.objects.get(me).map(|o| o.tf) else {
            return;
        };
        let mut cursor = self
            .cursor
            .map_or(t.position, |c| AbilityBody::pos(cx.w, c));
        cursor += self.input * self.cursor_sensitivity * dt;
        let l = Vec2::from_angle(-t.rotation).complex_mul(cursor - t.position);
        let lx = l.x.max(-x - radius).min(x + radius);
        let ly = l.y.max(-y - radius).min(y + radius);
        let l = Vec2::new(lx, ly);
        cursor = t.position + t.right.complex_mul(l);
        if let Some(c) = self.cursor {
            AbilityBody::set_pos(cx.w, c, cursor);
        }
        let b = self.min_size + radius * Fix::TWO;
        if l.x >= x && x <= self.min_size {
            self.active_corner.x = Fix::ONE;
        } else if l.x < -x && x <= self.min_size {
            self.active_corner.x = -Fix::ONE;
        }
        if l.y >= y && y <= self.min_size {
            self.active_corner.y = Fix::ONE;
        } else if l.y < -y && y <= self.min_size {
            self.active_corner.y = -Fix::ONE;
        }
        let c = Vec2::new(self.active_corner.x * x, self.active_corner.y * y);
        let d = l - c;
        let g0 = Vec2::new(d.x.min(b), d.y.min(b));
        let m0 = g0.magnitude();
        let mut g = if m0 == Fix::ZERO { Vec2::ZERO } else { g0 };
        if m0 < Fix::ONE {
            g = g0;
        }
        let g = Vec2::new(g.x * c.x.sign_fix(), g.y * c.y.sign_fix());
        let s = self.size_change_speed;
        let mut a = (x + g.x * pdt * s).max(self.min_size);
        let mut h = (y + g.y * pdt * s).max(self.min_size);
        if g.x >= Fix::ZERO && g.y >= Fix::ZERO {
            let k = pdt * s;
            let v = Vec2::new(g.x * k, g.y * k);
            let m = v.x.min(v.y);
            let e = v.x - v.y;
            if e > Fix::ZERO {
                (a, h) = self.new_size(x + e, y, radius, amax);
            } else if e < Fix::ZERO {
                let e = e * (-Fix::ONE);
                (h, a) = self.new_size(y + e, x, radius, amax);
            } else {
                h = y;
                a = x;
            }
            if platform_area(a + m, h + m, radius) > amax {
                let four = Fix::from_int(4);
                let mut q = four * h * radius + four * a * radius + four * h * a
                    + Fix::PI * radius * radius
                    - amax;
                let p = Fix::from_int(8) * radius + four * h + four * a;
                q = q * F_0_25;
                let p = p * F_0_25;
                let root = (p * p * F_0_25 - q).sqrt();
                let tt = (-p) * F_0_5 + root;
                h += tt;
                a += tt;
            } else {
                h += m;
                a += m;
            }
        } else if g.x > Fix::ZERO {
            (a, h) = self.new_size(a, h, radius, amax);
        } else if g.y > Fix::ZERO {
            (h, a) = self.new_size(h, a, radius, amax);
        }
        let shift = Vec2::new((a - x) * self.active_corner.x, (h - y) * self.active_corner.y);
        let shift = shift.complex_mul(t.right);
        if let Some(o) = cx.w.objects.get_mut(plat) {
            o.tf.position += shift;
            if let Kind::Platform(p) = &mut o.kind
                && let Some(sp) = &mut p.spring
            {
                let nh = sp.home + shift;
                sp.set_home(nh);
            }
        }
        AbilityBody::set_pos(cx.w, me, t.position + shift);
        resize_in_world(cx.w, plat, h, a, radius);
        if g.magnitude() > DEADZONE {
            if !self.expanding_width && g.x.abs() > g.y.abs() {
                let an = self.a_width_loop;
                ability_body::play(cx.w, &mut self.anim, an);
                self.expanding_width = true;
            } else if self.expanding_width && g.x.abs() <= g.y.abs() {
                let an = self.a_height_loop;
                ability_body::play(cx.w, &mut self.anim, an);
                self.expanding_width = false;
            }
        }
        if self.has_released || self.time_passed > self.max_size_time {
            self.cursor_visible = false;
            self.state = State::Exit;
            cx.w.emit(Cue::Sound("transform3"));
            let an = self.a_to_exit;
            ability_body::play_then(cx.w, &mut self.anim, an, CB_BEGIN_EJECTION);
            self.exit_time = self.time_passed;
        }
        let ppos = AbilityBody::pos(cx.w, plat);
        AbilityBody::set_pos(cx.w, me, ppos);
    }

    /// `PlatformTransform.ExitAbility(record)`.
    fn component_exit(&mut self, cx: &mut Cx<'_>, mut r: AbilityExitInfo) {
        let me = cx.me;
        self.cursor_visible = false;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        r.grounded_speed = Fix::ZERO;
        r.is_grounded = false;
        r.facing_right = self.input.x >= Fix::ZERO;
        r.position = AbilityBody::pos(cx.w, me);
        r.self_imposed_velocity = self.b.body.self_imposed_velocity;
        r.external_velocity = self.b.body.external_velocity;
        r.ground = None;
        if let Some(p) = cx.w.player_mut(cx.fw.owner) {
            p.in_platform_transform = false;
        }
        cx.exit_body(&mut self.b, r);
    }
}

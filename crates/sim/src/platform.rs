//! Platforms: `StickyRoundedRectangle`, `AnimateVelocity`, `ShakablePlatform`,
//! `AntiLockPlatform`, `VectorFieldPlatform`, `GrowOnStart`, `BoplBody`
//! (physics.md §2.4, §12).

use std::sync::Arc;

use crate::consts::FIXED_DT;
use crate::curve::Curve;
use crate::data::{OrbitDef, PathDef, PlatformDef, SpringDef};
use crate::fix::Fix;
use crate::object::{ObjId, PartKind};
use crate::physics::shapes::ShapeKind;
use crate::vec2::Vec2;
use crate::view::Visual;
use crate::world::{Kind, World};

/// `StickyRoundedRectangle` state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Sticky {
    /// grass 0, snow 1, ice 2, space 3, robot 4, slime 5.
    pub platform_type: i32,
    pub extents: Vec2,
    pub c: [Fix; 8],
    pub total: Fix,
    pub drop_players_for_frames: i32,
    pub drop_except: i32,
    pub drop_force: Fix,
    pub pickup_player_frames: i32,
    pub base_scale: Fix,
}

/// `AnimateVelocity` state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Spring {
    pub def: SpringDef,
    pub home: Vec2,
    pub home_rotation: Fix,
    pub initialized: bool,
    pub in_sudden_death: bool,
    pub sudden_death_stamp: Fix,
    pub random_sudden_death_delay: Fix,
    pub float_this_frame: bool,
    /// Rotation mode (abilities): spin curve, time, speed.
    pub spin: Option<(Arc<Curve>, Fix, Fix)>,
    /// `isBeingControlled` (set by the Push ability; read by black holes).
    pub is_being_controlled: bool,
}

/// `ShakablePlatform` state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Shake {
    pub basic_strength: Fix,
    pub t: Fix,
    pub duration: Fix,
    pub amount: Fix,
    pub priority: i32,
    pub curve: Option<Arc<Curve>>,
    pub last_offset: Vec2,
    pub home: Vec2,
}

/// `AntiLockPlatform` state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PathFollow {
    pub def: PathDef,
    pub target: usize,
}

/// A platform object's payload.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Platform {
    pub sticky: Sticky,
    pub spring: Option<Spring>,
    pub shake: Option<Shake>,
    pub orbit: Option<OrbitDef>,
    pub path: Option<PathFollow>,
    pub grow: Option<Fix>,
    /// `rr.timeSinceLastScaledUp`.
    pub time_since_last_scaled_up: Fix,
    /// `ResizablePlatform` state (Platform ability, abilities/platform.rs).
    pub resize: Option<crate::abilities::platform::ResizeState>,
    /// A `Boulder` (Throw ability; tag `boulder`).
    pub boulder: Option<crate::abilities::throw::BoulderState>,
}

impl Platform {
    /// Creates a level platform object (registration happens in the caller's batch).
    pub fn create(w: &mut World, def: &PlatformDef, index: usize) -> ObjId {
        let p = Platform {
            sticky: Sticky {
                platform_type: def.platform_type,
                base_scale: Fix::ONE,
                drop_except: -1,
                ..Sticky::default()
            },
            spring: def.spring.as_ref().map(|s| Spring {
                def: s.clone(),
                home: def.transform.position,
                home_rotation: s.home_rotation,
                initialized: false,
                in_sudden_death: false,
                sudden_death_stamp: Fix::ZERO,
                random_sudden_death_delay: Fix::ZERO,
                float_this_frame: false,
                spin: None,
                is_being_controlled: false,
            }),
            shake: def.hier_shake.map(|_| Shake {
                basic_strength: def.shake_strength,
                ..Shake::default()
            }),
            orbit: def.orbit,
            path: def.path.clone().map(|d| PathFollow { def: d, target: 0 }),
            grow: def.grow_on_start.map(|g| g.1),
            time_since_last_scaled_up: Fix::ZERO,
            resize: None,
            boulder: None,
        };
        let id = w.objects.create(Kind::Platform(Box::new(p)), def.layer);
        let o = w.objects.get_mut(id).unwrap();
        o.visual = Visual::Platform(index as u16);
        let t = def.transform;
        o.tf.position = t.position;
        o.tf.rotation = t.rotation;
        o.tf.right = t.right;
        o.tf.up = t.up;
        o.tf.offset = t.offset;
        let mut col = def.collider;
        col.body = Some(def.body);
        o.collider = Some(col);
        o.add_part(PartKind::Sticky, def.hier_sticky);
        if let Some(s) = &def.spring {
            let i = o.add_part(PartKind::AnimateVelocity, s.hier);
            o.parts[i].enabled = s.enabled;
        }
        if let Some(h) = def.hier_shake {
            o.add_part(PartKind::Shakable, h);
        }
        o.add_part(PartKind::BoplBody, def.hier_body);
        o.add_part(PartKind::Collider, def.hier_collider);
        if let Some((h, _)) = def.grow_on_start {
            o.add_part(PartKind::GrowOnStart, h);
        }
        if let Some(or) = &def.orbit {
            o.add_part(PartKind::VectorField, or.hier);
        }
        if let Some(pa) = &def.path {
            o.add_part(PartKind::AntiLock, pa.hier);
        }
        // Awake: UpdateBounds with the start extents.
        update_bounds(w, id);
        id
    }

    pub fn init(&mut self, w: &mut World, me: ObjId, k: PartKind) {
        match k {
            PartKind::AnimateVelocity => {
                if let Some(s) = &mut self.spring
                    && !s.initialized
                {
                    let t = w.objects.get(me).map(|o| o.tf).unwrap_or_default();
                    s.initialize(t.position, t.rotation);
                }
            }
            PartKind::Shakable => {
                if let Some(s) = &mut self.shake {
                    s.home = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
                }
            }
            PartKind::GrowOnStart => {
                if let Some(g) = self.grow {
                    // Scale setter (the payload is taken out: apply here directly).
                    put_back_and(w, me, self, |w| w.set_collider_scale(me, g));
                    self.sticky.base_scale = g;
                    w.destroy_part(me, PartKind::GrowOnStart);
                }
            }
            _ => {}
        }
    }

    pub fn update(&mut self, w: &mut World, me: ObjId, k: PartKind, dt: Fix, late: bool) {
        if late {
            return;
        }
        match k {
            PartKind::Sticky => self.sticky_update(w, me, dt),
            PartKind::AnimateVelocity => self.spring_update(w, me, dt),
            PartKind::Shakable => self.shake_update(w, me, dt),
            PartKind::AntiLock => self.antilock_update(w, me),
            PartKind::VectorField => self.orbit_update(w, me),
            PartKind::Resizable => crate::abilities::platform::resizable_update(self, w, me),
            PartKind::Behaviour(_) | PartKind::Bounds => {
                crate::abilities::throw::boulder_update(w, me, self, k)
            }
            _ => {}
        }
    }

    fn sticky_update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        if self.sticky.drop_players_for_frames >= 0 {
            self.sticky.drop_players_for_frames -= 1;
        }
        if self.sticky.pickup_player_frames >= 0 {
            self.sticky.pickup_player_frames -= 1;
        }
        self.time_since_last_scaled_up += dt * w.time.player_time_scale;
        let scale = w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale);
        let base = self.sticky.base_scale;
        if (self.time_since_last_scaled_up > Fix::from_int(10) || w.round.sudden_death)
            && scale > base
            && !w.is_time_stopped()
        {
            let s = scale.lerp(base, dt);
            put_back_and(w, me, self, |w| w.set_collider_scale(me, s));
        }
        put_back_and(w, me, self, |w| update_bounds(w, me));
    }

    fn spring_update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        let Some(s) = &mut self.spring else {
            return;
        };
        if w.round.sudden_death && !s.in_sudden_death {
            s.def.home_is_movable = true;
            s.in_sudden_death = true;
            s.sudden_death_stamp = w.round.sudden_death_stamp;
            s.random_sudden_death_delay = w
                .rng
                .random_fix(Fix::ZERO, s.def.max_random_sudden_death_delay);
        }
        let dtp = dt * w.time.player_time_scale;
        if s.in_sudden_death {
            let e = w.time.sim_time_since_level_loaded - s.sudden_death_stamp;
            let k = (e * s.def.sudden_death_acceleration).to_int().min(5);
            let c = s
                .def
                .sudden_death_curve
                .eval(e - s.random_sudden_death_delay);
            s.home += Vec2::DOWN * c * (s.def.sudden_death_speed * Fix::from_int(2 << k)) * dtp;
            if s.def.speed <= Fix::ZERO
                && let Some(b) = w.physics.body_mut(me)
                && b.gravity_scale == Fix::ZERO
            {
                b.gravity_scale = Fix::ONE;
                if let Some(ob) = w.physics.own_body_mut(me) {
                    ob.gravity_scale = Fix::ONE;
                }
            }
        }
        let pos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        let Some(body) = w.physics.body(me) else {
            return;
        };
        let mut f = s.def.speed * (s.home - pos) - s.def.mu * body.velocity;
        if s.float_this_frame {
            f = (-s.def.mu) * body.velocity;
            s.home = pos;
        }
        let imp = (f * dtp / body.inverse_mass) * Fix::from_int(60);
        w.physics.add_force(me, imp);
        let r0 = s.home_rotation;
        if let Some((curve, t, speed)) = &mut s.spin {
            let pts = w.time.player_time_scale;
            s.home_rotation += pts * dtp * (*speed * curve.eval(*t)) / s.def.rotation_resistence;
            s.home_rotation =
                ((s.home_rotation % Fix::PI_TIMES_2) + Fix::PI_TIMES_2) % Fix::PI_TIMES_2;
            *t += pts * dtp;
            if *t >= curve.last_time() {
                s.spin = None;
            }
        }
        if s.def.interpolate_rotation_home
            && let Some(b) = w.physics.body_mut(me)
        {
            let o = b.orientation;
            let mut d = r0 - o;
            if d.abs() > Fix::PI {
                d = -d.sign_fix() * (Fix::PI_TIMES_2 - d.abs());
            }
            b.angular_velocity = (b.angular_velocity + d * s.def.rotation_speed * dtp)
                * (Fix::ONE - s.def.rotation_mu);
        }
        let lim = Fix::from_int(2500);
        for c in [&mut s.home.x, &mut s.home.y] {
            if *c > lim {
                *c = lim;
            } else if *c < -lim {
                *c = -lim;
            }
        }
        s.float_this_frame = false;
    }

    fn shake_update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        let Some(s) = &mut self.shake else {
            return;
        };
        if s.t < s.duration {
            s.t += dt * w.time.player_time_scale;
            let amt = match &s.curve {
                Some(c) => c.eval(s.t) * s.amount,
                None => s.amount,
            };
            let off = w.rng.random_inside_unit_circle() * amt;
            if let Some(o) = w.objects.get_mut(me) {
                o.tf.position = o.tf.position - s.last_offset + off;
            }
            s.last_offset = off;
            if s.t >= s.duration {
                s.priority = 0;
                s.t = Fix::ZERO;
                s.duration = Fix::ZERO;
                s.amount = Fix::ZERO;
                s.curve = None;
            }
        }
    }

    fn antilock_update(&mut self, w: &mut World, me: ObjId) {
        let Some(p) = &mut self.path else {
            return;
        };
        if w.is_time_stopped()
            || w.time.sim_time_since_level_loaded < p.def.delay
            || w.round.sudden_death
            || p.def.points.is_empty()
        {
            return;
        }
        let pos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        let n = p.def.points.len();
        let target = p.def.points[p.target % n];
        w.physics
            .add_force(me, (target - pos).normalized_safe() * p.def.force);
        if let Some(s) = &mut self.spring {
            s.set_home(pos);
        }
        let prev = p.def.points[(p.target + n - 1) % n];
        if (target - prev).dot(pos - target) > Fix::ZERO {
            p.target = (p.target + 1) % n;
        }
    }

    fn orbit_update(&mut self, w: &mut World, me: ObjId) {
        let Some(d) = self.orbit else {
            return;
        };
        if w.is_time_stopped()
            || w.time.sim_time_since_level_loaded < d.delay
            || w.round.sudden_death
        {
            return;
        }
        let h = FIXED_DT * w.time.player_time_scale;
        let p = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        let Some(body) = w.physics.body(me) else {
            return;
        };
        let v = body.velocity;
        let r = p - d.center;
        let n = r.normalized_safe();
        let t = n.mul_by_neg_i();
        let mut vt = t.dot(v);
        let mut vn = n.dot(v);
        vn += n.dot(n * d.target_radius + d.center - p) * h * d.expand_speed;
        let (disp, new_vel);
        if r.sqr_magnitude() < d.dead_zone {
            new_vel = n * vn + t * vt;
            disp = Vec2::ZERO;
        } else {
            vt = vt.lerp(d.orbit_speed, h * d.orbit_accel);
            let x;
            if vt.sign() == d.orbit_speed.sign() {
                if vt.abs() >= d.orbit_speed.abs() {
                    x = vt - d.orbit_speed;
                    vt = vt - x;
                } else {
                    x = Fix::ZERO;
                }
            } else {
                x = vt;
                vt = Fix::ZERO;
            }
            let n2 = (p + t * vt * h - d.center).normalized_safe();
            let t2 = n2.mul_by_neg_i();
            let nv = t2 * vt;
            disp = n2 * r.magnitude() - nv * FIXED_DT - r;
            new_vel = nv + n2 * vn * d.normal_speed_friction + x * t;
        }
        if let Some(o) = w.objects.get_mut(me) {
            o.tf.position = o.tf.position + disp;
        }
        if let Some(b) = w.physics.body_mut(me) {
            b.velocity = new_vel;
        }
        let np = p + disp;
        if let Some(s) = &mut self.spring {
            s.set_home(np);
        }
    }
}

impl Spring {
    /// `Initialize(position, rotation)`.
    pub fn initialize(&mut self, pos: Vec2, rot: Fix) {
        self.initialized = true;
        self.home = pos;
        self.home_rotation = ((rot % Fix::PI_TIMES_2) + Fix::PI_TIMES_2) % Fix::PI_TIMES_2;
        self.is_being_controlled = false;
        self.in_sudden_death = false;
    }

    /// The `HomePosition` setter (ignored unless `HomeIsMovable`).
    pub fn set_home(&mut self, p: Vec2) {
        if self.def.home_is_movable {
            self.home = p;
        }
    }
}

/// Runs `f` with the platform payload temporarily put back (so world helpers
/// that inspect the platform see it), then takes it out again.
fn put_back_and(w: &mut World, me: ObjId, p: &mut Platform, f: impl FnOnce(&mut World)) {
    let taken = std::mem::replace(p, placeholder());
    w.put_kind(me, Kind::Platform(Box::new(taken)));
    f(w);
    if let Kind::Platform(b) = w.take_kind(me) {
        *p = *b;
    }
}

fn placeholder() -> Platform {
    Platform {
        sticky: Sticky::default(),
        spring: None,
        shake: None,
        orbit: None,
        path: None,
        grow: None,
        time_since_last_scaled_up: Fix::ZERO,
        resize: None,
        boulder: None,
    }
}

/// `UpdateBounds()`: extents and the perimeter table.
pub fn update_bounds(w: &mut World, me: ObjId) {
    let Some(o) = w.objects.get(me) else {
        return;
    };
    let Some(def) = o.collider else {
        return;
    };
    let (ext, radius) = match (def.initialized, w.physics.rr(me)) {
        (true, Some(r)) => (
            Vec2::new(Fix::ONE / r.b.ie.x, Fix::ONE / r.b.ie.y),
            r.radius,
        ),
        _ => (def.start_extents, def.start_radius),
    };
    debug_assert!(def.kind == ShapeKind::RoundedRect);
    if let Some(Kind::Platform(p)) = w.objects.get_mut(me).map(|o| &mut o.kind) {
        let s = &mut p.sticky;
        s.extents = ext;
        let ex = ext.x * Fix::TWO;
        let ey = ext.y * Fix::TWO;
        let cr = radius * Fix::PI_OVER_2;
        s.c = [ex, cr, ey, cr, ex, cr, ey, cr];
        s.total = s.c[0] * Fix::TWO + s.c[1] * Fix::from_int(4) + s.c[2] * Fix::TWO;
    }
}

/// `AmIdropped(id)`: the drop force while players are being dropped.
pub fn am_i_dropped(w: &World, plat: ObjId, id: i32) -> Option<Fix> {
    match &w.objects.get(plat)?.kind {
        Kind::Platform(p) if p.sticky.drop_players_for_frames > 0 && p.sticky.drop_except != id => {
            Some(p.sticky.drop_force)
        }
        _ => None,
    }
}

/// `DropAllAttachedPlayers(exceptId, force)`.
pub fn drop_all_attached_players(w: &mut World, plat: ObjId, except: i32, force: Fix) {
    if let Some(Kind::Platform(p)) = w.objects.get_mut(plat).map(|o| &mut o.kind) {
        p.sticky.drop_players_for_frames = 2;
        p.sticky.drop_except = except;
        p.sticky.drop_force = force;
    }
}

/// `pickUpPlayersInsteadOfSquish`.
pub fn picks_up_players(w: &World, plat: ObjId) -> bool {
    matches!(w.objects.get(plat).map(|o| &o.kind), Some(Kind::Platform(p)) if p.sticky.pickup_player_frames > 0)
}

/// `AddShake(duration, amount, priority, curve)`.
pub fn add_shake(
    w: &mut World,
    plat: ObjId,
    duration: Fix,
    amount: Fix,
    priority: i32,
    curve: Option<Arc<Curve>>,
) {
    if let Some(Kind::Platform(p)) = w.objects.get_mut(plat).map(|o| &mut o.kind)
        && let Some(s) = &mut p.shake
    {
        s.duration = duration;
        s.amount = amount;
        s.priority = priority;
        s.curve = curve;
        s.t = Fix::ZERO;
    }
}

//! DetPhysics: the world lists, `Simulate`, collision response and queries
//! (physics.md). Narrow phase and ray casts are in [`shapes`].

pub mod ropes;
pub mod shapes;

use std::cmp::Ordering;

use crate::consts::{F_0_001, FIXED_DT};
use crate::fix::Fix;
use crate::object::{ObjId, Objects, PartKind};
use crate::sort::introsort;
use crate::vec2::Vec2;
use shapes::*;

/// Per-scene DetPhysics parameters (physics.md §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PhysParams {
    pub gravity: Fix,
    pub rope_gravity: Fix,
    pub rope_iterations: i32,
    pub rope_player_mass01: Fix,
    pub rope_segment_radius: Fix,
    pub player_gravity: Fix,
    pub platform_stretchiness: Fix,
    pub max_stretch_length: Fix,
    pub grounded_player_rope_str: Fix,
    pub max_beam_distance: Fix,
    pub beam_width: Fix,
    pub beam_parent_offset: Fix,
    pub cross_beam_offset: Fix,
    pub beam_platform_push_force: Fix,
    pub beam_black_hole_push_force: Fix,
    pub beam_rigid_body_push_force: Fix,
    pub beam_hit_mask: u32,
}

impl Default for PhysParams {
    fn default() -> PhysParams {
        PhysParams {
            gravity: Fix(-171_798_691_840),
            rope_gravity: Fix(8_589_934_592),
            rope_iterations: 7,
            rope_player_mass01: Fix(3_865_470_464),
            rope_segment_radius: Fix(429_496_736),
            player_gravity: Fix(42_949_672_960),
            platform_stretchiness: Fix(171_798_688),
            max_stretch_length: Fix(429_496_729_600),
            grounded_player_rope_str: Fix(42_949_672_960),
            max_beam_distance: Fix(1_288_490_188_800),
            beam_width: Fix(8_589_934_592),
            beam_parent_offset: Fix(-10_737_418_240),
            cross_beam_offset: Fix(-6_442_450_944),
            beam_platform_push_force: Fix(6_442_450_944_000),
            beam_black_hole_push_force: Fix(4_294_967_296),
            beam_rigid_body_push_force: Fix(4_294_967_296_000),
            beam_hit_mask: 77_662_464,
        }
    }
}

/// `BoplBody` serialized parameters (physics.md §2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BodyDef {
    pub mass: Fix,
    pub bounciness: Fix,
    pub friction: Fix,
    pub dynamic_friction: Fix,
    pub moment_of_inertia: Fix,
    pub gravity_scale: Fix,
    pub air_friction01: Fix,
    pub rotational_drag01: Fix,
    pub start_velocity: Vec2,
    pub start_angular_velocity: Fix,
}

impl Default for BodyDef {
    fn default() -> BodyDef {
        BodyDef {
            mass: Fix::ONE,
            bounciness: Fix(2_147_483_648),
            friction: Fix(1_288_490_240),
            dynamic_friction: Fix(1_717_986_944),
            moment_of_inertia: Fix::ONE,
            gravity_scale: Fix::ONE,
            air_friction01: Fix::ONE,
            rotational_drag01: Fix::ONE,
            start_velocity: Vec2::ZERO,
            start_angular_velocity: Fix::ZERO,
        }
    }
}

impl BodyDef {
    /// `NewPhysicsBody(orientation)`.
    pub fn new_body(&self, orientation: Fix) -> Body {
        Body {
            inverse_mass: if self.mass.0 == 0 {
                Fix::ZERO
            } else {
                Fix::ONE / self.mass
            },
            inverse_moi: if self.moment_of_inertia.0 == 0 {
                Fix::ZERO
            } else {
                Fix::ONE / self.moment_of_inertia
            },
            velocity: self.start_velocity,
            angular_velocity: self.start_angular_velocity,
            bounciness: self.bounciness,
            static_friction: self.friction,
            dynamic_friction: self.dynamic_friction,
            air_drag: self.air_friction01,
            rotational_drag: self.rotational_drag01,
            gravity_scale: self.gravity_scale,
            orientation,
            ..Body::default()
        }
    }
}

/// `PhysicsBody`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Body {
    pub inverse_mass: Fix,
    pub velocity: Vec2,
    pub force: Vec2,
    pub angular_velocity: Fix,
    pub orientation: Fix,
    pub inverse_moi: Fix,
    pub bounciness: Fix,
    pub static_friction: Fix,
    pub dynamic_friction: Fix,
    pub gravity_scale: Fix,
    pub air_drag: Fix,
    pub rotational_drag: Fix,
    /// Composite parent (index into the composite pool).
    pub composite: Option<u16>,
    pub relative_pos: Vec2,
    pub relative_orientation: Fix,
}

impl Body {
    /// `IsTrigger()`.
    #[inline]
    pub fn is_trigger(&self) -> bool {
        self.inverse_mass.0 < 0
    }

    /// A trigger body (colliders without a `BoplBody`).
    pub fn trigger(orientation: Fix) -> Body {
        Body {
            inverse_mass: -Fix::ONE,
            orientation,
            ..Body::default()
        }
    }

    /// §4.2 integration; returns the displacement.
    fn integrate(&mut self, dt: Fix, gravity: Fix) -> Vec2 {
        let g = Vec2::new(dt * Fix::ZERO, dt * (gravity * self.gravity_scale));
        let f = Vec2::new(
            FIXED_DT * self.force.x * self.inverse_mass,
            FIXED_DT * self.force.y * self.inverse_mass,
        );
        self.velocity = self.velocity + g + f;
        self.orientation = self.orientation + dt * self.angular_velocity;
        self.orientation =
            ((self.orientation % Fix::PI_TIMES_2) + Fix::PI_TIMES_2) % Fix::PI_TIMES_2;
        if dt.0 != 0 {
            self.velocity = self.velocity * self.air_drag;
            self.angular_velocity = self.angular_velocity * self.rotational_drag;
        }
        self.force = Vec2::ZERO;
        dt * self.velocity
    }
}

/// Serialized collider parameters plus world registration state, stored on
/// the object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColliderDef {
    pub kind: ShapeKind,
    /// `startExtents` (box, rounded rect).
    pub start_extents: Vec2,
    /// `startRadius` (circle, rounded rect).
    pub start_radius: Fix,
    pub minimum_extents: Fix,
    pub min_scale: Fix,
    pub max_scale: Fix,
    pub manual_init: bool,
    /// The object has a `BoplBody` (else the collider is a trigger).
    pub body: Option<BodyDef>,
    /// Registered in the world lists.
    pub initialized: bool,
}

/// `PhysicsParent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Parent {
    pub obj: ObjId,
    pub hierarchy: i32,
}

/// One world list (parallel arrays).
#[derive(Clone, Debug, Default)]
pub struct List<S> {
    pub shapes: Vec<S>,
    pub bodies: Vec<Body>,
    pub parents: Vec<Parent>,
}

impl<S: Copy> List<S> {
    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    pub fn index_of(&self, obj: ObjId) -> Option<usize> {
        self.parents.iter().position(|p| p.obj == obj)
    }

    fn push(&mut self, s: S, b: Body, p: Parent) {
        self.shapes.push(s);
        self.bodies.push(b);
        self.parents.push(p);
    }

    fn remove(&mut self, i: usize) {
        self.shapes.remove(i);
        self.bodies.remove(i);
        self.parents.remove(i);
    }
}

/// A composite body (physics.md §10).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Composite {
    pub destroyed: bool,
    pub body: Body,
    pub position: Vec2,
    pub orientation: Fix,
    /// Member objects: rounded rects first in insertion order, then others.
    pub rrs: Vec<ObjId>,
    pub boxes: Vec<ObjId>,
    pub circles: Vec<ObjId>,
}

/// The other collider of a contact, as captured for the callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapeSnap {
    Circle(Circle),
    Box(BoxShape),
    RoundedRect(RoundedRect),
}

impl ShapeSnap {
    pub fn contains(&self, p: Vec2) -> bool {
        match self {
            ShapeSnap::Circle(c) => c.contains(p),
            ShapeSnap::Box(b) => b.contains(p),
            ShapeSnap::RoundedRect(r) => r.contains(p),
        }
    }
}

/// `CollisionInformation` delivered to a collider's listeners.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Contact {
    /// The receiving object.
    pub me: ObjId,
    /// The other object (`colliderPP`).
    pub other: ObjId,
    /// The other object's collider hierarchy number.
    pub other_hier: i32,
    /// The receiving collider's hierarchy number.
    pub my_hier: i32,
    pub collider: ShapeSnap,
    pub layer: i32,
    pub impact_velocity: Vec2,
    pub contact_point: Vec2,
    pub normal: Vec2,
    pub penetration: Fix,
}

/// A ray-cast result (`RaycastInformation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RayHit {
    pub near_pos: Vec2,
    pub far_pos: Vec2,
    pub near_dist: Fix,
    pub far_dist: Fix,
    pub layer: i32,
    pub obj: ObjId,
    pub hierarchy: i32,
    pub kind: ShapeKind,
}

impl RayHit {
    pub const NONE: RayHit = RayHit {
        near_pos: Vec2::ZERO,
        far_pos: Vec2::ZERO,
        near_dist: Fix::MAX,
        far_dist: Fix::ZERO,
        layer: 0,
        obj: ObjId::NONE,
        hierarchy: 0,
        kind: ShapeKind::RoundedRect,
    };

    pub fn hit(&self) -> bool {
        self.near_dist != Fix::MAX
    }
}

/// The physics world.
#[derive(Clone, Debug, Default)]
pub struct Physics {
    pub params: PhysParams,
    pub circles: List<Circle>,
    pub boxes: List<BoxShape>,
    pub rrs: List<RoundedRect>,
    pub composites: Vec<Composite>,
    /// Layer collision matrix (word `i` bit `j` = layers collide).
    pub matrix: [u32; 32],
    /// The rope pool (§11.1).
    pub ropes: ropes::Ropes,
}

/// Activity of a collider (physics.md §3.1).
fn is_active(objs: &Objects, obj: ObjId) -> bool {
    match objs.get(obj) {
        None => false,
        Some(o) => {
            o.part(PartKind::Collider).is_some_and(|p| p.enabled) && objs.active_in_hierarchy(obj)
        }
    }
}

fn collider_destroyed(objs: &Objects, obj: ObjId) -> bool {
    match objs.get(obj) {
        None => true,
        Some(o) => o.part(PartKind::Collider).is_none_or(|p| p.destroyed),
    }
}

/// The broad-phase pair lists of one `Simulate` (between its two halves).
pub struct BroadPhase([(PairKind, Vec<(usize, usize)>); 6]);

#[derive(Clone, Copy)]
enum PairKind {
    Rr,
    Br,
    Cr,
    Cc,
    Cb,
    Bb,
}

impl Physics {
    /// Whether two layers collide.
    #[inline]
    pub fn layers_collide(&self, a: i32, b: i32) -> bool {
        if !(0..32).contains(&a) || !(0..32).contains(&b) {
            return false;
        }
        self.matrix[a as usize] >> b & 1 != 0
    }

    pub fn clear(&mut self) {
        let params = self.params;
        let matrix = self.matrix;
        // The rope counter is a static of the original (never reset).
        let counter = self.ropes.counter;
        *self = Physics {
            params,
            matrix,
            ..Physics::default()
        };
        self.ropes.counter = counter;
    }

    // -- registration -------------------------------------------------------

    /// Registers a collider (`Init` / `ManualInit`, physics.md §2.2).
    pub fn register(&mut self, objs: &mut Objects, obj: ObjId, hierarchy: i32) {
        let Some(o) = objs.get_mut(obj) else {
            return;
        };
        let Some(def) = o.collider.as_mut() else {
            return;
        };
        if def.initialized {
            return;
        }
        def.initialized = true;
        let def = *def;
        let t = o.tf;
        let layer = o.layer as i32;
        let parent = Parent { obj, hierarchy };
        let body = match def.body {
            Some(b) => b.new_body(t.rotation),
            None => Body::trigger(t.rotation),
        };
        match def.kind {
            ShapeKind::Circle => {
                let c = Circle {
                    center: t.position + t.offset.complex_mul(t.right),
                    radius: def.start_radius,
                    layer,
                };
                self.circles.push(c, body, parent);
            }
            ShapeKind::Box => {
                let mut e = def.start_extents;
                if e.x.0 == 0 {
                    e.x = def.minimum_extents;
                }
                if e.y.0 == 0 {
                    e.y = def.minimum_extents;
                }
                let b = BoxShape {
                    center: t.position + t.offset.complex_mul(t.right),
                    right: t.right * e.x,
                    up: t.up * e.y,
                    ie: Vec2::new((Fix::ONE / e.x).abs(), (Fix::ONE / e.y).abs()),
                    layer,
                };
                self.boxes.push(b, body, parent);
            }
            ShapeKind::RoundedRect | ShapeKind::Rope => {
                let e = def.start_extents;
                let inv = |v: Fix| {
                    if v.0 == 0 {
                        Fix::ZERO
                    } else {
                        (Fix::ONE / v).abs()
                    }
                };
                let b = BoxShape {
                    center: t.position + t.offset.complex_mul(t.right),
                    right: t.right * e.x,
                    up: t.up * e.y,
                    ie: Vec2::new(inv(e.x), inv(e.y)),
                    layer,
                };
                self.rrs.push(
                    RoundedRect {
                        b,
                        radius: def.start_radius,
                        layer,
                    },
                    body,
                    parent,
                );
            }
        }
    }

    /// `RemoveDestroyedColliders()` (§3.2).
    pub fn remove_destroyed(&mut self, objs: &Objects) {
        let mut i = 0;
        while i < self.circles.len() {
            let o = self.circles.parents[i].obj;
            if collider_destroyed(objs, o) {
                self.detach_member(objs, ShapeKind::Circle, i);
                self.circles.remove(i);
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.rrs.len() {
            let o = self.rrs.parents[i].obj;
            if collider_destroyed(objs, o) {
                if let Some(ci) = self.rrs.bodies[i].composite {
                    self.dissolve_composite(objs, ci as usize, o);
                    // The list may have shifted; find the rr again.
                    if let Some(j) = self.rrs.index_of(o) {
                        self.rrs.remove(j);
                    }
                    i = 0;
                    continue;
                }
                self.rrs.remove(i);
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.boxes.len() {
            let o = self.boxes.parents[i].obj;
            if collider_destroyed(objs, o) {
                self.detach_member(objs, ShapeKind::Box, i);
                self.boxes.remove(i);
            } else {
                i += 1;
            }
        }
    }

    // -- accessors used by game code (§2.3) ---------------------------------

    /// The body of an object's collider (or its live composite's body).
    pub fn body(&self, obj: ObjId) -> Option<Body> {
        let (k, i) = self.find(obj)?;
        let b = *self.body_at(k, i);
        Some(match b.composite {
            Some(ci) if !self.composites[ci as usize].destroyed => {
                self.composites[ci as usize].body
            }
            _ => b,
        })
    }

    /// Mutable body (redirected to the composite when it is a live member).
    pub fn body_mut(&mut self, obj: ObjId) -> Option<&mut Body> {
        let (k, i) = self.find(obj)?;
        let c = self.body_at(k, i).composite;
        match c {
            Some(ci) if !self.composites[ci as usize].destroyed => {
                Some(&mut self.composites[ci as usize].body)
            }
            _ => Some(self.body_at_mut(k, i)),
        }
    }

    /// The object's own body, never redirected.
    pub fn own_body_mut(&mut self, obj: ObjId) -> Option<&mut Body> {
        let (k, i) = self.find(obj)?;
        Some(self.body_at_mut(k, i))
    }

    /// Finds a collider by object.
    pub fn find(&self, obj: ObjId) -> Option<(ShapeKind, usize)> {
        if let Some(i) = self.rrs.index_of(obj) {
            return Some((ShapeKind::RoundedRect, i));
        }
        if let Some(i) = self.boxes.index_of(obj) {
            return Some((ShapeKind::Box, i));
        }
        self.circles.index_of(obj).map(|i| (ShapeKind::Circle, i))
    }

    fn body_at(&self, k: ShapeKind, i: usize) -> &Body {
        match k {
            ShapeKind::Circle => &self.circles.bodies[i],
            ShapeKind::Box => &self.boxes.bodies[i],
            _ => &self.rrs.bodies[i],
        }
    }

    fn body_at_mut(&mut self, k: ShapeKind, i: usize) -> &mut Body {
        match k {
            ShapeKind::Circle => &mut self.circles.bodies[i],
            ShapeKind::Box => &mut self.boxes.bodies[i],
            _ => &mut self.rrs.bodies[i],
        }
    }

    /// The registered rounded rect of an object.
    pub fn rr(&self, obj: ObjId) -> Option<&RoundedRect> {
        self.rrs.index_of(obj).map(|i| &self.rrs.shapes[i])
    }

    pub fn rr_mut(&mut self, obj: ObjId) -> Option<&mut RoundedRect> {
        self.rrs.index_of(obj).map(|i| &mut self.rrs.shapes[i])
    }

    pub fn box_shape(&self, obj: ObjId) -> Option<&BoxShape> {
        self.boxes.index_of(obj).map(|i| &self.boxes.shapes[i])
    }

    pub fn box_mut(&mut self, obj: ObjId) -> Option<&mut BoxShape> {
        self.boxes.index_of(obj).map(|i| &mut self.boxes.shapes[i])
    }

    pub fn circle(&self, obj: ObjId) -> Option<&Circle> {
        self.circles.index_of(obj).map(|i| &self.circles.shapes[i])
    }

    pub fn circle_mut(&mut self, obj: ObjId) -> Option<&mut Circle> {
        self.circles
            .index_of(obj)
            .map(|i| &mut self.circles.shapes[i])
    }

    /// `AddForce(f)` on the body or composite.
    pub fn add_force(&mut self, obj: ObjId, f: Vec2) {
        if let Some(b) = self.body_mut(obj) {
            b.force += f;
        }
    }

    /// `ApplyImpulse` with a unit lever (§8.2); composite aware.
    pub fn apply_impulse(&mut self, obj: ObjId, imp: Vec2, lever: Vec2) {
        if let Some(b) = self.body_mut(obj) {
            apply_impulse(b, imp, lever);
        }
    }

    /// Copies the transform into the collider (`UpdatePhysicsPositions`).
    pub fn update_physics_positions(&mut self, objs: &Objects, obj: ObjId) {
        let Some((k, i)) = self.find(obj) else {
            return;
        };
        if let Some(ci) = self.body_at(k, i).composite
            && !self.composites[ci as usize].destroyed
        {
            self.sync_composite(objs, ci as usize);
            return;
        }
        let Some(o) = objs.get(obj) else {
            return;
        };
        let t = o.tf;
        match k {
            ShapeKind::Circle => {
                self.circles.shapes[i].center = t.position + t.right.complex_mul(t.offset);
                self.circles.bodies[i].orientation = t.rotation;
            }
            ShapeKind::Box => {
                let s = &mut self.boxes.shapes[i];
                s.set_rotation(t.rotation);
                s.center = t.position + t.right.complex_mul(t.offset);
                self.boxes.bodies[i].orientation = t.rotation;
            }
            _ => {
                let s = &mut self.rrs.shapes[i];
                s.b.set_rotation(t.rotation);
                s.b.center = t.position;
                self.rrs.bodies[i].orientation = t.rotation;
            }
        }
    }

    // -- composites (§10) ---------------------------------------------------

    fn sync_composite(&mut self, objs: &Objects, ci: usize) {
        let c = &self.composites[ci];
        if c.destroyed || c.rrs.is_empty() {
            return;
        }
        let first = c.rrs[0];
        let Some(t) = objs.get(first).map(|o| o.tf) else {
            return;
        };
        self.composites[ci].position = t.position;
        self.composites[ci].orientation = t.rotation;
        if let Some(i) = self.rrs.index_of(first) {
            self.rrs.shapes[i].b.set_rotation(t.rotation);
            self.rrs.shapes[i].b.center = t.position;
            self.rrs.bodies[i].orientation = t.rotation;
        }
        let pos = self.composites[ci].position;
        let members: Vec<(ShapeKind, ObjId)> = self.composites[ci]
            .boxes
            .iter()
            .map(|o| (ShapeKind::Box, *o))
            .chain(
                self.composites[ci]
                    .circles
                    .iter()
                    .map(|o| (ShapeKind::Circle, *o)),
            )
            .collect();
        for (k, o) in members {
            match k {
                ShapeKind::Box => {
                    if let Some(i) = self.boxes.index_of(o) {
                        let b = self.boxes.bodies[i];
                        self.boxes.shapes[i].center = pos + b.relative_pos.complex_mul(t.right);
                        let ori = t.rotation + b.relative_orientation;
                        self.boxes.bodies[i].orientation = ori;
                        self.boxes.shapes[i].set_rotation(ori);
                    }
                }
                _ => {
                    if let Some(i) = self.circles.index_of(o) {
                        let b = self.circles.bodies[i];
                        self.circles.shapes[i].center = pos + b.relative_pos.complex_mul(t.right);
                        self.circles.bodies[i].orientation = t.rotation + b.relative_orientation;
                    }
                }
            }
        }
    }

    /// `AddBoxToRR` / `AddCircleToRR`.
    pub fn weld_to_rr(&mut self, objs: &Objects, small: ObjId, rr_obj: ObjId) {
        let Some((sk, si)) = self.find(small) else {
            return;
        };
        let Some(ri) = self.rrs.index_of(rr_obj) else {
            return;
        };
        let existing = self.rrs.bodies[ri]
            .composite
            .filter(|c| !self.composites[*c as usize].destroyed);
        let ci = match existing {
            Some(c) => c as usize,
            None => {
                let c = match self.composites.iter().position(|c| c.destroyed) {
                    Some(c) => {
                        self.composites[c] = Composite::default();
                        c
                    }
                    None => {
                        self.composites.push(Composite::default());
                        self.composites.len() - 1
                    }
                };
                let big = self.rrs.bodies[ri];
                let bigpos = self.rrs.shapes[ri].b.center;
                let comp = &mut self.composites[c];
                comp.orientation = big.orientation;
                comp.position = bigpos;
                comp.body = big;
                comp.body.composite = None;
                comp.body.force = big.force;
                comp.rrs.push(rr_obj);
                self.rrs.bodies[ri].relative_orientation = Fix::ZERO;
                self.rrs.bodies[ri].relative_pos = Vec2::ZERO;
                self.rrs.bodies[ri].composite = Some(c as u16);
                self.rrs.bodies[ri].force = Vec2::ZERO;
                c
            }
        };
        let small_body = *self.body_at(sk, si);
        let small_pos = match sk {
            ShapeKind::Circle => self.circles.shapes[si].center,
            _ => self.boxes.shapes[si].center,
        };
        let comp = &mut self.composites[ci];
        let big = comp.body;
        let rel_ori = small_body.orientation - big.orientation;
        let rel_pos = (small_pos - comp.position).complex_mul(Vec2::from_angle(-comp.orientation));
        let inv_m = Fix::ONE / (Fix::ONE / small_body.inverse_mass + Fix::ONE / big.inverse_mass);
        let inv_i = Fix::ONE / (Fix::ONE / small_body.inverse_moi + Fix::ONE / big.inverse_moi);
        comp.body.inverse_mass = inv_m;
        comp.body.inverse_moi = inv_i;
        match sk {
            ShapeKind::Circle => comp.circles.push(small),
            _ => comp.boxes.push(small),
        }
        let b = self.body_at_mut(sk, si);
        b.relative_orientation = rel_ori;
        b.relative_pos = rel_pos;
        b.composite = Some(ci as u16);
        b.force = Vec2::ZERO;
        let _ = objs;
    }

    /// `RemoveCircleFromComposite` / `RemoveBoxFromComposite` (§10.4).
    pub fn remove_from_composite(&mut self, objs: &Objects, obj: ObjId) {
        if let Some((k, i)) = self.find(obj)
            && k != ShapeKind::RoundedRect
        {
            self.detach_member(objs, k, i);
        }
    }

    /// Removing a circle/box member (§10.4): re-sync, then the member leaves.
    fn detach_member(&mut self, objs: &Objects, k: ShapeKind, i: usize) {
        let b = *self.body_at(k, i);
        let Some(ci) = b.composite else {
            return;
        };
        let ci = ci as usize;
        if self.composites[ci].destroyed {
            return;
        }
        self.sync_composite(objs, ci);
        let obj = match k {
            ShapeKind::Circle => self.circles.parents[i].obj,
            _ => self.boxes.parents[i].obj,
        };
        let comp = &mut self.composites[ci];
        comp.boxes.retain(|o| *o != obj);
        comp.circles.retain(|o| *o != obj);
        self.body_at_mut(k, i).composite = None;
        let comp = &self.composites[ci];
        if comp.boxes.is_empty() && comp.circles.is_empty() {
            // Only the rounded rect remains: it leaves composite mode.
            let vel = comp.body.velocity;
            let ang = comp.body.angular_velocity;
            for r in comp.rrs.clone() {
                if let Some(j) = self.rrs.index_of(r) {
                    self.rrs.bodies[j].composite = None;
                    self.rrs.bodies[j].velocity = vel;
                    self.rrs.bodies[j].angular_velocity = ang;
                }
            }
            self.composites[ci].destroyed = true;
        }
    }

    /// Removing a rounded rect that carries a composite: its welded members
    /// leave the world (§3.2).
    fn dissolve_composite(&mut self, _objs: &Objects, ci: usize, _rr: ObjId) {
        let comp = std::mem::take(&mut self.composites[ci]);
        for o in comp.boxes {
            if let Some(j) = self.boxes.index_of(o) {
                self.boxes.remove(j);
            }
        }
        for o in comp.circles {
            if let Some(j) = self.circles.index_of(o) {
                self.circles.remove(j);
            }
        }
        for o in comp.rrs {
            if let Some(j) = self.rrs.index_of(o) {
                self.rrs.bodies[j].composite = None;
            }
        }
        self.composites[ci].destroyed = true;
    }

    // -- Simulate (§4) ------------------------------------------------------

    /// `Simulate(dt)`; returns the prepared callbacks in order.
    pub fn simulate(&mut self, objs: &mut Objects, dt: Fix, time_stopped: bool) -> Vec<Contact> {
        match self.simulate_begin(objs, dt, time_stopped) {
            Some(bp) => self.simulate_end(objs, bp),
            None => Vec::new(),
        }
    }

    /// Steps 1–7 of `Simulate` (up to the broad phase). `None` if all lists
    /// are empty (nothing else happens that tick: no ropes, no beams).
    /// Ropes and beams (step 8) run between this and [`Physics::simulate_end`].
    pub fn simulate_begin(
        &mut self,
        objs: &mut Objects,
        dt: Fix,
        time_stopped: bool,
    ) -> Option<BroadPhase> {
        self.remove_destroyed(objs);
        self.sync_in(objs);
        if self.circles.is_empty() && self.boxes.is_empty() && self.rrs.is_empty() {
            return None;
        }
        let dt = if time_stopped { Fix::ZERO } else { dt };
        let g = self.params.gravity;
        for i in 0..self.rrs.len() {
            let b = &mut self.rrs.bodies[i];
            if b.inverse_mass > Fix::ZERO
                && b.composite.is_none()
                && is_active(objs, self.rrs.parents[i].obj)
            {
                let d = b.integrate(dt, g);
                let o = b.orientation;
                let s = &mut self.rrs.shapes[i];
                s.b.center += d;
                s.b.set_rotation(o);
            }
        }
        for i in 0..self.circles.len() {
            let b = &mut self.circles.bodies[i];
            if b.inverse_mass > Fix::ZERO
                && b.composite.is_none()
                && is_active(objs, self.circles.parents[i].obj)
            {
                let d = b.integrate(dt, g);
                self.circles.shapes[i].center += d;
            }
        }
        for i in 0..self.boxes.len() {
            let b = &mut self.boxes.bodies[i];
            if b.inverse_mass > Fix::ZERO
                && b.composite.is_none()
                && is_active(objs, self.boxes.parents[i].obj)
            {
                let d = b.integrate(dt, g);
                let o = b.orientation;
                let s = &mut self.boxes.shapes[i];
                s.center += d;
                s.set_rotation(o);
            }
        }
        self.integrate_composites(objs, dt);
        let pairs = self.broad_phase(objs);
        // Step 8 begins with the ropes; beams follow in `World::tick`.
        self.simulate_ropes(objs, dt);
        Some(BroadPhase(pairs))
    }

    /// Steps 9–11 of `Simulate` with the pairs of [`Physics::simulate_begin`]
    /// (colliders added in between have no pairs this tick).
    pub fn simulate_end(&mut self, objs: &mut Objects, bp: BroadPhase) -> Vec<Contact> {
        let mut callbacks = Vec::new();
        for (kind, list) in bp.0 {
            self.narrow_and_resolve(objs, kind, &list, &mut callbacks);
        }
        self.write_back(objs);
        callbacks
    }

    fn sync_in(&mut self, objs: &Objects) {
        for i in 0..self.rrs.len() {
            if self.rrs.bodies[i].composite.is_some() {
                continue;
            }
            if let Some(o) = objs.get(self.rrs.parents[i].obj) {
                let t = o.tf;
                let s = &mut self.rrs.shapes[i];
                s.b.set_rotation(t.rotation);
                s.b.center = t.position;
                self.rrs.bodies[i].orientation = t.rotation;
            }
        }
        for i in 0..self.circles.len() {
            if self.circles.bodies[i].composite.is_some() {
                continue;
            }
            if let Some(o) = objs.get(self.circles.parents[i].obj) {
                let t = o.tf;
                self.circles.shapes[i].center = t.position + t.right.complex_mul(t.offset);
                self.circles.bodies[i].orientation = t.rotation;
            }
        }
        for i in 0..self.boxes.len() {
            if self.boxes.bodies[i].composite.is_some() {
                continue;
            }
            if let Some(o) = objs.get(self.boxes.parents[i].obj) {
                let t = o.tf;
                let s = &mut self.boxes.shapes[i];
                s.set_rotation(t.rotation);
                s.center = t.position + t.right.complex_mul(t.offset);
                self.boxes.bodies[i].orientation = t.rotation;
            }
        }
        for ci in 0..self.composites.len() {
            if !self.composites[ci].destroyed {
                self.sync_composite(objs, ci);
            }
        }
    }

    fn integrate_composites(&mut self, objs: &Objects, dt: Fix) {
        let g = self.params.gravity;
        for ci in 0..self.composites.len() {
            let c = &self.composites[ci];
            if c.destroyed || c.body.inverse_mass.0 == 0 || c.rrs.is_empty() {
                continue;
            }
            let first = c.rrs[0];
            let Some(fi) = self.rrs.index_of(first) else {
                continue;
            };
            if !(is_active(objs, first) && self.rrs.bodies[fi].inverse_mass > Fix::ZERO) {
                continue;
            }
            for r in self.composites[ci].rrs.clone() {
                if let Some(j) = self.rrs.index_of(r) {
                    let f = self.rrs.bodies[j].force;
                    self.composites[ci].body.force += f;
                    self.rrs.bodies[j].force = Vec2::ZERO;
                }
            }
            let comp = &mut self.composites[ci];
            let d = comp.body.integrate(dt, g);
            comp.position += d;
            comp.orientation = comp.body.orientation;
            let u = Vec2::from_angle(comp.orientation);
            let (pos, ori) = (comp.position, comp.orientation);
            for r in comp.rrs.clone() {
                if let Some(j) = self.rrs.index_of(r) {
                    self.rrs.shapes[j].b.center += d;
                    self.rrs.bodies[j].orientation = ori;
                    self.rrs.shapes[j].b.set_rotation(ori);
                }
            }
            for o in self.composites[ci].circles.clone() {
                if let Some(j) = self.circles.index_of(o) {
                    let b = self.circles.bodies[j];
                    self.circles.shapes[j].center = pos + b.relative_pos.complex_mul(u);
                    self.circles.bodies[j].orientation = ori + b.relative_orientation;
                }
            }
            for o in self.composites[ci].boxes.clone() {
                if let Some(j) = self.boxes.index_of(o) {
                    let b = self.boxes.bodies[j];
                    self.boxes.shapes[j].center = pos + b.relative_pos.complex_mul(u);
                    let ori2 = ori + b.relative_orientation;
                    self.boxes.bodies[j].orientation = ori2;
                    self.boxes.shapes[j].set_rotation(ori2);
                }
            }
        }
    }

    /// §6: AABBs, axis choice and sweep and prune; returns the pair lists in
    /// narrow-phase order.
    fn broad_phase(&mut self, objs: &Objects) -> [(PairKind, Vec<(usize, usize)>); 6] {
        let mut bb: Vec<Aabb> =
            Vec::with_capacity(self.rrs.len() + self.circles.len() + self.boxes.len());
        let mut sum = Vec2::ZERO;
        for i in 0..self.rrs.len() {
            if is_active(objs, self.rrs.parents[i].obj) {
                bb.push(self.rrs.shapes[i].aabb(i as i32));
                sum += self.rrs.shapes[i].b.center;
            }
        }
        for i in 0..self.circles.len() {
            if is_active(objs, self.circles.parents[i].obj) {
                bb.push(self.circles.shapes[i].aabb(i as i32));
                sum += self.circles.shapes[i].center;
            }
        }
        for i in 0..self.boxes.len() {
            if is_active(objs, self.boxes.parents[i].obj) {
                bb.push(self.boxes.shapes[i].aabb(i as i32));
                sum += self.boxes.shapes[i].center;
            }
        }
        let count = bb.len().max(1);
        let mean = sum / Fix::from_int(count as i32);
        let mut var = Vec2::ZERO;
        let mut acc = |c: Vec2| {
            let d = mean - c;
            var += Vec2::new(d.x * d.x, d.y * d.y);
        };
        for s in &self.rrs.shapes {
            acc(s.b.center);
        }
        for s in &self.circles.shapes {
            acc(s.center);
        }
        for s in &self.boxes.shapes {
            acc(s.center);
        }
        let along_x = var.x > var.y;
        let key = |a: &Aabb| {
            if along_x {
                (a.min.x, a.max.x)
            } else {
                (a.min.y, a.max.y)
            }
        };
        let other = |a: &Aabb| {
            if along_x {
                (a.min.y, a.max.y)
            } else {
                (a.min.x, a.max.x)
            }
        };
        introsort(&mut bb, |a, b| {
            let (am, bm) = (key(a).0, key(b).0);
            let s = (am - bm).sign();
            if s != 0 {
                return s.cmp(&0);
            }
            let s = a.shape as i32 - b.shape as i32;
            if s != 0 {
                return s.cmp(&0);
            }
            (a.id - b.id).cmp(&0)
        });
        let mut out: [(PairKind, Vec<(usize, usize)>); 6] = [
            (PairKind::Rr, Vec::new()),
            (PairKind::Br, Vec::new()),
            (PairKind::Cr, Vec::new()),
            (PairKind::Cc, Vec::new()),
            (PairKind::Cb, Vec::new()),
            (PairKind::Bb, Vec::new()),
        ];
        if bb.len() < 2 {
            return out;
        }
        let mut active: Vec<usize> = vec![0];
        for i in 1..bb.len() {
            let bi = bb[i];
            let mut k = 0;
            while k < active.len() {
                let a = bb[active[k]];
                if key(&a).1 <= key(&bi).0 {
                    active.remove(k);
                    continue;
                }
                if self.layers_collide(a.layer, bi.layer)
                    && other(&a).1 > other(&bi).0
                    && other(&a).0 < other(&bi).1
                {
                    let (ai, ii) = (a.id as usize, bi.id as usize);
                    use ShapeKind as S;
                    let sa = a.shape;
                    let si = bi.shape;
                    let (c, b, r) = (S::Circle as u8, S::Box as u8, S::RoundedRect as u8);
                    let push = |out: &mut [(PairKind, Vec<(usize, usize)>); 6], slot: usize, p| {
                        out[slot].1.push(p)
                    };
                    if sa == c && si == c {
                        push(&mut out, 3, (ai, ii));
                    } else if sa == c && si == b {
                        push(&mut out, 4, (ai, ii));
                    } else if sa == c && si == r {
                        push(&mut out, 2, (ai, ii));
                    } else if sa == b && si == c {
                        push(&mut out, 4, (ii, ai));
                    } else if sa == b && si == b {
                        push(&mut out, 5, (ai, ii));
                    } else if sa == b && si == r {
                        push(&mut out, 1, (ai, ii));
                    } else if sa == r && si == c {
                        push(&mut out, 2, (ii, ai));
                    } else if sa == r && si == b {
                        push(&mut out, 1, (ii, ai));
                    } else {
                        push(&mut out, 0, (ai, ii));
                    }
                }
                k += 1;
            }
            active.push(i);
        }
        out
    }

    fn shape_snap(&self, k: ShapeKind, i: usize) -> ShapeSnap {
        match k {
            ShapeKind::Circle => ShapeSnap::Circle(self.circles.shapes[i]),
            ShapeKind::Box => ShapeSnap::Box(self.boxes.shapes[i]),
            _ => ShapeSnap::RoundedRect(self.rrs.shapes[i]),
        }
    }

    fn parent_at(&self, k: ShapeKind, i: usize) -> Parent {
        match k {
            ShapeKind::Circle => self.circles.parents[i],
            ShapeKind::Box => self.boxes.parents[i],
            _ => self.rrs.parents[i],
        }
    }

    fn center_at(&self, k: ShapeKind, i: usize) -> Vec2 {
        match k {
            ShapeKind::Circle => self.circles.shapes[i].center,
            ShapeKind::Box => self.boxes.shapes[i].center,
            _ => self.rrs.shapes[i].b.center,
        }
    }

    fn move_center(&mut self, k: ShapeKind, i: usize, d: Vec2) {
        match k {
            ShapeKind::Circle => self.circles.shapes[i].center += d,
            ShapeKind::Box => self.boxes.shapes[i].center += d,
            _ => self.rrs.shapes[i].b.center += d,
        }
    }

    fn same_composite(&self, ka: ShapeKind, ia: usize, kb: ShapeKind, ib: usize) -> bool {
        let a = self.body_at(ka, ia).composite;
        let b = self.body_at(kb, ib).composite;
        match (a, b) {
            (Some(x), Some(y)) => x == y && !self.composites[x as usize].destroyed,
            _ => false,
        }
    }

    /// The body used for resolution (composite if a live member).
    fn eff_body(&self, k: ShapeKind, i: usize) -> Body {
        let b = *self.body_at(k, i);
        match b.composite {
            Some(c) if !self.composites[c as usize].destroyed => self.composites[c as usize].body,
            _ => b,
        }
    }

    fn eff_pos(&self, k: ShapeKind, i: usize) -> Vec2 {
        let b = self.body_at(k, i);
        match b.composite {
            Some(c) if !self.composites[c as usize].destroyed => {
                self.composites[c as usize].position
            }
            _ => self.center_at(k, i),
        }
    }

    fn eff_body_mut(&mut self, k: ShapeKind, i: usize) -> &mut Body {
        let c = self.body_at(k, i).composite;
        match c {
            Some(c) if !self.composites[c as usize].destroyed => {
                &mut self.composites[c as usize].body
            }
            _ => self.body_at_mut(k, i),
        }
    }

    fn narrow_and_resolve(
        &mut self,
        objs: &Objects,
        kind: PairKind,
        pairs: &[(usize, usize)],
        callbacks: &mut Vec<Contact>,
    ) {
        use ShapeKind as S;
        let (ka, kb) = match kind {
            PairKind::Rr => (S::RoundedRect, S::RoundedRect),
            PairKind::Br => (S::Box, S::RoundedRect),
            PairKind::Cr => (S::Circle, S::RoundedRect),
            PairKind::Cc => (S::Circle, S::Circle),
            PairKind::Cb => (S::Circle, S::Box),
            PairKind::Bb => (S::Box, S::Box),
        };
        let mut found: Vec<(usize, usize, Manifold)> = Vec::new();
        for &(a, b) in pairs {
            if self.same_composite(ka, a, kb, b) {
                continue;
            }
            let m = match kind {
                PairKind::Rr => rr_rr(&self.rrs.shapes[a], &self.rrs.shapes[b]),
                PairKind::Br => box_rr(&self.boxes.shapes[a], &self.rrs.shapes[b]),
                PairKind::Cr => circle_rr(&self.circles.shapes[a], &self.rrs.shapes[b]),
                PairKind::Cc => circle_circle(&self.circles.shapes[a], &self.circles.shapes[b]),
                PairKind::Cb => circle_box(&self.circles.shapes[a], &self.boxes.shapes[b]),
                PairKind::Bb => box_box(&self.boxes.shapes[a], &self.boxes.shapes[b]),
            };
            if let Some(m) = m {
                found.push((a, b, m));
            }
        }
        for &(a, b, m) in &found {
            let p1 = self.parent_at(ka, a);
            let p2 = self.parent_at(kb, b);
            let va = self.body_at(ka, a).velocity;
            let vb = self.body_at(kb, b).velocity;
            let p1_ok = !collider_destroyed(objs, p1.obj);
            if p1_ok {
                callbacks.push(Contact {
                    me: p1.obj,
                    other: p2.obj,
                    other_hier: p2.hierarchy,
                    my_hier: p1.hierarchy,
                    collider: self.shape_snap(kb, b),
                    layer: self.layer_at(kb, b),
                    impact_velocity: vb,
                    contact_point: m.contact,
                    normal: m.normal,
                    penetration: m.penetration,
                });
            }
            if !collider_destroyed(objs, p2.obj) && p1_ok {
                callbacks.push(Contact {
                    me: p2.obj,
                    other: p1.obj,
                    other_hier: p1.hierarchy,
                    my_hier: p2.hierarchy,
                    collider: self.shape_snap(ka, a),
                    layer: self.layer_at(ka, a),
                    impact_velocity: va,
                    contact_point: m.contact,
                    normal: m.normal,
                    penetration: m.penetration,
                });
            }
        }
        for &(a, b, m) in &found {
            if self.body_at(ka, a).is_trigger() || self.body_at(kb, b).is_trigger() {
                continue;
            }
            let pa = self.eff_pos(ka, a);
            let pb = self.eff_pos(kb, b);
            let ba = self.eff_body(ka, a);
            let bb = self.eff_body(kb, b);
            let (ia, ib) = resolve_collision(&m, &ba, &bb, pa, pb);
            for (lever, imp) in ia {
                apply_impulse(self.eff_body_mut(ka, a), imp, lever);
            }
            for (lever, imp) in ib {
                apply_impulse(self.eff_body_mut(kb, b), imp, lever);
            }
            let ima = self.body_at(ka, a).inverse_mass;
            let imb = self.body_at(kb, b).inverse_mass;
            let (da, db) = positional_correction_std(&m, ima, imb);
            self.move_center(ka, a, da);
            self.move_center(kb, b, db);
            if let Some(c) = self.body_at(ka, a).composite
                && !self.composites[c as usize].destroyed
            {
                self.composites[c as usize].position += da;
            }
            if let Some(c) = self.body_at(kb, b).composite
                && !self.composites[c as usize].destroyed
            {
                self.composites[c as usize].position += db;
            }
        }
    }

    fn layer_at(&self, k: ShapeKind, i: usize) -> i32 {
        match k {
            ShapeKind::Circle => self.circles.shapes[i].layer,
            ShapeKind::Box => self.boxes.shapes[i].layer,
            _ => self.rrs.shapes[i].layer,
        }
    }

    fn write_back(&self, objs: &mut Objects) {
        for i in 0..self.rrs.len() {
            if self.rrs.bodies[i].is_trigger() {
                continue;
            }
            if let Some(o) = objs.get_mut(self.rrs.parents[i].obj) {
                o.tf.set_rotation(self.rrs.bodies[i].orientation);
                o.tf.position = self.rrs.shapes[i].b.center - o.tf.right.complex_mul(o.tf.offset);
            }
        }
        for i in 0..self.circles.len() {
            if self.circles.bodies[i].is_trigger() {
                continue;
            }
            if let Some(o) = objs.get_mut(self.circles.parents[i].obj) {
                o.tf.set_rotation(self.circles.bodies[i].orientation);
                o.tf.position = self.circles.shapes[i].center - o.tf.right.complex_mul(o.tf.offset);
            }
        }
        for i in 0..self.boxes.len() {
            if self.boxes.bodies[i].is_trigger() {
                continue;
            }
            if let Some(o) = objs.get_mut(self.boxes.parents[i].obj) {
                o.tf.set_rotation(self.boxes.bodies[i].orientation);
                o.tf.position = self.boxes.shapes[i].center - o.tf.right.complex_mul(o.tf.offset);
            }
        }
    }

    // -- queries (§9) -------------------------------------------------------

    /// `RaycastToClosest(origin, dir, dist, mask)`.
    pub fn raycast_closest(
        &mut self,
        objs: &Objects,
        origin: Vec2,
        dir: Vec2,
        dist: Fix,
        mask: u32,
    ) -> RayHit {
        self.remove_destroyed(objs);
        let mut best = RayHit::NONE;
        let mut consider = |near: Fix, far: Fix, layer: i32, p: Parent, kind: ShapeKind| {
            if near < best.near_dist && near <= dist {
                best = RayHit {
                    near_pos: origin + dir * near,
                    far_pos: origin + dir * far,
                    near_dist: near,
                    far_dist: far,
                    layer,
                    obj: p.obj,
                    hierarchy: p.hierarchy,
                    kind,
                };
            }
        };
        for i in 0..self.rrs.len() {
            let s = &self.rrs.shapes[i];
            if mask >> (s.layer & 31) & 1 == 0 || !is_active(objs, self.rrs.parents[i].obj) {
                continue;
            }
            if let Some((n, f)) = ray_rr(origin, dir, s) {
                consider(n, f, s.layer, self.rrs.parents[i], ShapeKind::RoundedRect);
            }
        }
        for i in 0..self.circles.len() {
            let s = &self.circles.shapes[i];
            if mask >> (s.layer & 31) & 1 == 0 || !is_active(objs, self.circles.parents[i].obj) {
                continue;
            }
            if let Some((n, f)) = ray_circle(origin, dir, s) {
                consider(n, f, s.layer, self.circles.parents[i], ShapeKind::Circle);
            }
        }
        for i in 0..self.boxes.len() {
            let s = &self.boxes.shapes[i];
            if mask >> (s.layer & 31) & 1 == 0 || !is_active(objs, self.boxes.parents[i].obj) {
                continue;
            }
            if let Some((n, f)) = ray_box(origin, dir, s) {
                consider(n, f, s.layer, self.boxes.parents[i], ShapeKind::Box);
            }
        }
        best
    }

    /// `RaycastToClosestRoundedRect(origin, dir, dist)`: no layer mask.
    pub fn raycast_closest_rr(
        &mut self,
        objs: &Objects,
        origin: Vec2,
        dir: Vec2,
        dist: Fix,
    ) -> RayHit {
        self.remove_destroyed(objs);
        let mut best = RayHit::NONE;
        for i in 0..self.rrs.len() {
            if !is_active(objs, self.rrs.parents[i].obj) {
                continue;
            }
            let s = &self.rrs.shapes[i];
            if let Some((near, far)) = ray_rr(origin, dir, s)
                && near < best.near_dist
                && near <= dist
            {
                let p = self.rrs.parents[i];
                best = RayHit {
                    near_pos: origin + dir * near,
                    far_pos: origin + dir * far,
                    near_dist: near,
                    far_dist: far,
                    layer: s.layer,
                    obj: p.obj,
                    hierarchy: p.hierarchy,
                    kind: ShapeKind::RoundedRect,
                };
            }
        }
        best
    }

    /// `RaycastToClosestTriple`.
    pub fn raycast_closest_triple(
        &mut self,
        objs: &Objects,
        origin: Vec2,
        dir: Vec2,
        dist: Fix,
        mask: u32,
        radius: Fix,
    ) -> RayHit {
        let c = self.raycast_closest(objs, origin, dir, dist, mask);
        let l = self.raycast_closest(objs, origin + dir.mul_by_i() * radius, dir, dist, mask);
        let r = self.raycast_closest(objs, origin + dir.mul_by_neg_i() * radius, dir, dist, mask);
        if c.near_dist < l.near_dist && c.near_dist < r.near_dist {
            c
        } else if l.near_dist < c.near_dist && l.near_dist < r.near_dist {
            l
        } else if r.near_dist < c.near_dist && r.near_dist < l.near_dist {
            r
        } else {
            c
        }
    }

    /// `RaycastAll`: appends hits and sorts the whole list.
    pub fn raycast_all(
        &mut self,
        objs: &Objects,
        origin: Vec2,
        dir: Vec2,
        dist: Fix,
        mask: u32,
        out: &mut Vec<RayHit>,
        caster: Option<ObjId>,
    ) -> bool {
        self.remove_destroyed(objs);
        let mut added = false;
        let mut add =
            |near: Fix, far: Fix, layer: i32, p: Parent, kind: ShapeKind, out: &mut Vec<RayHit>| {
                if near <= dist && Some(p.obj) != caster {
                    out.push(RayHit {
                        near_pos: origin + dir * near,
                        far_pos: origin + dir * far,
                        near_dist: near,
                        far_dist: far,
                        layer,
                        obj: p.obj,
                        hierarchy: p.hierarchy,
                        kind,
                    });
                    added = true;
                }
            };
        for i in 0..self.rrs.len() {
            let s = &self.rrs.shapes[i];
            if mask >> (s.layer & 31) & 1 == 0 || !is_active(objs, self.rrs.parents[i].obj) {
                continue;
            }
            if let Some((n, f)) = ray_rr(origin, dir, s) {
                add(
                    n,
                    f,
                    s.layer,
                    self.rrs.parents[i],
                    ShapeKind::RoundedRect,
                    out,
                );
            }
        }
        for i in 0..self.circles.len() {
            let s = &self.circles.shapes[i];
            if mask >> (s.layer & 31) & 1 == 0 || !is_active(objs, self.circles.parents[i].obj) {
                continue;
            }
            if let Some((n, f)) = ray_circle(origin, dir, s) {
                add(
                    n,
                    f,
                    s.layer,
                    self.circles.parents[i],
                    ShapeKind::Circle,
                    out,
                );
            }
        }
        for i in 0..self.boxes.len() {
            let s = &self.boxes.shapes[i];
            if mask >> (s.layer & 31) & 1 == 0 || !is_active(objs, self.boxes.parents[i].obj) {
                continue;
            }
            if let Some((n, f)) = ray_box(origin, dir, s) {
                add(n, f, s.layer, self.boxes.parents[i], ShapeKind::Box, out);
            }
        }
        introsort(out, |a, b| match a.near_dist.cmp(&b.near_dist) {
            Ordering::Equal => a.hierarchy.cmp(&b.hierarchy),
            o => o,
        });
        added
    }

    /// `PointCheckAllRoundedRects(p)`: the last active rounded rect containing `p`.
    pub fn point_check_all_rrs(&mut self, objs: &Objects, p: Vec2) -> RayHit {
        self.remove_destroyed(objs);
        let mut best = RayHit::NONE;
        for i in 0..self.rrs.len() {
            if is_active(objs, self.rrs.parents[i].obj) && self.rrs.shapes[i].contains(p) {
                let par = self.rrs.parents[i];
                best = RayHit {
                    near_pos: p,
                    far_pos: p,
                    near_dist: F_0_001,
                    far_dist: F_0_001,
                    layer: self.rrs.shapes[i].layer,
                    obj: par.obj,
                    hierarchy: par.hierarchy,
                    kind: ShapeKind::RoundedRect,
                };
            }
        }
        best
    }

    /// `circleCastRoundedRect(pos, r)`: overlapping rounded rects in list order.
    pub fn circle_cast_rrs(&mut self, objs: &Objects, pos: Vec2, r: Fix, max: usize) -> Vec<ObjId> {
        self.remove_destroyed(objs);
        let c = Circle {
            center: pos,
            radius: r,
            layer: 0,
        };
        let mut out = Vec::new();
        for i in 0..self.rrs.len() {
            if out.len() >= max {
                break;
            }
            if is_active(objs, self.rrs.parents[i].obj) && circle_rr_bool(&c, &self.rrs.shapes[i]) {
                out.push(self.rrs.parents[i].obj);
            }
        }
        out
    }

    /// `circleCastCircles(pos, r, mask)`: active circles overlapping, in list
    /// order, up to `max`.
    pub fn circle_cast_circles(
        &mut self,
        objs: &Objects,
        pos: Vec2,
        r: Fix,
        mask: u32,
        max: usize,
    ) -> Vec<ObjId> {
        self.remove_destroyed(objs);
        let c = Circle {
            center: pos,
            radius: r,
            layer: 0,
        };
        let mut out = Vec::new();
        for i in 0..self.circles.len() {
            let s = &self.circles.shapes[i];
            if out.len() < max
                && mask >> (s.layer & 31) & 1 != 0
                && is_active(objs, self.circles.parents[i].obj)
                && circle_circle_bool(&c, s)
            {
                out.push(self.circles.parents[i].obj);
            }
        }
        out
    }

    /// `CircleCastAll(pos, r, mask)`: circles, then boxes, then rounded rects.
    pub fn circle_cast_all(
        &mut self,
        objs: &Objects,
        pos: Vec2,
        r: Fix,
        mask: u32,
        max: usize,
    ) -> Vec<ObjId> {
        self.remove_destroyed(objs);
        let c = Circle {
            center: pos,
            radius: r,
            layer: 0,
        };
        let mut out = Vec::new();
        for i in 0..self.circles.len() {
            let s = &self.circles.shapes[i];
            if out.len() < max
                && mask >> (s.layer & 31) & 1 != 0
                && is_active(objs, self.circles.parents[i].obj)
                && circle_circle_bool(&c, s)
            {
                out.push(self.circles.parents[i].obj);
            }
        }
        for i in 0..self.boxes.len() {
            let s = &self.boxes.shapes[i];
            if out.len() < max
                && mask >> (s.layer & 31) & 1 != 0
                && is_active(objs, self.boxes.parents[i].obj)
                && circle_box_bool(&c, s)
            {
                out.push(self.boxes.parents[i].obj);
            }
        }
        for i in 0..self.rrs.len() {
            let s = &self.rrs.shapes[i];
            if out.len() < max
                && mask >> (s.layer & 31) & 1 != 0
                && is_active(objs, self.rrs.parents[i].obj)
                && circle_rr_bool(&c, s)
            {
                out.push(self.rrs.parents[i].obj);
            }
        }
        out
    }

    /// `CircleCastClosestPlayers(pos, r, mult, ignore_id)`: the last Player-layer
    /// box (within the distance cap), then circle, overlapping the circle.
    pub fn circle_cast_closest_players(
        &mut self,
        objs: &Objects,
        pos: Vec2,
        r: Fix,
        mult: Fix,
        ignore: ObjId,
    ) -> Option<ObjId> {
        self.remove_destroyed(objs);
        let mut hit = None;
        let cap = Fix::from_int(100_000);
        let c = Circle {
            center: pos,
            radius: r,
            layer: 0,
        };
        for i in 0..self.boxes.len() {
            let s = &self.boxes.shapes[i];
            let o = self.boxes.parents[i].obj;
            if s.layer == 8
                && is_active(objs, o)
                && o != ignore
                && circle_box_bool(&c, s)
                && (s.center - pos).sqr_magnitude() < cap
            {
                hit = Some(o);
            }
        }
        let c = Circle {
            center: pos,
            radius: r * mult,
            layer: 0,
        };
        for i in 0..self.circles.len() {
            let s = &self.circles.shapes[i];
            let o = self.circles.parents[i].obj;
            if s.layer == 8
                && is_active(objs, o)
                && o != ignore
                && circle_circle_bool(&c, s)
                && (s.center - pos).sqr_magnitude() < cap
            {
                hit = Some(o);
            }
        }
        hit
    }
}

/// §8.1 `ResolveCollision`: returns the impulses (lever, impulse) for A and B.
#[allow(clippy::type_complexity)]
fn resolve_collision(
    m: &Manifold,
    pa: &Body,
    pb: &Body,
    pos_a: Vec2,
    pos_b: Vec2,
) -> (Vec<(Vec2, Vec2)>, Vec<(Vec2, Vec2)>) {
    let n = m.normal;
    let ra = (m.contact - pos_a).normalized_safe();
    let rb = (m.contact - pos_b).normalized_safe();
    let v = pb.velocity + Vec2::cross_sv(pb.angular_velocity, rb)
        - pa.velocity
        - Vec2::cross_sv(pa.angular_velocity, ra);
    let vn = v.dot(n);
    if vn > Fix::ZERO {
        return (Vec::new(), Vec::new());
    }
    let e = Fix::ONE + pa.bounciness.max(pb.bounciness);
    let ka = ra.cross(n);
    let ka = ka * ka * pa.inverse_moi;
    let kb = rb.cross(n);
    let kb = kb * kb * pb.inverse_moi;
    let k = Fix::ONE / (pa.inverse_mass + pb.inverse_mass + ka + kb);
    let j = (-e) * vn * k;
    let jv = j * n;
    let mut ia = vec![(ra, -jv)];
    let mut ib = vec![(rb, jv)];
    let t = Vec2::new(-n.y, n.x);
    let t = t.dot(v).sign_fix() * t;
    let vt = -t.dot(v);
    let jt = vt * k;
    let f = if jt.abs() < j * pa.static_friction.max(pb.static_friction) {
        jt * t
    } else {
        (-j) * t * pa.dynamic_friction.max(pb.dynamic_friction)
    };
    ia.push((ra, -f));
    ib.push((rb, f));
    (ia, ib)
}

/// §8.2 `ApplyImpulse`.
fn apply_impulse(b: &mut Body, imp: Vec2, r: Vec2) {
    b.velocity += b.inverse_mass * imp;
    b.angular_velocity += b.inverse_moi * r.cross(imp);
}

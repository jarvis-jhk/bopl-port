//! Tesla coil (`docs/spec/abilities/tesla-coil.md`): `PlantObjectOnPlatform`
//! and `PlaceSparkNode` plant `simple_SparkNode` coils (welded into the
//! platform's composite body, two per slot via a ring buffer); a pair is
//! joined by a `Chain_Lightning` beam (`StretchableLightning`). Coils and
//! beam carry Explosion components whose owner is never set (killer 0).

use std::sync::Arc;

use super::Behaviour;
use super::ability_body::{self, AbilityBody, add_force_at_position, advance_anim, play_named as play, play_named_then as play_then};
use super::prefab::{self, fx, prefab_field, root_fields, vec2_field};
use super::explosion::ExplosionInfo;
use super::engine::PlantObjectOnPlatform;
use super::prelude::*;
use super::smoke::RingBuffer;
use crate::anim::SpriteAnimator;
use crate::surface::Surface;
use crate::world::Kind;

/// Layers.
const LETHAL_TERRAIN: u8 = 16;
const NON_LETHAL_TERRAIN: u8 = 21;

/// Ability constants.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TeslaCfg {
    pub plant_offset: Vec2,
    pub cast_force: Fix,
    pub node: Option<Arc<NodeCfg>>,
}

/// `simple_SparkNode` + `Chain_Lightning`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct NodeCfg {
    pub prefab: usize,
    pub line_length: Fix,
    pub hit_offset_y: Fix,
    pub orb_data: u16,
    pub orb_hier: Option<i32>,
    pub explosion: Option<ExplosionInfo>,
    pub parts: Vec<(PartKind, i32, bool)>,
    pub beam: Option<Arc<BeamCfg>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BeamCfg {
    pub prefab: usize,
    pub anim_data: u16,
    pub animation: String,
    pub flicker_period: Fix,
    pub explosion: Option<ExplosionInfo>,
    pub parts: Vec<(PartKind, i32, bool)>,
}

impl NodeCfg {
    fn from_prefab(w: &World, prefab: usize) -> Option<NodeCfg> {
        let p = w.content.prefabs.get(prefab)?;
        let f = root_fields(p, "SimpleSparkNode")?;
        let root = p.objects.first()?;
        let mut parts = ability_body::parts_of(
            root,
            &[("SimpleSparkNode", PartKind::Behaviour(0)), ("Explosion", PartKind::Behaviour(1))],
            true,
        );
        // The orb animator lives on the child `orb`.
        let orb_hier = p
            .objects
            .iter()
            .skip(1)
            .find_map(|o| o.component("SpriteAnimator").and_then(|c| c.hierarchy));
        if let Some(h) = orb_hier {
            parts.push((PartKind::Animator, h, true));
            parts.sort_by_key(|p| p.1);
        }
        let beam = prefab_field(w, f, "lightningPrefab").and_then(|b| {
            let bp = w.content.prefabs.get(b)?;
            let bf = root_fields(bp, "StretchableLightning")?;
            Some(Arc::new(BeamCfg {
                prefab: b,
                anim_data: prefab::anim_data_field(w, bf, "animData").unwrap_or(0),
                animation: bf.str("AnimationToPlay").unwrap_or("").to_string(),
                flicker_period: fx(bf, "flickerPeriod", Fix::ZERO),
                explosion: root_fields(bp, "Explosion").map(ExplosionInfo::from_fields),
                parts: ability_body::prefab_parts(
                    w,
                    b,
                    &[("StretchableLightning", PartKind::Behaviour(0)), ("Explosion", PartKind::Behaviour(1))],
                ),
            }))
        });
        Some(NodeCfg {
            prefab,
            line_length: fx(f, "lightningLineLength", Fix::ZERO),
            hit_offset_y: fx(f, "hitBoxOffsetY", Fix::ONE),
            orb_data: prefab::anim_data_field(w, f, "OrbAnimationData").unwrap_or(0),
            orb_hier,
            explosion: root_fields(p, "Explosion").map(ExplosionInfo::from_fields),
            parts,
            beam,
        })
    }
}


/// The Tesla coil ability object: the shared `PlantObjectOnPlatform`
/// (engine.md, `engine::PlantObjectOnPlatform`) with the `PlaceSparkNode`
/// placer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TeslaCoil {
    pub cfg: Option<Arc<TeslaCfg>>,
    pub plant: PlantObjectOnPlatform,
    /// `plantBuffer` (2 coils per slot).
    pub buffer: RingBuffer,
}

impl TeslaCoil {
    /// The `cast` animation's end: plant on the ground or exit.
    fn plant(&mut self, cx: &mut Cx<'_>) {
        let Some(cfg) = self.cfg.clone() else {
            return;
        };
        let Some(g) = self.plant.plant_ground(cx.w) else {
            self.exit_component(cx, AbilityExitInfo::default());
            return;
        };
        self.place(cx, &cfg, g);
        cx.w.emit(Cue::Sound("plantObjectOnPlatform"));
        self.plant.begin_recover(cx.w);
    }

    /// `PlaceSparkNode.Place(owner, ground)`.
    fn place(&mut self, cx: &mut Cx<'_>, cfg: &TeslaCfg, g: ObjId) {
        let Some(ncfg) = cfg.node.clone() else {
            return;
        };
        let (me, owner) = (cx.me, cx.fw.owner);
        let w = &mut *cx.w;
        let Some(surf) = Surface::of(w, g) else {
            return;
        };
        let t = w.objects.get(me).map(|o| o.tf).unwrap_or_default();
        let m = if self.plant.facing_right { -Fix::ONE } else { Fix::ONE };
        let q = t.position + t.right * m * cfg.plant_offset.x;
        let mut p = surf.closest_point(q);
        let l = surf.param_from_position(p, Fix::ZERO);
        p = surf.position_from_param(l, Fix::ZERO);
        let n = surf.normal(p);
        p += n * cfg.plant_offset.y;
        let scale = t.scale;
        let node = spawn_node(w, &ncfg, p, owner);
        w.objects.set_parent(node, Some(g));
        if let Some(o) = w.objects.get_mut(node) {
            o.tf.position = p;
            o.tf.set_up(n);
        }
        PlantObjectOnPlatform::plant_fx(w, me, owner, p, n);
        self.buffer.add(w, node);
        let new_hier = node_hier(w, node);
        let child = self
            .buffer
            .slots
            .iter()
            .flatten()
            .copied()
            .find(|c| w.objects.exists(*c) && node_hier(w, *c) != new_hier);
        initialize(w, node, owner, child, cfg.plant_offset.y, l, g, scale, true);
        add_force_at_position(w, g, -n * cfg.cast_force, t.position, false);
    }
}

fn node_hier(w: &World, n: ObjId) -> i32 {
    w.objects
        .get(n)
        .and_then(|o| o.part(PartKind::Behaviour(0)))
        .map_or(0, |p| p.hier)
}

impl AbilityImpl for TeslaCoil {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let Some(p) = def.prefab else {
            return TeslaCoil::default();
        };
        let pf = &w.content.prefabs[p];
        let (Some(_), Some(ps)) = (root_fields(pf, "PlantObjectOnPlatform"), root_fields(pf, "PlaceSparkNode")) else {
            return TeslaCoil::default();
        };
        let node = prefab_field(w, ps, "plantablePrefab")
            .and_then(|i| NodeCfg::from_prefab(w, i))
            .map(Arc::new);
        let cfg = TeslaCfg {
            plant_offset: vec2_field(ps, "plantOffset").unwrap_or(Vec2::ZERO),
            cast_force: fx(ps, "castForce", Fix::from_int(10)),
            node,
        };
        TeslaCoil {
            cfg: Some(Arc::new(cfg)),
            plant: PlantObjectOnPlatform::new(w, def),
            buffer: RingBuffer::new(2),
        }
    }

    fn build(&mut self, w: &mut World, me: ObjId, fw: &mut Framework, def: &AbilityDef) {
        self.plant.body.install(fw);
        ability_body::add_parts(
            w,
            me,
            fw,
            def,
            &[
                ("PlantObjectOnPlatform", PartKind::Behaviour(0)),
                ("PlaceSparkNode", PartKind::Behaviour(1)),
            ],
        );
    }

    fn body(&self) -> Option<&AbilityBody> {
        self.cfg.as_ref().map(|_| &self.plant.body)
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        if self.cfg.is_some() {
            Some(&mut self.plant.body)
        } else {
            None
        }
    }

    fn sprite(&self) -> crate::data::SpriteId {
        self.plant.anim.sprite
    }

    fn look(&self, w: &World, me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        self.plant.look(w, me, l);
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        if self.cfg.is_none() {
            let i = cx.info();
            cx.exit(AbilityExitInfo::resume(&i));
            return;
        }
        self.plant.enter(cx);
    }

    fn update_part(&mut self, cx: &mut Cx<'_>, k: PartKind, dt: Fix, late: bool) {
        match (k, late) {
            (PartKind::Animator, false) => match advance_anim(cx.w, cx.fw.owner, &mut self.plant.anim) {
                Some(AnimCallback::Ability(super::engine::CB_PLANT)) => self.plant(cx),
                Some(AnimCallback::Ability(super::engine::CB_EXIT)) => {
                    self.exit_component(cx, AbilityExitInfo::default())
                }
                _ => {}
            },
            (PartKind::Behaviour(0), false) => {
                if let Some(info) = self.plant.update(cx, dt) {
                    self.exit_component(cx, info);
                }
            }
            (PartKind::Behaviour(1), _) => {}
            _ => {
                if let Some(r) = self.plant.body.update_part(cx, k, dt, late) {
                    self.exit_component(cx, r);
                }
            }
        }
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        ability_body::hurt_contact(self, cx, c);
    }

    /// `PlantObjectOnPlatform.ExitAbility(info)` (engine.md).
    fn exit_component(&mut self, cx: &mut Cx<'_>, info: AbilityExitInfo) {
        let rec = self.plant.exit_record(cx, info);
        cx.exit(rec);
    }
}

// -- the coils --------------------------------------------------------------------------

const CB_ACTIVATE_LIGHTNING: u16 = 1;
const CB_ACTIVATE_PARTNER: u16 = 2;
const CB_LOOP_ORB: u16 = 3;
const CB_UNFLICKER: u16 = 4;

/// A `SimpleSparkNode` coil.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Node {
    pub cfg: Arc<NodeCfg>,
    pub explosion: ExplosionInfo,
    pub id: i32,
    pub child: Option<ObjId>,
    pub parent: Option<ObjId>,
    pub connected: bool,
    pub beam: Option<ObjId>,
    pub ground: Option<ObjId>,
    pub local: Fix,
    pub surface_offset: Fix,
    pub hit_offset: Fix,
    pub prev_scale: Fix,
    pub start_scale: Fix,
    pub orb: SpriteAnimator,
}

/// A `Chain_Lightning` beam.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Beam {
    pub cfg: Arc<BeamCfg>,
    pub explosion: ExplosionInfo,
    pub id: i32,
    /// `InitLightning` was called.
    pub initialized: bool,
    pub time_passed: Fix,
    pub anim: SpriteAnimator,
}

/// Objects of this module.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    Node(Node),
    Beam(Beam),
}

impl Spawned {
    pub fn explosion(&self) -> Option<ExplosionInfo> {
        match self {
            Spawned::Node(n) => Some(n.explosion),
            Spawned::Beam(b) => Some(b.explosion),
        }
    }
}

fn default_explosion(e: Option<ExplosionInfo>) -> ExplosionInfo {
    e.unwrap_or(ExplosionInfo {
        owner: 0,
        can_hurt_owner: true,
        ignite_flammables: true,
        beam: false,
    })
}

/// Tagged `ChainLightning` (coils and beams).
pub fn is_chain_lightning(w: &World, obj: ObjId) -> bool {
    matches!(w.objects.get(obj).map(|o| &o.kind), Some(Kind::Behaviour(b)) if matches!(b.as_ref(), Behaviour::TeslaCoil(_)))
}

fn spawn_node(w: &mut World, cfg: &Arc<NodeCfg>, p: Vec2, owner: i32) -> ObjId {
    let orb = ability_body::animator(true);
    let n = Node {
        cfg: cfg.clone(),
        explosion: default_explosion(cfg.explosion),
        id: owner,
        child: None,
        parent: None,
        connected: false,
        beam: None,
        ground: None,
        local: Fix::ZERO,
        surface_offset: Fix::ZERO,
        hit_offset: Fix::ZERO,
        prev_scale: Fix::ONE,
        start_scale: Fix::ONE,
        orb,
    };
    ability_body::spawn_owned(w, cfg.prefab, Kind::Behaviour(Box::new(Behaviour::TeslaCoil(Spawned::Node(n)))), &cfg.parts, p, None, 0)
}

/// Runs `f` on node `id`: `mine` if it is the running node, else the
/// world's copy (payload taken out meanwhile).
fn with_node<R>(
    w: &mut World,
    id: ObjId,
    me: ObjId,
    mine: Option<&mut Node>,
    f: impl FnOnce(&mut World, &mut Node) -> R,
) -> Option<R> {
    if id == me {
        return mine.map(|n| f(w, n));
    }
    if !w.objects.exists(id) {
        return None;
    }
    w.with_kind(id, |w, k| match k {
        Kind::Behaviour(b) => match b.as_mut() {
            Behaviour::TeslaCoil(Spawned::Node(n)) => Some(f(w, n)),
            _ => None,
        },
        _ => None,
    })
}

/// `SimpleSparkNode.Initialize(owner, child, offset, L, ground, scale)`.
#[allow(clippy::too_many_arguments)]
fn initialize(
    w: &mut World,
    node: ObjId,
    owner: i32,
    child: Option<ObjId>,
    offset: Fix,
    l: Fix,
    g: ObjId,
    scale: Fix,
    init: bool,
) {
    let (lo, hi) = w
        .objects
        .get(node)
        .and_then(|o| o.collider)
        .map_or((Fix::ZERO, Fix::MAX), |c| (c.min_scale, c.max_scale));
    if let Some(c) = child {
        with_node(w, c, node, None, |_, cn| cn.parent = Some(node));
    }
    if init {
        ability_body::manual_init(w, node);
    }
    with_node(w, node, ObjId::NONE, None, |w, n| {
        n.start_scale = scale.clamp(lo, hi);
        n.id = owner;
        n.child = child;
        n.connected = false;
        n.local = l;
        n.surface_offset = offset;
        n.ground = Some(g);
        n.surface_offset = n.surface_offset * scale;
        n.hit_offset = n.cfg.hit_offset_y * scale;
        n.prev_scale = scale;
        let _ = w;
    });
    w.set_collider_scale(node, scale);
    w.physics.weld_to_rr(&w.objects, node, g);
    reposition_on_parent(w, node);
}

/// A Duplicator clone planted on `g`: `Initialize(owner, none, offset, l,
/// g, scale)` (collider initialised now).
pub fn init_duplicate(w: &mut World, n: ObjId, owner: i32, offset: Fix, l: Fix, g: ObjId, scale: Fix) {
    initialize(w, n, owner, None, offset, l, g, scale, true);
}

/// Re-`Initialize` a carried coil on boulder `b` (Throw): owner, child,
/// `SurfaceOffset / scale`, local position 0, scale, no collider init.
pub fn reattach(w: &mut World, n: ObjId, b: ObjId, scale: Fix) {
    let Some((owner, child, so)) = with_node(w, n, ObjId::NONE, None, |_, x| (x.id, x.child, x.surface_offset)) else {
        return;
    };
    initialize(w, n, owner, child, so / scale, Fix::ZERO, b, scale, false);
}

/// `RepositionOnParent()`.
pub fn reposition_on_parent(w: &mut World, node: ObjId) {
    let Some((g, l, so)) = with_node(w, node, ObjId::NONE, None, |_, n| (n.ground, n.local, n.surface_offset)) else {
        return;
    };
    let Some(g) = g.filter(|g| w.objects.alive(*g)) else {
        return;
    };
    let Some(surf) = Surface::of(w, g) else {
        return;
    };
    let hit_ok = w
        .objects
        .get(node)
        .and_then(|o| o.part(PartKind::Collider))
        .is_some_and(|p| !p.destroyed);
    if !hit_ok {
        return;
    }
    let t = w.objects.get(node).map(|o| o.tf).unwrap_or_default();
    let (rpos, rrot) = match w.physics.rr(g) {
        Some(r) => (r.b.center, w.physics.body(g).map_or(Fix::ZERO, |b| b.orientation)),
        None => (surf.position, Fix::ZERO),
    };
    let pivot = surf.position_from_param(l, Fix::ZERO) + t.up * so;
    let inv = Vec2::from_angle(Fix::PI_TIMES_2 - rrot);
    let rel = inv.complex_mul(pivot - rpos);
    ability_body::set_relative_position(w, node, rel, t.right.complex_mul(inv));
}

fn node_pos(w: &World, n: ObjId) -> Vec2 {
    w.objects.get(n).map_or(Vec2::ZERO, |o| o.tf.position)
}

fn node_up(w: &World, n: ObjId) -> Vec2 {
    w.objects.get(n).map_or(Vec2::UP, |o| o.tf.up)
}

/// `Deactivate()` on node `p` (and its child).
fn deactivate(w: &mut World, p: ObjId, me: ObjId, mut mine: Option<&mut Node>) {
    let r = with_node(w, p, me, mine.as_deref_mut(), |w, n| {
        n.connected = false;
        if let Some(b) = n.beam.take()
            && w.objects.exists(b)
        {
            w.destroy(b);
        }
        (n.child, w.objects.alive(p))
    });
    let Some((child, alive)) = r else {
        return;
    };
    if alive {
        ability_body::set_layer(w, p, NON_LETHAL_TERRAIN);
    }
    let Some(c) = child.filter(|c| w.objects.exists(*c)) else {
        return;
    };
    with_node(w, c, me, mine, |w, n| {
        n.connected = false;
        if let Some(b) = n.beam.take()
            && w.objects.exists(b)
        {
            w.destroy(b);
        }
    });
    if w.objects.alive(c) {
        ability_body::set_layer(w, c, NON_LETHAL_TERRAIN);
    }
}

impl Node {
    fn can_connect(&self, w: &World) -> bool {
        self.child
            .is_some_and(|c| w.objects.exists(c) && w.objects.active_in_hierarchy(c))
    }

    /// `ConnectToPartner()` (this node is the parent).
    fn connect(&mut self, w: &mut World, me: ObjId) {
        let Some(c) = self.child else {
            return;
        };
        self.connected = true;
        let orb_data = self.cfg.orb_data;
        play_then(w, &mut self.orb, orb_data, "EnterOrb", CB_ACTIVATE_LIGHTNING);
        with_node(w, c, me, None, |w, n| {
            n.connected = true;
            play_then(w, &mut n.orb, orb_data, "EnterOrb", CB_ACTIVATE_PARTNER);
        });
        let up = (node_pos(w, c) - node_pos(w, me)).normalized_safe();
        let Some(bc) = self.cfg.beam.clone() else {
            return;
        };
        let pos = node_pos(w, me);
        let beam = Beam {
            cfg: bc.clone(),
            explosion: default_explosion(bc.explosion),
            id: 0,
            initialized: false,
            time_passed: Fix::ZERO,
            anim: SpriteAnimator::default(),
        };
        let b = ability_body::spawn_owned(w, bc.prefab, Kind::Behaviour(Box::new(Behaviour::TeslaCoil(Spawned::Beam(beam)))), &bc.parts, pos, None, 0);
        if let Some(o) = w.objects.get_mut(b) {
            o.active = false;
            o.tf.set_up(up);
        }
        self.beam = Some(b);
        self.update_line(w, me);
        w.emit(Cue::Sound("loadLightning"));
        // InitLightning(id).
        let id = self.id;
        if let Some(Kind::Behaviour(bb)) = w.objects.get_mut(b).map(|o| &mut o.kind)
            && let Behaviour::TeslaCoil(Spawned::Beam(beam)) = bb.as_mut()
        {
            beam.id = id;
            beam.initialized = true;
            beam.time_passed = Fix::ZERO;
            let name = beam.cfg.animation.clone();
            let data = beam.cfg.anim_data;
            let mut anim = beam.anim;
            play(w, &mut anim, data, &name);
            if let Some(Kind::Behaviour(bb)) = w.objects.get_mut(b).map(|o| &mut o.kind)
                && let Behaviour::TeslaCoil(Spawned::Beam(beam)) = bb.as_mut()
            {
                beam.anim = anim;
            }
        }
    }

    /// `UpdateLightningLine()`.
    fn update_line(&self, w: &mut World, me: ObjId) {
        let (Some(c), Some(b)) = (self.child, self.beam) else {
            return;
        };
        let child_off = with_node(w, c, me, None, |_, n| n.hit_offset).unwrap_or(Fix::ZERO);
        let a = node_pos(w, me) + node_up(w, me) * self.hit_offset;
        let d = node_pos(w, c) + node_up(w, c) * child_off - a;
        let len = d.magnitude();
        let u = if len != Fix::ZERO { d / len } else { Vec2::UP };
        let up = Vec2::new(-u.y, u.x);
        if let Some(o) = w.objects.get_mut(b) {
            o.tf.position = a + d * F_0_5;
            o.tf.set_up(up);
        }
        // CalcExtents / SetExtents (before or after the box's Init).
        let ext_y = if ability_body::collider_initialized(w, b) {
            w.physics.box_shape(b).map_or(Fix::ZERO, |s| Fix::ONE / s.ie.y)
        } else {
            w.objects
                .get(b)
                .and_then(|o| o.collider)
                .map_or(Fix::ZERO, |c| c.start_extents.y)
        };
        let mut e = Vec2::new(len * F_0_5, ext_y);
        let min = w.objects.get(b).and_then(|o| o.collider).map_or(Fix::ZERO, |c| c.minimum_extents);
        if e.x == Fix::ZERO {
            e.x = min;
        }
        if e.y == Fix::ZERO {
            e.y = min;
        }
        if ability_body::collider_initialized(w, b) {
            if let Some(s) = w.physics.box_mut(b) {
                s.update_extents(e);
            }
        } else if let Some(col) = w.objects.get_mut(b).and_then(|o| o.collider.as_mut()) {
            col.start_extents = e;
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId) {
        let scale = w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale);
        if scale != self.prev_scale {
            let f = scale / self.prev_scale;
            self.surface_offset = self.surface_offset * f;
            self.hit_offset = self.hit_offset * f;
            self.prev_scale = scale;
        }
        if self.child.is_some_and(|c| w.objects.exists(c)) && self.connected && !self.can_connect(w) {
            deactivate(w, me, me, Some(self));
        }
        if self.connected
            && let Some(p) = self.parent.filter(|p| w.objects.exists(*p))
            && !w.objects.active_in_hierarchy(p)
        {
            deactivate(w, p, me, Some(self));
        }
        if !self.connected && self.can_connect(w) && w.objects.active_in_hierarchy(me) {
            self.connect(w, me);
        }
        if self.connected
            && let Some(b) = self.beam.filter(|b| w.objects.exists(*b))
        {
            if !self.child.is_some_and(|c| w.objects.exists(c)) {
                w.destroy(b);
            } else {
                self.update_line(w, me);
            }
        }
        if !self.connected && w.objects.get(me).is_some_and(|o| o.layer == LETHAL_TERRAIN) {
            ability_body::set_layer(w, me, NON_LETHAL_TERRAIN);
        }
    }

    fn on_orb(&mut self, w: &mut World, me: ObjId, code: u16) {
        let data = self.cfg.orb_data;
        match code {
            CB_ACTIVATE_LIGHTNING => {
                if let Some(b) = self.beam.filter(|b| w.objects.exists(*b))
                    && let Some(o) = w.objects.get_mut(b)
                {
                    o.active = true;
                }
                w.emit(Cue::Sound("lightning"));
                ability_body::set_layer(w, me, LETHAL_TERRAIN);
                play_then(w, &mut self.orb, data, "TransitionOrb", CB_LOOP_ORB);
            }
            CB_ACTIVATE_PARTNER => {
                // The parent's `ActivatePartnerLightning` acts on its child,
                // i.e. on this node.
                ability_body::set_layer(w, me, LETHAL_TERRAIN);
                play_then(w, &mut self.orb, data, "TransitionOrb", CB_LOOP_ORB);
            }
            CB_LOOP_ORB => play(w, &mut self.orb, data, "LoopOrb"),
            _ => {}
        }
    }
}

impl SpawnedImpl for Spawned {
    /// Nodes: the lit orb (object 1, its own animator) and its flare
    /// (object 2) while connected. Lightning: stretched along its length
    /// (x scale = length / `lightningLineLength`, the shader's `_Scale`),
    /// the inner strip (object 1) in step.
    fn look(&self, w: &World, me: ObjId, l: &mut crate::view::Look) {
        use crate::view::PartLook;
        match self {
            Spawned::Node(n) => {
                if n.connected {
                    l.parts
                        .push(PartLook::new(1).frame(n.orb.sprite, n.orb.flip_x));
                    l.parts.push(PartLook::new(2));
                } else {
                    l.parts.push(PartLook::hidden(1));
                }
            }
            Spawned::Beam(b) => {
                l.frame(&b.anim);
                let half = w
                    .physics
                    .box_shape(me)
                    .map_or(Fix::ONE, |s| Fix::ONE / s.ie.x.max(Fix(1)));
                let unit = w
                    .content
                    .prefabs
                    .iter()
                    .find_map(|p| {
                        p.objects
                            .first()?
                            .component("SimpleSparkNode")
                            .map(|c| c.fields.fix_or("lightningLineLength", Fix::ONE))
                    })
                    .unwrap_or(Fix::ONE);
                let scale = (half * Fix::TWO / unit.max(Fix::ONE / Fix::from_int(100)))
                    .max(Fix::ONE / Fix::from_int(100));
                l.stretch = Vec2::new(scale, Fix::ONE);
                l.params = [scale, Fix::ZERO, Fix::ZERO, Fix::ZERO];
                let mut p = PartLook::new(1).frame(b.anim.sprite, b.anim.flip_x);
                p.params = l.params;
                l.parts.push(p);
            }
        }
    }

    fn interact(&mut self, _w: &mut World, _me: ObjId, i: super::Interaction) -> Option<Fix> {
        match (self, i) {
            (Spawned::Node(_), super::Interaction::ItemKind) => Some(Fix::from_int(super::ITEM_SPARK_NODE)),
            _ => None,
        }
    }

    /// Clone rule: a node keeps its tuning, everything else is reset (the
    /// duplicator re-initialises it); beams are not duplicable.
    fn duplicate(&self, _w: &World, _me: ObjId) -> Option<Spawned> {
        match self {
            Spawned::Node(n) => Some(Spawned::Node(Node {
                cfg: n.cfg.clone(),
                explosion: ExplosionInfo {
                    owner: 0,
                    ..n.explosion
                },
                id: 0,
                child: None,
                parent: None,
                connected: false,
                beam: None,
                ground: None,
                local: Fix::ZERO,
                surface_offset: Fix::ZERO,
                hit_offset: Fix::ZERO,
                prev_scale: Fix::ONE,
                start_scale: Fix::ONE,
                orb: ability_body::animator(true),
            })),
            Spawned::Beam(_) => None,
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        if late {
            return;
        }
        match self {
            Spawned::Node(n) => match part {
                PartKind::Behaviour(0) => n.update(w, me),
                // The orb animator always stops during a time stop.
                PartKind::Animator if !w.is_time_stopped() => {
                    let c = w.content.clone();
                    if let Some(AnimCallback::Ability(code)) = n.orb.advance(&c, Fix::ZERO) {
                        n.on_orb(w, me, code);
                    }
                }
                _ => {}
            },
            Spawned::Beam(b) => match part {
                PartKind::Animator => {
                    if let Some(AnimCallback::Ability(CB_UNFLICKER)) = advance_anim(w, b.id, &mut b.anim) {
                        b.time_passed = Fix::ZERO;
                        let name = b.cfg.animation.clone();
                        play(w, &mut b.anim, b.cfg.anim_data, &name);
                        w.set_collider_enabled(me, true);
                    }
                }
                PartKind::Behaviour(0) => {
                    let enabled = w.objects.get(me).is_some_and(|o| o.part_enabled(PartKind::Collider));
                    if b.initialized && enabled {
                        b.time_passed += w.fixed_delta_time(b.id, dt);
                        if b.time_passed > b.cfg.flicker_period {
                            play_then(w, &mut b.anim, b.cfg.anim_data, "flicker", CB_UNFLICKER);
                            w.set_collider_enabled(me, false);
                        }
                    }
                }
                _ => {}
            },
        }
    }

    fn on_destroy(&mut self, w: &mut World, me: ObjId, part: PartKind) {
        let Spawned::Node(n) = self else {
            return;
        };
        if part != PartKind::Behaviour(0) {
            return;
        }
        if n.connected {
            deactivate(w, me, me, Some(n));
        }
        if let Some(p) = n.parent.filter(|p| w.objects.exists(*p))
            && with_node(w, p, me, None, |_, pn| pn.connected).unwrap_or(false)
        {
            deactivate(w, p, me, Some(n));
        }
    }

    fn lethal(&self, _w: &World, _me: ObjId) -> Option<super::Lethal> {
        self.explosion().map(|e| e.lethal())
    }

    fn always_enabled(&self, part: PartKind) -> bool {
        matches!(self, Spawned::Node(_)) && part == PartKind::Behaviour(0)
    }
}

/// Live coils of the world (tests, presentation).
pub fn nodes(w: &World) -> Vec<ObjId> {
    w.objects
        .ids()
        .filter(|&id| {
            matches!(&w.objects.get(id).map(|o| &o.kind), Some(Kind::Behaviour(b)) if matches!(b.as_ref(), Behaviour::TeslaCoil(Spawned::Node(_))))
                && !w.objects.get(id).is_some_and(|o| o.destroyed)
        })
        .collect()
}

/// Live beams.
pub fn beams(w: &World) -> Vec<ObjId> {
    w.objects
        .ids()
        .filter(|&id| {
            matches!(&w.objects.get(id).map(|o| &o.kind), Some(Kind::Behaviour(b)) if matches!(b.as_ref(), Behaviour::TeslaCoil(Spawned::Beam(_))))
                && !w.objects.get(id).is_some_and(|o| o.destroyed)
        })
        .collect()
}

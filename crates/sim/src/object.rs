//! The object model: game objects with a fixed transform, a list of
//! updatable parts (components), an optional collider and a typed payload
//! (engine.md §2, §5, §6).
//!
//! Objects live in a slot arena addressed by generation-checked [`ObjId`]s,
//! so "a destroyed object compares equal to null" becomes "the handle is
//! stale" ([`Objects::get`] returns `None`).

use crate::fix::Fix;
use crate::physics::ColliderDef;
use crate::vec2::Vec2;
use crate::world::Kind;

/// Handle of an object (slot index + generation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjId {
    pub idx: u32,
    pub generation: u32,
}

impl ObjId {
    /// A handle that never resolves.
    pub const NONE: ObjId = ObjId {
        idx: u32::MAX,
        generation: 0,
    };
}

/// `FixTransform` (engine.md §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FixTransform {
    pub position: Vec2,
    pub rotation: Fix,
    pub right: Vec2,
    pub up: Vec2,
    pub offset: Vec2,
    pub scale: Fix,
    /// `IsDestroyed` of the transform component.
    pub destroyed: bool,
    pub duplicated: bool,
}

impl Default for FixTransform {
    fn default() -> FixTransform {
        FixTransform {
            position: Vec2::ZERO,
            rotation: Fix::ZERO,
            right: Vec2::RIGHT,
            up: Vec2::UP,
            offset: Vec2::ZERO,
            scale: Fix::ONE,
            destroyed: false,
            duplicated: false,
        }
    }
}

impl FixTransform {
    /// `rotation = r`.
    pub fn set_rotation(&mut self, r: Fix) {
        self.rotation = r;
        self.right = Vec2::from_angle(r);
        self.up = self.right.mul_by_i();
    }

    /// `up = u`.
    pub fn set_up(&mut self, u: Vec2) {
        self.up = u;
        self.right = u.mul_by_neg_i();
        self.rotation = self.right.normalized_vector_angle();
    }

    /// `right = v`.
    pub fn set_right(&mut self, v: Vec2) {
        self.right = v;
        self.up = v.mul_by_i();
        self.rotation = v.normalized_vector_angle();
    }

    /// `SetScale_DPhysicsOnly(s)`.
    pub fn set_scale_dphysics_only(&mut self, s: Fix) {
        let k = s / self.scale;
        self.offset = Vec2::new(self.offset.x * k, self.offset.y * k);
        self.scale = s;
    }

    /// The reset done by `InstantiateFixed` without a rotation.
    pub fn reset_rotation(&mut self) {
        self.rotation = Fix::ZERO;
        self.up = Vec2::UP;
        self.right = Vec2::RIGHT;
    }
}

/// Which component an updatable part is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartKind {
    /// A `DPhysicsBox` / `DPhysicsCircle` / `DPhysicsRoundedRect` (only `Init`).
    Collider,
    /// `BoplBody` (rigid body parameters, magnet hold, scale tracking).
    BoplBody,
    // The bopl.
    PlayerPhysics,
    SlimeController,
    PlayerCollision,
    Bounds,
    PlayerBody,
    Animator,
    // Platforms.
    Sticky,
    AnimateVelocity,
    Shakable,
    AntiLock,
    VectorField,
    GrowOnStart,
    Resizable,
    // Level.
    GameSession,
    AbilitySpawner,
    KillZone,
    // Abilities: the framework component (`Ability` / `InstantAbility`).
    Ability,
    /// A behaviour of an ability module (`n` = module-defined index).
    Behaviour(u8),
    /// A tutorial component (`tutorial.rs`; the object's `Kind::Tutorial`
    /// says which).
    Tutorial,
}

/// One updatable component of an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Part {
    pub kind: PartKind,
    /// Unity `enabled`.
    pub enabled: bool,
    /// `IsDestroyed`.
    pub destroyed: bool,
    /// Hierarchy number (after spawn offsetting).
    pub hier: i32,
}

/// Maximum number of parts per object.
pub const MAX_PARTS: usize = 12;

/// The run-time state of an object's `BoplBody` component (physics.md §2.4,
/// magnet-gun.md holds). Updated generically by the updater at the
/// `PartKind::BoplBody` part; none of it is serialized in the prefabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BoplBody {
    /// `prevScale` (starts at 1; `Init` does not set it).
    pub prev_scale: Fix,
    /// `framesUntilDrop` (−1 = not held).
    pub frames_until_drop: i64,
    /// `magnetGunHolder`: the holding magnet-gun ability object.
    pub holder: Option<ObjId>,
    /// `isResizable` (tag `ResizablePlatform`, set at Awake).
    pub resizable: bool,
}

impl Default for BoplBody {
    fn default() -> BoplBody {
        BoplBody {
            prev_scale: Fix::ONE,
            frames_until_drop: -1,
            holder: None,
            resizable: false,
        }
    }
}

impl BoplBody {
    /// `IsBeingHeld()`.
    pub fn is_held(&self) -> bool {
        self.frames_until_drop >= 0
    }

    /// `Hold(frames, magnet)`.
    pub fn hold(&mut self, frames: i64, holder: ObjId) {
        self.holder = Some(holder);
        self.frames_until_drop = frames;
    }

    /// `Drop()`.
    pub fn drop_hold(&mut self) {
        self.holder = None;
        self.frames_until_drop = -1;
    }

    /// The hold counter of `UpdateSim`: decrement, drop at exactly 0.
    pub fn tick_hold(&mut self) {
        self.frames_until_drop -= 1;
        if self.frames_until_drop == 0 {
            self.drop_hold();
        }
    }
}

/// A game object.
#[derive(Clone, Debug)]
pub struct Object {
    pub generation: u32,
    pub live: bool,
    pub kind: Kind,
    pub tf: FixTransform,
    /// `activeSelf`.
    pub active: bool,
    pub parent: Option<ObjId>,
    pub children: Vec<ObjId>,
    pub layer: u8,
    /// `IPlayerIdHolder` id (0 = none).
    pub player_id: i32,
    /// Queued for destruction (`DestroyFix(gameObject)`).
    pub destroyed: bool,
    pub parts: [Part; MAX_PARTS],
    pub n_parts: u8,
    pub collider: Option<ColliderDef>,
    /// `BoplBody` state (meaningful if the object has a `BoplBody` part).
    pub bb: BoplBody,
    /// Presentation: which imported object this was built from.
    pub visual: crate::view::Visual,
}

impl Object {
    /// Index of the first part of a kind.
    pub fn part_index(&self, k: PartKind) -> Option<usize> {
        self.parts[..self.n_parts as usize]
            .iter()
            .position(|p| p.kind == k)
    }

    pub fn part(&self, k: PartKind) -> Option<&Part> {
        self.part_index(k).map(|i| &self.parts[i])
    }

    pub fn part_mut(&mut self, k: PartKind) -> Option<&mut Part> {
        self.part_index(k).map(move |i| &mut self.parts[i])
    }

    /// Enabled flag of a part (false if absent).
    pub fn part_enabled(&self, k: PartKind) -> bool {
        self.part(k).is_some_and(|p| p.enabled)
    }

    pub fn set_part_enabled(&mut self, k: PartKind, e: bool) {
        if let Some(p) = self.part_mut(k) {
            p.enabled = e;
        }
    }

    /// Adds a part (returns its index).
    pub fn add_part(&mut self, kind: PartKind, hier: i32) -> usize {
        let i = self.n_parts as usize;
        assert!(i < MAX_PARTS, "too many parts");
        self.parts[i] = Part {
            kind,
            enabled: true,
            destroyed: false,
            hier,
        };
        self.n_parts += 1;
        i
    }
}

const EMPTY_PART: Part = Part {
    kind: PartKind::Collider,
    enabled: false,
    destroyed: true,
    hier: 0,
};

/// The object arena.
#[derive(Clone, Debug, Default)]
pub struct Objects {
    pub slots: Vec<Object>,
    free: Vec<u32>,
}

impl Objects {
    /// Creates an object (active, no parts).
    pub fn create(&mut self, kind: Kind, layer: u8) -> ObjId {
        let obj = |generation| Object {
            generation,
            live: true,
            kind,
            tf: FixTransform::default(),
            active: true,
            parent: None,
            children: Vec::new(),
            layer,
            player_id: 0,
            destroyed: false,
            parts: [EMPTY_PART; MAX_PARTS],
            n_parts: 0,
            collider: None,
            bb: BoplBody::default(),
            visual: Default::default(),
        };
        if let Some(i) = self.free.pop() {
            let g = self.slots[i as usize].generation.wrapping_add(1);
            self.slots[i as usize] = obj(g);
            ObjId {
                idx: i,
                generation: g,
            }
        } else {
            let i = self.slots.len() as u32;
            self.slots.push(obj(1));
            ObjId {
                idx: i,
                generation: 1,
            }
        }
    }

    /// Frees a slot (the actual Unity destroy).
    pub fn free(&mut self, id: ObjId) {
        if let Some(o) = self.slots.get_mut(id.idx as usize)
            && o.live
            && o.generation == id.generation
        {
            o.live = false;
            o.kind = Kind::Empty;
            o.children = Vec::new();
            self.free.push(id.idx);
        }
    }

    #[inline]
    pub fn get(&self, id: ObjId) -> Option<&Object> {
        let o = self.slots.get(id.idx as usize)?;
        (o.live && o.generation == id.generation).then_some(o)
    }

    #[inline]
    pub fn get_mut(&mut self, id: ObjId) -> Option<&mut Object> {
        let o = self.slots.get_mut(id.idx as usize)?;
        (o.live && o.generation == id.generation).then_some(o)
    }

    /// Whether the handle still names a live object.
    #[inline]
    pub fn exists(&self, id: ObjId) -> bool {
        self.get(id).is_some()
    }

    /// `activeInHierarchy`.
    pub fn active_in_hierarchy(&self, id: ObjId) -> bool {
        let mut cur = Some(id);
        let mut depth = 0;
        while let Some(c) = cur {
            match self.get(c) {
                Some(o) if o.active => cur = o.parent,
                _ => return false,
            }
            depth += 1;
            if depth > 64 {
                return false;
            }
        }
        true
    }

    /// Unity's `x != null && !x.IsDestroyed` for a whole object.
    pub fn alive(&self, id: ObjId) -> bool {
        self.get(id).is_some_and(|o| !o.destroyed)
    }

    /// The nearest object (self first) with a player id, walking up parents.
    pub fn owner_id(&self, id: ObjId) -> i32 {
        let mut cur = Some(id);
        while let Some(c) = cur {
            match self.get(c) {
                Some(o) if o.player_id != 0 => return o.player_id,
                Some(o) => cur = o.parent,
                None => return 0,
            }
        }
        0
    }

    /// Sets the parent (keeps child lists in sync).
    pub fn set_parent(&mut self, child: ObjId, parent: Option<ObjId>) {
        if let Some(old) = self.get(child).and_then(|o| o.parent)
            && let Some(p) = self.get_mut(old)
        {
            p.children.retain(|c| *c != child);
        }
        if let Some(p) = parent
            && let Some(po) = self.get_mut(p)
        {
            po.children.push(child);
        }
        if let Some(c) = self.get_mut(child) {
            c.parent = parent;
        }
    }

    /// Live object ids in slot order.
    pub fn ids(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, o)| o.live)
            .map(|(i, o)| ObjId {
                idx: i as u32,
                generation: o.generation,
            })
    }
}

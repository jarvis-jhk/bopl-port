//! The ability framework (abilities.md §3–§13) and the plug-in registry.
//!
//! Each ability lives in its own module `abilities/<name>.rs` and implements
//! [`AbilityImpl`]; the [`registry!`] below lists every module once (one
//! line per ability) and generates the enum dispatch. A module may also
//! define objects it spawns (projectiles, explosions, ...) through its
//! `Spawned` type, which implements [`SpawnedImpl`]. See `docs/sim-design.md`.

pub mod ability_body;
pub mod explosion;
pub mod pickup;
pub mod prefab;
pub mod rope;

use crate::bopl::{Bopl, PlayerCollision, RopeLink, set_player_position};
use crate::data::{AbilityDef, AbilityKind};
use crate::fix::Fix;
use crate::object::{ObjId, PartKind};
use crate::physics::Contact;
use crate::player::CauseOfDeath;
use crate::vec2::Vec2;
use crate::view::Visual;
use crate::world::{Kind, World};

/// Everything an ability module needs (`use super::prelude::*;`).
pub mod prelude {
    pub use super::{
        AbilityExitInfo, AbilityImpl, Cx, Framework, Interaction, Lethal, NoSpawn, PlayerInfo,
        SpawnedImpl,
    };
    pub use crate::anim::{AnimCallback, SpriteAnimator};
    pub use crate::consts::*;
    pub use crate::data::{AbilityDef, Value};
    pub use crate::fix::Fix;
    pub use crate::object::{ObjId, PartKind};
    pub use crate::physics::Contact;
    pub use crate::player::CauseOfDeath;
    pub use crate::vec2::Vec2;
    pub use crate::view::Cue;
    pub use crate::world::World;
}

/// The entry snapshot handed to an ability (`PlayerInfo`, abilities.md §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlayerInfo {
    pub player_id: i32,
    /// `AbilityButtonUsedIndex012`.
    pub slot: usize,
    pub position: Vec2,
    pub external_velocity: Vec2,
    pub self_imposed_velocity: Vec2,
    pub is_grounded: bool,
    pub grounded_speed: Fix,
    pub rotation: Fix,
    pub forwards: Vec2,
    pub up: Vec2,
    pub facing_right: bool,
    pub ground: Option<ObjId>,
    pub grounded_local_position: Fix,
    pub radius: Fix,
    pub rope: Option<RopeLink>,
    pub use_rope_on_other_player: bool,
    /// `playerToUseRopeOn`: the remembered rope target (physics parent).
    pub rope_target: Option<ObjId>,
    /// The casting bopl (`slimeController`).
    pub controller: ObjId,
    pub collision_ignored: [Option<ObjId>; 6],
}

/// The exit record (`AbilityExitInfo`, abilities.md §8.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AbilityExitInfo {
    pub is_dead: bool,
    pub killer_id: i32,
    pub cause: CauseOfDeath,
    pub position: Vec2,
    pub external_velocity: Vec2,
    pub self_imposed_velocity: Vec2,
    pub facing_right: bool,
    pub is_grounded: bool,
    pub ground: Option<ObjId>,
    pub grounded_local_position: Fix,
    pub grounded_speed: Fix,
    pub radius: Fix,
    pub just_landed: bool,
    pub enter_turn: bool,
    pub enter_jump: bool,
    pub rope: Option<RopeLink>,
}

impl AbilityExitInfo {
    /// Return exactly to the entry state (used by abilities that end at once).
    pub fn resume(i: &PlayerInfo) -> AbilityExitInfo {
        AbilityExitInfo {
            position: i.position,
            external_velocity: i.external_velocity,
            self_imposed_velocity: if i.is_grounded {
                Vec2::ZERO
            } else {
                i.self_imposed_velocity
            },
            facing_right: i.facing_right,
            is_grounded: i.is_grounded,
            ground: i.ground,
            grounded_local_position: i.grounded_local_position,
            grounded_speed: i.grounded_speed,
            radius: i.radius,
            ..AbilityExitInfo::default()
        }
    }
}

/// Who owns a lethal object and whether it can hurt its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Lethal {
    pub owner: i32,
    pub can_hurt_owner: bool,
    /// Tagged `beam` (cause Beamed).
    pub beam: bool,
}

/// The `Ability` / `InstantAbility` base component state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Framework {
    pub owner: i32,
    pub controller: ObjId,
    pub instant: bool,
    pub cooldown: Fix,
    pub only_castable_on_ground: bool,
    pub kill_players_on_contact: bool,
    pub is_pushable: bool,
    pub is_castable: bool,
    pub cooldown_stopped_during_time_stop: bool,
    pub ignore_next_cooldown: bool,
    pub is_rope: bool,
    pub is_invisibility: bool,
    /// The snapshot of the current use.
    pub info: Option<PlayerInfo>,
    /// `prevScale` for scale propagation.
    pub prev_scale: Fix,
    /// Inside the ability right now.
    pub in_use: bool,
    /// The ability object has its own `PlayerCollision` (hurt box).
    pub collision: Option<PlayerCollision>,
}

/// Context of an ability call.
pub struct Cx<'a> {
    pub w: &'a mut World,
    pub me: ObjId,
    pub fw: &'a mut Framework,
}

impl Cx<'_> {
    /// `Ability.ExitAbility(record)` (abilities.md §8.2).
    pub fn exit(&mut self, mut info: AbilityExitInfo) {
        exit_ability(self.w, self.me, self.fw, &mut info);
    }

    /// [`Cx::exit`] of a module with a body: the body's rope goes into the
    /// record first (abilities.md §8.2 step 1).
    pub fn exit_body(&mut self, body: &mut ability_body::AbilityBody, mut info: AbilityExitInfo) {
        body.hand_back_rope(&mut info);
        self.exit(info);
    }

    pub fn info(&self) -> PlayerInfo {
        self.fw.info.expect("ability used without a snapshot")
    }

    pub fn owner(&self) -> i32 {
        self.fw.owner
    }

    /// `Ability.UpdateSim` (abilities.md §9): scale propagation (returns the
    /// multiplier when `Player.Scale` changed; the caller runs its
    /// `OnScaleChanged`), collider scale, `Player.SetPosition`.
    pub fn ability_update(&mut self) -> Option<Fix> {
        let scale = self.w.player(self.fw.owner).map_or(Fix::ONE, |p| p.scale());
        let mut changed = None;
        if scale != self.fw.prev_scale {
            let m = scale / self.fw.prev_scale;
            self.fw.prev_scale = scale;
            changed = Some(m);
        }
        if self
            .w
            .objects
            .get(self.me)
            .is_some_and(|o| o.collider.is_some())
        {
            self.w.set_collider_scale(self.me, scale);
        }
        let pos = self
            .w
            .objects
            .get(self.me)
            .map_or(Vec2::ZERO, |o| o.tf.position);
        let ctrl = self.fw.controller;
        let n = ability_count(self.w, ctrl);
        set_player_position(self.w, self.fw.owner, pos, ctrl, n);
        changed
    }

    /// The ability object is active and in use (its later components run).
    pub fn running(&self) -> bool {
        self.fw.in_use && self.w.objects.get(self.me).is_some_and(|o| o.active)
    }

    /// Whether the slot's button is still held.
    pub fn button_down(&self) -> bool {
        let slot = self.fw.info.map_or(0, |i| i.slot);
        self.w
            .player(self.fw.owner)
            .is_some_and(|p| p.inputs.input.ability(slot))
    }
}

/// An ability module (one file per ability).
///
/// The ability object always has the `Ability` part (its updates go to
/// [`AbilityImpl::tick`]) and, if the prefab root has one, its collider.
/// A module adds the prefab's other components in [`AbilityImpl::build`]:
/// either as separate parts (dispatched to [`AbilityImpl::init_part`] /
/// [`AbilityImpl::update_part`] at their own hierarchy positions) or it
/// runs them inside `tick` in hierarchy order (the object's updatables are
/// contiguous in the updater's list, so both give the original order).
pub trait AbilityImpl: Clone + std::fmt::Debug + std::hash::Hash + PartialEq + Eq {
    /// Builds the per-object state from the ability's data
    /// (`def.prefab` indexes `Content::prefabs`).
    fn new(w: &World, def: &AbilityDef) -> Self;

    /// Called by [`instantiate`] inside the spawn bracket after the
    /// `Ability` part and the root collider were added and before
    /// registration: add the prefab's other parts, set `fw.collision`
    /// ([`ability_body::AbilityBody::install`] / `install_parts`,
    /// [`ability_body::add_parts`]).
    fn build(&mut self, _w: &mut World, _me: ObjId, _fw: &mut Framework, _def: &AbilityDef) {}

    /// One update (`late` = `LateUpdateSim`) of the `Ability` part. The
    /// default runs the components before `Ability` ([`AbilityImpl::pre_update`]),
    /// the `Ability` component, then [`AbilityImpl::update`] /
    /// [`AbilityImpl::late_update`]; modules whose order differs override
    /// this and call [`Cx::ability_update`] at the right place.
    fn tick(&mut self, cx: &mut Cx<'_>, dt: Fix, late: bool) {
        if late {
            self.late_update(cx, dt);
            return;
        }
        self.pre_update(cx, dt);
        if !cx.running() {
            return;
        }
        if let Some(m) = cx.ability_update() {
            self.on_scale_changed(cx, m);
        }
        if cx.fw.in_use {
            self.update(cx, dt);
        }
    }

    /// Components that sit **before** the `Ability` component in the
    /// object's update order (e.g. a `SpriteAnimator` 1 before `Ability` 2),
    /// run at the Ability part's slot by the default [`AbilityImpl::tick`].
    fn pre_update(&mut self, _cx: &mut Cx<'_>, _dt: Fix) {}

    /// `UpdateSim` of the module's component(s) after `Ability`.
    fn update(&mut self, _cx: &mut Cx<'_>, _dt: Fix) {}

    /// `LateUpdateSim`.
    fn late_update(&mut self, _cx: &mut Cx<'_>, _dt: Fix) {}

    /// `Init()` of a part the module added in `build` (not `Ability`).
    fn init_part(&mut self, _cx: &mut Cx<'_>, _part: PartKind) {}

    /// `UpdateSim` / `LateUpdateSim` of a part the module added in `build`
    /// (not `Ability`), in its own hierarchy position.
    fn update_part(&mut self, _cx: &mut Cx<'_>, _part: PartKind, _dt: Fix, _late: bool) {}

    /// `OnEnterAbility()` (after the framework's enter steps, which include
    /// handing the snapshot's rope to [`AbilityImpl::body_mut`]). The default
    /// ends the ability at once (unimplemented abilities behave like
    /// `RandomAbility`).
    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        let i = cx.info();
        cx.exit(AbilityExitInfo::resume(&i));
    }

    /// The first `IAbilityComponent`'s `ExitAbility(info)`: the module fills
    /// the record fields it owns and calls the framework exit
    /// ([`Cx::exit_body`] for modules with a body, which hands the rope
    /// back). Used for the module's own exits and by others forcing an exit
    /// (death, bounds, drop-through platforms, the magnet gun).
    fn exit_component(&mut self, cx: &mut Cx<'_>, info: AbilityExitInfo) {
        match self.body_mut() {
            Some(b) => cx.exit_body(b, info),
            None => cx.exit(info),
        }
    }

    /// A contact on the ability object's own collider.
    fn on_contact(&mut self, _cx: &mut Cx<'_>, _c: &Contact) {}

    /// An animator callback with a module-defined code.
    fn on_anim(&mut self, _cx: &mut Cx<'_>, _code: u16) {}

    /// `OnScaleChanged(m)`.
    fn on_scale_changed(&mut self, _cx: &mut Cx<'_>, _m: Fix) {}

    /// `IsCastable(player)` beyond the framework flag.
    fn is_castable(&self, _w: &World, _fw: &Framework, _player: i32) -> bool {
        true
    }

    /// `PlayerCollision.killPlayer(killer, effect, ignoreInvulnerability,
    /// cause)` on the ability object (player.md §8.2); returns whether it was
    /// killed. The default runs the body's part ([`ability_body::AbilityBody::kill`]:
    /// rope release, invulnerability, death effect) and then
    /// [`AbilityImpl::on_killed`].
    fn kill(
        &mut self,
        cx: &mut Cx<'_>,
        killer: i32,
        effect: bool,
        ignore_invulnerability: bool,
        cause: CauseOfDeath,
    ) -> bool {
        if let Some(b) = self.body_mut()
            && !b.kill(cx.w, cx.me, cx.fw, effect, ignore_invulnerability)
        {
            return false;
        }
        self.on_killed(cx, killer, cause);
        true
    }

    /// The dead exit after a successful kill: the first ability
    /// component's exit with {position, isDead, killer, cause}.
    fn on_killed(&mut self, cx: &mut Cx<'_>, killer: i32, cause: CauseOfDeath) {
        let r = ability_body::dead_record(cx.w, cx.me, killer, cause);
        self.exit_component(cx, r);
    }

    /// The object's player body (`PlayerPhysics`/`PlayerBody`/collision),
    /// for the framework (rope hand-over) and for other modules (ropes,
    /// teleports, pushes, black-hole pull, magnets, shockwaves).
    fn body(&self) -> Option<&ability_body::AbilityBody> {
        None
    }

    fn body_mut(&mut self) -> Option<&mut ability_body::AbilityBody> {
        None
    }

    /// Another ability acts on this ability object through one of its
    /// component interfaces ([`Interaction`]; e.g. the Rock's `BounceBall`
    /// held by a magnet). `None` = no such component (the default).
    fn interact(&mut self, _cx: &mut Cx<'_>, _i: Interaction) -> Option<Fix> {
        None
    }

    /// Lethality of this ability object when it touches a bopl (Player layer).
    fn lethal(&self, fw: &Framework) -> Option<Lethal> {
        fw.kill_players_on_contact.then_some(Lethal {
            owner: fw.owner,
            can_hurt_owner: true,
            beam: false,
        })
    }

    /// Current sprite for the view.
    fn sprite(&self) -> crate::data::SpriteId {
        crate::data::SpriteId::NONE
    }

    /// How the ability object looks (presentation only; the default shows
    /// [`AbilityImpl::sprite`] on the player-coloured root renderer).
    fn look(&self, _w: &World, _me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        l.sprite = self.sprite();
    }
}

/// Objects spawned by an ability module (projectiles, explosions, ...).
pub trait SpawnedImpl: Clone + std::fmt::Debug + std::hash::Hash + PartialEq + Eq {
    fn init(&mut self, _w: &mut World, _me: ObjId, _part: PartKind) {}
    fn update(&mut self, _w: &mut World, _me: ObjId, _part: PartKind, _dt: Fix, _late: bool) {}
    fn on_contact(&mut self, _w: &mut World, _me: ObjId, _c: &Contact) {}
    fn on_destroy(&mut self, _w: &mut World, _me: ObjId, _part: PartKind) {}
    /// Owner and `canHurtOwner` when this object kills a bopl (Projectile,
    /// Explosion, LethalTerrain, orb layers). `None` = harmless.
    fn lethal(&self, _w: &World, _me: ObjId) -> Option<Lethal> {
        None
    }
    /// `IsEnabled()` overridden to true: the part also runs while its object
    /// is inactive (e.g. `SimpleSparkNode`).
    fn always_enabled(&self, _part: PartKind) -> bool {
        false
    }

    /// Another ability acts on this object through one of its component
    /// interfaces (`ISpell`, `BlackHole`, held items, ...). `None` = the
    /// object has no such component (the default).
    fn interact(&mut self, _w: &mut World, _me: ObjId, _i: Interaction) -> Option<Fix> {
        None
    }

    /// The payload of a Duplicator clone of this object (duplicator.md,
    /// clone rule: Unity copies the serialized fields and resets the
    /// runtime ones). The default copies everything; modules reset their
    /// runtime fields here. `None` = the object cannot be cloned.
    fn duplicate(&self, _w: &World, _me: ObjId) -> Option<Self>
    where
        Self: Sized,
    {
        Some(self.clone())
    }

    /// How the object looks (presentation only; default: the prefab's
    /// renderers as authored).
    fn look(&self, _w: &World, _me: ObjId, _l: &mut crate::view::Look) {}
}

/// Component interfaces of spawned objects used by other abilities (the
/// owning module answers in [`SpawnedImpl::interact`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interaction {
    /// `ISpell.AddScale(inc)` (scale rays on `spell`-layer objects).
    SpellAddScale(Fix),
    /// What kind of item the object is (answer: one of the `ITEM_*` codes).
    ItemKind,
    /// `FlammableSmoke.Ignite()` (rocket-engine flames).
    Ignite,
    /// `AttachRope(rope, top)` on a non-PlayerBody rope holder (the Rock's
    /// `BounceBall`).
    AttachRope { rope: Option<u16>, top: bool },
    /// The magnet holds the item for `dt`: a grenade's fuse
    /// (`selfDestructDelay += dt`) or a Rock ball's activation timer
    /// (`timeSinceActivation −= dt`) pauses.
    DelayFuse(Fix),
    /// Sent to a Duplicator clone after it was placed (duplicator.md): the
    /// type-specific fix-ups (grenade fuse, item owner, missile heading,
    /// planted engine/spike/spark node re-initialisation, explosion owner).
    /// `index` is the clone index (−1 for terrain), `shooter` the gun owner.
    /// (Planted engines, spikes and spark nodes are re-initialised by the
    /// duplicator through their modules' `init_duplicate`.)
    Duplicated {
        original: ObjId,
        index: i32,
        shooter: i32,
    },
}

/// [`Interaction::ItemKind`] answers (`Fix::from_int(code)`).
pub const ITEM_MISSILE: i32 = 1;
pub const ITEM_MINE: i32 = 2;
pub const ITEM_GRENADE: i32 = 3;
pub const ITEM_BOUNCE_BALL: i32 = 4;
pub const ITEM_ROCKET_ENGINE: i32 = 5;
pub const ITEM_SPIKE: i32 = 6;
pub const ITEM_SPARK_NODE: i32 = 7;

/// A module without spawned objects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NoSpawn;

impl SpawnedImpl for NoSpawn {}

macro_rules! registry {
    ($($name:literal => $m:ident :: $t:ident),* $(,)?) => {
        $(pub mod $m;)*

        /// Per-ability module state (one variant per module).
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub enum AbilityState {
            $($t($m::$t),)*
            /// An ability name without a module.
            Unknown,
        }

        /// Objects spawned by ability modules (plus framework objects).
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub enum Behaviour {
            Pickup(pickup::Pickup),
            $($t($m::Spawned),)*
        }

        impl AbilityState {
            /// The module for an ability-list name.
            pub fn create(w: &World, def: &AbilityDef) -> AbilityState {
                match def.name.as_str() {
                    $($name => AbilityState::$t(<$m::$t as AbilityImpl>::new(w, def)),)*
                    _ => AbilityState::Unknown,
                }
            }
        }

        impl Behaviour {
            /// The payload of a Duplicator clone ([`SpawnedImpl::duplicate`]).
            pub fn duplicate(&self, w: &World, me: ObjId) -> Option<Behaviour> {
                match self {
                    Behaviour::Pickup(b) => b.duplicate(w, me).map(Behaviour::Pickup),
                    $(Behaviour::$t(b) => b.duplicate(w, me).map(Behaviour::$t),)*
                }
            }
        }

        macro_rules! with_ability {
            ($s:expr, $a:ident => $e:expr, $d:expr) => {
                match $s {
                    $(AbilityState::$t($a) => $e,)*
                    AbilityState::Unknown => $d,
                }
            };
        }

        macro_rules! with_spawned {
            ($s:expr, $a:ident => $e:expr) => {
                match $s {
                    Behaviour::Pickup($a) => $e,
                    $(Behaviour::$t($a) => $e,)*
                }
            };
        }
    };
}

// One line per ability: ability-list name => module::Type.
registry! {
    "NullAbility" => null_ability::NullAbility,
    "Random" => random::Random,
    "Dash" => dash::Dash,
    "Grenade" => grenade::Grenade,
    "Bow" => bow::Bow,
    "Engine" => engine::Engine,
    "Blink gun" => blink_gun::BlinkGun,
    "Gust" => gust::Gust,
    "Growth ray" => growth_ray::GrowthRay,
    "Rock" => rock::Rock,
    "Missile" => missile::Missile,
    "Spike" => spike::Spike,
    "Time stop" => time_stop::TimeStop,
    "Smoke" => smoke::Smoke,
    "Platform" => platform::Platform,
    "Revival" => revival::Revival,
    "Roll" => roll::Roll,
    "Shrink ray" => shrink_ray::ShrinkRay,
    "Black hole" => black_hole::BlackHole,
    "Invisibility" => invisibility::Invisibility,
    "Meteor" => meteor::Meteor,
    "Throw" => throw::Throw,
    "Push" => push::Push,
    "Tesla coil" => tesla_coil::TeslaCoil,
    "Mine" => mine::Mine,
    "Teleport" => teleport::Teleport,
    "Drill" => drill::Drill,
    "Grappling hook" => grappling_hook::GrapplingHook,
    "Beam" => beam::Beam,
    "Duplicator" => duplicator::Duplicator,
    "Magnet gun" => magnet_gun::MagnetGun,
}

/// An ability object's payload.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AbilityObject {
    /// Ability-list index.
    pub def: usize,
    pub fw: Framework,
    pub state: AbilityState,
}

impl AbilityObject {
    pub fn init(&mut self, w: &mut World, me: ObjId, k: PartKind) {
        if k == PartKind::Ability {
            return;
        }
        let mut cx = Cx {
            w,
            me,
            fw: &mut self.fw,
        };
        with_ability!(&mut self.state, a => a.init_part(&mut cx, k), ());
    }

    pub fn update(&mut self, w: &mut World, me: ObjId, k: PartKind, dt: Fix, late: bool) {
        if k != PartKind::Ability {
            // Module components of the object (parts added in `build`).
            if self.fw.in_use {
                let mut cx = Cx {
                    w,
                    me,
                    fw: &mut self.fw,
                };
                with_ability!(&mut self.state, a => a.update_part(&mut cx, k, dt, late), ());
            }
            return;
        }
        if !self.fw.in_use {
            return;
        }
        let mut cx = Cx {
            w,
            me,
            fw: &mut self.fw,
        };
        with_ability!(&mut self.state, a => a.tick(&mut cx, dt, late), ());
    }

    pub fn on_contact(&mut self, w: &mut World, me: ObjId, c: Contact) {
        let mut cx = Cx {
            w,
            me,
            fw: &mut self.fw,
        };
        with_ability!(&mut self.state, a => a.on_contact(&mut cx, &c), ());
    }

    /// Sprite for the view.
    pub fn sprite(&self) -> crate::data::SpriteId {
        with_ability!(&self.state, a => a.sprite(), crate::data::SpriteId::NONE)
    }

    pub fn facing_right(&self) -> bool {
        self.fw.info.is_none_or(|i| i.facing_right)
    }

    /// The module's presentation state ([`AbilityImpl::look`]).
    pub fn look(&self, w: &World, me: ObjId, l: &mut crate::view::Look) {
        with_ability!(&self.state, a => a.look(w, me, &self.fw, l), ())
    }

    /// The module's ability body, if any.
    pub fn body(&self) -> Option<&ability_body::AbilityBody> {
        with_ability!(&self.state, a => a.body(), None)
    }

    pub fn body_mut(&mut self) -> Option<&mut ability_body::AbilityBody> {
        with_ability!(&mut self.state, a => a.body_mut(), None)
    }
}

/// The ability body of an ability object (`None` while its payload is
/// taken out for its own update).
pub fn ability_body(w: &World, a: ObjId) -> Option<&ability_body::AbilityBody> {
    match &w.objects.get(a)?.kind {
        Kind::Ability(x) => x.body(),
        _ => None,
    }
}

/// [`ability_body`], mutable (shockwave and gust pushes, black-hole pull).
pub fn ability_body_mut(w: &mut World, a: ObjId) -> Option<&mut ability_body::AbilityBody> {
    match &mut w.objects.get_mut(a)?.kind {
        Kind::Ability(x) => x.body_mut(),
        _ => None,
    }
}

/// `Ability.isPushable` of an ability object (None for other objects).
pub fn is_pushable(w: &World, a: ObjId) -> Option<bool> {
    fw_of(w, a).map(|f| f.is_pushable)
}

/// `IPlatformAttachable.RepositionOnParent()` of a platform's attachable
/// children (layers LethalTerrain, NonLethalTerrain, Projectile), called by
/// the rounded-rect scale setter before the shape is rescaled.
pub fn reposition_attachables(w: &mut World, plat: ObjId) {
    let children = w
        .objects
        .get(plat)
        .map(|o| o.children.clone())
        .unwrap_or_default();
    for c in children {
        let Some(o) = w.objects.get(c) else {
            continue;
        };
        if !matches!(o.layer, 16 | 21 | 9) {
            continue;
        }
        if let Kind::Behaviour(b) = &o.kind {
            match b.as_ref() {
                Behaviour::Spike(_) => spike::reposition_on_parent(w, c),
                Behaviour::TeslaCoil(tesla_coil::Spawned::Node(_)) => {
                    tesla_coil::reposition_on_parent(w, c)
                }
                _ => {}
            }
        }
    }
}

impl Behaviour {
    pub fn init(&mut self, w: &mut World, me: ObjId, k: PartKind) {
        with_spawned!(self, b => b.init(w, me, k))
    }
    pub fn update(&mut self, w: &mut World, me: ObjId, k: PartKind, dt: Fix, late: bool) {
        with_spawned!(self, b => b.update(w, me, k, dt, late))
    }
    pub fn on_contact(&mut self, w: &mut World, me: ObjId, c: Contact) {
        with_spawned!(self, b => b.on_contact(w, me, &c))
    }
    pub fn on_destroy(&mut self, w: &mut World, me: ObjId, k: PartKind) {
        with_spawned!(self, b => b.on_destroy(w, me, k))
    }
    pub fn lethal(&self, w: &World, me: ObjId) -> Option<Lethal> {
        with_spawned!(self, b => b.lethal(w, me))
    }
    pub fn always_enabled(&self, part: PartKind) -> bool {
        with_spawned!(self, b => b.always_enabled(part))
    }
    pub fn interact(&mut self, w: &mut World, me: ObjId, i: Interaction) -> Option<Fix> {
        with_spawned!(self, b => b.interact(w, me, i))
    }
    pub fn look(&self, w: &World, me: ObjId, l: &mut crate::view::Look) {
        with_spawned!(self, b => b.look(w, me, l))
    }
}

/// Emits a presentation prefab cue for a prefab reference `field` on one of
/// `obj`'s prefab components ([`crate::view::Cue::Fx`]); nothing if `obj` has
/// no prefab visual.
#[allow(clippy::too_many_arguments)]
pub fn fx(
    w: &mut World,
    obj: ObjId,
    field: &'static str,
    position: Vec2,
    rotation: Fix,
    scale: Fix,
    player_id: i32,
    value: Vec2,
) {
    if let Some(Visual::Prefab { prefab, .. }) = w.objects.get(obj).map(|o| o.visual) {
        w.emit(crate::view::Cue::Fx(crate::view::FxCue {
            prefab,
            field,
            position,
            rotation,
            scale,
            player_id,
            value,
            color: None,
            source: obj,
        }));
    }
}

/// Presentation of a gun ray (`spawnRayCastEffect`): the trail prefab
/// `trail` stretched from `start` along `dir` over `dist` (`value` =
/// (dist, did hit)), and the hit prefab `hit` at its end when it hit
/// (rotated to `normal`).
#[allow(clippy::too_many_arguments)]
pub fn ray_fx(
    w: &mut World,
    gun: ObjId,
    trail: &'static str,
    hit: &'static str,
    start: Vec2,
    dir: Vec2,
    dist: Fix,
    did_hit: bool,
    normal: Option<Vec2>,
    owner: i32,
) {
    let rot = Fix::atan2(dir.y, dir.x);
    let flag = if did_hit { Fix::ONE } else { Fix::ZERO };
    fx(
        w,
        gun,
        trail,
        start,
        rot,
        Fix::ONE,
        owner,
        Vec2::new(dist, flag),
    );
    if did_hit {
        let r = normal.map_or(Fix::ZERO, rotation_of_up);
        fx(
            w,
            gun,
            hit,
            start + dir * dist,
            r,
            Fix::ONE,
            owner,
            Vec2::ZERO,
        );
    }
}

/// The rotation (radians) whose local up points along `up`.
pub fn rotation_of_up(up: Vec2) -> Fix {
    Fix::atan2(up.y, up.x) - Fix::PI / Fix::TWO
}

/// [`fx`] with an explicit prefab (`Content::prefabs` index).
#[allow(clippy::too_many_arguments)]
pub fn fx_of(
    w: &mut World,
    prefab: u32,
    field: &'static str,
    position: Vec2,
    rotation: Fix,
    scale: Fix,
    player_id: i32,
    value: Vec2,
) {
    w.emit(crate::view::Cue::Fx(crate::view::FxCue {
        prefab,
        field,
        position,
        rotation,
        scale,
        player_id,
        value,
        color: None,
        source: ObjId::NONE,
    }));
}

/// A float `Color` field (`{r, g, b, a}`) as Fix rgba (presentation data).
pub fn color_field(v: &crate::data::Value) -> Option<[Fix; 4]> {
    let c = |k: &str| v.get(k).and_then(prefab::float_fix);
    Some([c("r")?, c("g")?, c("b")?, c("a").unwrap_or(Fix::ONE)])
}

/// The look of an ability-made platform (Throw's boulder, Platform's
/// resizable platform): `params` = inner extents x, y and radius of its
/// rounded rect (world units), boulder type + 1 (0 = not a boulder).
pub fn platform_look(
    w: &World,
    me: ObjId,
    p: &crate::platform::Platform,
    l: &mut crate::view::Look,
) {
    if let Some(rr) = w.physics.rr(me) {
        l.params[0] = Fix::ONE / rr.b.ie.x.max(Fix(1));
        l.params[1] = Fix::ONE / rr.b.ie.y.max(Fix(1));
        l.params[2] = rr.radius;
    }
    if p.boulder.is_some() {
        l.params[3] = Fix::from_int(p.sticky.platform_type + 1);
    }
    // The Platform ability's platform: its caster's `_ShadowColor`.
    if p.resize.is_some() {
        l.shadow_color = true;
        for id in w.objects.ids() {
            if let Some(Kind::Ability(a)) = w.objects.get(id).map(|o| &o.kind)
                && let AbilityState::Platform(pa) = &a.state
                && (pa.buffer == Some(me) || pa.current == Some(me))
            {
                l.player = a.fw.owner;
                break;
            }
        }
    }
}

/// Sends an [`Interaction`] to a spawned object (`None` if it has no such
/// component or is not a spawned object).
pub fn interact(w: &mut World, obj: ObjId, i: Interaction) -> Option<Fix> {
    if !matches!(
        w.objects.get(obj).map(|o| &o.kind),
        Some(Kind::Behaviour(_) | Kind::Ability(_))
    ) {
        return None;
    }
    w.with_kind(obj, |w, k| match k {
        Kind::Behaviour(b) => b.interact(w, obj, i),
        Kind::Ability(x) => {
            let mut cx = Cx {
                w,
                me: obj,
                fw: &mut x.fw,
            };
            with_ability!(&mut x.state, s => s.interact(&mut cx, i), None)
        }
        _ => None,
    })
}

/// Whether `obj` is a Rock ability object (its `BounceBall` component).
pub fn is_bounce_ball(w: &World, obj: ObjId) -> bool {
    match w.objects.get(obj).map(|o| &o.kind) {
        Some(Kind::Ability(a)) => w
            .content
            .abilities
            .get(a.def)
            .is_some_and(|d| d.name == "Rock"),
        _ => false,
    }
}

/// Global state of ability subsystems (ropes, beams, duplicates, parries).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Globals {
    /// Exits requested while the controller's payload was taken out
    /// (an ability ending inside the cast call); applied by the caster.
    pub pending_exits: Vec<(ObjId, AbilityExitInfo)>,
    /// `BlackHole.BlackHoleIndex` (spawn counter for sorting / merge ties;
    /// reset with the level).
    pub black_hole_index: i32,
    /// Changes to a caster's `PlayerBody` requested during an instant cast
    /// (grappling hook); applied by the caster after the cast.
    pub pending_body_ops: Vec<(ObjId, grappling_hook::BodyOp)>,
    /// Blink gun: the quantum tunnel list.
    pub blink: blink_gun::BlinkGlobals,
    /// Magnet gun: parry circles of this tick, BoplBody holds.
    pub magnet: magnet_gun::MagnetGlobals,
    /// The DetPhysics beam system.
    pub beams: beam::BeamSystem,
    /// Duplicator: the duplicated-objects list, running platform effects.
    pub duplicates: duplicator::DuplicateList,
}

fn ability_count(w: &World, ctrl: ObjId) -> usize {
    match w.objects.get(ctrl).map(|o| &o.kind) {
        Some(Kind::Bopl(b)) => b.ctrl.abilities.len(),
        _ => 0,
    }
}

fn fw_of(w: &World, a: ObjId) -> Option<&Framework> {
    match &w.objects.get(a)?.kind {
        Kind::Ability(x) => Some(&x.fw),
        _ => None,
    }
}

// -- queries used by the bopl ------------------------------------------------

pub fn cooldown(w: &World, a: ObjId) -> Fix {
    fw_of(w, a).map_or(Fix::ONE, |f| f.cooldown)
}
pub fn cooldown_stopped_during_time_stop(w: &World, a: ObjId) -> bool {
    fw_of(w, a).is_some_and(|f| f.cooldown_stopped_during_time_stop)
}
pub fn is_rope(w: &World, a: ObjId) -> bool {
    fw_of(w, a).is_some_and(|f| f.is_rope)
}
pub fn ignore_next_cooldown(w: &World, a: ObjId) -> bool {
    fw_of(w, a).is_some_and(|f| f.ignore_next_cooldown)
}
pub fn only_castable_on_ground(w: &World, a: ObjId) -> bool {
    fw_of(w, a).is_some_and(|f| f.only_castable_on_ground)
}
pub fn is_instant(w: &World, a: ObjId) -> bool {
    fw_of(w, a).is_some_and(|f| f.instant)
}

/// `IsCastable(player)` (abilities.md §5.3).
pub fn is_castable(w: &World, a: ObjId, player: i32) -> bool {
    let Some(Kind::Ability(x)) = w.objects.get(a).map(|o| &o.kind) else {
        return false;
    };
    if x.fw.instant {
        if x.fw.is_invisibility && w.player(player).is_some_and(|p| p.is_invisible) {
            return false;
        }
    } else if !x.fw.is_castable {
        return false;
    }
    with_ability!(&x.state, s => s.is_castable(w, &x.fw, player), true)
}

/// Rope cooldown update hook (grappling hook module).
pub fn update_rope_cooldowns(w: &mut World, a: ObjId, _bopl: ObjId, rope: Option<RopeLink>) {
    grappling_hook::update_cooldowns(w, a, rope);
}

/// `InstantiateFixed(abilityPrefab, (0,0))` + `SetActive(false)`.
pub fn instantiate(w: &mut World, idx: usize, owner: i32, controller: ObjId) -> ObjId {
    let def = w.content.abilities.get(idx).cloned();
    let Some(def) = def else {
        return ObjId::NONE;
    };
    let state = AbilityState::create(w, &def);
    let base_hier = def.fields.int("hierarchyNumber").map_or(1, |h| h as i32);
    // The prefab root: its hurt box collider, transform offset, layer and
    // whether it has a `PlayerCollision`.
    let root = def
        .prefab
        .and_then(|p| w.content.prefabs.get(p))
        .and_then(|p| p.objects.first())
        .map(|o| {
            (
                o.collider.zip(o.collider_hier),
                o.collider_enabled,
                o.fix.map_or(Vec2::ZERO, |t| t.offset),
                o.layer,
                o.component("PlayerCollision").is_some(),
            )
        });
    let fw = Framework {
        owner,
        controller,
        instant: def.kind == AbilityKind::Instant,
        cooldown: def.cooldown,
        only_castable_on_ground: def.only_castable_on_ground,
        kill_players_on_contact: def.kill_players_on_contact,
        is_pushable: def.is_pushable,
        is_castable: def.fields.boolean("isCastable").unwrap_or(true),
        cooldown_stopped_during_time_stop: def.cooldown_stopped_during_time_stop,
        ignore_next_cooldown: false,
        is_rope: def.is_rope,
        is_invisibility: def.fields.boolean("isInvisibilty").unwrap_or(false),
        info: None,
        prev_scale: Fix::ONE,
        in_use: false,
        collision: root.is_some_and(|r| r.4).then(|| PlayerCollision {
            ignored_beam_id: -1,
            ..PlayerCollision::default()
        }),
    };
    let total = def
        .prefab
        .and_then(|p| w.content.prefabs.get(p))
        .map_or(1, |p| p.updatable_count());
    w.open_bracket();
    let id = w.objects.create(
        Kind::Ability(Box::new(AbilityObject {
            def: idx,
            fw,
            state,
        })),
        root.map_or(8, |r| r.3),
    );
    if let Some(o) = w.objects.get_mut(id) {
        o.player_id = owner;
        o.visual = match def.prefab {
            Some(p) => Visual::Prefab {
                prefab: p as u32,
                object: 0,
            },
            None => Visual::None,
        };
        o.add_part(PartKind::Ability, base_hier);
        if let Some((shape, enabled, offset, _, _)) = root {
            if let Some((col, hier)) = shape {
                o.collider = Some(col);
                let k = o.add_part(PartKind::Collider, hier);
                o.parts[k].enabled = enabled;
            }
            o.tf.offset = offset;
        }
    }
    w.with_kind(id, |w, k| {
        if let Kind::Ability(x) = k {
            with_ability!(&mut x.state, s => s.build(w, id, &mut x.fw, &def), ());
            // The body belongs to the ability's owner.
            if let Some(b) = x.body_mut() {
                b.owner = owner;
            }
        }
    });
    let parts = w.objects.get(id).map_or(1, |o| o.n_parts as i32);
    w.register_all(id);
    w.close_bracket();
    // The prefab's other updatables (module components) take numbers too.
    w.consume_hierarchy((total - parts).max(0));
    if let Some(o) = w.objects.get_mut(id) {
        o.tf.position = Vec2::ZERO;
        o.tf.reset_rotation();
        o.active = false;
    }
    id
}

/// Enters an ability (abilities.md §7) with the snapshot built by the bopl.
pub fn enter(w: &mut World, a: ObjId, info: PlayerInfo) {
    let ctrl = info.controller;
    w.with_kind(a, |w, k| {
        let Kind::Ability(x) = k else {
            return;
        };
        if x.fw.instant {
            // InstantAbility.EnterAbility: activate, cast, deactivate.
            x.fw.info = Some(info);
            if let Some(o) = w.objects.get_mut(a) {
                o.active = true;
                o.tf.position = info.position;
            }
            let mut cx = Cx {
                w,
                me: a,
                fw: &mut x.fw,
            };
            with_ability!(&mut x.state, s => {
                // Instant abilities run their logic in on_enter and never exit.
                let saved = cx.fw.in_use;
                cx.fw.in_use = true;
                s.update(&mut cx, Fix::ZERO);
                cx.fw.in_use = saved;
            }, ());
            if let Some(o) = w.objects.get_mut(a) {
                o.active = false;
            }
            return;
        }
        x.fw.info = Some(info);
        x.fw.in_use = true;
        if let Some(o) = w.objects.get_mut(a) {
            o.active = true;
        }
        if let Some(c) = x.fw.collision.as_mut() {
            c.ignored = info.collision_ignored;
        }
        // The ability body takes over the snapshot's rope (§7.1 step 2.3).
        if let Some(b) = x.body_mut() {
            b.take_snapshot_rope(w, &info);
        }
        let scale = w.player(x.fw.owner).map_or(Fix::ONE, |p| p.scale());
        if w.objects.get(a).is_some_and(|o| o.collider.is_some()) {
            w.set_collider_scale(a, scale);
        }
        let pos = w.objects.get(a).map_or(info.position, |o| o.tf.position);
        let n = ability_count(w, ctrl);
        set_player_position(w, x.fw.owner, pos, ctrl, n);
        let mut cx = Cx {
            w,
            me: a,
            fw: &mut x.fw,
        };
        with_ability!(&mut x.state, s => s.on_enter(&mut cx), {
            let i = cx.info();
            cx.exit(AbilityExitInfo::resume(&i));
        });
    });
}

/// `Ability.ExitAbility(record)` (abilities.md §8.2).
fn exit_ability(w: &mut World, me: ObjId, fw: &mut Framework, info: &mut AbilityExitInfo) {
    // No guard against a second exit (spec-review item 2): every call runs
    // in full, like the original.
    fw.in_use = false;
    let pos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
    let ctrl = fw.controller;
    let n = ability_count(w, ctrl);
    set_player_position(w, fw.owner, pos, ctrl, n);
    if let Some(o) = w.objects.get_mut(me) {
        o.active = false;
    }
    let ignored = fw.collision.as_ref().map(|c| c.ignored);
    let info = *info;
    // Deliver to the controller (deferred if it is running right now).
    match w.objects.get(ctrl).map(|o| &o.kind) {
        Some(Kind::Bopl(_)) => {
            w.with_kind(ctrl, |w, k| {
                if let Kind::Bopl(b) = k {
                    if let Some(ig) = ignored {
                        b.collision.ignored = ig;
                    }
                    b.on_exit_ability(w, ctrl, info);
                }
            });
        }
        Some(Kind::Empty) => w.ability_globals.pending_exits.push((ctrl, info)),
        _ => {}
    }
}

/// `Ability.ExitAbility(record)` called from outside the ability object
/// (black hole, magnet gun): the module's own exit cleanup is skipped.
pub fn framework_exit(w: &mut World, a: ObjId, info: AbilityExitInfo) {
    w.with_kind(a, |w, k| {
        if let Kind::Ability(x) = k
            && !x.fw.instant
        {
            let mut cx = Cx {
                w,
                me: a,
                fw: &mut x.fw,
            };
            // `Ability.ExitAbility` itself hands the body's rope back.
            match with_ability!(&mut x.state, s => s.body_mut(), None) {
                Some(b) => cx.exit_body(b, info),
                None => cx.exit(info),
            }
        }
    });
}

/// Runs `f` on the `PlayerPhysics` and `PlayerBody` of a bopl or of an
/// ability object that has them (`None` otherwise, or while that object's
/// payload is taken out for its own update).
pub fn with_player_body<R>(
    w: &mut World,
    obj: ObjId,
    f: impl FnOnce(&mut World, &mut crate::bopl::PlayerPhysics, &mut crate::bopl::PlayerBody) -> R,
) -> Option<R> {
    w.with_kind(obj, |w, k| match k {
        Kind::Bopl(b) => Some(f(w, &mut b.physics, &mut b.body)),
        Kind::Ability(x) => {
            with_ability!(&mut x.state, s => s.body_mut().map(|b| f(w, &mut b.physics, &mut b.body)), None)
        }
        _ => None,
    })
}

/// Calls the first `IAbilityComponent`'s `ExitAbility(info)` of ability
/// object `a` from outside its own update (magnet-gun pickup, ...).
pub fn exit_component(w: &mut World, a: ObjId, info: AbilityExitInfo) {
    w.with_kind(a, |w, k| {
        if let Kind::Ability(x) = k {
            let mut cx = Cx {
                w,
                me: a,
                fw: &mut x.fw,
            };
            with_ability!(&mut x.state, s => s.exit_component(&mut cx, info), cx.exit(info));
        }
    });
}

/// Applies exits that were deferred while the bopl was running.
pub fn apply_pending_exits(w: &mut World, me: ObjId, b: &mut Bopl) {
    grappling_hook::apply_body_ops(w, me, &mut b.body);
    let mut i = 0;
    while i < w.ability_globals.pending_exits.len() {
        if w.ability_globals.pending_exits[i].0 == me {
            let (_, info) = w.ability_globals.pending_exits.remove(i);
            b.on_exit_ability(w, me, info);
        } else {
            i += 1;
        }
    }
}

/// The ability object `obj` has its own `PlayerCollision`.
pub fn has_player_collision(w: &World, obj: ObjId) -> bool {
    fw_of(w, obj).is_some_and(|f| f.collision.is_some())
}

/// The casting bopl behind an ability object.
pub fn controller_of(w: &World, obj: ObjId) -> Option<ObjId> {
    fw_of(w, obj).map(|f| f.controller)
}

fn active_ability(w: &World, ctrl: ObjId) -> Option<ObjId> {
    let Some(Kind::Bopl(b)) = w.objects.get(ctrl).map(|o| &o.kind) else {
        return None;
    };
    b.ctrl
        .abilities
        .iter()
        .copied()
        .find(|a| fw_of(w, *a).is_some_and(|f| f.in_use))
}

/// `GetActivePlayerCollision().IsInvulnerable` while in an ability.
pub fn active_collision_invulnerable(w: &World, ctrl: ObjId) -> bool {
    active_ability(w, ctrl)
        .and_then(|a| fw_of(w, a))
        .and_then(|f| f.collision.as_ref())
        .is_some_and(|c| c.is_invulnerable())
}

/// Kills the active ability body (it exits as dead).
pub fn kill_active_ability(
    w: &mut World,
    ctrl: ObjId,
    killer: i32,
    effect: bool,
    ign: bool,
    cause: CauseOfDeath,
) {
    let Some(a) = active_ability(w, ctrl) else {
        return;
    };
    w.with_kind(a, |w, k| {
        if let Kind::Ability(x) = k {
            let mut cx = Cx {
                w,
                me: a,
                fw: &mut x.fw,
            };
            with_ability!(&mut x.state, s => { s.kill(&mut cx, killer, effect, ign, cause); }, ());
        }
    });
}

/// Kill rules by layer for the other object of a hurtbox contact
/// (player.md §8.1 step 5).
pub fn lethal_info(
    w: &World,
    other: ObjId,
    layer: u8,
    squished: bool,
    ignored_beam: i32,
) -> Option<Lethal> {
    let o = w.objects.get(other)?;
    let pid = o.player_id;
    match layer {
        26 => match &o.kind {
            Kind::Behaviour(b) => b.lethal(w, other),
            _ => None,
        },
        18 | 16
            if ignored_beam >= 0
                && beam::beam_box_id(w, other).is_some_and(|id| id.abs() == ignored_beam) =>
        {
            None
        }
        18 | 16 => match &o.kind {
            Kind::Behaviour(b) => b.lethal(w, other).or(Some(Lethal {
                owner: w.objects.owner_id(other),
                can_hurt_owner: true,
                beam: false,
            })),
            _ => Some(Lethal {
                owner: w.objects.owner_id(other),
                can_hurt_owner: true,
                beam: false,
            }),
        },
        9 => match &o.kind {
            Kind::Behaviour(b) => b.lethal(w, other).or(Some(Lethal {
                owner: pid,
                can_hurt_owner: false,
                beam: false,
            })),
            _ => Some(Lethal {
                owner: pid,
                can_hurt_owner: false,
                beam: false,
            }),
        },
        8 => {
            // Only ability objects (self or parent) that kill on contact.
            let mut cur = Some(other);
            while let Some(c) = cur {
                let ob = w.objects.get(c)?;
                if let Kind::Ability(x) = &ob.kind {
                    return with_ability!(&x.state, s => s.lethal(&x.fw), None);
                }
                cur = ob.parent;
            }
            None
        }
        _ if squished => Some(Lethal {
            owner: crate::consts::ENVIRONMENT_ID,
            can_hurt_owner: false,
            beam: false,
        }),
        _ => None,
    }
}

// -- hooks for subsystems owned by ability modules -------------------------------

/// Rope handling in `PlayerPhysics.LateUpdateSim` (grappling hook).
pub fn rope_grounded_late_update(w: &mut World, me: ObjId, b: &mut Bopl) {
    let cfg = rope::RopeCfg::player(w);
    let p = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
    let Some(n) = b
        .physics
        .attached_ground
        .and_then(|g| crate::surface::Surface::of(w, g))
        .map(|s| s.normal(p))
    else {
        return;
    };
    if let Some(un) = rope::grounded_late_update(w, &mut b.body, p, n, cfg) {
        if un {
            b.unground(w, me, true, false);
        }
        if let Some(l) = b.body.rope {
            let p = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
            rope::pin_end(w, l, p);
        }
    }
}
/// The body holds an enabled rope.
pub fn rope_enabled(w: &World, rope: Option<RopeLink>) -> bool {
    rope::enabled(w, rope)
}
/// `PlayerBody.UpdateSim` roped branch; true if it moved the body.
pub fn rope_body_update(w: &mut World, me: ObjId, b: &mut Bopl, dt: Fix) -> bool {
    let cfg = rope::RopeCfg::player(w);
    let id = b.ctrl.player_number;
    rope::body_update(w, me, id, &mut b.body, Some(&b.physics), cfg, dt)
}
/// `PlayerBody.LateUpdateSim` rope part.
pub fn rope_body_late_update(w: &mut World, _me: ObjId, b: &mut Bopl) {
    let cfg = rope::RopeCfg::player(w);
    rope::body_late_update(w, &mut b.body, cfg);
}
/// `AttachRope(null)` on death.
pub fn release_rope(w: &mut World, _me: ObjId, b: &mut Bopl) {
    if let Some(l) = b.body.rope {
        let cfg = rope::RopeCfg::player(w);
        rope::attach_rope(w, &mut b.body, cfg, None, l.top);
    }
}
/// Tick step 12.
pub fn clear_rope_flags(w: &mut World) {
    w.physics.ropes.very_late_update();
}
/// Tick step 5 (duplicator).
pub fn update_duplicates(w: &mut World) {
    duplicator::update_duplicates(w);
}
/// `DetPhysics.Simulate` step 8, between the broad phase and the narrow
/// phase: ropes (grappling hook), then beams.
pub fn physics_mid_step(w: &mut World) {
    beam::update_beams(w);
}

/// Tick step 8 (magnet-gun parries).
pub fn static_update(w: &mut World) {
    magnet_gun::static_update(w);
}

/// End of invisibility (invisibility module).
pub fn invisibility_ended(w: &mut World, me: ObjId, b: &mut Bopl) {
    invisibility::invisibility_ended(w, me, b);
}

/// Death with placed revive positions (Revival module).
pub fn revive_on_death(w: &mut World, me: ObjId, b: &mut Bopl, killer: i32, cause: CauseOfDeath) {
    revival::revive_on_death(w, me, b, killer, cause);
}

/// `PrepareToRevive` resets the revive slot (Revival module).
pub fn on_prepare_to_revive(w: &mut World, me: ObjId, b: &mut Bopl) {
    revival::on_prepare_to_revive(w, me, b);
}

/// Spawns an ability pickup (`DynamicAbilityPickup`).
pub fn spawn_pickup(w: &mut World, pos: Vec2, ability: Option<usize>, dir: Vec2) {
    pickup::spawn(w, pos, ability, dir);
}

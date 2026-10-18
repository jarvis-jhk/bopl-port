//! The presentation `View`: what to draw this tick and which cues to play
//! (see `docs/sim-design.md` §View). Everything here is derived from the
//! state; building a view never changes the simulation.

use crate::data::SpriteId;
use crate::fix::{Fault, Fix};
use crate::object::ObjId;
use crate::player::CauseOfDeath;
use crate::vec2::Vec2;
use crate::world::{Kind, World};

/// Where an object's look comes from (stored on every object).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Visual {
    #[default]
    None,
    /// Level platform `i` (index into `LevelData::platforms`).
    Platform(u16),
    /// Level static collider `i` (`LevelData::statics`).
    Static(u16),
    /// The bopl (sprite from its animator, material from the player colour).
    Bopl,
    /// Object `object` of `Content::prefabs[prefab]`.
    Prefab { prefab: u32, object: u16 },
}

/// Stable id of an event: rollback re-simulation of the same tick produces
/// the same ids, so the presentation plays each cue once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EventId {
    pub tick: i64,
    pub seq: u32,
}

/// Presentation-only effect kinds (the original spawns these as prefabs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EffectKind {
    /// `deathEffect` (gibs, ground splash).
    Death,
    /// `WaterDeathEffect` (splash at the water line).
    WaterSplash,
    /// Blast-zone exit up / sideways (uses the death effect prefab).
    BlastZone,
    /// Revive / respawn effect.
    Revive,
    /// Ability-defined effect (module code).
    Ability(u16),
}

/// A cue or notification.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Cue {
    /// Play a named sound (`AudioManager` names: `jump`, `land`, ...).
    Sound(&'static str),
    /// Play a random sound with this name prefix (`easeInBlock`, ...).
    SoundPrefix(&'static str),
    /// A per-player sound: the name gets the player id appended
    /// (`slimeJump` + 1 = `slimeJump1`).
    PlayerSound(&'static str, i32),
    /// Spawn a presentation effect at a position.
    Effect {
        kind: EffectKind,
        position: Vec2,
        player_id: i32,
    },
    PlayerSpawned {
        id: i32,
    },
    /// A player's last life ended.
    PlayerDied {
        id: i32,
        killer: i32,
        cause: CauseOfDeath,
    },
    /// Camera trauma (shake strength, original float units ×1000).
    Shake(i32),
    RoundWon {
        team: i32,
    },
    RoundDraw,
    WinnerScreen,
    SuddenDeath,
    /// A fixed-point exception (division by zero, ...): the round ends as a
    /// draw like on the original's online host.
    Fault(Fault),
    /// A presentation prefab the original instantiates here (dust, sparks,
    /// flares, ray trails, ...).
    Fx(FxCue),
}

/// [`FxCue::prefab`] of references on the level's `DetPhysics` component
/// (beam and duplicate effects).
pub const LEVEL_PREFAB: u32 = u32::MAX;

/// `Instantiate(prefabField, position, rotation)` of a presentation-only
/// prefab: `prefab` is the emitting object's prefab (`Content::prefabs`
/// index) and `field` the serialized prefab reference on one of its
/// components (`dustParticlePrefab`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FxCue {
    pub prefab: u32,
    pub field: &'static str,
    pub position: Vec2,
    /// Radians.
    pub rotation: Fix,
    pub scale: Fix,
    pub player_id: i32,
    /// Per-effect value (ray length, end point x, ...).
    pub value: Vec2,
    /// Start colour of its particles (rgba).
    pub color: Option<[Fix; 4]>,
    /// The emitting object (`ObjId::NONE` when not tied to one).
    pub source: ObjId,
}

/// One event of a tick.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Event {
    pub id: EventId,
    pub cue: Cue,
}

/// A level platform's pose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatformView {
    /// Index into `LevelData::platforms` (session collection order).
    pub index: usize,
    pub object: ObjId,
    pub position: Vec2,
    pub rotation: Fix,
    /// Collider scale (1 = authored size).
    pub scale: Fix,
    pub visible: bool,
}

/// What kind of dynamic object a sprite is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpriteKind {
    Bopl,
    Ability,
    Spawned,
}

/// Pose of a child renderer relative to its object's visual root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartPose {
    pub position: Vec2,
    /// Radians.
    pub rotation: Fix,
    pub scale: Vec2,
}

/// A child renderer of an object's prefab visual as the module shows it
/// (`object` indexes the visual's prefab objects).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartLook {
    pub object: u16,
    pub visible: bool,
    /// Current frame; `NONE` = the prefab's sprite.
    pub sprite: SpriteId,
    pub flip_x: bool,
    /// Local pose; `None` = the prefab's.
    pub pose: Option<PartPose>,
    /// Renderer alpha multiplier.
    pub alpha: Fix,
    /// Shader values (as [`Look::params`]).
    pub params: [Fix; 4],
    /// Tinted with the owner's player-material `_ShadowColor` (white for
    /// the black slime), keeping the renderer's alpha.
    pub shadow_color: bool,
    /// Replaces the renderer colour (rgba).
    pub tint: Option<[Fix; 4]>,
    /// Uses the owner's player material.
    pub player_material: bool,
}

/// A presentation prefab kept alive at an object while listed (particle
/// trails, auras): `field` is a prefab reference on one of the object's
/// prefab components (as [`FxCue::field`]); its particles stop emitting
/// when it is no longer listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attach {
    pub field: &'static str,
    /// World position and rotation (radians).
    pub position: Vec2,
    pub rotation: Fix,
    pub scale: Fix,
    /// Start colour of its particles (rgba).
    pub color: Option<[Fix; 4]>,
    /// Its particles take the owner's `_ShadowColor`.
    pub shadow_color: bool,
}

impl PartLook {
    /// A visible part with the prefab's sprite and pose.
    pub fn new(object: u16) -> PartLook {
        PartLook {
            object,
            visible: true,
            sprite: SpriteId::NONE,
            flip_x: false,
            pose: None,
            alpha: Fix::ONE,
            params: [Fix::ZERO; 4],
            shadow_color: false,
            tint: None,
            player_material: false,
        }
    }

    /// A hidden part.
    pub fn hidden(object: u16) -> PartLook {
        PartLook {
            visible: false,
            ..PartLook::new(object)
        }
    }

    /// With an animator's frame.
    pub fn frame(mut self, sprite: SpriteId, flip_x: bool) -> PartLook {
        self.sprite = sprite;
        self.flip_x = flip_x;
        self
    }

    pub fn posed(mut self, position: Vec2, rotation: Fix, scale: Vec2) -> PartLook {
        self.pose = Some(PartPose {
            position,
            rotation,
            scale,
        });
        self
    }
}

/// What a module reports about how one of its objects looks right now
/// (presentation only; filled by the modules' `look` hooks). The meaning
/// of `params` is per visual (documented at the module's hook).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    /// Current frame of the root renderer; `NONE` = the prefab's sprite.
    pub sprite: SpriteId,
    pub flip_x: bool,
    /// Root renderer alpha multiplier (invisibility, fades).
    pub alpha: Fix,
    /// Extra render scale of the root renderer (stretched smears, beams).
    pub stretch: Vec2,
    /// The root renderer draws nothing (its parts may).
    pub hidden: bool,
    /// The root renderer uses the owner's player material (ability bodies
    /// get it on enter in the original; arrows, rope colours).
    pub player_material: bool,
    /// Child renderers the module controls (others keep the prefab's
    /// state).
    pub parts: Vec<PartLook>,
    /// Module-defined presentation values (shader inputs, phases).
    pub params: [Fix; 4],
    /// A looping sound that plays while the object shows it.
    pub loop_sound: Option<&'static str>,
    /// The root renderer (and the object's particles) are tinted with the
    /// owner's `_ShadowColor`.
    pub shadow_color: bool,
    /// Presentation prefabs following the object.
    pub attach: Vec<Attach>,
    /// The player the object shows (colour, material), if not its owner
    /// object's (0 = the object's owner).
    pub player: i32,
    /// Other objects of the module that are hidden right now (a cursor).
    pub hide: Vec<ObjId>,
}

impl Default for Look {
    fn default() -> Look {
        Look {
            sprite: SpriteId::NONE,
            flip_x: false,
            alpha: Fix::ONE,
            stretch: Vec2::new(Fix::ONE, Fix::ONE),
            hidden: false,
            player_material: false,
            parts: Vec::new(),
            params: [Fix::ZERO; 4],
            loop_sound: None,
            shadow_color: false,
            attach: Vec::new(),
            player: 0,
            hide: Vec::new(),
        }
    }
}

impl Look {
    /// The same frame on the prefab objects that have their own
    /// `SpriteAnimator` playing in step with the root (explosion layers).
    pub fn frame_children(&mut self, w: &World, prefab: Option<usize>) {
        let Some(p) = prefab.and_then(|i| w.content.prefabs.get(i)) else {
            return;
        };
        for (i, o) in p.objects.iter().enumerate().skip(1) {
            if o.component("SpriteAnimator").is_some() {
                self.parts
                    .push(PartLook::new(i as u16).frame(self.sprite, self.flip_x));
            }
        }
    }

    /// Sets the root frame from an animator.
    pub fn frame(&mut self, a: &crate::anim::SpriteAnimator) {
        self.sprite = a.sprite;
        self.flip_x = a.flip_x;
    }
}

/// A rope (DetPhysics rope body) as a point chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RopeView {
    pub points: Vec<Vec2>,
    pub owner: i32,
}

/// A dynamic sprite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteView {
    pub object: ObjId,
    pub kind: SpriteKind,
    pub visual: Visual,
    /// Current frame (map with `Content::sprites`); `NONE` = use the prefab's.
    pub sprite: SpriteId,
    pub flip_x: bool,
    pub position: Vec2,
    pub rotation: Fix,
    pub scale: Fix,
    /// Unity layer (8 Player, 12 effect, ...).
    pub layer: u8,
    pub player_id: i32,
    pub team: i32,
    /// Player colour index (material), -1 if none.
    pub color: i32,
    /// Facing (the bopl's `FlippingTransform`).
    pub facing_right: bool,
    /// The module's presentation state.
    pub look: Look,
}

/// One ability slot of a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotView {
    /// Ability-list index.
    pub ability: usize,
    pub timer: Fix,
    pub cooldown: Fix,
    pub castable: bool,
}

/// Per-player state for HUD, camera and arrows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerView {
    pub id: i32,
    pub team: i32,
    pub color: i32,
    pub alive: bool,
    pub spawned: bool,
    pub in_ability: bool,
    pub invisible: bool,
    pub position: Vec2,
    pub bopl: Option<ObjId>,
    pub games_won: i32,
    pub kills: i32,
    pub deaths: i32,
    pub won_this_round: bool,
    pub cause_of_death: CauseOfDeath,
    pub slots: Vec<SlotView>,
    /// Chosen abilities (ability-list indices, 1 = Random), edited by the
    /// mid-game wheel.
    pub chosen: Vec<usize>,
    pub kills_at_start_of_round: i32,
    pub eaten_this_round: i32,
    pub is_most_recent_winner: bool,
    pub uses_mouse: bool,
    pub scale: Fix,
    /// The bopl stands on a platform (slime trail drawn).
    pub grounded: bool,
    /// Presentation alpha of the bopl, its ability objects and indicators
    /// (invisibility.md: fades out over the first second, back in over the
    /// last, at least 0.2).
    pub alpha: Fix,
}

/// `AnimationCurve.EaseInOut(0, 1, 1, 0)`: 1 − (3t² − 2t³) on [0, 1].
fn ease_out_fade(t: Fix) -> Fix {
    if t <= Fix::ZERO {
        return Fix::ONE;
    }
    if t >= Fix::ONE {
        return Fix::ZERO;
    }
    Fix::ONE - (Fix::THREE * t * t - Fix::TWO * t * t * t)
}

/// The invisibility alpha of a player (`SetAlphaBasedOnPlayerInvisibiltyTime`).
/// Online, the original keeps a local player at least 0.2 visible to
/// themselves; that floor is the presentation's (it knows which players are
/// local), not applied here.
pub fn player_alpha(p: &crate::player::Player) -> Fix {
    if !p.is_invisible {
        return Fix::ONE;
    }
    let remaining = Fix::from_int(5) - p.time_spent_invisible;
    if remaining <= Fix::ONE {
        ease_out_fade(remaining)
    } else {
        ease_out_fade(p.time_since_entered_ability)
    }
}

/// Round state for the HUD and flow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoundView {
    pub in_progress: bool,
    pub decided: bool,
    pub winner_screen: bool,
    pub sudden_death: bool,
    pub time_stopped: bool,
}

/// Everything the presentation needs for one tick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub tick: i64,
    pub time_since_level_loaded: Fix,
    pub platforms: Vec<PlatformView>,
    pub sprites: Vec<SpriteView>,
    pub players: Vec<PlayerView>,
    pub round: RoundView,
    pub events: Vec<Event>,
    /// Session flow: circles, wheels, winner-screen timing (`flow.rs`).
    pub flow: crate::flow::Flow,
    /// Ability slots shown (`NumberOfAbilities`).
    pub number_of_abilities: usize,
    /// Enabled ropes.
    pub ropes: Vec<RopeView>,
}

impl World {
    /// Builds the presentation view of the current state.
    pub fn view(&self) -> View {
        let mut v = View {
            tick: self.time.simulation_ticks,
            time_since_level_loaded: self.time.sim_time_since_level_loaded,
            round: RoundView {
                in_progress: self.round.game_in_progress,
                decided: self.round.decided,
                winner_screen: self.round.winner_screen,
                sudden_death: self.round.sudden_death,
                time_stopped: self.is_time_stopped(),
            },
            events: self.events.clone(),
            flow: self.flow.clone(),
            number_of_abilities: self.settings.number_of_abilities,
            ..View::default()
        };
        for (i, &id) in self.platforms.iter().enumerate() {
            match self.objects.get(id) {
                Some(o) => v.platforms.push(PlatformView {
                    index: i,
                    object: id,
                    position: o.tf.position,
                    rotation: o.tf.rotation,
                    scale: o.tf.scale,
                    visible: !o.destroyed && self.objects.active_in_hierarchy(id),
                }),
                None => v.platforms.push(PlatformView {
                    index: i,
                    object: id,
                    position: Vec2::ZERO,
                    rotation: Fix::ZERO,
                    scale: Fix::ONE,
                    visible: false,
                }),
            }
        }
        // Platforms outside the session's list (Duplicator clones).
        for id in self.objects.ids() {
            let o = self.objects.get(id).unwrap();
            if let (Kind::Platform(_), Visual::Platform(i)) = (&o.kind, o.visual)
                && !self.platforms.contains(&id)
            {
                v.platforms.push(PlatformView {
                    index: i as usize,
                    object: id,
                    position: o.tf.position,
                    rotation: o.tf.rotation,
                    scale: o.tf.scale,
                    visible: !o.destroyed && self.objects.active_in_hierarchy(id),
                });
            }
        }
        for id in self.objects.ids() {
            let o = self.objects.get(id).unwrap();
            if !self.objects.active_in_hierarchy(id) || o.destroyed {
                continue;
            }
            let pid = self.objects.owner_id(id);
            let pl = self.player(pid);
            let mut look = Look::default();
            let (kind, sprite, flip, facing) = match &o.kind {
                Kind::Bopl(b) => {
                    look.alpha = pl.map_or(Fix::ONE, player_alpha);
                    (
                        SpriteKind::Bopl,
                        b.anim.sprite,
                        b.anim.flip_x,
                        b.ctrl.facing_right,
                    )
                }
                Kind::Ability(a) => {
                    look.player_material = true;
                    a.look(self, id, &mut look);
                    look.alpha = look.alpha * pl.map_or(Fix::ONE, player_alpha);
                    (
                        SpriteKind::Ability,
                        look.sprite,
                        look.flip_x,
                        a.facing_right(),
                    )
                }
                Kind::Behaviour(b) => {
                    b.look(self, id, &mut look);
                    (SpriteKind::Spawned, look.sprite, look.flip_x, true)
                }
                // Ability-made platforms (Throw's boulders, Platform's
                // resizable platform): params = shape extents x, y and
                // radius (unscaled), boulder sprite index + 1.
                Kind::Platform(p) if matches!(o.visual, Visual::Prefab { .. }) => {
                    crate::abilities::platform_look(self, id, p, &mut look);
                    (SpriteKind::Spawned, look.sprite, look.flip_x, true)
                }
                // Plain module objects (the Platform ability's cursor).
                Kind::Empty if matches!(o.visual, Visual::Prefab { .. }) => {
                    (SpriteKind::Spawned, look.sprite, look.flip_x, true)
                }
                _ => continue,
            };
            let (pid, pl) = if look.player != 0 {
                (look.player, self.player(look.player))
            } else {
                (pid, pl)
            };
            v.sprites.push(SpriteView {
                object: id,
                kind,
                visual: o.visual,
                sprite,
                flip_x: flip,
                position: o.tf.position,
                rotation: o.tf.rotation,
                scale: o.tf.scale,
                layer: o.layer,
                player_id: pid,
                team: pl.map_or(-1, |p| p.team),
                color: pl.map_or(-1, |p| p.color),
                facing_right: facing,
                look,
            });
        }
        let hidden: Vec<ObjId> = v.sprites.iter().flat_map(|s| s.look.hide.clone()).collect();
        if !hidden.is_empty() {
            v.sprites.retain(|s| !hidden.contains(&s.object));
        }
        for r in &self.physics.ropes.bodies {
            if r.enabled && r.count >= 2 {
                v.ropes.push(RopeView {
                    points: r.segment[..r.count.min(r.segment.len())].to_vec(),
                    owner: r.owner as i32,
                });
            }
        }
        for p in &self.players {
            let mut slots = Vec::new();
            let mut spawned = false;
            let mut in_ability = false;
            let mut grounded = false;
            if let Some(Kind::Bopl(b)) = p.bopl.and_then(|b| self.objects.get(b)).map(|o| &o.kind) {
                spawned = b.ctrl.has_spawned;
                grounded = b.physics.is_grounded;
                in_ability = b.ctrl.is_in_ability;
                for (k, a) in b.ctrl.abilities.iter().enumerate() {
                    let (ability, cooldown) = match self.objects.get(*a).map(|o| &o.kind) {
                        Some(Kind::Ability(ab)) => (ab.def, ab.fw.cooldown),
                        _ => (0, Fix::ONE),
                    };
                    slots.push(SlotView {
                        ability,
                        timer: b.ctrl.cooldown_timers.get(k).copied().unwrap_or(Fix::ZERO),
                        cooldown,
                        castable: matches!(
                            self.objects.get(*a).map(|o| &o.kind),
                            Some(Kind::Ability(_))
                        ) && b.is_ability_castable(self, k),
                    });
                }
            }
            v.players.push(PlayerView {
                id: p.id,
                team: p.team,
                color: p.color,
                alive: p.is_alive(),
                spawned,
                in_ability,
                invisible: p.is_invisible,
                position: p.position,
                bopl: p.bopl,
                games_won: p.games_won,
                kills: p.kills,
                deaths: p.deaths,
                won_this_round: p.won_this_round,
                cause_of_death: p.cause_of_death,
                slots,
                chosen: p.abilities.clone(),
                kills_at_start_of_round: p.kills_at_start_of_round,
                eaten_this_round: p.players_eaten_count_this_round,
                is_most_recent_winner: p.is_most_recent_winner,
                uses_mouse: p.uses_mouse,
                scale: p.scale(),
                grounded,
                alpha: player_alpha(p),
            });
        }
        v
    }
}

//! Gust (`docs/spec/abilities/gust.md`): the shared `QuickSpell` component
//! ("pose, cast a spell object, pose, return"; also used by Revival). The
//! `AirWave_Shockwave` it casts is the shared Shockwave component of
//! abilities/mine.md (`explosion::spawn_shockwave`).
//!
//! Update order inside the Gust object: Ability 1 (framework),
//! SpriteAnimator 2, PlayerPhysics 3, DestroyIfOutsideSceneBounds 4,
//! PlayerCollision 5, QuickSpell 6 (no per-tick logic), PlayerBody 7,
//! DPhysicsCircle 8.


use super::ability_body::{self, AbilityBody};
use super::prefab;
use super::prelude::*;
use crate::data::AnimRef;

pub const CB_CAST: u16 = 1;
pub const CB_EXIT: u16 = 2;

/// Sound names used by quick spells (cues need static names).
fn sound_name(s: &str) -> Option<&'static str> {
    match s {
        "airWave" => Some("airWave"),
        "reviveCast" => Some("reviveCast"),
        _ => None,
    }
}

/// `QuickSpell` (gust.md §QuickSpell) with its ability body.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QuickSpell {
    pub b: AbilityBody,
    pub anim: SpriteAnimator,
    pub enter_offset: Vec2,
    pub exit_offset: Vec2,
    /// `shockwaveOffset` (× player scale).
    pub spell_offset: Vec2,
    /// Index of the spell prefab.
    pub spell: Option<usize>,
    pub audio_enter: Option<&'static str>,
    pub audio_cast: Option<&'static str>,
    pub a_cast_air: AnimRef,
    pub a_exit: AnimRef,
}

impl QuickSpell {
    pub fn new(w: &World, def: &AbilityDef) -> QuickSpell {
        let f = prefab::component_fields(w, def, "QuickSpell");
        let data = f
            .reference("animData")
            .and_then(|r| prefab::anim_data_index(w, r))
            .unwrap_or(0);
        QuickSpell {
            b: AbilityBody::new(w, def),
            anim: ability_body::animator(prefab::ability_prefab(w, def).is_some_and(prefab::root_always_stop)),
            enter_offset: f.vec2("EnterAbilityOffset").unwrap_or_default(),
            exit_offset: f.vec2("ExitAbilityOffset").unwrap_or_default(),
            spell_offset: f.vec2("shockwaveOffset").unwrap_or_default(),
            spell: f
                .reference("spell")
                .and_then(|r| prefab::prefab_index(w, r)),
            audio_enter: f.str("AudioToPlayOnEnter").and_then(sound_name),
            audio_cast: f.str("AudioToPlayOnCast").and_then(sound_name),
            a_cast_air: prefab::anim(w, data, "castAir"),
            a_exit: prefab::anim(w, data, "exit"),
        }
    }

    /// `OnEnterAbility`.
    pub fn enter(&mut self, cx: &mut Cx<'_>) {
        let info = cx.info();
        let me = cx.me;
        self.b.sync_physics_to(cx.w, me, &info);
        AbilityBody::update_collider(cx.w, me);
        AbilityBody::set_pos(cx.w, me, info.position + self.enter_offset);
        self.b.unground(cx.w, me, true, true);
        self.b.body.self_imposed_velocity = Vec2::ZERO;
        self.b.body.external_velocity = Vec2::ZERO;
        let a = self.a_cast_air;
        ability_body::play_then(cx.w, &mut self.anim, a, CB_CAST);
        if let Some(s) = self.audio_enter {
            cx.w.emit(Cue::Sound(s));
        }
    }

    /// The spell position of *Cast* (before the Revival clamp).
    pub fn spell_position(&self, cx: &Cx<'_>) -> Vec2 {
        let scale = cx.w.player(cx.fw.owner).map_or(Fix::ONE, |p| p.scale());
        AbilityBody::pos(cx.w, cx.me) + self.spell_offset * scale
    }

    /// The end of *Cast* (after the spell was spawned): cast sound and the
    /// `exit` animation.
    pub fn finish_cast(&mut self, cx: &mut Cx<'_>) {
        // Presentation: `ParticleCastPrefab` at the spell point, scaled by
        // the player's scale when `scaleParticleSize`.
        let at = self.spell_position(cx);
        let scale = if super::prefab::object_fields(cx.w, cx.me, "QuickSpell")
            .and_then(|f| f.boolean("scaleParticleSize"))
            .unwrap_or(false)
        {
            cx.w.player(cx.fw.owner).map_or(Fix::ONE, |p| p.scale())
        } else {
            Fix::ONE
        };
        super::fx(
            cx.w,
            cx.me,
            "ParticleCastPrefab",
            at,
            Fix::ZERO,
            scale,
            cx.fw.owner,
            Vec2::ZERO,
        );
        if let Some(s) = self.audio_cast {
            cx.w.emit(Cue::Sound(s));
        }
        let a = self.a_exit;
        ability_body::play_then(cx.w, &mut self.anim, a, CB_EXIT);
    }

    /// `QuickSpell.ExitAbility(record)`.
    pub fn exit(&mut self, cx: &mut Cx<'_>, mut r: AbilityExitInfo) {
        let me = cx.me;
        AbilityBody::set_rotation(cx.w, me, Fix::ZERO);
        self.b.unground(cx.w, me, true, false);
        r.grounded_speed = self.b.physics.grounded_speed;
        r.is_grounded = self.b.physics.is_grounded;
        r.position = AbilityBody::pos(cx.w, me) + self.exit_offset;
        r.self_imposed_velocity = self.b.body.self_imposed_velocity;
        r.external_velocity = self.b.body.external_velocity;
        r.ground = self.b.ground(cx.w);
        r.grounded_local_position = self.b.physics.local_platform_pos;
        r.radius = self.b.physics.radius;
        cx.exit_body(&mut self.b, r);
    }

    /// The per-tick components after the framework part: SpriteAnimator,
    /// PlayerPhysics, bounds, PlayerCollision, (QuickSpell), then `extra`
    /// (Revive), then PlayerBody. Returns the animator callback code (the
    /// caller handles `CB_CAST`; `CB_EXIT` is handled here).
    pub fn update(&mut self, cx: &mut Cx<'_>) -> Option<u16> {
        let owner = cx.fw.owner;
        let code = ability_body::advance_code(cx.w, owner, &mut self.anim);
        if code == Some(CB_EXIT) {
            self.exit(cx, AbilityExitInfo::default());
            return None;
        }
        code
    }

    /// The components after the animator (called by the module after it
    /// handled the animator callback).
    pub fn update_rest(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        let me = cx.me;
        if !active(cx) {
            return;
        }
        self.b.physics_update(cx.w, me, dt);
        if let Some(r) = self.b.bounds_update(cx.w, me) {
            self.exit(cx, r);
            return;
        }
        AbilityBody::collision_update(cx.fw);
        self.b.body_update(cx.w, me, dt);
    }

    pub fn late_update(&mut self, cx: &mut Cx<'_>) {
        self.b.physics_late_update(cx.w, cx.me);
        self.b.body_late_update(cx.w, cx.me);
    }
}

fn active(cx: &Cx<'_>) -> bool {
    cx.w.objects.get(cx.me).is_some_and(|o| o.active)
}

/// The Gust ability.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Gust {
    pub q: QuickSpell,
}

/// Objects the Gust spawns (its `AirWave_Shockwave` is the shared
/// Shockwave, `explosion::spawn_shockwave`).
pub type Spawned = NoSpawn;

impl AbilityImpl for Gust {
    fn exit_component(&mut self, cx: &mut Cx<'_>, r: AbilityExitInfo) {
        self.q.exit(cx, r);
    }

    fn new(w: &World, def: &AbilityDef) -> Self {
        Gust {
            q: QuickSpell::new(w, def),
        }
    }

    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        self.q.enter(cx);
    }

    fn update(&mut self, cx: &mut Cx<'_>, dt: Fix) {
        if self.q.update(cx) == Some(CB_CAST) {
            let p = self.q.spell_position(cx);
            if let Some(prefab) = self.q.spell {
                let scale = cx.w.player(cx.fw.owner).map_or(Fix::ONE, |p| p.scale());
                super::explosion::spawn_shockwave(cx.w, prefab, p, cx.fw.owner, scale, Fix::ONE, &[]);
            }
            self.q.finish_cast(cx);
        }
        self.q.update_rest(cx, dt);
    }

    fn late_update(&mut self, cx: &mut Cx<'_>, _dt: Fix) {
        self.q.late_update(cx);
    }

    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        ability_body::hurt_contact(self, cx, c);
    }


    fn sprite(&self) -> crate::data::SpriteId {
        self.q.anim.sprite
    }

    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        Some(&mut self.q.b)
    }
}

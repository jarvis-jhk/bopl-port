//! Invisibility (`docs/spec/abilities/invisibility.md`): an instant ability
//! (`InstantAbility` + `Invisibility.CastInvisibility`). The 5 s timer runs
//! in `Player.UpdatePlayer` (world.rs); its end is handled by
//! [`invisibility_ended`] (the `SlimeController.invisibiltyEnded` hook).
//! Invisibility has no collision effect; Mine targeting and the
//! presentation alpha read `Player::is_invisible`.

use super::prelude::*;
use super::AbilityState;
use crate::bopl::Bopl;
use crate::world::Kind;

/// Cooldown while invisible ("not until it ends"), integer 10000.
const WHILE_INVISIBLE: i32 = 10_000;

/// The `Invisibility` component.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Invisibility {
    /// `Invisibility.cooldown`: the slot cooldown after the first use.
    pub cooldown: Fix,
}

/// Objects this module spawns.
pub type Spawned = NoSpawn;

impl AbilityImpl for Invisibility {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let f = super::prefab::component_fields(w, def, "Invisibility");
        Invisibility {
            cooldown: f.fix_or("cooldown", Fix::from_int(6)),
        }
    }

    /// Instant abilities run their on-enter function here
    /// (`CastInvisibility`, then the framework's `AudioOnCast`).
    fn update(&mut self, cx: &mut Cx<'_>, _dt: Fix) {
        let owner = cx.fw.owner;
        if let Some(p) = cx.w.player_mut(owner) {
            p.is_invisible = true;
            p.time_spent_invisible = Fix::ZERO;
            p.time_since_entered_ability = Fix::ZERO;
        }
        cx.fw.cooldown = Fix::from_int(WHILE_INVISIBLE);
        cx.w.emit(Cue::Sound("invisibility"));
    }

    fn on_enter(&mut self, _cx: &mut Cx<'_>) {}
}

/// End of invisibility (`invisibiltyEnded`): among the instant slots with
/// an Invisibility component, the one with the smallest timer (first on
/// ties) restarts at 0; every such slot still at cooldown 10000 gets its
/// after-use cooldown back.
pub fn invisibility_ended(w: &mut World, _me: ObjId, b: &mut Bopl) {
    let mut best: Option<(usize, Fix)> = None;
    for (i, a) in b.ctrl.abilities.iter().enumerate() {
        let Some(Kind::Ability(x)) = w.objects.get_mut(*a).map(|o| &mut o.kind) else {
            continue;
        };
        if !x.fw.instant {
            continue;
        }
        let AbilityState::Invisibility(inv) = &x.state else {
            continue;
        };
        let t = b.ctrl.cooldown_timers.get(i).copied().unwrap_or(Fix::ZERO);
        if best.is_none_or(|(_, m)| t < m) {
            best = Some((i, t));
        }
        if x.fw.cooldown == Fix::from_int(WHILE_INVISIBLE) {
            x.fw.cooldown = inv.cooldown;
        }
    }
    if let Some((i, _)) = best
        && let Some(t) = b.ctrl.cooldown_timers.get_mut(i)
    {
        *t = Fix::ZERO;
    }
}

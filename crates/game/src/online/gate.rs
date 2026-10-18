//! Sim cues under rollback: what the presentation may play.
//!
//! Online, frames are simulated more than once: predicted, then (after a
//! rollback) again with the real inputs. Every cue of the sim has an id
//! `(tick, seq)` that is a pure function of the simulation, but a corrected
//! frame can emit different cues under the same ids. [`EventGate`] plays a
//! cue when it is *first seen*: its identity is `(tick, kind and subject of
//! the cue without positions, n-th such cue in the tick)`, so a
//! re-simulation that produces the same cue (perhaps at a slightly other
//! position) stays silent and one that produces a new cue plays it.
//! Identities of confirmed ticks are forgotten.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use bopl_sim::view::{Cue, Event};
use bopl_sim::world::Fnv64;

/// Identity of a cue without positions.
fn identity(c: &Cue) -> u64 {
    let mut h = Fnv64::default();
    std::mem::discriminant(c).hash(&mut h);
    match c {
        Cue::Sound(n) | Cue::SoundPrefix(n) => n.hash(&mut h),
        Cue::PlayerSound(n, id) => (n, id).hash(&mut h),
        Cue::Effect {
            kind, player_id, ..
        } => (kind, player_id).hash(&mut h),
        Cue::PlayerSpawned { id } => id.hash(&mut h),
        Cue::PlayerDied { id, .. } => id.hash(&mut h),
        Cue::Shake(s) => s.hash(&mut h),
        Cue::RoundWon { team } => team.hash(&mut h),
        Cue::Fx(f) => (f.prefab, f.field, f.player_id).hash(&mut h),
        other => format!("{other:?}").hash(&mut h),
    }
    h.finish()
}

#[derive(Default)]
pub struct EventGate {
    /// (tick, identity, n-th) of cues already played.
    seen: BTreeSet<(i64, u64, u32)>,
    out: Vec<Event>,
}

impl EventGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// The cues of one simulated tick (first time or again).
    pub fn on_advance(&mut self, events: &[Event]) {
        let mut nth: BTreeMap<(i64, u64), u32> = BTreeMap::new();
        for e in events {
            let id = identity(&e.cue);
            let n = nth.entry((e.id.tick, id)).or_insert(0);
            if self.seen.insert((e.id.tick, id, *n)) {
                self.out.push(e.clone());
            }
            *n += 1;
        }
    }

    /// The cues to present since the last call; forgets ticks up to
    /// `confirmed` (they cannot be simulated again).
    pub fn take(&mut self, confirmed: i64) -> Vec<Event> {
        if let Some(&(t, _, _)) = self.seen.iter().next()
            && t <= confirmed
        {
            self.seen = self.seen.split_off(&(confirmed + 1, 0, 0));
        }
        std::mem::take(&mut self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bopl_sim::Vec2;
    use bopl_sim::fix::Fix;
    use bopl_sim::view::{EffectKind, EventId};

    fn ev(tick: i64, seq: u32, cue: Cue) -> Event {
        Event {
            id: EventId { tick, seq },
            cue,
        }
    }

    #[test]
    fn first_seen_once_across_resimulation() {
        let mut g = EventGate::new();
        let fx = |x: i32| Cue::Effect {
            kind: EffectKind::Death,
            position: Vec2::new(Fix::from_int(x), Fix::ZERO),
            player_id: 2,
        };
        g.on_advance(&[
            ev(10, 0, Cue::Sound("jump")),
            ev(10, 1, Cue::Sound("jump")),
            ev(10, 2, fx(1)),
        ]);
        assert_eq!(g.take(5).len(), 3, "both jumps and the effect play");
        // Rollback: tick 10 again, the effect slightly elsewhere.
        g.on_advance(&[
            ev(10, 0, Cue::Sound("jump")),
            ev(10, 1, Cue::Sound("jump")),
            ev(10, 2, fx(2)),
        ]);
        assert!(g.take(9).is_empty(), "already played");
        // A corrected tick with a new cue under an old id plays it.
        g.on_advance(&[ev(
            10,
            0,
            Cue::PlayerDied {
                id: 3,
                killer: 1,
                cause: Default::default(),
            },
        )]);
        assert_eq!(g.take(10).len(), 1);
        // Confirmed ticks are forgotten.
        assert!(g.seen.is_empty());
    }
}

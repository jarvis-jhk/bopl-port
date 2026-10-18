//! Teleport (`docs/spec/abilities/teleport.md`): an instant ability that
//! places a marker (`TeleportIndicator`) and on the next use swaps
//! everything near the player with everything near the marker; moved
//! objects hide for 0.6 s (`TeleportedObjectEffect` ghosts).

use super::Behaviour;
use super::ability_body::{self as body};
use super::prefab::{self, root_component};
use super::prelude::*;
use super::rope::{self, RopeCfg};
use crate::world::Kind;

/// Prefab values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TeleportDef {
    pub up_cast_offset: Fix,
    pub radius: Fix,
    pub indicator: Option<usize>,
    pub teleport_radius: Fix,
    pub max_scale: Fix,
    pub min_scale: Fix,
    pub mask: u32,
    pub ghost: Option<usize>,
    pub ghost_duration: Fix,
    pub ghost_strength: Fix,
}

/// Module state (one per ability object).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Teleport {
    pub def: TeleportDef,
    /// The live marker (if any).
    pub marker: Option<ObjId>,
}

/// Spawned objects.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    Marker(Marker),
    Ghost(Ghost),
}

impl AbilityImpl for Teleport {
    fn new(w: &World, def: &AbilityDef) -> Self {
        let c = &w.content;
        let t = root_component(c, def, "Teleport");
        let ind = prefab::prefab_ref(
            c,
            t.and_then(|t| t.fields.reference("tpiPrefab")),
            "INDICATOR_portal",
        );
        let ic = ind.and_then(|p| prefab::component(c, p, "TeleportIndicator"));
        let ghost = prefab::prefab_ref(
            c,
            ic.and_then(|i| i.fields.reference("teleportingSpriteEffectPrefab")),
            "teleportingObjectEffect",
        );
        let gc = ghost.and_then(|p| prefab::component(c, p, "TeleportedObjectEffect"));
        Teleport {
            def: TeleportDef {
                up_cast_offset: prefab::fix_field(t, "upCastOffset", Fix(6_442_450_944)),
                radius: prefab::fix_field(t, "radius", Fix::TWO),
                indicator: ind,
                teleport_radius: prefab::fix_field(ic, "teleportRadius", Fix(22_162_030_592)),
                max_scale: prefab::fix_field(ic, "maxScale", Fix::from_int(6)),
                min_scale: prefab::fix_field(ic, "minScale", F_0_1),
                mask: prefab::int_field(ic, "teleportableThings", 80_293_632) as u32,
                ghost,
                ghost_duration: prefab::fix_field(gc, "duration", Fix(2_576_980_480)),
                ghost_strength: prefab::fix_field(gc, "strength", F_0_25),
            },
            marker: None,
        }
    }

    /// `Teleport.CastAbility` (the instant cast).
    fn update(&mut self, cx: &mut Cx<'_>, _dt: Fix) {
        let owner = cx.owner();
        let w = &mut *cx.w;
        let scale = w.player(owner).map_or(Fix::ONE, |p| p.scale());
        let pos = w.objects.get(cx.me).map_or(Vec2::ZERO, |o| o.tf.position);
        let live = self.marker.filter(|m| w.objects.alive(*m));
        let Some(m) = live else {
            w.emit(Cue::Sound("swapIndicator"));
            cx.fw.ignore_next_cooldown = true;
            let Some(p) = self.def.indicator else {
                return;
            };
            let id = body::spawn_prefab(
                w,
                p,
                Kind::Behaviour(Box::new(Behaviour::Teleport(Spawned::Marker(Marker {
                    def: self.def,
                    owner,
                    scale,
                    teleport: false,
                    player_radius: Fix::ZERO,
                    player_position: Vec2::ZERO,
                })))),
                &[
                    ("TeleportIndicator", PartKind::Behaviour(0)),
                    ("BoplBody", PartKind::BoplBody),
                    (body::COLLIDER, PartKind::Collider),
                ],
                pos + Vec2::UP * self.def.up_cast_offset,
                None,
            );
            self.marker = Some(id);
            let me = cx.me;
            super::fx(w, me, "onSpawnIndicatorParticlePrefab", pos, Fix::ZERO, scale, owner, Vec2::ZERO);
            return;
        };
        cx.fw.ignore_next_cooldown = false;
        w.emit(Cue::Sound("swap"));
        // Presentation: the cast flashes at both ends and the particles
        // flying between them (`value` = marker − caster).
        let me = cx.me;
        let mpos = w.objects.get(m).map_or(pos, |o| o.tf.position);
        let v = mpos - pos;
        super::fx(w, me, "onCastParticlePrefabSmall", pos, Fix::ZERO, scale, owner, Vec2::ZERO);
        super::fx(w, me, "onCastParticlePrefab", mpos, Fix::ZERO, scale, owner, Vec2::ZERO);
        if v.sqr_magnitude() > Fix::ONE {
            super::fx(w, me, "betweenPortalsParticlePrefab", pos, Fix::ZERO, Fix::ONE, owner, v);
            super::fx(w, me, "betweenPortalsParticlePrefab", mpos, Fix::PI, Fix::ONE, owner, v);
        }
        if let Some(Kind::Behaviour(b)) = w.objects.get_mut(m).map(|o| &mut o.kind)
            && let Behaviour::Teleport(Spawned::Marker(mk)) = b.as_mut()
        {
            mk.player_position = pos;
            mk.player_radius = self.def.radius;
            mk.teleport = true;
        }
    }
}

/// The marker (`TeleportIndicator`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Marker {
    pub def: TeleportDef,
    pub owner: i32,
    /// The marker's own scale (radius factor), not the collider's.
    pub scale: Fix,
    pub teleport: bool,
    pub player_radius: Fix,
    pub player_position: Vec2,
}

impl Marker {
    /// `AddScale(d)` (shrink/growth rays, `ISpell`).
    pub fn add_scale(&mut self, d: Fix) {
        self.scale = (self.scale + d).min(self.def.max_scale).max(self.def.min_scale);
    }

    fn do_teleport(&self, w: &mut World, me: ObjId) {
        let k = w.player(self.owner).map_or(Fix::ONE, |p| p.scale());
        let mpos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
        let a = w.physics.circle_cast_all(
            &w.objects,
            self.player_position,
            self.player_radius * k,
            self.def.mask,
            100,
        );
        let b = w.physics.circle_cast_all(
            &w.objects,
            mpos,
            self.def.teleport_radius * self.scale,
            self.def.mask,
            100,
        );
        let v = mpos - self.player_position;
        for &o in &a {
            self.teleport_object(w, o, v);
        }
        for &o in &b {
            if !a.contains(&o) {
                self.teleport_object(w, o, -v);
            }
        }
    }

    /// `teleportObject(o, d)`.
    fn teleport_object(&self, w: &mut World, o: ObjId, d: Vec2) {
        let Some(ob) = w.objects.get(o) else {
            return;
        };
        let layer = ob.layer;
        let pobj = prefab::prefab_object(w, o);
        let boulder = pobj.is_some_and(|p| p.tag == "boulder");
        let attachable = body::is_platform_attachable(w, o);
        if (layer == 11 && !boulder) || (layer == 16 && attachable) {
            return;
        }
        let rotation = ob.tf.rotation;
        let has_sprite = match ob.kind {
            Kind::Bopl(_) => true,
            _ => pobj.is_some_and(|p| p.component("SpriteRenderer").is_some()),
        };
        if layer == 8 {
            w.with_kind(o, |w, k| match k {
                Kind::Bopl(b) => {
                    if let Some(l) = b.body.rope {
                        let cfg = RopeCfg::player(w);
                        rope::attach_rope(w, &mut b.body, cfg, None, l.top);
                    }
                    b.body.self_imposed_velocity = Vec2::ZERO;
                    b.unground(w, o, true, false);
                }
                Kind::Ability(x) => {
                    if let Some(ab) = x.body_mut()
                        && ab.cfg.has_physics
                    {
                        if let Some(l) = ab.body.rope {
                            ab.attach_rope(w, None, l.top);
                        }
                        ab.body.self_imposed_velocity = Vec2::ZERO;
                        ab.unground(w, o, true, false);
                    }
                }
                _ => {}
            });
        }
        if !has_sprite {
            return;
        }
        if let Some(g) = self.def.ghost {
            let pos = w.objects.get(o).map_or(Vec2::ZERO, |ob| ob.tf.position);
            body::spawn_prefab(
                w,
                g,
                Kind::Behaviour(Box::new(Behaviour::Teleport(Spawned::Ghost(Ghost {
                    target: o,
                    time_passed: Fix::ZERO,
                    prev_shake: Vec2::ZERO,
                    duration: self.def.ghost_duration,
                    strength: self.def.ghost_strength,
                })))),
                &[("TeleportedObjectEffect", PartKind::Behaviour(1))],
                pos,
                Some(rotation),
            );
        }
        if let Some(ob) = w.objects.get_mut(o) {
            ob.tf.position += d;
            ob.active = false;
        }
    }
}

/// A `TeleportedObjectEffect`: hides an object for `duration`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ghost {
    pub target: ObjId,
    pub time_passed: Fix,
    pub prev_shake: Vec2,
    pub duration: Fix,
    pub strength: Fix,
}

impl SpawnedImpl for Spawned {
    /// The marker's particles in the owner's `_ShadowColor`; the ghost is a
    /// white silhouette of its target's current sprite at its scale.
    fn look(&self, w: &World, me: ObjId, l: &mut crate::view::Look) {
        match self {
            Spawned::Marker(m) => {
                l.shadow_color = true;
                // The marker object has no owner of its own: colour from
                // the casting player (`SetParticleColor`).
                l.player = m.owner;
            }
            Spawned::Ghost(g) => {
                let Some(t) = w.objects.get(g.target) else {
                    l.hidden = true;
                    return;
                };
                match &t.kind {
                    Kind::Bopl(b) => {
                        l.sprite = b.anim.sprite;
                        l.flip_x = b.anim.flip_x != !b.ctrl.facing_right;
                    }
                    Kind::Ability(a) => {
                        let mut al = crate::view::Look::default();
                        a.look(w, g.target, &mut al);
                        l.sprite = al.sprite;
                        l.flip_x = al.flip_x;
                    }
                    _ => {}
                }
                if t.layer == 23 {
                    l.hidden = true;
                    l.parts.push(crate::view::PartLook::hidden(1));
                }
                let own = w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale).max(Fix(1));
                let k = t.tf.scale / own;
                l.stretch = Vec2::new(k, k);
            }
        }
    }

    /// `ISpell.AddScale(d)` (shrink/growth rays) on the marker.
    fn interact(&mut self, _w: &mut World, _me: ObjId, i: super::Interaction) -> Option<Fix> {
        match (self, i) {
            (Spawned::Marker(m), super::Interaction::SpellAddScale(d)) => {
                m.add_scale(d);
                Some(Fix::ZERO)
            }
            _ => None,
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        if late {
            return;
        }
        match self {
            Spawned::Marker(m) if part == PartKind::Behaviour(0) => {
                if m.teleport {
                    m.do_teleport(w, me);
                    m.teleport = false;
                    // The marker's logic, body, collider and transform are
                    // destroyed (only the particles linger).
                    w.destroy(me);
                }
            }
            Spawned::Ghost(g) if part == PartKind::Behaviour(1) => {
                g.time_passed += dt * w.time.player_time_scale;
                if g.time_passed > g.duration {
                    if w.objects.get(g.target).is_some_and(|o| !o.tf.destroyed)
                        && let Some(o) = w.objects.get_mut(g.target)
                    {
                        o.active = true;
                        let p = o.tf.position;
                        super::fx(w, me, "spawnParticlePrefab", p, Fix::ZERO, Fix::ONE, 0, Vec2::ZERO);
                    }
                    w.destroy(me);
                }
                if !w.is_time_stopped() {
                    let v = w.rng.random_inside_unit_circle() * g.strength;
                    if let Some(o) = w.objects.get_mut(me) {
                        o.tf.position -= g.prev_shake;
                        o.tf.position += v;
                    }
                    g.prev_shake = v;
                }
            }
            _ => {}
        }
    }
}

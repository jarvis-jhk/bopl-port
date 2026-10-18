//! Smoke (`docs/spec/abilities/smoke.md`): `ThrowItem2` (grenade.rs) with
//! Smoke's values, the smoke-grenade burst, the flammable clouds
//! (`FlammableSmoke`) and (canonical home) the component ring buffer
//! (engine.md §10b).

use std::sync::Arc;

use super::Behaviour;
use super::ability_body::{self, AbilityBody, Bounds, play_named as play, play_named_then as play_then, prefab_parts};
use super::prefab::{self, fx, prefab_field, root_fields, vec2_list};
use super::grenade::ThrowItem2;
use super::prelude::*;
use super::Interaction;
use crate::anim::SpriteAnimator;
use crate::world::Kind;

/// `RingBuffer<T>` with `reBalance = true` over object handles (a slot is
/// empty when its object no longer exists: Unity's destroyed-object null,
/// i.e. after the end-of-tick removal).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RingBuffer {
    pub slots: Vec<Option<ObjId>>,
    pub current: usize,
}

impl RingBuffer {
    pub fn new(n: usize) -> RingBuffer {
        RingBuffer {
            slots: vec![None; n],
            current: 0,
        }
    }

    fn occupied(&self, w: &World, i: usize) -> bool {
        self.slots[i].is_some_and(|id| w.objects.exists(id))
    }

    /// `Add(x)`: rebalance into the first empty slot, else evict (destroy)
    /// the oldest.
    pub fn add(&mut self, w: &mut World, x: ObjId) {
        self.add_by(w, x, |w, id| w.objects.exists(id));
    }

    /// [`RingBuffer::add`] where an entry counts while `live(w, id)` (the
    /// buffer holds a component: e.g. an arrow's BoplBody, destroyed when
    /// the arrow lands).
    pub fn add_by(&mut self, w: &mut World, x: ObjId, live: impl Fn(&World, ObjId) -> bool) {
        let n = self.slots.len();
        if n == 0 {
            return;
        }
        let occupied = |w: &World, s: &Self, i: usize| s.slots[i].is_some_and(|id| live(w, id));
        if occupied(w, self, self.current) {
            for k in 0..n {
                let e = (self.current + k) % n;
                if !occupied(w, self, e) {
                    for j in 0..k {
                        let to = (e + n - j) % n;
                        let from = (e + n - j - 1) % n;
                        self.slots[to] = self.slots[from];
                    }
                    self.slots[self.current] = None;
                    break;
                }
            }
        }
        if occupied(w, self, self.current)
            && let Some(old) = self.slots[self.current]
        {
            w.destroy(old);
        }
        self.slots[self.current] = Some(x);
        self.current = (self.current + 1) % n;
    }

    /// Live entries.
    pub fn live(&self, w: &World) -> usize {
        (0..self.slots.len()).filter(|&i| self.occupied(w, i)).count()
    }
}

// -- the ability --------------------------------------------------------------------

/// The Smoke ability (ThrowItem2 with the smoke grenade).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Smoke(pub ThrowItem2);

impl AbilityImpl for Smoke {
    fn new(w: &World, def: &AbilityDef) -> Self {
        Smoke(ThrowItem2::new(w, def))
    }
    fn build(&mut self, w: &mut World, me: ObjId, fw: &mut Framework, def: &AbilityDef) {
        self.0.build(w, me, fw, def);
    }
    fn on_enter(&mut self, cx: &mut Cx<'_>) {
        self.0.on_enter(cx);
    }
    fn update_part(&mut self, cx: &mut Cx<'_>, k: PartKind, dt: Fix, late: bool) {
        self.0.update_part(cx, k, dt, late);
    }
    fn on_contact(&mut self, cx: &mut Cx<'_>, c: &Contact) {
        self.0.on_contact(cx, c);
    }
    fn on_scale_changed(&mut self, cx: &mut Cx<'_>, m: Fix) {
        self.0.on_scale_changed(cx, m);
    }
    fn exit_component(&mut self, cx: &mut Cx<'_>, info: AbilityExitInfo) {
        self.0.exit_component(cx, info);
    }
    fn body(&self) -> Option<&AbilityBody> {
        self.0.body_opt()
    }
    fn body_mut(&mut self) -> Option<&mut AbilityBody> {
        self.0.body_opt_mut()
    }
    fn sprite(&self) -> crate::data::SpriteId {
        self.0.anim.sprite
    }
    fn look(&self, w: &World, me: ObjId, _fw: &Framework, l: &mut crate::view::Look) {
        self.0.look(w, me, l);
    }
}

// -- the burst ------------------------------------------------------------------------

/// `SmokeGrenadeExplode2` values plus the cloud prefab.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SmokeBurstCfg {
    pub cloud: Option<Arc<CloudCfg>>,
    /// The visual burst (`SmokeExplosion 1`): consumes hierarchy numbers.
    pub visual: Option<usize>,
    pub offsets: Vec<Vec2>,
    pub spread: Fix,
    pub noise: Fix,
}

impl SmokeBurstCfg {
    pub fn from_fields(w: &World, f: &Value) -> SmokeBurstCfg {
        SmokeBurstCfg {
            cloud: prefab_field(w, f, "Smoke")
                .and_then(|p| CloudCfg::from_prefab(w, p))
                .map(Arc::new),
            visual: prefab_field(w, f, "SmokeExplosion"),
            offsets: vec2_list(f, "cloudOffsets"),
            spread: fx(f, "spread", Fix::ONE),
            noise: fx(f, "noise", Fix::ONE),
        }
    }
}

/// `SpawnCloud()`: four clouds around the grenade (smoke.md *Burst*). The
/// caller destroys the grenade.
pub fn burst(w: &mut World, me: ObjId, c: &SmokeBurstCfg, owner: i32, thrower: Option<ObjId>, grenade_alive: bool) {
    let (pos, gscale, duplicated) = match w.objects.get(me) {
        Some(o) => (o.tf.position, o.tf.scale, o.tf.duplicated),
        None => return,
    };
    for &o in &c.offsets {
        let r = w.rng.random_inside_unit_circle();
        let offset = gscale * (o + r * c.noise) * c.spread;
        let rotation = w.rng.random_fix(Fix::ZERO, Fix::PI_TIMES_2);
        let at = pos + offset;
        if let Some(cc) = &c.cloud {
            let cloud = spawn_cloud(w, cc, at, rotation, owner);
            w.set_collider_scale(cloud, gscale);
            let thrower_ok = grenade_alive && thrower.is_some_and(|t| w.objects.alive(t));
            if thrower_ok && let Some(t) = thrower {
                add_to_buffer(w, t, cloud);
            }
            if duplicated {
                // A Duplicator clone's clouds vanish after 1200 ticks.
                super::duplicator::add_to_delete_list(w, cloud, 1200);
            }
        }
        if let Some(v) = c.visual {
            w.consume_hierarchy(prefab::updatables_of(w, v));
            w.emit(Cue::Effect {
                kind: crate::view::EffectKind::Ability(v as u16),
                position: at,
                player_id: owner,
            });
        }
    }
    w.emit(Cue::Sound("smokeBombExplosion"));
}

/// `thrower.effectBuffer.Add(cloud)` on a Smoke (or Grenade) ability object.
fn add_to_buffer(w: &mut World, thrower: ObjId, cloud: ObjId) {
    let mut buf = match w.objects.get_mut(thrower).map(|o| &mut o.kind) {
        Some(Kind::Ability(a)) => match &mut a.state {
            super::AbilityState::Smoke(s) => std::mem::take(&mut s.0.effect_buffer),
            super::AbilityState::Grenade(s) => std::mem::take(&mut s.0.effect_buffer),
            _ => return,
        },
        _ => return,
    };
    buf.add(w, cloud);
    if let Some(Kind::Ability(a)) = w.objects.get_mut(thrower).map(|o| &mut o.kind) {
        match &mut a.state {
            super::AbilityState::Smoke(s) => s.0.effect_buffer = buf,
            super::AbilityState::Grenade(s) => s.0.effect_buffer = buf,
            _ => {}
        }
    }
}

// -- the clouds -------------------------------------------------------------------------

/// `FlammableSmoke` (SmokeCircle prefab).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CloudCfg {
    pub prefab: usize,
    pub radii: Vec<Fix>,
    pub min_delay: Fix,
    pub max_delay: Fix,
    pub explosion: Option<usize>,
    pub anim_data: u16,
    pub always_stop: bool,
    pub parts: Vec<(PartKind, i32, bool)>,
    pub bounds: Bounds,
}

impl CloudCfg {
    pub fn from_prefab(w: &World, prefab: usize) -> Option<CloudCfg> {
        let p = w.content.prefabs.get(prefab)?;
        let f = root_fields(p, "FlammableSmoke")?;
        let radii = f
            .list("SmokeRadius")
            .iter()
            .filter_map(|v| match v {
                Value::Fix(x) => Some(*x),
                Value::Int(i) => Some(Fix(*i)),
                _ => None,
            })
            .collect();
        Some(CloudCfg {
            prefab,
            radii,
            min_delay: fx(f, "minDetonationDelay", Fix::HALF),
            max_delay: fx(f, "maxDetonationDelay", Fix::HALF),
            explosion: prefab_field(w, f, "ExplosionPrefab"),
            anim_data: prefab::anim_data_field(w, f, "animData").unwrap_or(0),
            always_stop: root_fields(p, "SpriteAnimator")
                .and_then(|a| a.boolean("AlwaysStopDuringTimeStop"))
                .unwrap_or(false),
            parts: prefab_parts(w, prefab, &[("FlammableSmoke", PartKind::Behaviour(0))]),
            bounds: Bounds::of_prefab(w, prefab),
        })
    }
}

const CB_LOOP: u16 = 1;

/// A smoke cloud.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Cloud {
    pub cfg: Arc<CloudCfg>,
    pub owner: i32,
    pub enter: bool,
    pub anim: SpriteAnimator,
    /// `timeSinceDetonated` (−1 = not ignited).
    pub timer: Fix,
    pub delay: Fix,
}

/// `InstantiateFixed(Smoke, pos, rotation)` + owner (the Awake radius set).
pub fn spawn_cloud(w: &mut World, cfg: &Arc<CloudCfg>, pos: Vec2, rotation: Fix, owner: i32) -> ObjId {
    let anim = ability_body::animator(cfg.always_stop);
    let c = Cloud {
        cfg: cfg.clone(),
        owner,
        enter: false,
        anim,
        timer: -Fix::ONE,
        delay: Fix::ZERO,
    };
    let id = ability_body::spawn_owned(w, cfg.prefab, Kind::Behaviour(Box::new(Behaviour::Smoke(Spawned::Cloud(c)))), &cfg.parts, pos, Some(rotation), owner);
    // Awake: radius := SmokeRadius[0] · Scale (scale 1, before init).
    if let Some(r0) = cfg.radii.first()
        && let Some(o) = w.objects.get_mut(id)
        && let Some(col) = o.collider.as_mut()
    {
        col.start_radius = *r0 * o.tf.scale;
    }
    id
}

impl Cloud {
    fn set_radius(w: &mut World, me: ObjId, r: Fix) {
        let initialized = ability_body::collider_initialized(w, me);
        if initialized {
            if let Some(c) = w.physics.circle_mut(me) {
                c.radius = r;
            }
        } else if let Some(col) = w.objects.get_mut(me).and_then(|o| o.collider.as_mut()) {
            col.start_radius = r;
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        let n = self.cfg.radii.len();
        if n == 0 {
            return;
        }
        let i = if self.enter {
            (self.anim.passed_sprites() - 1).clamp(0, n as i32 - 1) as usize
        } else {
            n - 1
        };
        let scale = w.objects.get(me).map_or(Fix::ONE, |o| o.tf.scale);
        Self::set_radius(w, me, self.cfg.radii[i] * scale);
        if self.timer >= Fix::ZERO {
            self.timer += dt * w.time.player_time_scale;
            if self.timer > self.delay {
                let pos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
                if let Some(e) = self.cfg.explosion {
                    super::explosion::spawn(w, e, pos, Some(scale), self.owner);
                }
                w.destroy(me);
            }
        }
    }
}

/// Objects of this module.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Spawned {
    Cloud(Cloud),
}

impl SpawnedImpl for Spawned {
    /// A cloud: its frame, also on the `whiteness` layer (object 1).
    fn look(&self, _w: &World, _me: ObjId, l: &mut crate::view::Look) {
        let Spawned::Cloud(c) = self;
        l.frame(&c.anim);
        l.parts
            .push(crate::view::PartLook::new(1).frame(c.anim.sprite, c.anim.flip_x));
    }

    fn init(&mut self, w: &mut World, _me: ObjId, part: PartKind) {
        let Spawned::Cloud(c) = self;
        if let PartKind::Behaviour(0) = part {
            c.enter = true;
            play_then(w, &mut c.anim, c.cfg.anim_data, "enter", CB_LOOP);
            c.delay = w.rng.random_fix(c.cfg.min_delay, c.cfg.max_delay);
            c.timer = -Fix::ONE;
        }
    }

    fn update(&mut self, w: &mut World, me: ObjId, part: PartKind, dt: Fix, late: bool) {
        let Spawned::Cloud(c) = self;
        if late {
            return;
        }
        match part {
            PartKind::Bounds => {
                let b = c.cfg.bounds;
                b.check_and_destroy(w, me);
            }
            PartKind::Animator => {
                // No IPlayerIdHolder on the cloud: frozen during any time stop.
                if !w.is_time_stopped() {
                    let cc = w.content.clone();
                    if let Some(AnimCallback::Ability(CB_LOOP)) = c.anim.advance(&cc, Fix::ZERO) {
                        c.enter = false;
                        play(w, &mut c.anim, c.cfg.anim_data, "loop");
                    }
                }
            }
            PartKind::Behaviour(0) => c.update(w, me, dt),
            _ => {}
        }
    }

    fn on_contact(&mut self, w: &mut World, me: ObjId, ct: &Contact) {
        let Spawned::Cloud(c) = self;
        if super::explosion::ignites_flammables(w, ct.other) {
            c.ignite(w, me);
        }
    }

    /// `FlammableSmoke.Ignite()` from rocket-engine flames; the duplicator's
    /// item fix-up (`Item.OwnerId` := the original's).
    fn interact(&mut self, w: &mut World, me: ObjId, i: Interaction) -> Option<Fix> {
        let Spawned::Cloud(c) = self;
        match i {
            Interaction::Ignite => {
                c.ignite(w, me);
                Some(Fix::ZERO)
            }
            Interaction::Duplicated { original, .. } => {
                if let Some(Kind::Behaviour(b)) = w.objects.get(original).map(|o| &o.kind)
                    && let Behaviour::Smoke(Spawned::Cloud(o)) = b.as_ref()
                {
                    c.owner = o.owner;
                }
                Some(Fix::ZERO)
            }
            _ => None,
        }
    }

    /// Clone rule: tuning carries over; `enter`, the ignition timer (−1),
    /// the delay, the animation and `Item.OwnerId` (0) are runtime.
    fn duplicate(&self, _w: &World, _me: ObjId) -> Option<Spawned> {
        let Spawned::Cloud(c) = self;
        Some(Spawned::Cloud(Cloud {
            cfg: c.cfg.clone(),
            owner: 0,
            enter: false,
            anim: ability_body::animator(c.anim.always_stop_during_time_stop),
            timer: -Fix::ONE,
            delay: Fix::ZERO,
        }))
    }
}

impl Cloud {
    /// `Ignite()`: timer 0, collider disabled.
    fn ignite(&mut self, w: &mut World, me: ObjId) {
        self.timer = Fix::ZERO;
        w.set_collider_enabled(me, false);
    }
}

/// Live smoke clouds in the world.
pub fn clouds(w: &World) -> Vec<ObjId> {
    w.objects
        .ids()
        .filter(|&id| {
            matches!(&w.objects.get(id).map(|o| &o.kind), Some(Kind::Behaviour(b)) if matches!(b.as_ref(), Behaviour::Smoke(_)))
        })
        .collect()
}

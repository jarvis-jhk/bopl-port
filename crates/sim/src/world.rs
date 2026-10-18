//! The world: object store, updater (admission batches, update order,
//! spawn/destroy semantics), time, players, RNG and physics (engine.md).

use std::sync::Arc;

use crate::abilities::{self, AbilityObject, Behaviour};
use crate::bopl::Bopl;
use crate::consts::{ENVIRONMENT_ID, FIXED_DT};
use crate::data::{Content, LevelData, LevelWorld};
use crate::fix::{Fault, Fix, take_fault};
use crate::input::PlayerInput;
use crate::object::{ObjId, Object, Objects, PartKind};
use crate::physics::{Contact, Physics};
use crate::platform::Platform;
use crate::player::{Player, PlayerSetup};
use crate::rng::Rng;
use crate::session::{GameSession, Spawner};
use crate::sort::introsort;
use crate::view::{Cue, Event, EventId};

/// The typed payload of an object.
#[derive(Clone, Debug, Default, Hash)]
pub enum Kind {
    /// No payload (or temporarily taken out during its own update).
    #[default]
    Empty,
    Bopl(Box<Bopl>),
    Platform(Box<Platform>),
    Session(Box<GameSession>),
    Spawner(Box<Spawner>),
    /// An ability object (framework + module behaviour).
    Ability(Box<AbilityObject>),
    /// An object spawned by an ability module.
    Behaviour(Box<Behaviour>),
    /// A collider-only object (water box, kill zone).
    Static,
    /// A tutorial object (handler, arrow, pickup, dummy, ground).
    Tutorial(Box<crate::tutorial::Part>),
}

/// An entry of the updatable list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Upd {
    pub obj: ObjId,
    pub part: u8,
    pub hier: i32,
}

/// Clocks and time scale (engine.md §1, §9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GameTime {
    pub player_time_scale: Fix,
    pub sim_time_passed: Fix,
    pub sim_time_since_level_loaded: Fix,
    pub simulation_ticks: i64,
    pub loaded_level_this_update: bool,
}

impl Default for GameTime {
    fn default() -> GameTime {
        GameTime {
            player_time_scale: Fix::ONE,
            sim_time_passed: Fix::ZERO,
            sim_time_since_level_loaded: Fix::ZERO,
            simulation_ticks: 0,
            loaded_level_this_update: false,
        }
    }
}

/// Global round flags shared by the match handler and platforms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RoundFlags {
    pub sudden_death: bool,
    pub sudden_death_stamp: Fix,
    pub game_in_progress: bool,
    pub game_over: bool,
    pub game_paused: bool,
    /// The round was decided (winner screen pending or shown).
    pub decided: bool,
    pub winner_screen: bool,
    /// The round ended because a [`Fault`] was raised (draw, like the
    /// original online host).
    pub ended_by_fault: bool,
}

/// Match settings that are simulation inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MatchSettings {
    /// Ability slots per player (1..3).
    pub number_of_abilities: usize,
    /// Seed for the RNG at every level load.
    pub seed: u32,
}

impl Default for MatchSettings {
    fn default() -> MatchSettings {
        MatchSettings {
            number_of_abilities: 3,
            seed: 1,
        }
    }
}

/// The whole simulation state. `Clone` is the rollback snapshot.
#[derive(Clone, Debug)]
pub struct World {
    pub content: Arc<Content>,
    pub level: Option<Arc<LevelData>>,
    pub objects: Objects,
    pub physics: Physics,
    pub players: Vec<Player>,
    pub rng: Rng,
    pub time: GameTime,
    pub round: RoundFlags,
    pub settings: MatchSettings,
    pub updatables: Vec<Upd>,
    pub to_be_added: Vec<Upd>,
    bracket: Option<Vec<Upd>>,
    pub prefabs_added: i32,
    destroy_objects: Vec<ObjId>,
    pub session: Option<ObjId>,
    /// Platform objects in the session's collection order.
    pub platforms: Vec<ObjId>,
    /// Events of the last tick (presentation cues, gameplay notifications).
    pub events: Vec<Event>,
    event_seq: u32,
    /// First fault of the session (if any) and the tick it happened.
    pub fault: Option<(i64, Fault)>,
    /// Ability-framework global state (duplicates, parries, ropes, beams).
    pub ability_globals: abilities::Globals,
    /// Session flow: level bag, ability-select circles and wheels, winner
    /// screen inputs (`flow.rs`).
    pub flow: crate::flow::Flow,
    /// The tutorial's progress (tutorial scene only, `tutorial.rs`).
    pub tutorial: Option<Box<crate::tutorial::State>>,
}

impl World {
    /// A new world for a set of players (no level loaded yet).
    pub fn new(content: Arc<Content>, players: &[PlayerSetup], settings: MatchSettings) -> World {
        crate::lut::init();
        let physics = Physics {
            matrix: content.layer_matrix,
            ..Physics::default()
        };
        World {
            content,
            level: None,
            objects: Objects::default(),
            physics,
            players: players.iter().map(Player::new).collect(),
            rng: Rng::default(),
            time: GameTime::default(),
            round: RoundFlags::default(),
            settings,
            updatables: Vec::new(),
            to_be_added: Vec::new(),
            bracket: None,
            prefabs_added: 0,
            destroy_objects: Vec::new(),
            session: None,
            platforms: Vec::new(),
            events: Vec::new(),
            event_seq: 0,
            fault: None,
            ability_globals: abilities::Globals::default(),
            flow: crate::flow::Flow::default(),
            tutorial: None,
        }
    }

    // -- players and time ---------------------------------------------------

    pub fn player(&self, id: i32) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn player_mut(&mut self, id: i32) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    /// `IsTimeStopped()`.
    pub fn is_time_stopped(&self) -> bool {
        self.time.player_time_scale < Fix(42_949_672)
    }

    /// `IsTimeStoppedFor(id)`.
    pub fn is_time_stopped_for(&self, id: i32) -> bool {
        if id == ENVIRONMENT_ID {
            return false;
        }
        self.is_time_stopped()
            && !self
                .player(id)
                .is_some_and(|p| p.is_protected_from_time_stop)
    }

    /// `GameTime.FixedDeltaTime(id, dt)`.
    pub fn fixed_delta_time(&self, id: i32, dt: Fix) -> Fix {
        match self.player(id) {
            None => dt,
            Some(p) if p.is_protected_from_time_stop => dt,
            Some(_) => dt * self.time.player_time_scale,
        }
    }

    pub fn is_space(&self) -> bool {
        self.level
            .as_ref()
            .is_some_and(|l| l.world == LevelWorld::Space)
    }

    // -- events ---------------------------------------------------------------

    /// Emits a presentation cue / notification with a rollback-stable id.
    pub fn emit(&mut self, cue: Cue) {
        let id = EventId {
            tick: self.time.simulation_ticks,
            seq: self.event_seq,
        };
        self.event_seq += 1;
        self.events.push(Event { id, cue });
    }

    /// Emits a cue after the tick's clocks advanced (session flow): it
    /// belongs to the tick just simulated, continuing its sequence numbers.
    pub(crate) fn emit_late(&mut self, cue: Cue) {
        let id = EventId {
            tick: self.time.simulation_ticks - 1,
            seq: self.event_seq,
        };
        self.event_seq += 1;
        self.events.push(Event { id, cue });
    }

    // -- objects --------------------------------------------------------------

    pub fn obj(&self, id: ObjId) -> Option<&Object> {
        self.objects.get(id)
    }

    pub fn obj_mut(&mut self, id: ObjId) -> Option<&mut Object> {
        self.objects.get_mut(id)
    }

    /// Takes an object's payload out (for its own update).
    pub fn take_kind(&mut self, id: ObjId) -> Kind {
        match self.objects.get_mut(id) {
            Some(o) => std::mem::take(&mut o.kind),
            None => Kind::Empty,
        }
    }

    pub fn put_kind(&mut self, id: ObjId, k: Kind) {
        if let Some(o) = self.objects.get_mut(id) {
            o.kind = k;
        }
    }

    /// Registers part `part` of `obj` (Unity `Awake` → `RegisterUpdatable`).
    pub fn register(&mut self, obj: ObjId, part: usize) {
        let hier = self.objects.get(obj).map_or(0, |o| o.parts[part].hier);
        let u = Upd {
            obj,
            part: part as u8,
            hier,
        };
        match &mut self.bracket {
            Some(b) => b.push(u),
            None => self.to_be_added.push(u),
        }
    }

    /// Registers every part of an object in order.
    pub fn register_all(&mut self, obj: ObjId) {
        let n = self.objects.get(obj).map_or(0, |o| o.n_parts as usize);
        for i in 0..n {
            self.register(obj, i);
        }
    }

    /// Opens the `InstantiateFixed` registration bracket.
    pub fn open_bracket(&mut self) {
        self.bracket = Some(Vec::new());
    }

    /// Closes the bracket: offsets the numbers and appends the batch.
    pub fn close_bracket(&mut self) {
        let Some(buf) = self.bracket.take() else {
            return;
        };
        let off = self.prefabs_added.wrapping_add(100_000_000);
        for mut u in buf.iter().copied() {
            u.hier = u.hier.wrapping_add(off);
            if let Some(o) = self.objects.get_mut(u.obj) {
                o.parts[u.part as usize].hier = u.hier;
            }
            self.to_be_added.push(u);
        }
        self.prefabs_added = self.prefabs_added.wrapping_add(buf.len() as i32);
    }

    /// Accounts for a spawned prefab that the port does not simulate
    /// (presentation effects): it consumes `n` hierarchy numbers.
    pub fn consume_hierarchy(&mut self, n: i32) {
        self.prefabs_added = self.prefabs_added.wrapping_add(n);
    }

    /// `DestroyFix(gameObject)` (engine.md §5).
    pub fn destroy(&mut self, id: ObjId) {
        if !self.objects.exists(id) {
            return;
        }
        let mut stack = vec![(id, 0)];
        while let Some((o, depth)) = stack.pop() {
            if let Some(ob) = self.objects.get_mut(o) {
                for p in &mut ob.parts[..ob.n_parts as usize] {
                    p.destroyed = true;
                }
                if depth <= 1 {
                    ob.tf.destroyed = true;
                }
                ob.destroyed = true;
                for c in ob.children.clone() {
                    stack.push((c, depth + 1));
                }
            }
        }
        self.destroy_objects.push(id);
    }

    /// `DestroyFix(component)`.
    pub fn destroy_part(&mut self, id: ObjId, kind: PartKind) {
        if let Some(p) = self.objects.get_mut(id).and_then(|o| o.part_mut(kind)) {
            p.destroyed = true;
        }
    }

    fn free_recursive(&mut self, id: ObjId) {
        let children = self
            .objects
            .get(id)
            .map(|o| o.children.clone())
            .unwrap_or_default();
        for c in children {
            self.free_recursive(c);
        }
        if let Some(p) = self.objects.get(id).and_then(|o| o.parent)
            && let Some(po) = self.objects.get_mut(p)
        {
            po.children.retain(|c| *c != id);
        }
        self.objects.free(id);
    }

    fn part_gone(&self, u: &Upd) -> bool {
        match self.objects.get(u.obj) {
            None => true,
            Some(o) => o.parts[u.part as usize].destroyed,
        }
    }

    fn removal_pass(&mut self) {
        let mut i = self.updatables.len();
        while i > 0 {
            i -= 1;
            let u = self.updatables[i];
            if self.part_gone(&u) {
                if self.objects.exists(u.obj) {
                    self.on_destroy_updatable(u);
                }
                self.updatables.remove(i);
            }
        }
    }

    /// `IsEnabled()` of an updatable.
    fn is_enabled(&self, u: &Upd) -> bool {
        let Some(o) = self.objects.get(u.obj) else {
            return false;
        };
        let p = &o.parts[u.part as usize];
        match p.kind {
            PartKind::Collider => false,
            PartKind::PlayerBody => {
                p.enabled
                    && self.objects.active_in_hierarchy(u.obj)
                    && match &o.kind {
                        Kind::Bopl(b) => b.body.simulated,
                        // Ability bodies: `simulated` is 1 in every prefab.
                        Kind::Ability(_) => true,
                        _ => false,
                    }
            }
            PartKind::Behaviour(_) => {
                p.enabled
                    && (self.objects.active_in_hierarchy(u.obj)
                        || matches!(&o.kind, Kind::Behaviour(b) if b.always_enabled(p.kind)))
            }
            _ => p.enabled && self.objects.active_in_hierarchy(u.obj),
        }
    }

    // -- the tick -------------------------------------------------------------

    /// Applies this tick's inputs (online semantics, player.md §2.3).
    pub fn apply_inputs(&mut self, inputs: &[PlayerInput]) {
        let stopped = self.is_time_stopped();
        for (i, p) in self.players.iter_mut().enumerate() {
            if !stopped || p.is_protected_from_time_stop {
                p.inputs.input = inputs.get(i).copied().unwrap_or_default();
                p.inputs.uses_mouse = p.uses_mouse;
            }
        }
    }

    /// One simulation tick with the given inputs (list order of players).
    pub fn step(&mut self, inputs: &[PlayerInput]) {
        self.apply_inputs(inputs);
        self.tick();
        crate::flow::update(self, inputs);
    }

    /// `TickSimulation(dt)` (engine.md §4).
    pub fn tick(&mut self) {
        let dt = FIXED_DT;
        self.events.clear();
        self.event_seq = 0;
        // 3. players
        for i in 0..self.players.len() {
            self.players[i].inputs.update_edges();
            self.update_player(i, dt);
        }
        // 4. admit
        let mut i = self.to_be_added.len();
        while i > 0 {
            i -= 1;
            let u = self.to_be_added[i];
            if self.part_gone(&u) {
                self.to_be_added.remove(i);
            }
        }
        introsort(&mut self.to_be_added, |a, b| {
            (a.hier.wrapping_sub(b.hier)).cmp(&0)
        });
        let mut m = 0;
        while m < self.to_be_added.len() {
            let u = self.to_be_added[m];
            if !self.part_gone(&u) {
                self.init_part(u);
            }
            m += 1;
        }
        let batch = std::mem::take(&mut self.to_be_added);
        self.updatables.extend(batch);
        // 5. duplicated objects
        if !self.is_time_stopped() {
            abilities::update_duplicates(self);
        }
        // 6. removal
        self.removal_pass();
        // 7. update
        let mut i = 0;
        while i < self.updatables.len() {
            let u = self.updatables[i];
            if !self.part_gone(&u) && self.is_enabled(&u) {
                self.update_part(u, dt, false);
            }
            i += 1;
        }
        // 8. static ability checks (magnet parry)
        abilities::static_update(self);
        // 9. first tick of a level: no physics
        if self.time.loaded_level_this_update {
            self.time.sim_time_passed += dt;
            self.time.sim_time_since_level_loaded += dt;
            self.time.simulation_ticks += 1;
            self.time.loaded_level_this_update = false;
            self.check_fault();
            return;
        }
        // 10. physics
        let stopped = self.is_time_stopped();
        let contacts = match self.physics.simulate_begin(&mut self.objects, dt, stopped) {
            Some(bp) => {
                // Step 8 of `Simulate`: ropes and beams (ability-owned).
                abilities::physics_mid_step(self);
                self.physics.simulate_end(&mut self.objects, bp)
            }
            None => Vec::new(),
        };
        for c in contacts {
            self.dispatch_contact(c);
        }
        // 11. late update
        let mut i = 0;
        while i < self.updatables.len() {
            let u = self.updatables[i];
            if !self.part_gone(&u) && self.is_enabled(&u) {
                self.update_part(u, dt, true);
            }
            i += 1;
        }
        // 12. ropes
        abilities::clear_rope_flags(self);
        // 13. removal
        self.removal_pass();
        // 14. destroy queue
        let q = std::mem::take(&mut self.destroy_objects);
        for id in q {
            self.free_recursive(id);
        }
        // 15. clocks
        self.time.sim_time_passed += dt;
        self.time.sim_time_since_level_loaded += dt;
        self.time.simulation_ticks += 1;
        self.check_fault();
    }

    fn check_fault(&mut self) {
        if let Some(f) = take_fault() {
            if self.fault.is_none() {
                self.fault = Some((self.time.simulation_ticks, f));
            }
            self.emit(Cue::Fault(f));
            crate::session::end_round_by_fault(self);
        }
    }

    /// `Player.UpdatePlayer(dt)`.
    fn update_player(&mut self, i: usize, dt: Fix) {
        let id = self.players[i].id;
        let fdt = self.fixed_delta_time(id, dt);
        let p = &mut self.players[i];
        p.became_visible = false;
        if p.is_invisible {
            p.time_spent_invisible += fdt;
            p.time_since_entered_ability += fdt;
            if p.time_spent_invisible > Fix::from_int(5) {
                p.is_invisible = false;
                p.time_spent_invisible = Fix::ZERO;
                p.time_since_entered_ability = Fix::ZERO;
                p.became_visible = true;
            }
        } else {
            p.time_spent_invisible = Fix::ZERO;
            p.time_since_entered_ability = Fix::ZERO;
        }
        p.setter_last_frame = p.setter_this_frame;
        p.setter_this_frame = None;
    }

    // -- dispatch ---------------------------------------------------------------

    fn init_part(&mut self, u: Upd) {
        let kind = self
            .objects
            .get(u.obj)
            .map(|o| o.parts[u.part as usize].kind);
        match kind {
            Some(PartKind::Collider) => {
                let manual = self
                    .objects
                    .get(u.obj)
                    .and_then(|o| o.collider)
                    .is_some_and(|c| c.manual_init);
                if !manual {
                    self.physics.register(&mut self.objects, u.obj, u.hier);
                }
            }
            // `BoplBody.Init` is empty.
            Some(PartKind::BoplBody) => {}
            Some(k) => self.with_kind(u.obj, |w, kd| match kd {
                Kind::Bopl(b) => b.init(w, u.obj, k),
                Kind::Platform(p) => p.init(w, u.obj, k),
                Kind::Session(s) => s.init(w, u.obj),
                Kind::Spawner(s) => s.init(w, u.obj),
                Kind::Ability(a) => a.init(w, u.obj, k),
                Kind::Behaviour(b) => b.init(w, u.obj, k),
                Kind::Tutorial(t) => crate::tutorial::init(w, u.obj, t),
                _ => {}
            }),
            None => {}
        }
    }

    fn update_part(&mut self, u: Upd, dt: Fix, late: bool) {
        let Some(k) = self
            .objects
            .get(u.obj)
            .map(|o| o.parts[u.part as usize].kind)
        else {
            return;
        };
        if k == PartKind::BoplBody {
            // The generic `BoplBody` component (no LateUpdateSim).
            if !late {
                crate::abilities::ability_body::bopl_body_update(self, u.obj);
            }
            return;
        }
        self.with_kind(u.obj, |w, kd| match kd {
            Kind::Bopl(b) => b.update(w, u.obj, k, dt, late),
            Kind::Platform(p) => p.update(w, u.obj, k, dt, late),
            Kind::Session(s) if !late => s.update(w, u.obj, dt),
            Kind::Spawner(s) if !late => s.update(w, u.obj, dt),
            Kind::Ability(a) => a.update(w, u.obj, k, dt, late),
            Kind::Behaviour(b) => b.update(w, u.obj, k, dt, late),
            Kind::Tutorial(t) if !late => crate::tutorial::update(w, u.obj, t, dt),
            _ => {}
        });
    }

    fn on_destroy_updatable(&mut self, u: Upd) {
        let Some(k) = self
            .objects
            .get(u.obj)
            .map(|o| o.parts[u.part as usize].kind)
        else {
            return;
        };
        self.with_kind(u.obj, |w, kd| {
            if let Kind::Behaviour(b) = kd {
                b.on_destroy(w, u.obj, k);
            }
        });
    }

    /// Runs `f` with the object's payload taken out.
    pub fn with_kind<R>(&mut self, id: ObjId, f: impl FnOnce(&mut World, &mut Kind) -> R) -> R {
        let mut k = self.take_kind(id);
        let r = f(self, &mut k);
        // The object may have been freed meanwhile (it cannot: frees happen at
        // step 14), so `put_kind` only fails for stale handles.
        if matches!(self.objects.get(id).map(|o| &o.kind), Some(Kind::Empty)) {
            self.put_kind(id, k);
        }
        r
    }

    /// Delivers a contact to the receiving collider's listeners.
    pub fn dispatch_contact(&mut self, c: Contact) {
        if !self.objects.exists(c.me) {
            return;
        }
        self.with_kind(c.me, |w, kd| match kd {
            Kind::Bopl(b) => b.on_contact(w, c.me, c),
            Kind::Platform(_) => {}
            Kind::Ability(a) => a.on_contact(w, c.me, c),
            Kind::Behaviour(b) => b.on_contact(w, c.me, c),
            Kind::Static => crate::session::static_contact(w, c),
            Kind::Tutorial(t) => crate::tutorial::on_contact(w, c.me, t, c),
            _ => {}
        });
    }

    // -- colliders ------------------------------------------------------------

    /// The collider `Scale` setter (physics.md §2.3).
    pub fn set_collider_scale(&mut self, id: ObjId, value: Fix) {
        let Some(o) = self.objects.get_mut(id) else {
            return;
        };
        let Some(def) = o.collider else {
            return;
        };
        let old = o.tf.scale;
        let k = Fix::ONE / old;
        let s = value.max(def.min_scale).min(def.max_scale);
        o.tf.set_scale_dphysics_only(s);
        let f = k * s;
        if !def.initialized {
            if let Some(c) = o.collider.as_mut() {
                c.start_radius = c.start_radius * f;
                c.start_extents = c.start_extents * f;
            }
            return;
        }
        let grew = old < value;
        let is_platform = matches!(o.kind, Kind::Platform(_));
        if let Kind::Platform(p) = &mut o.kind
            && old > p.sticky.base_scale
            && grew
        {
            p.time_since_last_scaled_up = Fix::ZERO;
        }
        match def.kind {
            crate::physics::shapes::ShapeKind::Circle => {
                if let Some(c) = self.physics.circle_mut(id) {
                    c.radius = c.radius * f;
                }
            }
            crate::physics::shapes::ShapeKind::Box => {
                if let Some(b) = self.physics.box_mut(id) {
                    b.right = b.right * f;
                    b.up = b.up * f;
                    b.ie = b.ie / f;
                }
            }
            _ => {
                // Platform attachables re-place themselves first (on the
                // old shape), physics.md §2.3.
                if is_platform {
                    crate::abilities::reposition_attachables(self, id);
                }
                if let Some(r) = self.physics.rr_mut(id) {
                    r.radius = r.radius * f;
                    r.b.right = r.b.right * f;
                    r.b.up = r.b.up * f;
                    r.b.ie = r.b.ie / f;
                }
            }
        }
    }

    /// Enables/disables the collider component of an object.
    pub fn set_collider_enabled(&mut self, id: ObjId, e: bool) {
        if let Some(o) = self.objects.get_mut(id) {
            o.set_part_enabled(PartKind::Collider, e);
        }
    }

    // -- hashing ----------------------------------------------------------------

    /// The original's desync checksum over all non-destroyed fixed transforms.
    pub fn checksum(&self) -> Fix {
        let mut s = Fix::ZERO;
        for o in self
            .objects
            .slots
            .iter()
            .filter(|o| o.live && !o.tf.destroyed)
        {
            let t = &o.tf;
            s = s.slow_add(
                t.position.x * Fix::TWO + t.position.y * Fix::THREE + t.rotation * Fix::from_int(5),
            );
        }
        s
    }

    /// A 64-bit hash of the complete simulation state (for determinism tests
    /// and desync detection).
    pub fn state_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = Fnv64::default();
        self.rng.hash(&mut h);
        self.time.hash(&mut h);
        self.round.hash(&mut h);
        self.players.hash(&mut h);
        self.updatables.hash(&mut h);
        self.prefabs_added.hash(&mut h);
        for o in self.objects.slots.iter() {
            o.live.hash(&mut h);
            if !o.live {
                continue;
            }
            o.generation.hash(&mut h);
            o.tf.hash(&mut h);
            o.active.hash(&mut h);
            o.parent.hash(&mut h);
            o.layer.hash(&mut h);
            o.player_id.hash(&mut h);
            o.destroyed.hash(&mut h);
            o.parts[..o.n_parts as usize].hash(&mut h);
            o.collider.hash(&mut h);
            o.bb.hash(&mut h);
            o.kind.hash(&mut h);
        }
        self.physics.rrs.shapes.hash(&mut h);
        self.physics.rrs.bodies.hash(&mut h);
        self.physics.rrs.parents.hash(&mut h);
        self.physics.boxes.shapes.hash(&mut h);
        self.physics.boxes.bodies.hash(&mut h);
        self.physics.circles.shapes.hash(&mut h);
        self.physics.circles.bodies.hash(&mut h);
        self.physics.composites.hash(&mut h);
        self.physics.ropes.hash(&mut h);
        self.ability_globals.hash(&mut h);
        self.flow.hash(&mut h);
        self.tutorial.hash(&mut h);
        h.finish()
    }
}

/// FNV-1a (64-bit): a stable, platform-independent hasher.
#[derive(Clone, Copy)]
pub struct Fnv64(u64);

impl Default for Fnv64 {
    fn default() -> Fnv64 {
        Fnv64(0xcbf2_9ce4_8422_2325)
    }
}

impl std::hash::Hasher for Fnv64 {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

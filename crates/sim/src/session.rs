//! The match handler of a level (`GameSessionHandler`): level load,
//! platform fly-in/out, player spawning, round-end detection, sudden death
//! (engine.md §10–§12, match-flow.md §6–§10), plus the ability spawner.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::abilities;
use crate::bopl::Bopl;
use crate::data::LevelData;
use crate::fix::Fix;
use crate::object::{ObjId, PartKind};
use crate::physics::Contact;
use crate::platform::{Platform, update_bounds};
use crate::vec2::Vec2;
use crate::view::{Cue, Visual};
use crate::world::{Kind, World};

/// A function step of a routine queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SessionFn {
    SpawnPlayers,
    PrepareWinnerTriangle,
    EnablePlatforms,
    ActivateWinnerTriangle,
    AnimateOutLevel,
}

/// A loop step of a routine queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SessionLoop {
    AnimateIn,
    AnimateOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Routine {
    Delay(Fix),
    Delay1,
    Function(SessionFn),
    Loop(SessionLoop),
}

/// `RoutineQueue` (engine.md §10): at most one step per update.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RoutineQueue {
    pub items: VecDeque<Routine>,
    pub delayed: Fix,
}

/// `GameSessionHandler`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct GameSession {
    pub start: Vec<Vec2>,
    pub target: Vec<Vec2>,
    pub start_rot: Vec<Fix>,
    pub delay: Vec<Fix>,
    pub played: Vec<bool>,
    pub t: Fix,
    pub level_q: RoutineQueue,
    pub player_q: RoutineQueue,
    pub time_since_game_over: Fix,
}

impl GameSession {
    fn run_queue(w: &mut World, me: ObjId, s: &mut GameSession, which: u8, dt: Fix) {
        let q = if which == 0 {
            &mut s.level_q
        } else {
            &mut s.player_q
        };
        let Some(&r) = q.items.front() else {
            return;
        };
        match r {
            Routine::Delay(d) => {
                q.delayed += dt;
                if q.delayed > d {
                    q.delayed = Fix::ZERO;
                    q.items.pop_front();
                }
            }
            Routine::Delay1 => {
                q.items.pop_front();
            }
            Routine::Function(f) => {
                q.items.pop_front();
                s.call(w, me, f);
            }
            Routine::Loop(l) => {
                if !s.run_loop(w, l, dt) {
                    let q = if which == 0 {
                        &mut s.level_q
                    } else {
                        &mut s.player_q
                    };
                    q.items.pop_front();
                }
            }
        }
    }

    fn level(w: &World) -> Arc<LevelData> {
        w.level.clone().expect("session without a level")
    }

    /// `Awake`: move every platform to its start position.
    pub fn awake(w: &mut World, s: &mut GameSession) {
        let lv = Self::level(w);
        let n = w.platforms.len();
        s.start = vec![Vec2::ZERO; n];
        s.target = vec![Vec2::ZERO; n];
        s.start_rot = vec![Fix::ZERO; n];
        s.delay = vec![Fix::ZERO; n];
        s.played = vec![false; n];
        let radius = lv.session.level_animation_radius;
        for i in 0..n {
            let id = w.platforms[i];
            if !w.objects.alive(id) {
                continue;
            }
            let t = lv.platforms[i].transform;
            s.target[i] = t.position;
            s.start_rot[i] = t.rotation;
            s.start[i] =
                t.position + (t.position + Vec2::UP * Fix::from_int(20)).normalized_safe() * radius;
            if let Some(o) = w.objects.get_mut(id) {
                o.tf.position = s.start[i];
                o.tf.set_rotation(s.start_rot[i]);
            }
        }
    }

    pub fn init(&mut self, w: &mut World, _me: ObjId) {
        let lv = Self::level(w);
        for i in 0..w.platforms.len() {
            let id = w.platforms[i];
            if !w.objects.alive(id) {
                continue;
            }
            w.set_collider_enabled(id, false);
            let (sp, sr) = (self.start[i], self.start_rot[i]);
            if let Some(o) = w.objects.get_mut(id) {
                if let Kind::Platform(p) = &mut o.kind
                    && let Some(s) = &mut p.spring
                {
                    s.initialize(sp, sr);
                }
                o.set_part_enabled(PartKind::AnimateVelocity, false);
                o.tf.set_rotation(sr);
                o.tf.position = sp;
            }
            self.delay[i] = w.rng.random_fix(Fix::ZERO, Fix::ONE) * lv.session.max_random_delay;
        }
        w.round.game_in_progress = false;
        for p in &mut w.players {
            p.is_protected_from_time_stop = false;
        }
        w.time.player_time_scale = Fix::ONE;
        w.round.game_paused = false;
        // StartSpawnPlayersRoutine
        for p in &mut w.players {
            p.won_this_round = false;
            p.kills_at_start_of_round = p.kills;
            p.cause_of_death = crate::player::CauseOfDeath::NotDeadYet;
        }
        w.emit(Cue::Sound("go"));
        let sd = &lv.session;
        self.player_q.items.extend([
            Routine::Delay(sd.time_before_two),
            Routine::Function(SessionFn::SpawnPlayers),
            Routine::Delay(sd.time_before_go),
            Routine::Function(SessionFn::PrepareWinnerTriangle),
        ]);
        // StartAnimateInLevelRoutine
        self.level_q
            .items
            .push_back(Routine::Delay(crate::consts::F_0_1));
        self.played = vec![false; w.platforms.len()];
        self.t = sd.start_progress;
        self.level_q
            .items
            .push_back(Routine::Loop(SessionLoop::AnimateIn));
        self.level_q
            .items
            .push_back(Routine::Function(SessionFn::EnablePlatforms));
    }

    pub fn update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        Self::run_queue(w, me, self, 0, dt);
        Self::run_queue(w, me, self, 1, dt);
        if w.round.game_over {
            self.time_since_game_over += dt;
        } else {
            self.time_since_game_over = Fix::ZERO;
        }
        let lv = Self::level(w);
        if !w.round.sudden_death
            && w.round.game_in_progress
            && w.time.sim_time_since_level_loaded > lv.session.time_before_sudden_death
        {
            activate_sudden_death(w, true);
        }
        let alive = w.players.iter().filter(|p| p.is_alive()).count();
        if !w.round.game_in_progress {
            return;
        }
        let teams_left = teams_left(w);
        let n = w.players.len();
        if n == 1 && alive == 0 {
            w.round.game_in_progress = false;
            let team = w.players[0].team;
            self.win_game(w, team);
        } else if n > 1 && teams_left == 1 {
            w.round.game_in_progress = false;
            // The first alive (connected) player's team (QUIRK 4).
            if let Some(team) = (0..n)
                .find(|&i| w.players[i].is_alive() && !crate::flow::is_gone(w, i))
                .map(|i| w.players[i].team)
            {
                self.win_game(w, team);
            }
        } else if n > 1 && teams_left == 0 {
            w.round.game_in_progress = false;
            self.declare_undecided(w);
        }
    }

    fn call(&mut self, w: &mut World, _me: ObjId, f: SessionFn) {
        match f {
            SessionFn::SpawnPlayers => spawn_players(w),
            SessionFn::PrepareWinnerTriangle => {}
            SessionFn::EnablePlatforms => {
                for i in 0..w.platforms.len() {
                    let id = w.platforms[i];
                    if !w.objects.alive(id) {
                        continue;
                    }
                    let tgt = self.target[i];
                    if let Some(o) = w.objects.get_mut(id) {
                        if let Kind::Platform(p) = &mut o.kind
                            && let Some(s) = &mut p.spring
                        {
                            s.home = tgt;
                        }
                        o.set_part_enabled(PartKind::AnimateVelocity, true);
                    }
                    w.set_collider_enabled(id, true);
                    update_bounds(w, id);
                }
            }
            SessionFn::ActivateWinnerTriangle => {
                for p in &mut w.players {
                    p.is_protected_from_time_stop = false;
                }
                w.time.player_time_scale = Fix::ZERO;
                w.round.game_paused = true;
                w.round.winner_screen = true;
                self.level_q
                    .items
                    .push_back(Routine::Function(SessionFn::AnimateOutLevel));
                w.emit(Cue::WinnerScreen);
            }
            SessionFn::AnimateOutLevel => {
                for i in 0..w.platforms.len() {
                    let id = w.platforms[i];
                    if !w.objects.alive(id) {
                        continue;
                    }
                    if let Some(o) = w.objects.get_mut(id) {
                        o.set_part_enabled(PartKind::AnimateVelocity, false);
                        self.target[i] = o.tf.position;
                    }
                }
                self.played.fill(false);
                self.t = Self::level(w).session.start_progress;
                self.level_q
                    .items
                    .push_back(Routine::Loop(SessionLoop::AnimateOut));
            }
        }
    }

    /// The fly-in / fly-out loops: `end · c + start · (One − c)` per axis.
    fn run_loop(&mut self, w: &mut World, l: SessionLoop, dt: Fix) -> bool {
        let lv = Self::level(w);
        let sd = &lv.session;
        let mut done = true;
        self.t += dt / sd.level_animation_time;
        let (curve, threshold) = match l {
            SessionLoop::AnimateIn => (&sd.animate_in, sd.block_in_delay),
            SessionLoop::AnimateOut => (&sd.animate_out, sd.block_out_delay),
        };
        for i in 0..w.platforms.len() {
            let id = w.platforms[i];
            if !w.objects.alive(id) {
                if l == SessionLoop::AnimateIn {
                    self.played[i] = true;
                }
                continue;
            }
            if self.t < Fix::ONE + self.delay[i] {
                done = false;
            }
            if self.t > threshold + self.delay[i] && !self.played[i] {
                w.emit(Cue::SoundPrefix("easeInBlock"));
                self.played[i] = true;
            }
            let c = curve.eval(self.t - self.delay[i]);
            let (from, to) = match l {
                SessionLoop::AnimateIn => (self.start[i], self.target[i]),
                SessionLoop::AnimateOut => (self.target[i], self.start[i]),
            };
            let p = Vec2::new(
                to.x * c + from.x * (Fix::ONE - c),
                to.y * c + from.y * (Fix::ONE - c),
            );
            if let Some(o) = w.objects.get_mut(id) {
                o.tf.position = p;
            }
        }
        !done
    }

    fn win_game(&mut self, w: &mut World, team: i32) {
        activate_sudden_death(w, false);
        for p in &mut w.players {
            p.is_most_recent_winner = false;
        }
        for i in 0..w.players.len() {
            // Players who left (online) cannot win.
            if w.players[i].team == team && !crate::flow::is_gone(w, i) {
                win_a_game(&mut w.players[i]);
            }
        }
        w.emit(Cue::RoundWon { team });
        self.prepare_next_level(w);
    }

    /// `DeclareGameUndecided` (a draw: the last to die win).
    pub fn declare_undecided(&mut self, w: &mut World) {
        w.round.game_in_progress = false;
        activate_sudden_death(w, false);
        // Players who left (online) neither count nor win.
        let gone: Vec<bool> = (0..w.players.len())
            .map(|i| crate::flow::is_gone(w, i))
            .collect();
        let mut m = 0i64;
        for (i, p) in w.players.iter().enumerate() {
            if !gone[i] && m < p.timestamp_when_killed {
                m = p.timestamp_when_killed;
            }
        }
        for (i, p) in w.players.iter_mut().enumerate() {
            p.is_most_recent_winner = !gone[i] && p.timestamp_when_killed == m;
        }
        let teams: Vec<i32> = w
            .players
            .iter()
            .filter(|p| p.is_most_recent_winner)
            .map(|p| p.team)
            .collect();
        for (i, p) in w.players.iter_mut().enumerate() {
            if teams.contains(&p.team) && !gone[i] {
                p.is_most_recent_winner = true;
            }
        }
        for p in &mut w.players {
            if p.is_most_recent_winner {
                win_a_game(p);
            }
        }
        w.emit(Cue::RoundDraw);
        self.prepare_next_level(w);
    }

    fn prepare_next_level(&mut self, w: &mut World) {
        w.round.game_in_progress = false;
        w.round.game_over = true;
        w.round.decided = true;
        let lv = Self::level(w);
        self.player_q.items.push_back(Routine::Delay(
            lv.session.delay_before_next_level_input - Fix::from_long(2),
        ));
        self.player_q
            .items
            .push_back(Routine::Function(SessionFn::ActivateWinnerTriangle));
    }
}

fn win_a_game(p: &mut crate::player::Player) {
    p.games_won += 1;
    p.is_most_recent_winner = true;
    p.won_this_round = true;
}

fn activate_sudden_death(w: &mut World, on: bool) {
    w.round.sudden_death = on;
    if on {
        w.round.sudden_death_stamp = w.time.sim_time_since_level_loaded;
        w.emit(Cue::SuddenDeath);
    }
}

/// `TeamsLeft()`: distinct teams with an alive player (players who left
/// an online session do not count, `flow::set_gone`).
pub fn teams_left(w: &World) -> usize {
    let ps = &w.players;
    let live = |i: usize| ps[i].is_alive() && !crate::flow::is_gone(w, i);
    let mut n = 0;
    for i in 0..ps.len() {
        if live(i) && !(i + 1..ps.len()).any(|j| live(j) && ps[j].team == ps[i].team) {
            n += 1;
        }
    }
    n
}

/// Spawn point of a team slot (4 per level, generated ones beyond).
fn team_spawn(lv: &LevelData, slot: usize) -> Vec2 {
    if slot < 4 {
        lv.team_spawns[slot]
    } else {
        lv.extra_spawns
            .get(slot - 4)
            .copied()
            .unwrap_or(lv.team_spawns[slot % 4])
    }
}

/// `SpawnPlayers()` (match-flow.md §7).
fn spawn_players(w: &mut World) {
    w.round.game_in_progress = true;
    w.round.game_over = false;
    let lv = w.level.clone().expect("level");
    // Players who left an online session do not spawn (`flow::set_gone`).
    let gone: Vec<bool> = (0..w.players.len())
        .map(|i| crate::flow::is_gone(w, i))
        .collect();
    let mut teams: Vec<i32> = Vec::new();
    for (i, p) in w.players.iter().enumerate() {
        if !gone[i] && !teams.contains(&p.team) {
            teams.push(p.team);
        }
    }
    teams.sort();
    let original = w.players.len() <= 4;
    let mut counter = [0i32; 32];
    let s = lv.teammate_spacing;
    let n_ab = w.settings.number_of_abilities;
    for (i, &left) in gone.iter().enumerate() {
        if left {
            w.players[i].players_and_clones_still_alive = 0;
            w.players[i].still_alive_this_round = false;
            continue;
        }
        let team = w.players[i].team;
        let slot = teams.iter().position(|t| *t == team).unwrap_or(0);
        w.players[i].players_and_clones_still_alive = 1;
        let base = team_spawn(&lv, slot);
        let j = counter[slot];
        let pos = if j == 0 {
            base
        } else if original {
            // QUIRK: the second test indexes the counter by the raw team id.
            if counter[(team as usize).min(31)] != 1 {
                base + Vec2::new(s, (-s) * crate::consts::F_0_5)
            } else {
                base - Vec2::new(s, s * crate::consts::F_0_5)
            }
        } else {
            // Generalisation for > 4 players (match-flow.md §17.2).
            let k = Fix::from_int((j + 1) / 2);
            let sign = if j % 2 == 1 { -Fix::ONE } else { Fix::ONE };
            base + Vec2::new(sign * k * s, (-s) * crate::consts::F_0_5)
        };
        counter[slot] += 1;
        let id = w.players[i].id;
        let bopl = Bopl::spawn(w, pos, id);
        w.players[i].bopl = Some(bopl);
        w.players[i].spawn_position = pos;
        // Abilities: one inactive object per slot; Random resolved now.
        let chosen: Vec<usize> = w.players[i].abilities.iter().copied().take(n_ab).collect();
        let all_random = chosen.iter().all(|a| *a == 1);
        let mut offensive = false;
        let mut current = Vec::new();
        let mut objs = Vec::new();
        let count = w.content.abilities.len() as i32;
        for (k, &a) in chosen.iter().enumerate() {
            let mut idx = a;
            if a == 1 && count > 2 {
                idx = (w.rng.random_int(0, count - 2) + 2) as usize;
                offensive |= w.content.abilities[idx].offensive;
                if all_random && !offensive && k == chosen.len() - 1 {
                    idx = (w.rng.random_int(0, count - 2) + 2) as usize;
                }
            }
            current.push(idx);
            objs.push(abilities::instantiate(w, idx, id, bopl));
        }
        w.players[i].current_abilities = current;
        if let Some(Kind::Bopl(b)) = w.objects.get_mut(bopl).map(|o| &mut o.kind) {
            b.ctrl.abilities = objs;
        }
    }
}

/// Loads a level (PreLevelLoad, scene batch, PostLevelLoad; engine.md §11).
pub fn load_level(w: &mut World, level: Arc<LevelData>) {
    // PreLevelLoad
    for p in &mut w.players {
        p.reset_for_level();
    }
    w.time.player_time_scale = Fix::ONE;
    w.updatables.clear();
    w.to_be_added.clear();
    w.objects = Default::default();
    w.physics.clear();
    w.physics.params = level.physics;
    w.platforms.clear();
    w.time.sim_time_since_level_loaded = Fix::ZERO;
    w.round = Default::default();
    w.ability_globals = Default::default();
    // Per-level flow state: circles slide down, wheels close.
    w.flow.players = vec![Default::default(); w.players.len()];
    w.flow.next = None;
    w.flow.winner_screen_at = None;
    w.flow.request = None;
    let seed = w.settings.seed;
    w.rng.init_seed(seed);
    w.level = Some(level.clone());
    w.tutorial = None;
    if let Some(def) = level.tutorial.clone() {
        load_tutorial(w, &level, &def);
        w.time.loaded_level_this_update = true;
        return;
    }
    // The scene: session handler, platforms, statics, spawners.
    let sid = w.objects.create(Kind::Empty, 8);
    w.objects
        .get_mut(sid)
        .unwrap()
        .add_part(PartKind::GameSession, level.session.hier);
    w.session = Some(sid);
    for (i, pd) in level.platforms.iter().enumerate() {
        let id = Platform::create(w, pd, i);
        w.platforms.push(id);
    }
    for (i, sd) in level.statics.iter().enumerate() {
        let id = w.objects.create(Kind::Static, sd.layer);
        let o = w.objects.get_mut(id).unwrap();
        o.visual = Visual::Static(i as u16);
        let t = sd.transform;
        o.tf.position = t.position;
        o.tf.rotation = t.rotation;
        o.tf.right = t.right;
        o.tf.up = t.up;
        o.tf.offset = t.offset;
        o.collider = Some(sd.collider);
        let k = o.add_part(PartKind::Collider, sd.hier);
        o.parts[k].enabled = sd.enabled;
    }
    for sp in &level.spawners {
        let id = w.objects.create(Kind::Spawner(Box::default()), 0);
        let o = w.objects.get_mut(id).unwrap();
        o.tf.position = sp.position;
        o.add_part(PartKind::AbilitySpawner, sp.hier);
    }
    // Awake of the session handler (moves platforms to their start).
    let mut s = GameSession::default();
    GameSession::awake(w, &mut s);
    w.put_kind(sid, Kind::Session(Box::new(s)));
    let ids: Vec<ObjId> = w.objects.ids().collect();
    for id in ids {
        w.register_all(id);
    }
    // PostLevelLoad
    w.time.loaded_level_this_update = true;
}

/// The tutorial scene: platforms (in place, no fly-in), statics, then the
/// tutorial handler and objects with the player (`tutorial.rs`).
fn load_tutorial(w: &mut World, level: &Arc<LevelData>, def: &Arc<crate::data::TutorialDef>) {
    w.session = None;
    for (i, pd) in level.platforms.iter().enumerate() {
        let id = Platform::create(w, pd, i);
        w.platforms.push(id);
    }
    for (i, sd) in level.statics.iter().enumerate() {
        let id = w.objects.create(Kind::Static, sd.layer);
        let o = w.objects.get_mut(id).unwrap();
        o.visual = Visual::Static(i as u16);
        let t = sd.transform;
        o.tf.position = t.position;
        o.tf.rotation = t.rotation;
        o.tf.right = t.right;
        o.tf.up = t.up;
        o.tf.offset = t.offset;
        o.collider = Some(sd.collider);
        let k = o.add_part(PartKind::Collider, sd.hier);
        o.parts[k].enabled = sd.enabled;
    }
    let ids: Vec<ObjId> = w.objects.ids().collect();
    for id in ids {
        w.register_all(id);
    }
    crate::tutorial::load(w, level, def);
}

/// A fault ends the round as a draw (the original online host's handler).
pub fn end_round_by_fault(w: &mut World) {
    if !w.round.game_in_progress {
        return;
    }
    w.round.ended_by_fault = true;
    if let Some(sid) = w.session {
        w.with_kind(sid, |w, k| {
            if let Kind::Session(s) = k {
                s.declare_undecided(w);
            }
        });
    }
}

/// Contacts of collider-only objects (kill zones).
pub fn static_contact(_w: &mut World, _c: Contact) {}

/// `AbilitySpawner` (abilities.md §11.1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Spawner {
    pub time: Fix,
    pub count: i32,
}

impl Spawner {
    pub fn init(&mut self, _w: &mut World, _me: ObjId) {}

    pub fn update(&mut self, w: &mut World, me: ObjId, dt: Fix) {
        if !w.is_time_stopped() {
            self.time += dt;
        }
        if self.time > Fix::from_int(40) {
            self.time = Fix::ZERO;
            let pos = w.objects.get(me).map_or(Vec2::ZERO, |o| o.tf.position);
            let dir = w.rng.random_unit_vector();
            abilities::spawn_pickup(w, pos, None, dir);
            self.count += 1;
            if self.count >= 3
                && let Some(o) = w.objects.get_mut(me)
            {
                o.set_part_enabled(PartKind::AbilitySpawner, false);
            }
        }
    }
}

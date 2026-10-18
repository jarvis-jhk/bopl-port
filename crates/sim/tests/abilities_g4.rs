//! Scenario tests for the G4 abilities (Bow, Black hole, Time stop,
//! Invisibility, Revival, Platform, Gust, Push, Random) on Level1 of the
//! real import (skipped without an import in `cache/import` / the user
//! data dir), plus a 12-player determinism / rollback run using them.

use std::sync::Arc;

use bopl_content::Loaded;
use bopl_sim::abilities::{AbilityState, Behaviour};
use bopl_sim::input::{BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2, BTN_JUMP, PlayerInput};
use bopl_sim::object::ObjId;
use bopl_sim::player::{CauseOfDeath, PlayerSetup};
use bopl_sim::world::{Kind, MatchSettings, World};
use bopl_sim::{Fix, Vec2};

// Ability-list indices.
const RANDOM: usize = 1;
const DASH: usize = 2;
const BOW: usize = 4;
const GUST: usize = 7;
const TIME_STOP: usize = 12;
const PLATFORM: usize = 14;
const REVIVAL: usize = 15;
const BLACK_HOLE: usize = 18;
const INVISIBILITY: usize = 19;
const PUSH: usize = 22;

/// Angle bytes: +x, ≈ +y, −x.
const RIGHT: u8 = 1;
const UP: u8 = 65;
const LEFT: u8 = 128;

fn loaded() -> Option<Loaded> {
    let dir = bopl_content::default_import_dir();
    if !dir.join("index.json").exists() {
        eprintln!("no import at {}; skipping", dir.display());
        return None;
    }
    Some(bopl_content::load(dir).expect("import loads"))
}

fn world(l: &Loaded, level: usize, players: &[PlayerSetup], seed: u32) -> World {
    let lv = bopl_content::load_level(l, level).unwrap();
    let mut w = World::new(
        Arc::clone(&l.content),
        players,
        MatchSettings {
            number_of_abilities: 3,
            seed,
        },
    );
    bopl_sim::session::load_level(&mut w, lv);
    w
}

fn setup(id: i32, team: i32, abilities: [usize; 3]) -> PlayerSetup {
    PlayerSetup {
        id,
        team,
        color: id - 1,
        abilities: abilities.to_vec(),
        uses_keyboard_and_mouse: false,
    }
}

/// Two players on Level1 after 10 s (they stand on platforms from tick
/// ≈ 200 on and every slot with a cooldown ≤ 6 is ready):
/// player 1 at x ≈ −14.4 with `a`, player 2 at x ≈ 12.7 (`team2` 1) or
/// next to player 1 (`team2` 0) with `b`.
fn duo(l: &Loaded, a: [usize; 3], b: [usize; 3], team2: i32) -> World {
    let mut w = world(l, 6, &[setup(1, 0, a), setup(2, team2, b)], 1);
    idle(&mut w, 600);
    w
}

fn idle(w: &mut World, n: usize) {
    let k = w.players.len();
    for _ in 0..n {
        w.step(&vec![PlayerInput::default(); k]);
    }
}

fn step1(w: &mut World, p1: PlayerInput) {
    let mut v = vec![PlayerInput::default(); w.players.len()];
    v[0] = p1;
    w.step(&v);
}

fn hold(slot: usize, angle: u8) -> PlayerInput {
    PlayerInput::new(angle, BTN_ABILITY0 << slot)
}

fn bopl(w: &World, id: i32) -> Option<&bopl_sim::bopl::Bopl> {
    let b = w.player(id)?.bopl?;
    match &w.objects.get(b)?.kind {
        Kind::Bopl(b) => Some(b),
        _ => None,
    }
}

fn pos(w: &World, id: i32) -> Vec2 {
    let b = w.player(id).and_then(|p| p.bopl);
    b.and_then(|b| w.objects.get(b))
        .map_or(Vec2::ZERO, |o| o.tf.position)
}

fn in_ability(w: &World, id: i32) -> bool {
    bopl(w, id).is_some_and(|b| b.ctrl.is_in_ability)
}

fn slot_obj(w: &World, id: i32, k: usize) -> ObjId {
    bopl(w, id).unwrap().ctrl.abilities[k]
}

fn ability(w: &World, id: i32, k: usize) -> &bopl_sim::abilities::AbilityObject {
    match &w.objects.get(slot_obj(w, id, k)).unwrap().kind {
        Kind::Ability(a) => a,
        _ => panic!("not an ability object"),
    }
}

/// All live spawned objects of the given kind.
fn spawned(w: &World, f: impl Fn(&Behaviour) -> bool) -> Vec<ObjId> {
    w.objects
        .ids()
        .filter(|id| {
            let o = w.objects.get(*id).unwrap();
            !o.destroyed && matches!(&o.kind, Kind::Behaviour(b) if f(b))
        })
        .collect()
}

fn arrows(w: &World) -> Vec<ObjId> {
    spawned(w, |b| matches!(b, Behaviour::Bow(_)))
}

/// Walk left and jump every half second (off Level1's left platforms).
fn left_jumping(t: i64) -> PlayerInput {
    PlayerInput::new(LEFT, if t % 30 == 0 { BTN_JUMP } else { 0 })
}

fn f(x: Fix) -> f64 {
    x.to_f64()
}

// -- Bow ------------------------------------------------------------------------------------

/// Tapping the button fires as soon as the transform animation reaches its
/// third frame (10 ticks after the cast) with no charge: speed 15.
#[test]
fn bow_tap_fires_uncharged_after_ten_ticks() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = duo(&l, [BOW, GUST, DASH], [BOW, GUST, DASH], 1);
    step1(&mut w, hold(0, UP));
    let cast = w.time.simulation_ticks;
    assert!(in_ability(&w, 1));
    let mut fired = None;
    for _ in 0..40 {
        if fired.is_none() && !arrows(&w).is_empty() {
            fired = Some(w.time.simulation_ticks);
        }
        step1(&mut w, PlayerInput::new(UP, 0));
    }
    // The arrow is spawned in the BowTransform update of tick cast + 10.
    assert_eq!(fired, Some(cast + 10));
    let a = arrows(&w);
    assert!(!a.is_empty());
}

/// Holding for the whole load animation charges to frame 4: start speed
/// 5 · 15 = 75 and hurt-owner delay 5 / 75.
#[test]
fn bow_full_charge_speed_and_delay() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = duo(&l, [BOW, GUST, DASH], [BOW, GUST, DASH], 1);
    for _ in 0..80 {
        step1(&mut w, hold(0, UP));
    }
    assert!(in_ability(&w, 1));
    assert!(arrows(&w).is_empty(), "no shot while held");
    step1(&mut w, PlayerInput::new(UP, 0));
    let a = arrows(&w);
    assert_eq!(a.len(), 1);
    let o = w.objects.get(a[0]).unwrap();
    let v0 = o.collider.unwrap().body.unwrap().start_velocity;
    assert!(
        (f(v0.magnitude()) - 75.0).abs() < 1e-3,
        "speed {}",
        f(v0.magnitude())
    );
    if let Kind::Behaviour(b) = &o.kind
        && let Behaviour::Bow(bopl_sim::abilities::bow::Spawned::Arrow(ar)) = b.as_ref()
    {
        assert_eq!(ar.owner, 1);
        assert!((f(ar.hurt_owner_after) - 5.0 / 75.0).abs() < 1e-6);
    } else {
        panic!("arrow payload");
    }
}

/// A partly charged lob over the middle platform kills player 2 (killer 1,
/// cause Other); the grounded shot recoils into the ground, so the shooter
/// is back on its platform at once.
#[test]
fn bow_arrow_kills_other_player() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = duo(&l, [BOW, GUST, DASH], [BOW, GUST, DASH], 1);
    for _ in 0..45 {
        step1(&mut w, hold(0, 54));
    }
    idle(&mut w, 1);
    assert!(!in_ability(&w, 1));
    assert!(bopl(&w, 1).unwrap().physics.is_grounded);
    idle(&mut w, 300);
    let p2 = w.player(2).unwrap();
    assert!(!p2.is_alive(), "player 2 survived");
    assert_eq!(p2.cause_of_death, CauseOfDeath::Other);
    assert_eq!(w.player(1).unwrap().kills, 1);
    assert!(w.player(1).unwrap().is_alive());
}

/// An arrow shot straight up falls back on its owner after the hurt-owner
/// delay and kills it (cause Accident).
#[test]
fn bow_arrow_kills_owner_after_delay() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = duo(&l, [BOW, GUST, DASH], [BOW, GUST, DASH], 1);
    for _ in 0..80 {
        step1(&mut w, hold(0, UP));
    }
    idle(&mut w, 400);
    let p1 = w.player(1).unwrap();
    assert!(!p1.is_alive());
    assert_eq!(p1.cause_of_death, CauseOfDeath::Accident);
    assert!(w.player(2).unwrap().is_alive());
}

// -- Gust -------------------------------------------------------------------------------------

/// The gust: 6 + 11 animator updates, a shockwave that pushes a teammate
/// once (grounded: × 0.5) and platforms for 16 pushes, then destroys
/// itself; the caster returns airborne.
#[test]
fn gust_pushes_teammate_and_platforms() {
    let Some(l) = loaded() else {
        return;
    };
    // Players 1 and 2 are teammates (player 2 spawns 2.63 to the left);
    // player 3 keeps the round going.
    let mut w = world(
        &l,
        6,
        &[
            setup(1, 0, [GUST, BOW, DASH]),
            setup(2, 0, [GUST, BOW, DASH]),
            setup(3, 1, [GUST, BOW, DASH]),
        ],
        1,
    );
    idle(&mut w, 600);
    let p2_before = bopl(&w, 2).unwrap().body.external_velocity;
    assert_eq!(p2_before, Vec2::ZERO);
    let plats: Vec<(ObjId, Vec2)> = w
        .platforms
        .iter()
        .map(|p| (*p, w.objects.get(*p).unwrap().tf.position))
        .collect();
    step1(&mut w, hold(0, 0));
    let cast = w.time.simulation_ticks;
    let mut wave_seen = None;
    let mut wave_gone = None;
    let mut exit = None;
    let mut pushed = None;
    for _ in 0..40 {
        idle(&mut w, 1);
        let t = w.time.simulation_ticks;
        let waves = spawned(&w, |b| {
            matches!(
                b,
                Behaviour::Mine(bopl_sim::abilities::mine::Spawned::Shockwave(_))
            )
        });
        if wave_seen.is_none() && !waves.is_empty() {
            wave_seen = Some(t);
        }
        if wave_seen.is_some() && wave_gone.is_none() && waves.is_empty() {
            wave_gone = Some(t);
        }
        if exit.is_none() && !in_ability(&w, 1) {
            exit = Some(t);
            assert!(
                !bopl(&w, 1).unwrap().physics.is_grounded,
                "returns airborne"
            );
        }
        if pushed.is_none() && bopl(&w, 2).unwrap().body.external_velocity != Vec2::ZERO {
            pushed = Some(t);
        }
    }
    // Cast on the 7th animator update (tick cast + 6), exit on the 11th
    // update of `exit` (cast + 17).
    assert_eq!(wave_seen, Some(cast + 6));
    assert_eq!(exit, Some(cast + 17));
    // Init (next tick) + 15 updates push; the 16th update destroys it.
    assert_eq!(wave_gone, Some(cast + 6 + 16));
    assert_eq!(
        pushed,
        Some(cast + 7),
        "teammate pushed on the shockwave's Init"
    );
    let moved = plats
        .iter()
        .filter(|(p, p0)| w.objects.get(*p).is_some_and(|o| o.tf.position != *p0))
        .count();
    assert!(moved > 0, "no platform pushed");
}

// -- Time stop ----------------------------------------------------------------------------------

/// Channel 9 s, cast on the 17th animator update, then the sphere stops
/// time for exactly 690 ticks (its 691st update restores it); the caster's
/// team is protected, other players freeze (inputs, animators, cooldowns).
#[test]
fn time_stop_lasts_691_sphere_updates() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = duo(&l, [TIME_STOP, GUST, DASH], [GUST, BOW, DASH], 1);
    // Cooldown 3 + 1.5 s start offset have passed at tick 240 + 30.
    idle(&mut w, 40);
    step1(&mut w, hold(0, 0));
    assert!(in_ability(&w, 1));
    let mut stopped_ticks = 0;
    let mut first_stop = None;
    let mut resumed = None;
    let mut sphere_seen = None;
    for _ in 0..1400 {
        step1(&mut w, hold(0, 0));
        let t = w.time.simulation_ticks;
        if sphere_seen.is_none() && !spawned(&w, |b| matches!(b, Behaviour::TimeStop(_))).is_empty()
        {
            sphere_seen = Some(t);
        }
        if w.is_time_stopped() {
            stopped_ticks += 1;
            if first_stop.is_none() {
                first_stop = Some(t);
                assert!(w.player(1).unwrap().is_protected_from_time_stop);
                assert!(!w.player(2).unwrap().is_protected_from_time_stop);
            }
        } else if first_stop.is_some() && resumed.is_none() {
            resumed = Some(t);
        }
    }
    let s = sphere_seen.expect("sphere spawned");
    assert_eq!(
        first_stop,
        Some(s + 1),
        "stop starts with the sphere's first update"
    );
    assert_eq!(stopped_ticks, 690);
    assert_eq!(resumed, Some(s + 691));
    assert!(!w.player(1).unwrap().is_protected_from_time_stop);
}

/// Releasing the button before 9 s cancels: no sphere, the ability exits.
#[test]
fn time_stop_release_cancels() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = duo(&l, [TIME_STOP, GUST, DASH], [GUST, BOW, DASH], 1);
    idle(&mut w, 40);
    for _ in 0..120 {
        step1(&mut w, hold(0, 0));
    }
    assert!(in_ability(&w, 1));
    step1(&mut w, PlayerInput::default());
    assert!(!in_ability(&w, 1));
    idle(&mut w, 30);
    assert!(spawned(&w, |b| matches!(b, Behaviour::TimeStop(_))).is_empty());
    assert!(!w.is_time_stopped());
}

/// While stopped, frozen players do not move; their cooldowns stand still.
#[test]
fn time_stop_freezes_others() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = duo(&l, [TIME_STOP, GUST, DASH], [GUST, BOW, DASH], 1);
    idle(&mut w, 40);
    while !w.is_time_stopped() && w.time.simulation_ticks < 2000 {
        let mut v = vec![hold(0, 0), PlayerInput::default()];
        if w.time.simulation_ticks > 900 {
            v[1] = PlayerInput::new(LEFT, BTN_JUMP);
        }
        w.step(&v);
    }
    assert!(w.is_time_stopped());
    let p2 = pos(&w, 2);
    let t2 = bopl(&w, 2).unwrap().ctrl.cooldown_timers.clone();
    for _ in 0..200 {
        w.step(&[PlayerInput::new(RIGHT, 0), PlayerInput::new(LEFT, BTN_JUMP)]);
    }
    assert_eq!(pos(&w, 2), p2);
    assert_eq!(bopl(&w, 2).unwrap().ctrl.cooldown_timers, t2);
}

// -- Invisibility ---------------------------------------------------------------------------------

/// Invisible for 5 s of owner time (301 ticks), cooldown 10000 meanwhile,
/// then the 6 s cooldown starts from 0 when it ends.
#[test]
fn invisibility_five_seconds_then_cooldown() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = duo(&l, [INVISIBILITY, GUST, DASH], [GUST, BOW, DASH], 1);
    step1(&mut w, hold(0, 0));
    let cast = w.time.simulation_ticks;
    assert!(w.player(1).unwrap().is_invisible);
    assert!(!in_ability(&w, 1), "instant ability");
    assert_eq!(ability(&w, 1, 0).fw.cooldown, Fix::from_int(10_000));
    let mut visible = None;
    for _ in 0..400 {
        step1(&mut w, hold(0, 0));
        if visible.is_none() && !w.player(1).unwrap().is_invisible {
            visible = Some(w.time.simulation_ticks);
            assert_eq!(ability(&w, 1, 0).fw.cooldown, Fix::from_int(6));
            assert_eq!(bopl(&w, 1).unwrap().ctrl.cooldown_timers[0], Fix::ZERO);
        }
    }
    assert_eq!(visible, Some(cast + 301));
}

// -- Revival ----------------------------------------------------------------------------------------

/// A planted marker turns the next death into a clone respawn at the
/// marker; the death after that is final.
#[test]
fn revival_marker_respawns_a_clone() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = duo(&l, [REVIVAL, GUST, DASH], [GUST, BOW, DASH], 1);
    // Revival cooldown 10 + 1.5 s.
    idle(&mut w, 300);
    let start = pos(&w, 1);
    step1(&mut w, hold(0, 0));
    assert!(in_ability(&w, 1));
    idle(&mut w, 30);
    assert!(!in_ability(&w, 1));
    let markers = w.player(1).unwrap().respawn_positions.clone();
    assert_eq!(markers.len(), 1);
    let m = w.objects.get(markers[0]).unwrap().tf.position;
    assert!(
        (f(m.y) - (f(start.y) + 1.21)).abs() < 0.1,
        "marker {m:?} start {start:?}"
    );
    // Walk left (jumping) into the water.
    let mut died = None;
    for _ in 0..600 {
        let t = w.time.simulation_ticks;
        step1(&mut w, left_jumping(t));
        if died.is_none() && w.player(1).unwrap().deaths == 1 {
            died = Some(w.time.simulation_ticks);
            break;
        }
    }
    assert!(died.is_some(), "did not die");
    let p = w.player(1).unwrap();
    assert!(p.is_alive(), "a clone keeps the player alive");
    assert!(p.respawn_positions.is_empty());
    // Wisp (61 updates) + flight + spawn animation (106).
    let mut respawned = None;
    for _ in 0..400 {
        idle(&mut w, 1);
        if respawned.is_none() && bopl(&w, 1).is_some_and(|b| b.ctrl.has_spawned) {
            respawned = Some(w.time.simulation_ticks);
        }
    }
    let r = respawned.expect("clone respawned");
    assert!(r > died.unwrap() + 61 + 106, "respawn too early");
    let at = pos(&w, 1);
    assert!(
        (f(at.x) - f(m.x)).abs() < 3.0,
        "clone at {at:?}, marker {m:?}"
    );
    // Die again: final.
    for _ in 0..900 {
        let t = w.time.simulation_ticks;
        step1(&mut w, left_jumping(t));
    }
    let p = w.player(1).unwrap();
    assert_eq!(p.deaths, 2);
    assert!(!p.is_alive());
}

// -- Black hole --------------------------------------------------------------------------------------

/// The hole appears 6 in front of and 3 above the caster, the slot is spent
/// for the round, platforms are pulled, and a plain player touching it is
/// eaten (killed with the owner as killer).
#[test]
fn black_hole_once_per_round_pulls_and_eats() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = duo(&l, [BLACK_HOLE, GUST, DASH], [GUST, BOW, DASH], 1);
    idle(&mut w, 80);
    let p1 = pos(&w, 1);
    step1(&mut w, hold(0, 0));
    assert!(in_ability(&w, 1));
    let mut hole = None;
    for _ in 0..40 {
        idle(&mut w, 1);
        if hole.is_none() {
            hole = spawned(&w, |b| matches!(b, Behaviour::BlackHole(_)))
                .first()
                .copied();
        }
    }
    let hole = hole.expect("hole spawned");
    assert!(!in_ability(&w, 1));
    assert!(!ability(&w, 1, 0).fw.is_castable, "once per round");
    let hp = w.objects.get(hole).unwrap().tf.position;
    let d = hp - p1;
    assert!(
        (f(d.x) - 6.0).abs() < 1.0 && (f(d.y) - 3.0).abs() < 1.0,
        "offset {d:?}"
    );
    // The slot is never castable again this round.
    assert!(!bopl_sim::abilities::is_castable(&w, slot_obj(&w, 1, 0), 1));
    // Platforms far away are pulled.
    let far: Vec<(ObjId, Vec2)> = w
        .platforms
        .iter()
        .filter_map(|p| w.objects.get(*p).map(|o| (*p, o.tf.position)))
        .filter(|(_, q)| f((*q - hp).magnitude()) > 10.0)
        .collect();
    assert!(!far.is_empty());
    let v0: Vec<Vec2> = far
        .iter()
        .map(|(p, _)| w.physics.body(*p).map_or(Vec2::ZERO, |b| b.velocity))
        .collect();
    idle(&mut w, 5);
    let changed = far
        .iter()
        .zip(&v0)
        .filter(|((p, _), v)| w.physics.body(*p).is_some_and(|b| b.velocity != **v))
        .count();
    assert!(changed > 0, "no far platform pulled");
    // A second hole put on player 2 eats it (ignoring invulnerability).
    let prefab = bopl_sim::abilities::prefab::prefab_index(&w, "sharedassets0.assets:1414")
        .expect("BlackHole2 prefab");
    let at = pos(&w, 2);
    if let Some(Kind::Bopl(b)) = w
        .player(2)
        .and_then(|p| p.bopl)
        .and_then(|b| w.objects.get_mut(b))
        .map(|o| &mut o.kind)
    {
        b.collision.set_invulnerable(true, 1);
    }
    bopl_sim::abilities::black_hole::spawn_hole(&mut w, prefab, at, 1);
    idle(&mut w, 3);
    let p2 = w.player(2).unwrap();
    assert!(!p2.is_alive(), "eaten");
    assert_eq!(w.player(1).unwrap().kills, 1);
}

// -- Platform ------------------------------------------------------------------------------------------

fn resizable_platforms(w: &World) -> Vec<ObjId> {
    w.objects
        .ids()
        .filter(|id| {
            matches!(&w.objects.get(*id).unwrap().kind, Kind::Platform(p) if p.resize.is_some())
                && !w.objects.get(*id).unwrap().destroyed
        })
        .collect()
}

/// The platform appears on the 25th animator update, grows while the stick
/// pushes the cursor, the player pops out on top; a second cast removes
/// the first platform (0.5 s removal animation).
#[test]
fn platform_create_size_exit_and_replace() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = duo(&l, [PLATFORM, GUST, DASH], [GUST, BOW, DASH], 1);
    idle(&mut w, 200);
    step1(&mut w, hold(0, 0));
    let cast = w.time.simulation_ticks;
    let mut created = None;
    let mut exit = None;
    for i in 0..400 {
        // Hold and push the cursor up for 2 s, then release.
        let inp = if i < 120 {
            hold(0, UP)
        } else {
            PlayerInput::default()
        };
        step1(&mut w, inp);
        let t = w.time.simulation_ticks;
        if created.is_none() && !resizable_platforms(&w).is_empty() {
            created = Some(t);
        }
        if exit.is_none() && !in_ability(&w, 1) {
            exit = Some(t);
        }
    }
    assert_eq!(created, Some(cast + 24));
    assert!(exit.is_some(), "never exited");
    let plats = resizable_platforms(&w);
    assert_eq!(plats.len(), 1);
    let rr = w.physics.rr(plats[0]).unwrap();
    // The cursor pushed up: the height grew.
    let (wx, wy) = (f(Fix::ONE / rr.b.ie.x), f(Fix::ONE / rr.b.ie.y));
    assert!(wy > 5.0, "platform did not grow: half extents {wx} {wy}");
    assert!(
        (f(rr.radius) - 3.0).abs() < 1e-6,
        "bevel radius from EnterScaleAnimation"
    );
    assert!(bopl(&w, 1).is_some());
    // Second cast: the first platform is removed.
    idle(&mut w, 400);
    step1(&mut w, hold(0, 0));
    for _ in 0..100 {
        step1(&mut w, PlayerInput::default());
    }
    let now = resizable_platforms(&w);
    assert_eq!(now.len(), 1);
    assert_ne!(now[0], plats[0]);
}

// -- Push --------------------------------------------------------------------------------------------------

/// Push: the grabbed platform is pushed along the stick and flagged as
/// controlled while held; releasing clears the flag.
#[test]
fn push_moves_the_platform() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = duo(&l, [PUSH, GUST, DASH], [GUST, BOW, DASH], 1);
    let g = bopl(&w, 1)
        .unwrap()
        .physics
        .attached_ground
        .expect("grounded");
    let g0 = w.objects.get(g).unwrap().tf.position;
    step1(&mut w, hold(0, 0));
    assert!(in_ability(&w, 1));
    for _ in 0..60 {
        step1(&mut w, hold(0, RIGHT));
    }
    let controlled = matches!(&w.objects.get(g).unwrap().kind,
        Kind::Platform(p) if p.spring.as_ref().is_some_and(|s| s.is_being_controlled));
    assert!(controlled);
    let g1 = w.objects.get(g).unwrap().tf.position;
    assert!(f(g1.x - g0.x) > 0.5, "platform moved {:?} -> {:?}", g0, g1);
    step1(&mut w, PlayerInput::default());
    assert!(!in_ability(&w, 1));
    let controlled = matches!(&w.objects.get(g).unwrap().kind,
        Kind::Platform(p) if p.spring.as_ref().is_some_and(|s| s.is_being_controlled));
    assert!(!controlled);
}

// -- Random ------------------------------------------------------------------------------------------------

/// Random slots are resolved to concrete abilities at spawn.
#[test]
fn random_resolves_at_spawn() {
    let Some(l) = loaded() else {
        return;
    };
    let w = duo(&l, [RANDOM, RANDOM, RANDOM], [GUST, BOW, DASH], 1);
    let p = w.player(1).unwrap();
    assert_eq!(p.current_abilities.len(), 3);
    for (k, &a) in p.current_abilities.iter().enumerate() {
        assert!(a >= 2, "slot {k} unresolved: {a}");
        assert_eq!(ability(&w, 1, k).def, a);
        assert!(!matches!(ability(&w, 1, k).state, AbilityState::Random(_)));
    }
}

// -- Determinism ----------------------------------------------------------------------------------------------

/// A deterministic pseudo-random input script per player (long holds so
/// channels, charges and sizes happen).
fn inputs(n: usize, t: u64, seed: u64) -> Vec<PlayerInput> {
    (0..n)
        .map(|i| {
            let mut x = seed
                ^ (t / 11).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ (i as u64 + 1).wrapping_mul(0xD1B5_4A32_D192_ED03);
            x ^= x >> 33;
            x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
            x ^= x >> 29;
            let angle = if x & 3 == 0 { 0 } else { (x >> 8) as u8 };
            let mut b = 0;
            if (x >> 20) & 15 == 0 {
                b |= BTN_JUMP;
            }
            // Every fourth player holds its buttons for long windows (time
            // stop channels, full bow charges, big platforms); the others
            // tap and hold briefly.
            let long = i % 4 == 2;
            let win = if long { 700 } else { 97 };
            let hold = (t / win + i as u64) % 5;
            let on = |bit: u64| long || (x >> bit) & 3 != 0;
            if hold == 1 && on(24) {
                b |= BTN_ABILITY0;
            }
            if hold == 2 && on(26) {
                b |= BTN_ABILITY1;
            }
            if hold == 3 && on(28) {
                b |= BTN_ABILITY2;
            }
            PlayerInput::new(angle, b)
        })
        .collect()
}

fn twelve_g4() -> Vec<PlayerSetup> {
    let g4 = [
        BOW,
        GUST,
        TIME_STOP,
        PLATFORM,
        REVIVAL,
        BLACK_HOLE,
        INVISIBILITY,
        PUSH,
        RANDOM,
    ];
    (0..12)
        .map(|i| PlayerSetup {
            id: i + 1,
            team: i % 6,
            color: i,
            abilities: vec![
                g4[i as usize % g4.len()],
                g4[(i as usize + 3) % g4.len()],
                g4[(i as usize + 5) % g4.len()],
            ],
            uses_keyboard_and_mouse: i == 3,
        })
        .collect()
}

fn run(w: &mut World, from: u64, to: u64, seed: u64) {
    let n = w.players.len();
    for t in from..to {
        w.step(&inputs(n, t, seed));
    }
}

/// 12 players with the G4 abilities: identical hashes for identical inputs
/// over several rounds and level loads, and no faults.
#[test]
fn determinism_twelve_players_g4() {
    let Some(l) = loaded() else {
        return;
    };
    let levels = [6usize, 18, 28, 126];
    let lv: Vec<_> = levels
        .iter()
        .map(|&i| bopl_content::load_level(&l, i).unwrap())
        .collect();
    let mk = || {
        let mut w = World::new(
            Arc::clone(&l.content),
            &twelve_g4(),
            MatchSettings {
                number_of_abilities: 3,
                seed: 11,
            },
        );
        bopl_sim::session::load_level(&mut w, lv[0].clone());
        w
    };
    let (mut a, mut b) = (mk(), mk());
    let mut next = 0;
    let mut rounds = 0;
    let mut stopped = 0;
    let mut cues = std::collections::BTreeMap::new();
    for chunk in 0..120u64 {
        for t in chunk * 100..chunk * 100 + 100 {
            let inp = inputs(12, t, 5);
            a.step(&inp);
            b.step(&inp);
            if a.is_time_stopped() {
                stopped += 1;
            }
            for e in &a.events {
                if let bopl_sim::view::Cue::Sound(n) = e.cue {
                    *cues.entry(n).or_insert(0) += 1;
                }
            }
        }
        assert_eq!(a.state_hash(), b.state_hash(), "tick {}", (chunk + 1) * 100);
        if a.round.winner_screen && a.time.sim_time_since_level_loaded > Fix::from_int(20) {
            rounds += 1;
            next = (next + 1) % lv.len();
            bopl_sim::session::load_level(&mut a, lv[next].clone());
            bopl_sim::session::load_level(&mut b, lv[next].clone());
        }
    }
    let casts: i32 = a.players.iter().map(|p| p.deaths).sum();
    eprintln!(
        "g4 determinism: {rounds} rounds, {casts} deaths, {stopped} stopped ticks, fault {:?}",
        a.fault
    );
    eprintln!("cues: {cues:?}");
    assert!(rounds >= 2);
    assert!(a.fault.is_none(), "fault {:?}", a.fault);
    // Every G4 ability was cast at least once.
    for cue in [
        "arrowShoot",
        "airWave",
        "timeStop",
        "transform2",
        "reviveCast",
        "spawnBlackHole",
        "invisibility",
    ] {
        assert!(cues.get(cue).copied().unwrap_or(0) > 0, "never cast: {cue}");
    }
}

/// Rollback with the G4 abilities: snapshot, mispredict, restore, replay.
#[test]
fn rollback_g4_equals_straight_run() {
    let Some(l) = loaded() else {
        return;
    };
    let mut straight = world(&l, 6, &twelve_g4(), 3);
    run(&mut straight, 0, 3000, 1);
    let mut rb = world(&l, 6, &twelve_g4(), 3);
    run(&mut rb, 0, 1200, 1);
    for k in 0..20u64 {
        let base = 1200 + k * 90;
        let snap = rb.clone();
        run(&mut rb, base, base + 8, 999 + k);
        rb = snap;
        run(&mut rb, base, base + 90, 1);
    }
    assert_eq!(rb.state_hash(), straight.state_hash());
    assert_eq!(rb.checksum(), straight.checksum());
    assert_eq!(rb.view(), straight.view());
}

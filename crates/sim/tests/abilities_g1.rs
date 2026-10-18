//! Scenario tests of the G1 abilities (Blink gun, Growth ray, Shrink ray,
//! Duplicator, Magnet gun, Beam) on real imported levels. Skipped without
//! an import (`$BOPL_ROOT/cache/import`, the dev dir or the user data dir).

use std::sync::Arc;

use bopl_content::Loaded;
use bopl_sim::fix::Fix;
use bopl_sim::input::{BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2, BTN_JUMP, PlayerInput};
use bopl_sim::object::ObjId;
use bopl_sim::player::{CauseOfDeath, PlayerSetup};
use bopl_sim::vec2::Vec2;
use bopl_sim::view::Cue;
use bopl_sim::world::{Kind, MatchSettings, World};

const BLINK: usize = 6;
const GROWTH: usize = 8;
const SHRINK: usize = 17;
const BEAM: usize = 28;
const DUPLICATOR: usize = 29;
const MAGNET: usize = 30;

fn loaded() -> Option<Loaded> {
    let mut dirs = Vec::new();
    if let Some(r) = std::env::var_os("BOPL_ROOT") {
        dirs.push(std::path::PathBuf::from(r).join("cache/import"));
    }
    dirs.push(bopl_content::default_import_dir());
    for d in dirs {
        if d.join("index.json").exists() {
            return Some(bopl_content::load(d).expect("import loads"));
        }
    }
    eprintln!("no import; skipping");
    None
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

fn setup(id: i32, team: i32, abilities: Vec<usize>) -> PlayerSetup {
    PlayerSetup {
        id,
        team,
        color: id - 1,
        abilities,
        uses_keyboard_and_mouse: false,
    }
}

fn bopl(w: &World, id: i32) -> Option<ObjId> {
    w.player(id)?.bopl.filter(|b| w.objects.exists(*b))
}

fn bopl_pos(w: &World, id: i32) -> Option<Vec2> {
    Some(w.objects.get(bopl(w, id)?)?.tf.position)
}

fn spawned(w: &World, id: i32) -> bool {
    bopl(w, id)
        .and_then(|b| w.objects.get(b))
        .is_some_and(|o| matches!(&o.kind, Kind::Bopl(b) if b.ctrl.has_spawned))
}

fn grounded(w: &World, id: i32) -> bool {
    bopl(w, id)
        .and_then(|b| w.objects.get(b))
        .is_some_and(|o| matches!(&o.kind, Kind::Bopl(b) if b.physics.is_grounded))
}

fn in_ability(w: &World, id: i32) -> bool {
    bopl(w, id)
        .and_then(|b| w.objects.get(b))
        .is_some_and(|o| matches!(&o.kind, Kind::Bopl(b) if b.ctrl.is_in_ability))
}

/// The ability object of a slot.
fn ability_obj(w: &World, id: i32, slot: usize) -> Option<ObjId> {
    let b = bopl(w, id)?;
    match &w.objects.get(b)?.kind {
        Kind::Bopl(b) => b.ctrl.abilities.get(slot).copied(),
        _ => None,
    }
}

fn cooldown_of(w: &World, a: ObjId) -> Fix {
    match w.objects.get(a).map(|o| &o.kind) {
        Some(Kind::Ability(x)) => x.fw.cooldown,
        _ => Fix::ZERO,
    }
}

fn timer(w: &World, id: i32, slot: usize) -> Fix {
    let b = bopl(w, id).unwrap();
    match &w.objects.get(b).unwrap().kind {
        Kind::Bopl(b) => b.ctrl.cooldown_timers[slot],
        _ => Fix::ZERO,
    }
}

fn idle(n: usize) -> Vec<PlayerInput> {
    vec![PlayerInput::default(); n]
}

/// Steps with idle input until every player has spawned and is grounded.
fn settle(w: &mut World) {
    let n = w.players.len();
    for _ in 0..600 {
        w.step(&idle(n));
        if (1..=n as i32).all(|i| spawned(w, i) && grounded(w, i)) {
            // Wait for the cooldowns (−1.5 + 2.5..5.5 s).
            for _ in 0..(8 * 60) {
                w.step(&idle(n));
            }
            return;
        }
    }
    panic!("players did not settle");
}

/// Angle byte for a direction (counter-clockwise from +x, 1 = +x).
fn dir_byte(v: Vec2) -> u8 {
    // `angle_byte_from_vector` expects a unit vector.
    bopl_sim::input::angle_byte_from_vector(v.normalized_safe())
}

fn events(w: &World) -> Vec<Cue> {
    w.events.iter().map(|e| e.cue.clone()).collect()
}

/// Holds the slot button for `hold` ticks aiming at `aim` (other players
/// idle), releases and runs `after` more ticks; returns all cues.
fn cast(w: &mut World, id: i32, slot: usize, aim: Vec2, hold: usize, after: usize) -> Vec<Cue> {
    let n = w.players.len();
    let btn = [BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2][slot];
    let mut cues = Vec::new();
    for t in 0..hold + after {
        let mut inp = idle(n);
        let b = if t < hold { btn } else { 0 };
        inp[(id - 1) as usize] = PlayerInput::new(dir_byte(aim), b);
        w.step(&inp);
        cues.extend(events(w));
    }
    cues
}

fn has_sound(c: &[Cue], s: &str) -> bool {
    c.iter().any(|e| matches!(e, Cue::Sound(x) if *x == s))
}

fn died(c: &[Cue], id: i32) -> Option<(i32, CauseOfDeath)> {
    c.iter().find_map(|e| match e {
        Cue::PlayerDied {
            id: i,
            killer,
            cause,
        } if *i == id => Some((*killer, *cause)),
        _ => None,
    })
}

// ---------------------------------------------------------------------------
// Blink gun
// ---------------------------------------------------------------------------

/// Two players face each other on Level1; player 1 blinks player 2: the
/// victim vanishes for 1.2 s and reappears, the gun's cooldown becomes 5.
#[test]
fn blink_gun_blinks_player() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(
        &l,
        6,
        &[setup(1, 0, vec![BLINK]), setup(2, 1, vec![BLINK])],
        3,
    );
    settle(&mut w);
    let a = ability_obj(&w, 1, 0).unwrap();
    assert_eq!(cooldown_of(&w, a), Fix::from_f32(2.5));
    let p1 = bopl_pos(&w, 1).unwrap();
    let p2 = bopl_pos(&w, 2).unwrap();
    let aim = p2 - p1;
    let n = w.players.len();
    let mut cues = Vec::new();
    // Press (cast tick) and release on the next tick: the gun fires on the
    // first update that sees the button up.
    let mut inp = idle(n);
    inp[0] = PlayerInput::new(dir_byte(aim), BTN_ABILITY0);
    w.step(&inp);
    cues.extend(events(&w));
    assert!(in_ability(&w, 1), "player 1 is a gun now");
    assert!(has_sound(&cues, "prepareToQuantum"));
    assert_eq!(timer(&w, 1, 0), Fix::ZERO);
    let mut inp = idle(n);
    inp[0] = PlayerInput::new(dir_byte(aim), 0);
    w.step(&inp);
    cues.extend(events(&w));
    assert!(has_sound(&cues, "laserShoot"), "fired");
    assert!(has_sound(&cues, "teleport"), "victim blinked");
    let b2 = bopl(&w, 2).unwrap();
    assert!(!w.objects.get(b2).unwrap().active, "victim hidden");
    assert_eq!(
        cooldown_of(&w, a),
        Fix::from_int(5),
        "player hit → cooldown 5"
    );
    // The victim is back after the 1.2 s lifespan.
    let mut back = None;
    for t in 0..120 {
        w.step(&idle(n));
        cues.extend(events(&w));
        if back.is_none() && w.objects.get(b2).unwrap().active {
            back = Some(t);
        }
    }
    let back = back.expect("victim reappears");
    assert!((70..76).contains(&back), "back after {back} ticks");
    assert!(has_sound(&cues, "teleport2"));
    assert!(!in_ability(&w, 1), "gun exited");
    assert!(w.fault.is_none());
}

// ---------------------------------------------------------------------------
// Magnet gun
// ---------------------------------------------------------------------------

fn bopl_state(w: &World, id: i32) -> Option<bopl_sim::bopl::State> {
    match &w.objects.get(bopl(w, id)?)?.kind {
        Kind::Bopl(b) => Some(b.ctrl.state),
        _ => None,
    }
}

/// Player 1 pulls its teammate (no team check; teammates spawn 2.63 apart)
/// in, grabs it (forced ragdoll, carried) and throws it on release.
#[test]
fn magnet_gun_grabs_and_throws_player() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(
        &l,
        6,
        &[
            setup(1, 0, vec![MAGNET]),
            setup(2, 0, vec![MAGNET]),
            setup(3, 1, vec![MAGNET]),
        ],
        5,
    );
    settle(&mut w);
    let p1 = bopl_pos(&w, 1).unwrap();
    let p2 = bopl_pos(&w, 2).unwrap();
    let n = w.players.len();
    let mut cues = Vec::new();
    let mut grabbed = None;
    for t in 0..400 {
        let mut inp = idle(n);
        let aim = bopl_pos(&w, 2).map_or(p2 - p1, |p| p - p1);
        inp[0] = PlayerInput::new(dir_byte(aim), BTN_ABILITY0);
        w.step(&inp);
        cues.extend(events(&w));
        if has_sound(&cues, "magnetPickup") {
            grabbed = Some(t);
            break;
        }
    }
    let grabbed = grabbed.expect("magnet picks the player up");
    // The enter animation (28 ticks) ends on its 29th advance; the first
    // advance is in the cast tick, so pulling starts on tick 28.
    assert_eq!(grabbed, 28, "grabbed right after the enter animation");
    assert!(has_sound(&cues, "magnetEnter"));
    // Carried for a while, ragdolled.
    for _ in 0..20 {
        let mut inp = idle(n);
        inp[0] = PlayerInput::new(dir_byte(Vec2::UP), BTN_ABILITY0);
        w.step(&inp);
    }
    assert_eq!(bopl_state(&w, 2), Some(bopl_sim::bopl::State::Ragdoll));
    // Release: thrown upwards.
    let mut inp = idle(n);
    inp[0] = PlayerInput::new(dir_byte(Vec2::UP), 0);
    w.step(&inp);
    let c = events(&w);
    assert!(has_sound(&c, "magnetShoot"), "{c:?}");
    let b2 = bopl(&w, 2).unwrap();
    let v = match &w.objects.get(b2).unwrap().kind {
        Kind::Bopl(b) => b.body.self_imposed_velocity,
        _ => unreachable!(),
    };
    assert!(v.y > Fix::from_int(50), "thrown up: {v:?}");
    assert_eq!(bopl_state(&w, 2), Some(bopl_sim::bopl::State::Ragdoll));
    assert!(w.fault.is_none());
}

// ---------------------------------------------------------------------------
// Beam
// ---------------------------------------------------------------------------

/// Player 1 beams player 2 across Level1: killed with cause Beamed and
/// killer 1; the beamer survives its own beam.
#[test]
fn beam_kills_enemy() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(
        &l,
        6,
        &[setup(1, 0, vec![BEAM]), setup(2, 1, vec![BEAM])],
        7,
    );
    settle(&mut w);
    let p1 = bopl_pos(&w, 1).unwrap();
    let p2 = bopl_pos(&w, 2).unwrap();
    let n = w.players.len();
    let mut cues = Vec::new();
    let mut fired = None;
    let mut dead = None;
    for t in 0..400 {
        let mut inp = idle(n);
        inp[0] = PlayerInput::new(dir_byte(p2 - p1), BTN_ABILITY0);
        w.step(&inp);
        let c = events(&w);
        if fired.is_none() && has_sound(&c, "beamFire") {
            fired = Some(t);
        }
        if let Some(d) = died(&c, 2) {
            dead = Some((t, d));
        }
        cues.extend(c);
        if dead.is_some() {
            break;
        }
    }
    let fired = fired.expect("beam starts");
    // The animator advances in the cast tick (0): enterFrame1 (4 ticks)
    // ends on its 5th advance (tick 4), `enter` (72) begins there and ends
    // on its 73rd advance (tick 77).
    assert_eq!(fired, 77, "beam start tick");
    let (t, (killer, cause)) = dead.expect("victim beamed");
    assert_eq!((killer, cause), (1, CauseOfDeath::Beamed));
    // The box is created on the first loop tick and only gets broad-phase
    // pairs one tick later.
    assert_eq!(t, fired + 1, "kill one tick after the box appears");
    assert!(
        w.player(1).unwrap().is_alive(),
        "beamer survives its own beam"
    );
    assert!(
        cues.iter()
            .any(|c| matches!(c, Cue::SoundPrefix("beamSparks")))
    );
    assert!(w.fault.is_none());
}

// ---------------------------------------------------------------------------
// Duplicator
// ---------------------------------------------------------------------------

fn bopls_of(w: &World, id: i32) -> Vec<ObjId> {
    w.objects
        .ids()
        .filter(|o| {
            matches!(&w.objects.get(*o).unwrap().kind, Kind::Bopl(b) if b.ctrl.player_number == id)
        })
        .collect()
}

/// Shooting a player spawns an ability-less clone that pops into the air
/// and counts as an extra life.
#[test]
fn duplicator_clones_player() {
    let Some(l) = loaded() else {
        return;
    };
    // Teammates spawn 2.63 apart (no team check); a third player keeps the
    // round running.
    let mut w = world(
        &l,
        6,
        &[
            setup(1, 0, vec![DUPLICATOR]),
            setup(2, 0, vec![DUPLICATOR]),
            setup(3, 1, vec![DUPLICATOR]),
        ],
        9,
    );
    settle(&mut w);
    let p1 = bopl_pos(&w, 1).unwrap();
    let p2 = bopl_pos(&w, 2).unwrap();
    let cues = cast(&mut w, 1, 0, p2 - p1, 1, 1);
    assert!(has_sound(&cues, "fireRaygun"));
    assert_eq!(w.player(2).unwrap().players_and_clones_still_alive, 2);
    let all = bopls_of(&w, 2);
    assert_eq!(all.len(), 2, "the victim has a clone");
    let clone = all.into_iter().find(|b| Some(*b) != bopl(&w, 2)).unwrap();
    match &w.objects.get(clone).unwrap().kind {
        Kind::Bopl(b) => {
            assert!(b.ctrl.is_abilityless_clone);
            assert!(b.ctrl.abilities.is_empty());
            assert!(b.ctrl.has_spawned);
            // Popped into the air: (dir slerped toward the ground normal) ·
            // 1.3 · jumpStrength.
            assert!(b.body.self_imposed_velocity.magnitude() > Fix::from_int(30));
        }
        _ => unreachable!(),
    }
    assert!(w.fault.is_none());
}

/// Shooting an ability pickup makes three pickups of the same ability that
/// fan out and despawn after 30 s.
#[test]
fn duplicator_clones_pickup_with_lifespan() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(
        &l,
        6,
        &[setup(1, 0, vec![DUPLICATOR]), setup(2, 1, vec![DUPLICATOR])],
        11,
    );
    settle(&mut w);
    let p1 = bopl_pos(&w, 1).unwrap();
    let at = p1 + Vec2::new(Fix::from_int(8), Fix::from_int(1));
    bopl_sim::abilities::spawn_pickup(&mut w, at, Some(BEAM), Vec2::UP);
    let n = w.players.len();
    for _ in 0..12 {
        w.step(&idle(n));
    }
    let pickups = |w: &World| -> Vec<(ObjId, Option<usize>)> {
        w.objects
            .ids()
            .filter_map(|o| match &w.objects.get(o).unwrap().kind {
                Kind::Behaviour(b) => match b.as_ref() {
                    bopl_sim::abilities::Behaviour::Pickup(p) => Some((o, p.ability)),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    };
    let orig = pickups(&w);
    assert_eq!(orig.len(), 1);
    let pp = w.objects.get(orig[0].0).unwrap().tf.position;
    let me = bopl_pos(&w, 1).unwrap();
    cast(&mut w, 1, 0, pp - me, 1, 2);
    let all = pickups(&w);
    assert_eq!(all.len(), 4, "three clones");
    assert!(all.iter().all(|(_, a)| *a == Some(BEAM)), "{all:?}");
    // Clones live 1800 ticks (30 s), counted from the shot (the level's
    // spawner may add pickups meanwhile: follow the clones by handle).
    let clones: Vec<ObjId> = all
        .iter()
        .map(|p| p.0)
        .filter(|o| *o != orig[0].0)
        .collect();
    for _ in 0..1795 {
        w.step(&idle(n));
    }
    assert!(
        clones.iter().all(|c| w.objects.exists(*c)),
        "clones still there"
    );
    for _ in 0..10 {
        w.step(&idle(n));
    }
    assert!(
        clones.iter().all(|c| !w.objects.exists(*c)),
        "clones despawned"
    );
    assert!(w.objects.exists(orig[0].0), "the original stays");
    assert!(w.fault.is_none());
}

/// Shooting a platform duplicates it after 1.3 s (permanent until the same
/// gun makes another).
#[test]
fn duplicator_clones_platform() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(
        &l,
        6,
        &[setup(1, 0, vec![DUPLICATOR]), setup(2, 1, vec![DUPLICATOR])],
        13,
    );
    settle(&mut w);
    let platforms = |w: &World| {
        w.objects
            .ids()
            .filter(|o| matches!(w.objects.get(*o).unwrap().kind, Kind::Platform(_)))
            .count()
    };
    let before = platforms(&w);
    let cues = cast(&mut w, 1, 0, Vec2::DOWN, 1, 1);
    assert!(has_sound(&cues, "fireDuplicatorOnPlatform"), "{cues:?}");
    let n = w.players.len();
    let mut made = None;
    let mut cues = Vec::new();
    for t in 0..120 {
        w.step(&idle(n));
        cues.extend(events(&w));
        if made.is_none() && platforms(&w) > before {
            made = Some(t);
        }
    }
    let made = made.expect("platform cloned");
    // 1.3 s = 78 ticks: the effect is admitted the tick after the shot.
    assert!((76..82).contains(&made), "cloned at {made}");
    assert!(has_sound(&cues, "duplicatePlatform"));
    assert_eq!(platforms(&w), before + 1);
    let v = w.view();
    assert_eq!(v.platforms.len(), before + 1, "the clone is drawn");
    assert!(w.fault.is_none());
}

// ---------------------------------------------------------------------------
// Growth ray / Shrink ray
// ---------------------------------------------------------------------------

fn scale_of(w: &World, id: i32) -> Fix {
    w.player(id).unwrap().scale()
}

/// Growth ray on a teammate: +≈0.486 scale over the 0.5 s curve, applied as
/// per-tick increments; the hurt box follows. Shrink ray takes it back down.
#[test]
fn growth_and_shrink_ray_change_scale() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(
        &l,
        6,
        &[
            setup(1, 0, vec![GROWTH, SHRINK]),
            setup(2, 0, vec![GROWTH]),
            setup(3, 1, vec![GROWTH]),
        ],
        15,
    );
    settle(&mut w);
    let p1 = bopl_pos(&w, 1).unwrap();
    let p2 = bopl_pos(&w, 2).unwrap();
    assert_eq!(scale_of(&w, 2), Fix::ONE);
    let cues = cast(&mut w, 1, 0, p2 - p1, 1, 40);
    assert!(has_sound(&cues, "fireRaygun"));
    assert!(has_sound(&cues, "grow"));
    let s = scale_of(&w, 2);
    assert!(
        s > Fix::from_f32(1.45) && s < Fix::from_f32(1.52),
        "grown to {s:?}"
    );
    assert_eq!(scale_of(&w, 1), Fix::ONE, "the shooter is unchanged");
    let b2 = bopl(&w, 2).unwrap();
    assert_eq!(w.objects.get(b2).unwrap().tf.scale, s, "hurt box follows");
    // Shrink it back (slot 1); wait for the shrink cooldown.
    for _ in 0..200 {
        w.step(&idle(w.players.len()));
    }
    let p1 = bopl_pos(&w, 1).unwrap();
    let p2 = bopl_pos(&w, 2).unwrap();
    let cues = cast(&mut w, 1, 1, p2 - p1, 1, 40);
    assert!(has_sound(&cues, "shrink"));
    let s2 = scale_of(&w, 2);
    assert!(s2 < s - Fix::from_f32(0.4), "shrunk to {s2:?}");
    assert!(w.fault.is_none());
}

/// A player ScaleChanger freezes while time is stopped (the victim is not
/// protected) and resumes afterwards.
#[test]
fn growth_ray_pauses_in_time_stop() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(
        &l,
        6,
        &[
            setup(1, 0, vec![GROWTH]),
            setup(2, 0, vec![GROWTH]),
            setup(3, 1, vec![GROWTH]),
        ],
        17,
    );
    settle(&mut w);
    let p1 = bopl_pos(&w, 1).unwrap();
    let p2 = bopl_pos(&w, 2).unwrap();
    cast(&mut w, 1, 0, p2 - p1, 1, 5);
    let s = scale_of(&w, 2);
    assert!(s != Fix::ONE, "growing");
    // Stop time (what the time-stop ability does) with nobody protected.
    w.time.player_time_scale = Fix::ZERO;
    let n = w.players.len();
    for _ in 0..30 {
        w.step(&idle(n));
    }
    assert_eq!(scale_of(&w, 2), s, "frozen");
    w.time.player_time_scale = Fix::ONE;
    for _ in 0..40 {
        w.step(&idle(n));
    }
    assert!(scale_of(&w, 2) > s + Fix::from_f32(0.3), "resumed");
    assert!(w.fault.is_none());
}

// ---------------------------------------------------------------------------
// Ability bodies: kill attribution, cooldowns
// ---------------------------------------------------------------------------

/// Player 2 is a blink gun (button held) when player 1's beam hits it: the
/// gun body dies, the player dies with killer 1 / Beamed and drops a pickup.
#[test]
fn beam_kills_gun_body() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(
        &l,
        6,
        &[setup(1, 0, vec![BEAM]), setup(2, 1, vec![BLINK])],
        19,
    );
    settle(&mut w);
    let p1 = bopl_pos(&w, 1).unwrap();
    let p2 = bopl_pos(&w, 2).unwrap();
    let n = w.players.len();
    let mut dead = None;
    for t in 0..300 {
        let mut inp = idle(n);
        inp[0] = PlayerInput::new(dir_byte(p2 - p1), BTN_ABILITY0);
        // Player 2 turns into its gun late and keeps aiming at the floor.
        if t >= 60 {
            inp[1] = PlayerInput::new(dir_byte(Vec2::DOWN), BTN_ABILITY0);
        }
        w.step(&inp);
        if t == 70 {
            assert!(in_ability(&w, 2), "player 2 is a gun");
        }
        if let Some(d) = died(&events(&w), 2) {
            dead = Some((t, d));
            break;
        }
    }
    let (_, (killer, cause)) = dead.expect("gun body beamed");
    assert_eq!((killer, cause), (1, CauseOfDeath::Beamed));
    assert_eq!(w.player(1).unwrap().kills, 1);
    assert!(w.fault.is_none());
}

/// Blink gun cooldown: 5 s after blinking a player, back to 2.5 s after
/// blinking a platform; the slot cannot fire before its timer exceeds it.
#[test]
fn blink_gun_cooldowns() {
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(
        &l,
        6,
        &[setup(1, 0, vec![BLINK]), setup(2, 1, vec![BLINK])],
        21,
    );
    settle(&mut w);
    let a = ability_obj(&w, 1, 0).unwrap();
    // Shoot the floor: a platform (wall) blink keeps the base cooldown.
    let c = cast(&mut w, 1, 0, Vec2::DOWN, 1, 2);
    assert!(has_sound(&c, "laserShoot"));
    assert_eq!(cooldown_of(&w, a), Fix::from_f32(2.5));
    // The platform shakes for 1 s, then vanishes (4 s).
    let n = w.players.len();
    let mut gone = false;
    for _ in 0..90 {
        w.step(&idle(n));
        gone |= w.view().platforms.iter().any(|p| !p.visible);
    }
    assert!(gone, "the platform vanished");
    // Not ready yet: a press 1 s after the shot does nothing.
    let t0 = timer(&w, 1, 0);
    assert!(t0 < Fix::from_f32(2.5));
    let c = cast(&mut w, 1, 0, Vec2::DOWN, 1, 1);
    assert!(!has_sound(&c, "prepareToQuantum"));
    assert!(w.fault.is_none());
}

// ---------------------------------------------------------------------------
// Determinism and rollback with the G1 abilities
// ---------------------------------------------------------------------------

const G1: [usize; 6] = [BLINK, GROWTH, SHRINK, BEAM, DUPLICATOR, MAGNET];

/// 12 players, each with three of the G1 abilities.
fn twelve_g1() -> Vec<PlayerSetup> {
    (0..12)
        .map(|i| PlayerSetup {
            id: i + 1,
            team: i % 6,
            color: i,
            abilities: vec![
                G1[i as usize % 6],
                G1[(i as usize + 1) % 6],
                G1[(i as usize + 3) % 6],
            ],
            uses_keyboard_and_mouse: i == 5,
        })
        .collect()
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 29;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 32)
}

/// Scripted input: in windows of 128 ticks each player holds one ability
/// button for a random part of the window (up to 111 ticks, long enough for
/// a beam to start), aiming somewhere, and jumps now and then — so guns aim
/// and fire, beams sweep, magnets pull.
fn inputs_g1(n: usize, t: u64, seed: u64) -> Vec<PlayerInput> {
    (0..n)
        .map(|i| {
            let win = t / 128;
            let x = mix(seed ^ win.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((i as u64 + 1) << 40));
            let y = mix(x ^ (t / 6));
            let in_win = t % 128;
            let hold = 2 + (x >> 8) % 110;
            let slot = (x >> 16) % 4;
            let mut b = 0;
            if slot < 3 && in_win < hold {
                b |= [BTN_ABILITY0, BTN_ABILITY1, BTN_ABILITY2][slot as usize];
            }
            if (y >> 3).is_multiple_of(23) {
                b |= BTN_JUMP;
            }
            let angle = if (y >> 12).is_multiple_of(5) {
                0
            } else {
                (((x >> 24) as u8) ^ ((in_win as u8) / 24)) | 1
            };
            PlayerInput {
                angle,
                buttons: b,
                keys: (y >> 20) as u8 & 0x0f,
            }
        })
        .collect()
}

fn run_g1(w: &mut World, from: u64, to: u64, seed: u64) {
    let n = w.players.len();
    for t in from..to {
        w.step(&inputs_g1(n, t, seed));
    }
}

/// Same inputs → identical state over 6000 ticks per start level, 12
/// players using only G1 abilities, levels reloaded at the winner screen.
#[test]
fn determinism_twelve_players_g1() {
    let Some(l) = loaded() else {
        return;
    };
    let levels = [6usize, 18, 28, 126];
    let lv: Vec<_> = levels
        .iter()
        .map(|&i| bopl_content::load_level(&l, i).unwrap())
        .collect();
    let mut casts = [0usize; 6];
    for (start, &level) in levels.iter().enumerate() {
        let mk = || {
            let mut w = World::new(
                Arc::clone(&l.content),
                &twelve_g1(),
                MatchSettings {
                    number_of_abilities: 3,
                    seed: 31,
                },
            );
            bopl_sim::session::load_level(&mut w, lv[start].clone());
            w
        };
        let (mut a, mut b) = (mk(), mk());
        let mut next = start;
        let mut hashes = Vec::new();
        for chunk in 0..60u64 {
            for t in chunk * 100..chunk * 100 + 100 {
                let inp = inputs_g1(12, t, level as u64);
                a.step(&inp);
                b.step(&inp);
                for e in &a.events {
                    let k = match e.cue {
                        Cue::Sound("laserShoot") => 0,
                        Cue::Sound("grow") => 1,
                        Cue::Sound("shrink") => 2,
                        Cue::Sound("beamFire") => 3,
                        Cue::Sound("fireDuplicatorOnPlatform") => 4,
                        Cue::Sound("magnetEnter") => 5,
                        _ => continue,
                    };
                    casts[k] += 1;
                }
            }
            assert_eq!(
                a.state_hash(),
                b.state_hash(),
                "level {level} tick {}",
                (chunk + 1) * 100
            );
            hashes.push(a.state_hash());
            if a.round.winner_screen && a.time.sim_time_since_level_loaded > Fix::from_int(20) {
                next = (next + 1) % lv.len();
                bopl_sim::session::load_level(&mut a, lv[next].clone());
                bopl_sim::session::load_level(&mut b, lv[next].clone());
            }
        }
        hashes.dedup();
        assert!(hashes.len() > 30, "level {level}: state did not evolve");
        assert!(a.fault.is_none(), "level {level}: fault {:?}", a.fault);
    }
    eprintln!("G1 uses (blink, grow, shrink, beam, dup-platform, magnet): {casts:?}");
    assert!(
        casts.iter().all(|c| *c > 0),
        "every G1 ability was used: {casts:?}"
    );
}

/// Rollback with the G1 abilities: snapshot, mispredict, restore, replay →
/// the same state, checksum and view as a straight run.
#[test]
fn rollback_g1_equals_straight_run() {
    let Some(l) = loaded() else {
        return;
    };
    let mk = || world(&l, 18, &twelve_g1(), 33);
    let mut straight = mk();
    run_g1(&mut straight, 0, 3000, 5);
    let mut rb = mk();
    run_g1(&mut rb, 0, 900, 5);
    for k in 0..24u64 {
        let base = 900 + k * 87;
        let snap = rb.clone();
        run_g1(&mut rb, base, base + 9, 777 + k);
        rb = snap;
        run_g1(&mut rb, base, base + 87, 5);
    }
    run_g1(&mut rb, 900 + 24 * 87, 3000, 5);
    assert_eq!(rb.time.simulation_ticks, straight.time.simulation_ticks);
    assert_eq!(rb.state_hash(), straight.state_hash());
    assert_eq!(rb.checksum(), straight.checksum());
    assert_eq!(rb.view(), straight.view());
}

// ---------------------------------------------------------------------------
// The beam system (DetPhysics.UpdateBeams) directly
// ---------------------------------------------------------------------------

fn beam_body(id: i32, owner: i32, pos: Vec2, dir: Vec2) -> bopl_sim::abilities::beam::BeamBody {
    bopl_sim::abilities::beam::BeamBody {
        id,
        owner,
        position: pos,
        direction: dir.normalized(),
        time_passed: Fix::ONE,
        scale: Fix::ONE,
        distance: Fix::ZERO,
        ground: None,
    }
}

fn active_beam_boxes(w: &World) -> usize {
    w.ability_globals
        .beams
        .pool
        .iter()
        .filter(|o| w.objects.get(**o).is_some_and(|ob| ob.active))
        .count()
}

/// A beam aimed at the water reflects at the fixed water line (two boxes,
/// the mirrored one with the negated id); two crossing beams merge into an
/// environmental parent beam (owner 1000) with the summed scale.
#[test]
fn beam_system_reflects_and_merges() {
    use bopl_sim::abilities::beam::{Role, add_beam_body};
    let Some(l) = loaded() else {
        return;
    };
    let mut w = world(
        &l,
        6,
        &[setup(1, 0, vec![BEAM]), setup(2, 1, vec![BEAM])],
        23,
    );
    settle(&mut w);
    let n = w.players.len();
    // Far left of the level, pointing down-right into the water.
    let p = Vec2::new(Fix::from_int(-80), Fix::from_int(0));
    let d = Vec2::new(Fix::from_f32(0.6), Fix::from_f32(-0.8));
    add_beam_body(&mut w, beam_body(500, 1, p, d));
    w.step(&idle(n));
    let s = &w.ability_globals.beams;
    assert!(s.is_reflected[0], "reflected at the water line");
    assert_eq!(s.bodies[1].id, -500, "mirrored segment");
    assert!(s.bodies[1].direction.y > Fix::ZERO, "mirrored upwards");
    assert_eq!(active_beam_boxes(&w), 2);
    // Two beams crossing at a right angle above the level: merged.
    let a = Vec2::new(Fix::from_int(-30), Fix::from_int(30));
    let b = Vec2::new(Fix::from_int(30), Fix::from_int(30));
    let da = Vec2::new(Fix::ONE, Fix::ONE);
    let db = Vec2::new(-Fix::ONE, Fix::ONE);
    add_beam_body(&mut w, beam_body(501, 1, a, da));
    add_beam_body(&mut w, beam_body(502, 2, b, db));
    w.step(&idle(n));
    let s = &w.ability_globals.beams;
    assert_eq!(s.total_last, 3, "two beams + the merged parent");
    assert_eq!(s.roles[0], Role::Child);
    assert_eq!(s.roles[1], Role::Child);
    assert_eq!(s.roles[2], Role::Parent);
    assert_eq!(s.bodies[2].owner, 1000);
    assert_eq!(s.bodies[2].id, i32::MAX - 1);
    assert_eq!(s.bodies[2].scale, Fix::TWO, "summed strength");
    assert!(
        s.bodies[2].direction.y > Fix::from_f32(0.99),
        "merged straight up"
    );
    // Next tick without beams: every box is deactivated.
    w.step(&idle(n));
    assert_eq!(active_beam_boxes(&w), 0);
    assert!(w.fault.is_none());
}

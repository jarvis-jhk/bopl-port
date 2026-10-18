# bopl-sim design

How `crates/sim` (`bopl-sim`) and `crates/content` (`bopl-content`) are
built. Behaviour is specified in `docs/spec/`; this file is about structure.

## Rules the code keeps

- Pure function of `(World, inputs) → World`. No I/O, no wall clock, no
  threads, no `HashMap` iteration, no floats in logic. The one spec'd float
  (the time-stop lifetime, engine.md §9) is for the time-stop module.
- `Fix` (`fix.rs`) is the game's Q32.32 bit for bit; every operator is the
  spec'd one (wrapping `+ − ·`, rounding `/`, the trig and `Atan2` quirks).
  Verified against all golden vectors in `scratch/fixoracle/vectors.json`
  (`tests/fix_vectors.rs`, `tests/phys_vectors.rs`; found through
  `BOPL_ROOT` in worktrees; skipped without the file).
- Operations that throw in the original (division by zero, `Sqrt`/`Acos`/
  `Log2` domain, `MinValue % −1`) return 0 and raise a thread-local **fault**.
  After each tick `World::tick` takes the fault, records it in `World::fault`,
  emits `Cue::Fault` and ends the round as a draw (the original online
  host's behaviour). Deviation: the original aborts the rest of that tick;
  we finish it with the 0 results.
- Sin/tan tables (`lut.rs`): generated once per process (`OnceLock`, ≈0.2 s)
  from fixmath.md §5.2 with sin/cos in double-double arithmetic (only IEEE
  `+ − × ÷`), rounded once to `f64`; SHA-256 checked in a unit test. (The
  pure-Rust `libm` 0.2.16 also reproduces both tables, 0 mismatches; we keep
  the dependency-free generator.) The tables are shared, never part of the
  cloned state.

## State

```
World (Clone = rollback snapshot)
├── content: Arc<Content>        static data, shared by all clones
├── level:   Option<Arc<LevelData>>
├── objects: Objects             slot arena of game objects (generation handles)
├── physics: Physics             DetPhysics lists (rr, circles, boxes), composites
├── players: Vec<Player>         list order = join order, ids 1..N (N ≤ 12)
├── rng:     Rng                 the Unity.Mathematics xorshift
├── time, round, settings
├── updatables, to_be_added      the updater (engine.md §3–§4)
├── events: Vec<Event>           this tick's cues
└── ability_globals              pending exits, beams, duplicates, parries, blink tunnels (ability-owned; ropes live in physics.ropes)
```

- **Objects** (`object.rs`): `Object { tf: FixTransform, active, parent,
  children, layer, player_id, destroyed, parts: [Part; 12], collider:
  Option<ColliderDef>, bb: BoplBody, kind: Kind, visual }`. A `Part` is one updatable
  component (`PartKind`, `enabled`, `destroyed`, hierarchy number). The typed
  payload `Kind` holds the component data: `Bopl`, `Platform`, `Session`,
  `Spawner`, `Ability`, `Behaviour` (ability-spawned objects), `Static`.
- **Updater** (`world.rs`): `register` (Awake) appends `Upd {obj, part, hier}`
  to `to_be_added`, or to the open `InstantiateFixed` bracket, which adds
  `prefabsAdded + 100000000` on close. `tick()` is engine.md §4 step by step:
  edges, admission (introsort by hierarchy number, `Init` loop re-reading the
  length), removal passes, update, physics (+ callbacks), late update,
  destroy queue, clocks. Destroyed objects are flagged at once and freed in
  step 14; handles go stale (`Objects::get` → `None`), which is the port's
  "destroyed object == null".
- **Dispatch**: a part runs with its object's payload taken out
  (`World::with_kind`), so component code gets `&mut World` and `&mut` own
  data at the same time. While taken, the object's `kind` is `Kind::Empty`;
  code that may reach back into the running object (an ability ending inside
  its own cast) is deferred through `ability_globals.pending_exits`.
- **Presentation effects** (death gibs, water splash) are not simulated;
  they become `Cue::Effect` events but still consume their prefabs' hierarchy
  numbers (`World::consume_hierarchy`) so later numbers match the original.
- `SetActive(false/true)` only toggles `Object::active`: no re-registration,
  no change of collider list order, no `Init` (spec review item 4).
- **Hash**: `World::state_hash()` (FNV-1a over all live state) for tests and
  desync checks; `World::checksum()` is the original's transform checksum.

## Modules

| File | What |
|---|---|
| `fix.rs`, `lut.rs`, `vec2.rs`, `rng.rs`, `curve.rs`, `sort.rs`, `consts.rs` | fixmath.md, engine.md §4.1 |
| `input.rs` | `PlayerInput` (3-byte Pod: angle byte, jump/ability0-2/start/select bits, WASD bits), input profile, byte↔vector tables |
| `object.rs`, `world.rs` | object model, updater, tick, colliders' scale setter, hashing |
| `physics/` | DetPhysics: `shapes.rs` (narrow phase, ray casts, point helpers — all vector-verified), `mod.rs` (lists, Simulate, broad phase, response, queries, composites) |
| `anim.rs` | `SpriteAnimator` (callbacks are codes the owner dispatches) |
| `surface.rs` | perimeter parameterisation of sticky platforms (player.md §5) |
| `bopl.rs` | `PlayerPhysics`, `SlimeController`, `PlayerCollision`, bounds, `PlayerBody`; kill flow |
| `platform.rs` | `StickyRoundedRectangle`, `AnimateVelocity`, `ShakablePlatform`, `AntiLockPlatform`, `VectorFieldPlatform`, `GrowOnStart`, `BoplBody` |
| `session.rs` | `GameSessionHandler` (fly-in/out, spawning, round end, sudden death), `RoutineQueue`, `AbilitySpawner`, `load_level` |
| `abilities/` | framework, registry, pickup, one module per ability |
| `view.rs` | the presentation `View` |
| `data.rs` | static data types produced by `bopl-content` |

## Ability architecture

The 29 abilities were written in parallel by four groups and unified at
integration (`docs/review/abilities-integration.md`). One module per
ability under `crates/sim/src/abilities/`, plus the shared pieces below.

### Framework and registry (`abilities/mod.rs`)

- abilities.md §3–§13: slots, cooldowns, castability, entry snapshot
  `PlayerInfo`, exit record `AbilityExitInfo`, `Ability.ExitAbility`
  (`Cx::exit`, no re-entrancy guard), scale propagation, `Player.SetPosition`,
  `KillPlayersOnContact`, kill rules by layer (`lethal_info`).
- `registry!` with one line per ability-list entry generates `enum
  AbilityState` (one variant per module) and `enum Behaviour` (each module's
  `Spawned` type plus the pickup) and the dispatch. Enum dispatch keeps the
  state `Clone + Hash + Eq` (rollback snapshots are plain clones).
- `instantiate` builds the ability object inside an `InstantiateFixed`
  bracket: the `Ability` part, the prefab root's layer, offset and collider
  part, then `AbilityImpl::build` adds the module's parts; the prefab's
  other updatables consume their hierarchy numbers; the module's body gets
  the owner.
- `enter` (abilities.md §7.1): snapshot, ignored colliders, **the snapshot's
  rope goes to the module's body** (`AbilityBody::take_snapshot_rope`),
  collider scale, `SetPosition`, `on_enter`. Every exit of a module with a
  body goes through `Cx::exit_body`, which puts the body's rope into the
  record first (§8.2 step 1); `framework_exit` (black holes eating an
  ability user) skips the module's own cleanup but still hands the rope back.

### Hooks

`AbilityImpl` (ability objects):

| Hook | Meaning |
|---|---|
| `new(w, def)` | read the prefab data |
| `build(w, me, fw, def)` | add parts (`AbilityBody::install` / `install_parts`, `ability_body::add_parts`) |
| `tick(cx, dt, late)` | the `Ability` part; default = `pre_update` → `Cx::ability_update` → `update` (`late_update` when late) |
| `pre_update`, `update`, `late_update` | components before / after `Ability` when they run inside `tick` |
| `init_part`, `update_part` | components added as their own parts (any `PartKind` other than `Ability`) |
| `on_enter`, `exit_component` | `OnEnterAbility`; the first `IAbilityComponent.ExitAbility(record)` (also used by others: magnet gun, bounds, drop-through) |
| `kill`, `on_killed` | `PlayerCollision.killPlayer` on the object: default = `AbilityBody::kill` (rope release, invulnerability, death effect) then `on_killed` = `exit_component(dead_record)` |
| `body`, `body_mut` | the object's `AbilityBody` (rope hand-over, black-hole pull, shockwaves, magnets, teleports, hooks) |
| `interact` | component interfaces other modules use (`Interaction`) |
| `on_contact`, `on_anim`, `on_scale_changed`, `is_castable`, `lethal`, `sprite` | as named |

Both component models give the original update order: an object's
updatables are contiguous in the updater's list, so a module may run its
components inside `tick` in hierarchy order (G3/G4 style) or add them as
parts dispatched to `update_part` (G1/G2 style).

`SpawnedImpl` (module objects, `Kind::Behaviour`): `init`, `update`,
`on_contact`, `on_destroy`, `lethal`, `always_enabled` (runs while its
object is inactive, `SimpleSparkNode`), `interact`, `duplicate` (the
Duplicator clone rule: serialized fields copied, runtime ones reset;
`None` = not clonable).

`Interaction` (answered by the owning module, `abilities::interact(w, obj,
i)`): `SpellAddScale` (revive and teleport markers), `ItemKind` (grenade,
mine, missile, rock ball, rocket engine, spike, spark node),
`DelayFuse` (magnet hold: grenade fuse / rock timer), `Ignite` (smoke
clouds, engine flames), `AttachRope` (the Rock's `BounceBall`),
`Duplicated` (clone fix-ups). Calls whose target type is known go to the
module's `pub` functions instead (black holes: `black_hole::{mass, grow,
add_force, add_force_less_mass_influence, set_mass, hold,
take_magnet_holder}`; planted terrain clones: `engine/spike/tesla_coil::
init_duplicate`; rope anchors: `grappling_hook::{spawn_platform_anchor,
platform_anchor, carry_anchor_to}`; boulders: `throw::boulder_hold`).

### Shared pieces

- **`ability_body.rs`** — the one player body of ability objects
  (`AbilityBody` + `BodyCfg`): PlayerPhysics (`physics_update`,
  `physics_late_update` with the grounded rope pull, `move_` / `macho_move`
  returning the drop-through record, `velocity_based_raycasts` /
  `collision_raycasts`, `attach_to_ground`, `unground`, `jump`,
  `move_along_current_platform`, `move_along_swap_if_blocked`,
  `sync_physics_to(&PlayerInfo)`), PlayerBody with the rope coupling of
  `rope.rs` (`body_update`, `body_late_update`, `attach_rope`,
  `take_snapshot_rope`, `hand_back_rope`, `clear_rope`), PlayerCollision
  (`kill_on_collision`, `kill_player_on_collision`, `collision_contact`,
  `kill`, `hurt_contact`), bounds (`bounds_update`), the standard parts
  (`update_part`), ground events for the Beam. Also `Bounds` / `Effect`
  (DestroyIfOutsideSceneBounds incl. live explosion effects), the one spawn
  path (`spawn_prefab` by class names, `spawn_with_parts` / `spawn_owned`
  with explicit parts, `parts_of`, `add_parts`, `prefab_parts`),
  `bopl_body_update` / `is_held` / `holder_of`, animator helpers
  (`advance_anim`, `advance_code`, `play`, `play_then`, …) and physics
  helpers (`manual_init`, `add_force_at_position`, `set_relative_*`,
  `set_layer`, `is_platform_attachable`, `shape_snap`, `touch`, …).
- **`prefab.rs`** — content lookups: prefabs by reference, components,
  fields (raw / float path / vectors / curves), animations, `object_tag`
  (prefab objects and level platforms / statics).
- **`object::BoplBody`** — every object's BoplBody state (prevScale from 1,
  `framesUntilDrop`, `magnetGunHolder`, `isResizable`), updated by the
  updater at the object's `PartKind::BoplBody` part (physics.md §2.4).
- **`explosion.rs`** (spec mine.md) — the one `Explosion` and `Shockwave`
  (`explosion::spawn`, `spawn_shockwave` for stand-alone shockwave prefabs:
  gusts and meteor impacts; objects in `mine::Spawned::{Explosion,
  Shockwave}`; `info`, `ignites_flammables`).
- Other shared components live with their spec: `blink_gun::GunTransform`
  / `GunAbility` (guns), `growth_ray` (scale rays), `gust::QuickSpell`
  (Gust, Revival), `grenade::ThrowItem2` (Grenade, Smoke),
  `engine::PlantObjectOnPlatform` (Engine, Tesla coil), `smoke::RingBuffer`
  (smoke, grenades, bows, engines, tesla coils), `rope.rs` +
  `physics/ropes.rs` + `grappling_hook.rs` (the rope system), `beam.rs`
  (DetPhysics beams), `duplicator.rs` (duplicate list, clones).

### Engine hooks

Called by the bopl / world at the original's places: `rope_*` (bopl rope
coupling), `release_rope`, `update_rope_cooldowns`, `clear_rope_flags`
(step 12), `update_duplicates` (step 5), `physics_mid_step` (DetPhysics
step 8 after the ropes: beams), `static_update` (step 8: magnet parries),
`revive_on_death`, `on_prepare_to_revive`, `invisibility_ended`,
`reposition_attachables` (rounded-rect scale setter, before the shape),
`apply_pending_exits` (exits / caster-body changes requested while the bopl
was taken out).

## Content (`crates/content`)

`bopl_content::load(dir)` reads the import (`bopl_import::GameData`) and
builds `Arc<Content>`: sprite table (`SpriteId` ↔ import sprite id), all
`AnimationData`, all 132 prefabs as generic `Prefab`s, the typed
`PlayerPrefab` and `PickupDef`, the ability list, the layer collision
matrix. `load_level(&loaded, build_index)` builds `LevelData`: session
values and curves, DetPhysics parameters, platforms in the session
handler's collection order (DFS pre-order under the `Level` root, active
only), static colliders (water box), spawners, the 4 team spawns and 8
generated extra spawns (`spawns.rs`, match-flow.md §17.2). Fix values are
copied as raw bits. `replay.rs` reads and writes the original's `.rep`
(header → setups/settings, per-tick inputs, `viewer_inputs`: the original
viewer's time-stop input rule); `scenario.rs` compiles the golden scenarios
(`scripts/dev/golden/scenarios/*.txt`) to replays. Bins: `sim-trace`
(per-tick CSV) and `golden` (gen / sim / track / compare against captures of
the original, `docs/reference-capture.md`).

## Running a match

```rust
let loaded = bopl_content::load(bopl_content::default_import_dir())?;
let level = bopl_content::load_level(&loaded, 6)?;          // Level1
let mut w = World::new(loaded.content.clone(), &setups, MatchSettings { number_of_abilities: 3, seed });
bopl_sim::session::load_level(&mut w, level);
loop {
    w.step(&inputs);           // one PlayerInput per player, list order
    let view = w.view();       // draw + play view.events (dedup by id)
}
```

Next-level flow (bag, winner-screen input) is M4: today the host decides
when to call `load_level` again (after `round.winner_screen`).

## View

`World::view() -> View` (pure, cheap):

- `platforms: Vec<PlatformView>`: index into `LevelData::platforms` (and
  `PlatformDef::visual.object`, the import object id), `position`,
  `rotation` (radians, Fix), collider `scale`, `visible`.
- `sprites: Vec<SpriteView>`: bopls, active ability objects and spawned
  objects: `object` (stable handle), `kind`, `visual` (`Visual::Bopl` /
  `Prefab {prefab, object}` / ...), current `sprite` (`SpriteId`; map with
  `Content::sprites[id]` to the import sprite id) and `flip_x`, `position`,
  `rotation`, `scale`, Unity `layer`, `player_id`, `team`, `color` (player
  colour → material), `facing_right`. Sorting layer/order and material come
  from the visual's import object (`PlayerPrefab::visual` for the bopl).
  `look: Look` is the module's presentation state, filled by
  `AbilityImpl::look` / `SpawnedImpl::look` (and `abilities::platform_look`
  for ability platforms): animator frame and flip, alpha (owner's
  invisibility fade included), stretch, hidden, player material, per-child
  `PartLook`s (visibility, frame, pose, alpha, tint, shadow colour, four
  shader values), four shader values for the root, a loop sound, attached
  particle prefabs (`Attach`), a player override and objects to hide. Looks
  read the state only; nothing in them is hashed.
- `ropes: Vec<RopeView>`: the enabled rope bodies' point chains and owner.
- `players: Vec<PlayerView>`: id, team, colour, alive/spawned/in-ability/
  invisible, aim position, score counters, cause of death, ability slots
  (index, cooldown timer and length).
- `round: RoundView` and `events: Vec<Event>`.
- **Events**: `Event { id: EventId { tick, seq }, cue }`. Ids are a pure
  function of the simulation, so when rollback re-simulates a tick the
  same cue gets the same id: play an id once, drop ids of ticks that were
  rolled back without re-emitting them. Cues: `Sound(name)`,
  `SoundPrefix(prefix)`, `PlayerSound(name, id)` (name + id, e.g.
  `slimeJump1`), `Effect { kind, position, player_id }`, `PlayerSpawned`,
  `PlayerDied { id, killer, cause }`, `Shake`, `RoundWon`, `RoundDraw`,
  `WinnerScreen`, `SuddenDeath`, `Fault`, and `Fx(FxCue)`: a
  presentation-only `Instantiate` of the prefab referenced by a field
  (`dustParticlePrefab`, ...) on the emitting object's prefab (or the
  level's `DetPhysics` with `LEVEL_PREFAB`), with position, rotation,
  scale, a value, an optional particle colour and the emitting object.
  Events are not part of `state_hash`, so presentation cues never change
  the simulation's hashes.
- Fix → float conversion (`Fix::to_f32`) is presentation-only; interpolate
  between two views for rendering.

## Tests and tools

- `crates/sim/tests/fix_vectors.rs`, `phys_vectors.rs`: golden vectors.
- `crates/sim` unit tests: LUT hashes, introsort.
- `crates/content/tests/sim_levels.rs` (needs an import): sandbox scenario
  (spawn timing, corner walking, jump, water death, round end), 12-player
  determinism over 10 000 ticks from 8 start levels with level reloads,
  rollback (clone, mispredict, restore, replay) equals a straight run,
  clone + step cost (best of five batches; `BOPL_SKIP_TIMING=1` skips the
  bound).
- `crates/sim/tests/abilities_g{1,2,3,4}.rs` (need an import;
  `bopl-content` is a dev-dependency of `bopl-sim`): scenario tests per
  ability of each group on real levels and a 12-player determinism +
  rollback run per group.
- `crates/sim/tests/abilities_all.rs`: 12 players with all 29 abilities
  (and Random) on 7 levels with level changes at the winner screen: two
  runs compared every 50 ticks, a rollback run (8 mispredicted windows)
  equal to a straight run, every ability cast at least once. Prints `FP
  <level> <tick> <state hash> <behaviour hash>` checkpoints
  (`BOPL_FP_STEP`, `--nocapture`), used to compare refactors.
- `crates/sim/tests/abilities_interactions.rs`: cross-module wiring
  (black-hole growth / holds / clones, the shared shockwave on black holes,
  BoplBody magnet holds, prevScale).
- `cargo run --release -p bopl-content --bin sim-trace -- --help`: scripted
  inputs or a `.rep` replay → per-tick CSV.
- `crates/content/tests/golden_scenarios.rs`: every golden scenario through
  our sim (viewer input rule) against `tests/golden_expected.txt` (end tick,
  trajectory hash, death/round/winner ticks), numbers from our sim only,
  blessed after the comparison with the original (`docs/review/feel-pass.md`).

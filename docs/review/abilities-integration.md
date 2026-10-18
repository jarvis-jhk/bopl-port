# Review log: abilities integration (M5)

The four ability groups (G1–G4, logs `abilities-g{1..4}.md`) worked in
parallel from the same base; their branches were merged on
`abilities-integration` (G4, G3, then G2, G1) and unified. This log lists
what was unified, every behaviour change that came with it (with the reason
it is the original's behaviour), the interactions wired between groups,
spec fixes, and what is still open. The final architecture is in
`docs/sim-design.md` ("Ability architecture").

## Method

- All group test suites were run before and after every step.
- `tests/abilities_all.rs` (12 players, all 29 abilities and Random, 7
  levels, level changes) prints a **state hash** and a **behaviour hash**
  (player positions, life, kills, deaths) every 500 ticks
  (`BOPL_FP_STEP` for finer steps). Pure refactors must keep the behaviour
  hash; struct changes alter the state hash by construction. Where the
  behaviour hash moved, the first diverging tick was found (per-tick
  fingerprints of the previous commit built from a `git archive` copy) and
  the cause identified with a per-object state dump; each such cause is
  listed below. Every one is a group copy deviating from the spec or the
  original (the decompilation was consulted only to settle the cases).

## Merges

- G2: G2's `ability_body.rs` collided with G4's (add/add) and was kept as a
  temporary `ability_body_g2.rs`; `instantiate` kept G4's root collider for
  modules without G2's `ObjectSpec`; `Platform` got both `resize` (G4) and
  `boulder` (G2).
- G1: `Simulate` was split by both G1 (beams between broad and narrow phase)
  and G3 (ropes): `simulate_begin` now ends with the ropes, the beams run in
  `World::tick`, then `simulate_end` (DetPhysics step 8 order: ropes, then
  beams). The trait got the union of the hooks for the merge commit.

## Unified

1. **One ability-object body** (`abilities/ability_body.rs`, `AbilityBody` +
   `BodyCfg`) replaces G2's `ability_body.rs`, G3's `body.rs`, G4's
   `ability_body.rs` and G1's body in `blink_gun.rs`. It has G3's rope
   coupling (PlayerPhysics grounded pull, PlayerBody roped integration and
   late update), every movement routine any group needed, the kill rules,
   bounds, standard parts, ground events; animators are the modules' own
   (`advance_anim` / `play_then` helpers).
2. **Rope hand-over in the framework**: `enter` gives the snapshot's rope
   to the module's body (abilities.md §7.1 step 2.3), `Cx::exit_body`
   puts it back into the exit record (§8.2 step 1), also for
   `framework_exit`.
3. **One spawn path**: `spawn_prefab` (part list by component class) /
   `spawn_with_parts` / `spawn_owned` (explicit parts) for every module
   object; `add_parts` / `install_parts` for ability-object components.
   Platform-kind objects (Throw boulders, Platform's ResizablePlatform) are
   still built through the platform constructor (see gaps).
4. **One hook set**: `build(w, me, fw, def)`, `tick` / `pre_update` /
   `update` / `late_update`, `init_part` / `update_part`, `exit_component`,
   `kill` / `on_killed` (defaults do the body part and the dead exit),
   `body` / `body_mut`, `interact`. Removed: `object_spec`, `exit_with`,
   `ability_body`, `body_g2*`, `player_body`, `exit_ability_component`,
   `with_ability_body`, `ability_body_of`, `ability_is_pushable`,
   `black_hole_add_force`, G2's `BodyEvent`, per-module `kill` / `on_killed`
   copies that equalled the defaults, `mine::dead_exit`.
5. **One BoplBody** (`object::BoplBody` on every object, updated by the
   updater): replaces G2's `BoplBodyState`, the bow arrows' scale tracking,
   `platform.rs`'s fields, the duplicator rock's copy and G1's global
   magnet-hold list.
6. **One Shockwave/Explosion** (`explosion.rs`, spec home mine.md): gust's
   and meteor's copies are gone; stand-alone shockwave prefabs spawn through
   `explosion::spawn_shockwave` (`mine::Spawned::Shockwave`).
7. **One PlantObjectOnPlatform** (`engine.rs`): Tesla coil's copy is gone
   (behaviour hashes unchanged).
8. **One RingBuffer** (`smoke.rs`, with `add_by` for component-liveness
   entries such as arrows' BoplBodies): bow's and engine's copies are gone.
9. **One set of prefab lookups** (`abilities/prefab.rs`) from the four
   groups' helper sets.

## Behaviour changes (justified)

| # | Change | Why (spec / original) | First seen |
|---|---|---|---|
| 1 | G3 and G1 ability objects registered the root collider twice (instantiate and the module's build) and so consumed one hierarchy number too few per instantiation | engine.md §3: an instantiation consumes exactly the prefab's updatable count | state hash only (order kept) |
| 2 | Bodies of G2/G4 abilities now carry a held rope (G4 even dropped it every PlayerBody update); G1 stored the link without `AttachRope` | abilities.md §7.1 step 2.3, §8.2 step 1; grappling-hook.md (AttachRope) | level 6 tick 577: a player inside Platform held a rope |
| 3 | Every module's body is reachable by other modules (black-hole pull, shockwaves, hooks, teleports) | black-hole.md gravity on Player-layer boxes, mine.md shockwave on Player layer | level 6 tick 2554: a black hole pulled a shrink-ray body |
| 4 | Bodies of G4 modules had owner 0 (slimeJump cue id 0, owner time scale of player 0) | the body belongs to the ability's owner | found by the fingerprint diff (fixed before committing) |
| 5 | `kill` on any ability body now does the body part (rope release, invulnerability, death-effect hierarchy numbers) before the dead exit; G4 skipped it on bopl-driven kills, G2 always passed "no effect" | player.md §8.2, blink-gun.md "Ability object player body" | — |
| 6 | Spawned objects honour their prefab components' `enabled` flags (G2 enabled every part) | engine.md §3 (component enabled state) | — |
| 7 | Squish test needs a PlayerPhysics (G3 rule) for every body | player.md §8.1 (`physics.IsGrounded`) | — |
| 8 | Meteor: the player push uses the scaled `defaultForce`; black holes in the hit set get `BlackHole.AddForce` (G3 used the unscaled force for players and skipped black holes) | the original scales the component fields after instantiating (meteor.md, mine.md) | level 17 tick 4454; level 28 tick 3863 |
| 9 | Platform-attachable test for shockwave lethal-terrain pushes and teleports: the IPlatformAttachable component list (G4: "no parent", G2: own types or composite member) | mine.md step 2 | — |
| 10 | Boulders recognised by tag `boulder` everywhere (G2 used `Platform::boulder`) | mine.md step 3, grappling-hook.md | — |
| 11 | `BoplBody.prevScale` starts at 1 and is not set at Init (G2, the bow and `platform.rs` set it at Init): a body spawned at scale ≠ 1 (boulders of scaled throwers, scaled items, GrowOnStart platforms) gets the inverse-mass rescale on its first update; composite rule per the original | decompilation (field initialiser, empty Init); physics.md §2.4 fixed | level 6 tick 2394: a 0.51-scale boulder |
| 12 | Magnet holds count down at each body's own BoplBody update (G1: once per tick at step 8, "exact only for objects after the magnet") | magnet-gun.md BoplBody holds | — |
| 13 | Bounds effects: one implementation (G2's: effect offsets added, live explosion effects); presentation cue kinds unified | engine.md §7.1 | presentation only |

The golden replay (`scratch/ref/replays/1.rep`, Bow/Gust/Dash on Level1)
ends with the player's death at tick 6636 and the round decided at tick
6637 (expected ≤ 6655; 6636 after G4 too) — unchanged by every step and
after merging main. `scripts/check-determinism.sh --game` (Linux vs the
Windows build under Wine) agrees on all xcheck runs and the bot-match
events.

## Interactions wired

- Rock: rope coupling (`BounceBall` enter / update / late update / exit /
  `AttachRope`, contacts only without a rope), hook fired at a rock ball,
  magnet hold pauses its timer, item kind.
- Black holes: rope anchors reel the rope in and swallow it; magnet holder
  semantics (`Hold` clears the holder: the countdown is inert); growth /
  shrink rays; shockwaves; clones (mass from `startingMass` at Awake, owner
  1000, then the duplicator's mass and push).
- Throw carries rocket engines (re-`RocketEngineInit` on the boulder, the
  engine restarts) and rope anchors (re-anchored at local 1/2, half the rope
  shifted, previous positions reset, one `SimulateRope`).
- Magnet gun: boulders (tag test, extra offset, `v/35`, `Boulder.Hold`,
  hitbox disabled for its landing raycast, 30000 fire force, unground when
  standing on it), item kinds (missile re-aim, mine velocity), grenade fuse
  pause, held victim's rope (air time 1/10 when thrown up, rope end at the
  held position), mines / grenades ignore their holder's hurt box, arrows
  drop the hold on collision and don't turn while held.
- Smoke: rocket-engine flames ignite clouds (`circleCastCircles`, buffer
  one); grenades and smoke grenades read `Projectile.IgnitesExplosives` from
  the data (arrows: false, so arrows neither detonate grenades nor burst
  smoke grenades).
- Scale rays: black holes (`Grow`), revive markers and teleport markers
  (`ISpell.AddScale`).
- Duplicator: `duplicate()` for grenade, mine, missile, arrow, smoke cloud,
  black hole, explosion, spike, rocket engine, spark node (plus the existing
  pickup and Rock_noPlayer); fix-ups per duplicator.md (grenade flags and
  fuse `detonationTime − (j+1)·0.1`, item owners, missile heading, explosion
  owner); planted engine / spike / spark node clones re-initialised on the
  platform (`init_duplicate`, scale = the hit object's); boulder and
  ResizablePlatform runtime fields reset in platform clones; clouds of a
  duplicated smoke grenade on the delete list for 1200 ticks.
- Platform ability: a held rope is handed to a new `RopeAttachment` on the
  created platform (closest point to centre − up · 10).
- Level object tags are in the content (`PlatformDef::tag`,
  `StaticDef::tag`); `object_tag` covers level objects; the blink gun's
  `InvincibilityZone` test is the real tag test.
- Usage in the combined run (35 000 ticks): platform rope hand-over 429
  anchor-ticks, ignited clouds 434, roped rock 489, Throw carrying a spike
  or coil 908 ticks; magnet item holds and Throw carrying an engine did not
  occur there and have scenario tests in `tests/abilities_interactions.rs`;
  rope anchors on black holes, magnet-held boulders and carried rope
  anchors occurred in neither.

## Data checked

- No object of the game's data is on the **orb** layer (all 124 levels and
  all prefabs scanned): the orb-type branches of black hole, blink gun,
  growth ray and duplicator never apply; left as comments.
- No level object is tagged `InvincibilityZone`, `ResizablePlatform` or
  `boulder`; the latter two are prefab roots (already in the import). The
  import format did not change (tags were already imported; only the
  content passes them on), so `FORMAT_VERSION` stays 1.

## Spec fixes

| File | Was | Now |
|---|---|---|
| physics.md §2.4 | BoplBody "scale changed since last tick" (initial value unstated) | `prevScale` starts at 1, Init is empty, composite rule, hold counter semantics |
| player.md §6.6 | only the exit-record `SyncPhysicsTo` | the entry `SyncPhysicsTo(PlayerInfo)` of ability bodies; the drop-through record of ability bodies |
| player.md §5.5 | recursion "with the remaining movement" | the remaining movement is multiplied by `dt` again in the recursion (G3) |
| player.md §2.2 | — | `JoyStickAngleByteFromVector` does not normalise; callers pass unit vectors |
| engine.md §7.1 | ability-object bounds exit cause unstated | default cause Other (G2, G4) |
| mine.md (Shockwave) | — | transform positions, ignore list, spawner-scaled forces (also the player push), stand-alone shockwave prefabs |

## Small items

- `platform.rs` prevScale at Init: fixed (row 11).
- Golden vectors: `fix_vectors` / `phys_vectors` find the vectors through
  `BOPL_ROOT` when the checkout has no `scratch/` (they silently skipped in
  worktrees); all pass.
- `angle_byte_from_vector` stays literal (the original never normalises and
  its callers pass unit vectors); `angle_byte_from_direction` normalises
  for tools (`PlayerInput::with_direction`, `sim-trace`, the G2 missile
  test).
- `clone_and_step_cost`: best of five batches, bound 2 ms,
  `BOPL_SKIP_TIMING=1` skips it.

## Remaining gaps

- Platform-kind prefab objects (Throw boulders, the ResizablePlatform) are
  built by their modules through the platform constructor rather than
  `spawn_with_parts`.
- Two component models coexist by design (components run inside `tick` or
  as parts); both reproduce the order, but a new module has to pick one.
- Presentation: spawned objects and ability-made platforms have no sprite
  in `View` yet (G4 item); bounds effects and magnet sorting layers are
  cues only.
- The duplicator's newer platform behaviour (boulder clones) is not shipped
  in the game and not implemented (G1).
- Scenario coverage of the new wiring is partial (black holes, shockwaves,
  holds, prevScale, magnet item, Throw + engine). Rock ropes, engine flame
  ignition and the Platform rope hand-over are only exercised by the random
  12-player runs; black-hole rope reel-in, magnet-held boulders and carried
  rope anchors are untested.

# Review log: abilities group G1 (M5)

Blink gun, Growth ray, Shrink ray, Duplicator, Magnet gun, Beam. Spec
errors and omissions found while implementing them (each checked against
the decompilation, then fixed in our own words in the spec file named).

| # | Spec | Was | Now |
|---|---|---|---|
| 1 | blink-gun.md (new section "Ability object player body") | The body components of ability objects (PlayerPhysics, PlayerBody, PlayerCollision, bounds, hurt box, animator) were only referenced ("see player.md"); `SyncPhysicsTo(PlayerInfo)` was not specified anywhere (player.md has the exit-record version). | Section added: registration at instantiation, `SyncPhysicsTo(PlayerInfo)` (grounded path does not check the platform, uses the ability's own radius after `UpdateScale`), which code raises the ground events, the drop-through exit record, `killPlayer` on an ability body (no camera shake, death effect consumes hierarchy numbers), out-of-bounds exit with the **default cause Other**. |
| 2 | blink-gun.md (GunTransform step 5) | `UnGround(nullRotation, no sound)` ambiguous; "active-and-enabled" of what. | `nullRotation = true, sound = false`; the attached `StickyRoundedRectangle`'s `isActiveAndEnabled`. |
| 3 | blink-gun.md (Blink, Water) | "one recursion level". | Shoot always calls Blink with "not a bounce" (the call passes the assignment expression `alreadyHitWater = false`), so the Water branch also runs on the bounce shot; recursion still ends because the bounce shot turns a Water hit into a self hit. |
| 4 | blink-gun.md (QuantumTunnel.Init) | Missing step. | The victim's rotation is re-assigned through the rotation setter, re-deriving `right`/`up` from the stored angle (sim-visible). |
| 5 | magnet-gun.md (RaycastForWalls) | Side ray origins without the `d · s.x` term; side distances without `+ s.x`; centre ray origin not stated. | Full formula (both terms are 0 with the data); centre ray from the body position; a missed centre ray counts as MaxValue in the `min`. |
| 6 | magnet-gun.md (holds) | `BoplBody.Hold`, `IsBeingHeld`, `magnetGunHolder` (stealing) not specified. | Added: `framesUntilDrop`, drop at exactly 0, `IsBeingHeld ⇔ ≥ 0`, stealing through `magnetGunHolder`. |
| 7 | beam.md (Enter step 4) | Orb created "on the first use of this ability instance". | Created whenever no live orb exists (the orb destroys itself after its exit animation, so most uses create a new one), at `FirePos` after the staff direction is set. |
| 8 | beam.md (`orbAnim`) | `DisableIfNotHeld_deter` only sketched; orb animator timing missing. | Update order inside the orb, hold/deactivate/destroy rules, the orb's Explosion does nothing per tick, its animator has no player-id holder (frozen in any time stop), `whiteExit` destroys it. |
| 9 | beam.md (step 7) | "default priority". | Priority 1 (`AddShake` default). |
| 10 | beam.md (IntersectBeams quirk) | "uses the first test's B distance". | It reads the first test's *output*, which `LineToLineIntersection` writes even on a miss ((0, 0) if parallel, else the unclipped `(tA, tB)`). |
| 11 | beam.md (crossBeams step 7) | — | After a head-on crossing the "merged beam" is `a`, so `a` is tested twice; reflection lengths use the stored reflection distance. |
| 12 | beam.md (renderBeams) | "pool grown …, Parent beams extended backwards by 2.5". | Creation path (InstantiateFixed, SetDimensions before `ManualInit`), re-activation, the deactivation loop's stop condition; the 2.5 extension applies to the box only (a copy), impacts use the stored beam. |
| 13 | duplicator.md (DuplicateObject) | **Open question**: per-object-type clone rule. | "Clone rule" section: Unity's Instantiate semantics (serialized fields copied with current values, others reset to their initialisers, enabled/active copied, Awake now, Init next tick) and a per-class table of what carries over (FixTransform, colliders, BoplBody, Item, Grenade, Mine, Missile, Arrow, Projectile, BlackHole, Explosion, DynamicAbilityPickup, SpriteAnimator, platform components, planted terrain). Notable: a clone's `offset` is the original's already-scaled offset while its scale restarts at 1. |
| 14 | duplicator.md (player clone) | "fresh player body …, then ReSpawn, Spawn". | Full sequence: player prefab instance (consumes its hierarchy numbers), empty abilities/indicators, `PrepareToRevive`, `isAbilitylessClone`, `ReSpawn` (which starts the spawn animation, so `Spawn()` fires a second time 106 animator advances later — literal), `Spawn`, `PopIntoAir`. |
| 15 | duplicator.md (DuplicatePlatformEffect per tick) | "recompute the target with the platform temporarily at effectScale". | `ScaleEffect` really sets the platform's collider scale and sets it back (setter round trip), re-reads extents/radius at that scale; `PlatformSpawnPos` threshold is `One / 20` (raw 214748365) vs the shot's `f005` (raw 214748368). |

## Port notes (not spec errors)

- Ability objects get their extra components through `AbilityImpl::build`
  (prefab hierarchy numbers), per-part dispatch `update_part` / `init_part`;
  the shared body is `blink_gun::AbilityBody`.
- Cross-module component interfaces (black holes, spells, held items, item
  kinds, duplicated-clone fix-ups) go through `abilities::Interaction`; the
  owning modules (black hole, grenade, mine, missile, rock, engine, spike,
  tesla coil, revival/teleport spells) answer them. Until they do, those
  effects are no-ops (e.g. Growth ray on a black hole, magnet on a missile).
- Per-module clone resets: `SpawnedImpl::duplicate` (default copies all
  state); the pickup's is implemented (`initSelf`).
- `BoplBody` hold counters are kept in `ability_globals.magnet.holds` and
  advance once per tick at engine step 8 (the original advances each body
  at its own BoplBody update) — exact only for objects that update after the
  magnet in the list.

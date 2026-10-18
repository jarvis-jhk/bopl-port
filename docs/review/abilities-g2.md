# Ability group G2: spec fixes (M5)

Abilities: Grenade (ThrowItem2), Smoke (+ ring buffer), Mine (+ Explosion /
Shockwave), Missile, Rock, Throw, Tesla coil, Spike. Each entry: what the
spec said, what the reference does (consulted only for that point), and the
fix applied to our spec.

| # | Spec | Issue | Fix |
|---|---|---|---|
| 1 | grenade.md (ThrowItem2 Enter) | `SyncPhysicsTo(entry snapshot)` was used by every ability body but defined nowhere (player.md only has the exit-record overload). | Defined it: grounded branch takes the snapshot platform and parameter and places the body with the ability's **own** radius, keeps the snapshot external velocity and grounded speed, `alignRotation`; airborne branch copies velocities and position, rotation 0. No platform-alive check. |
| 2 | grenade.md (thrown grenade) | `XGunExplosion` was described only by reference to mine.md, whose values are MineExplosion's. XGunExplosion differs (box, trauma) and has **two** Shockwave components (radius 18 and 7, different masks). | Added the XGunExplosion values, both shockwaves, the update order and the animator's id holder. |
| 3 | grenade.md (edge cases) | Not stated: a grenade with two detonating contacts in one physics step explodes twice (callbacks do not check `IsDestroyed`). | Added (literal behaviour, also missiles). |
| 4 | mine.md (Enter) | Listed `direction = UP` first and the input-direction update later; the reference sets `direction = UP` (and `hasFired = false`) **after** the update, overwriting it. No observable effect (the direction is recomputed every update before any Fire), but the order was wrong. | Reordered; noted the dummy is a plain Unity instantiate (no hierarchy numbers). |
| 5 | mine.md (collision callback) | Missing condition: LethalTerrain contacts count only if the other object is tagged `ChainLightning`. | Added, with the note that LethalTerrain is not in the data's Igniters mask, so the clause never applies. |
| 6 | tesla-coil.md (Interactions) | Claimed live coils (LethalTerrain + ChainLightning tag) trigger mines. They do not: LethalTerrain is not in Igniters (262912 = Player, Projectile, Explosion). Only the beam (layer Explosion) does. | Corrected. |
| 7 | mine.md (Shockwave) | `ActivateOnStart` read as meaningful; it is never read (Init always fires). The Shockwave is the object's `IPlayerIdHolder`; a prefab may have several. | Documented. |
| 8 | mine.md (scan) | "carries no player id" was ambiguous; the test is "has no `IPlayerIdHolder` component"; a holder whose id names no player is skipped. | Clarified. |
| 9 | smoke.md (burst) | Did not say the four `SmokeExplosion 1` visuals are spawned with `InstantiateFixed` (they consume hierarchy numbers). | Added. |
| 10 | smoke.md (clouds) | Not stated: every igniting contact resets the ignition timer; the cloud animator has no player-id holder (frozen in any time stop). | Added. |
| 11 | tesla-coil.md | Coil/beam height relative to bopls unstated (standing bopls pass under; found while writing the scenario test, confirmed by the offsets). Reposition order on platform rescale unstated. | Added both. |
| 12 | spike.md | `SetRelativePositionOfChildBody` / `SetRelativeOrientationOfChildBody` were used but not defined: `relativePos := p + ComplexMul(r, transform.offset)`. The rescale reposition runs before the platform's shape is rescaled. | Defined; documented the order. |
| 13 | rock.md (edge cases) | `exitScale` is stale (0 before the first exit phase) if the rock dies out of bounds; the exit then sets scale 0 → clamped to 0.3. | Added. |
| 14 | missile.md (edge cases) | The drop-through exit continues into the steering step; the airborne exits bypass SparkShot's exit. | Added. |

Cross-cutting notes (not in G2 files, for the owner of the shared specs):

- engine.md §7.1: the ability-object out-of-bounds exit leaves `causeOfDeath`
  at its default (`Other`), unlike the bopl path (`Accident`).
- physics.md §2.3: `AddForceAtPosition` in Force mode (spike knock-back,
  cast pushes) is an instantaneous impulse without the ×60.
- abilities.md §14 lists Explosion/Shockwave under mine.md; XGunExplosion's
  values now live in grenade.md.

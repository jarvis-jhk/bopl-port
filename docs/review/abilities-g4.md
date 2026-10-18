# Spec review: G4 abilities (M5)

Bow, Black hole, Time stop, Invisibility, Revival, Platform, Gust, Push,
Random — implemented in `crates/sim/src/abilities/` from the specs; the
decompilation was consulted only where a spec was ambiguous, and the spec
was then fixed (2026-10-17). One line per fix: file, what was wrong, what
is right. Severity as in the spec review (private) (HIGH = wrong game behaviour, MED =
bit-exactness / state-machine detail, LOW = wording / missing detail).

## Spec fixes

| Sev | File | Was | Now |
|---|---|---|---|
| HIGH | revival.md (ReviveParticle) | "each tick" one movement step | `InitRevive` registers the particle's updatable a second time (outside the spawn bracket, same offset hierarchy number), so `UpdateSim` runs twice per tick: 2 · speed · pdt per tick (60 units/s). Respawn delay formula updated. |
| MED | platform.md (Size, area) | `area = 4hr + 4wh + 4wr + πr²` | exact evaluation order `((2·h)·r)·2 + ((2·h + 2·r)·2)·w + (π·r)·r` (physics.md §12.6); GetNewSize products spelled out |
| MED | platform.md (Resize) | `area(w, h, r)` for the inverse mass | the wrapper calls `ResizePlatform(h, w, r)` and PlatformArea receives the height first: `area(h, w, r)` (not symmetric in Fix rounding); MOI product order |
| MED | platform.md (Size step 4) | `g = min(...) ⊙ sign` | `g0 := 0` when `|g0|` is 0 (magnitude underflow), then the sign product |
| MED | bow.md (per tick 6–7) | "clamp to 9", "air control blend 0.5 (as GunTransform)" | `if |v| > 9: v := normalized(v) · 9`; fired: `v0 = v; AirealMovement(InputVector.x); v := Lerp(v0, new, 0.5)`; same normalise rule for 19 |
| MED | bow.md (per tick 4) | landing exit "framework exit directly" | steps 5–8 still run on the exit tick (Move, DropPlatformTest: a possible second exit, no guard) |
| MED | bow.md (Enter) | `SyncPhysicsTo(snapshot)` undefined (player.md has only the exit overload) | the PlayerInfo overload written out (grounded: local position with the ability body's radius, external kept, selfImposed 0, groundedSpeed, alignRotation; airborne: copy, rotation 0) |
| LOW | bow.md (Enter) | — | the gravity modifier persists across casts of a slot (0.03 or 1) |
| LOW | bow.md | — | grounded shot recoils into the platform: the bow re-lands (justlanded) on the shot tick |
| LOW | bow.md (Exit, Shoot) | "unground (no sound)"; ring buffer of arrows | `UnGround(nullRotation, no sound)`, it is also the death/bounds/drop exit; the ring buffer holds the arrows' BoplBodies |
| MED | gust.md (Shockwave) | "the push stops on the update whose timer exceeds 0.25" | 16 pushes (Init + 15 UpdateSims); the 16th UpdateSim destroys the gust |
| MED | gust.md (Shockwave) | — | hits use the hit's and the shockwave's **transform positions**; duplicates / destroyed / collider-less hits skipped; player and lethal-terrain pushes only at activation |
| LOW | gust.md (QuickSpell Exit) | "unground (no sound)" | `UnGround(nullRotation, no sound)`; also the death / bounds exit |
| MED | time-stop.md (push-out, squish) | `CircleCastAll(position, collider radius, …)` | centred on the transform position (no (0, 1.77) offset) with the hurt circle's world radius; the CollisionTest uses the registered circle (offset centre); the squish point test uses the transform position |
| LOW | time-stop.md (End) | "unless the game has ended" | = `HasGameEnded()` = round no longer in progress |
| MED | black-hole.md (gravity) | skip only `d = 0` | also skip when `d³` underflows to 0; lists circles → boxes → rounded rects; hole position = its transform; range uses the hole's current radius |
| MED | black-hole.md (gravity) | force "added to the collider's body" | added to the collider's **own** list entry (no composite redirect: welded circles/boxes get nothing, rounded rects feed the composite) |
| LOW | black-hole.md (gravity) | Player-layer boxes | the code also tests a non-existent layer "Ability" (−1): only Player |
| LOW | black-hole.md (eating) | — | ability-object kill: position = its transform, cause Other; plain player: `killPlayer(owner, no effect, ignore invulnerability, Other)` |

Checked and found right: invisibility.md (end handling: smallest timer
restarts, every slot still at 10000 gets 6), push.md (Force-mode
AddForceAtPosition, home lerp/clamp, release paths), time-stop.md (691st
sphere update with f32 accumulation — verified by the test
`time_stop_lasts_691_sphere_updates`), abilities.md §13 Random.

## Items for specs owned elsewhere (not edited)

- player.md §6.6: add the `SyncPhysicsTo(PlayerInfo)` overload (now in
  bow.md Enter) next to the exit overload.
- player.md §6.4 / abilities.md §8.3: `DropPlatformTest` inside an ability
  calls the first IAbilityComponent's `ExitAbility(record)` with the drop
  record; every component exit overwrites the record's movement fields
  from its own body, so the drop record only contributes its defaults.
- engine.md §7.1: the self-destruct of an ability object passes a record
  with the default cause (`Other`), not `Accident`.
- mine.md (Shockwave): the same transform-position / once-at-activation
  details as gust.md above apply to MineExplosion (CallEveryFrame false:
  one `UpdateHitObjects` from Init, then only UpdateSim's `killAfterOnFrame`).

## Implementation notes (shared pieces other groups can reuse)

- `abilities/ability_body.rs` (new): `AbilityBody` = the ability prefab's
  PlayerPhysics (its own tuning), PlayerBody, PlayerCollision kill rules on
  the hurt box (`handle_hurt_contact`, `kill`), DestroyIfOutsideSceneBounds
  (`out_of_bounds`, also for spawned objects), SpriteAnimator wrapper;
  `spawn_prefab` (InstantiateFixed of a root-object prefab with parts by
  component class and correct hierarchy-number consumption);
  `add_force_at_position`; prefab/animation lookups.
- `gust.rs`: `QuickSpell` (also used by Revival) and the full `Shockwave`
  (`spawn_shockwave`) — usable by Mine/Grenade explosions.
- `bow.rs`: `RingBuffer` (smoke.md semantics), `body_scale_tracking`
  (BoplBody inverse-mass rescale on scale change).
- `black_hole.rs`: `add_force`, `add_force_less_mass_influence`, `grow`,
  `mass`, `spawn_hole` for beams, rays, magnet gun, duplicator.
- `revival.rs`: the death-with-markers branch, clones, `ReviveEffect`,
  `ReviveParticle`; `time_stop.rs`: `fix_to_f32`.

## Open questions

- Orbs (layer orb) of type shrink/grow/blink are exempt from black-hole
  gravity/eating in the original; no orb objects exist in the port yet, so
  the type test is not implemented.
- Shockwave on LethalTerrain: "no IPlatformAttachable" is approximated as
  "no parent object" until such objects exist.
- Ropes: Platform's rope hand-over at CreatePlatform and every PlayerBody
  rope branch are not implemented (grappling-hook group).
- Presentation: spawned objects and the ability-made ResizablePlatform have
  no sprite in `View` yet (`View` only lists level platforms; Behaviour
  sprites are `NONE`).

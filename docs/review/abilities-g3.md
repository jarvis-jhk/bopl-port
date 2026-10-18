# Review log: abilities group G3 (M5)

Dash, Roll, Teleport, Grappling hook (with the rope system), Engine, Drill,
Meteor. Spec errors/gaps found while implementing, each fixed in the
ability spec; the decompilation was consulted only to resolve them.

## Spec fixes

1. **dash.md — finish target.** The no-wall finish computes the target from
   the body position at the finish tick (`body + d·13`, water clip with
   `body.y`), not from the fire origin. The two are equal unless something
   moved the body during the hold (a rope pull). Also noted: the enter's
   `SyncPhysicsTo` leaves a grounded entry's platform referenced in the
   exit record; the dasher's self-contact has no collider shape; the fall
   state entered by the exit re-faces along the held stick.
2. **grappling-hook.md — rope/platform contact.** "Closest surface point"
   is the physics `RoundedRect.GetClosestPoint` (physics.md §9.4), not the
   platform-surface version (player.md §5.4); containment is the rounded
   rect `OverlapPoint`.
3. **grappling-hook.md — PlayerBody targets.** Both "use rope on other
   player" and the hook's Player-layer hit apply to any body with a
   `PlayerBody` (bopls and ability bodies), stated explicitly.
4. **grappling-hook.md — pool references.** Rope references are pool
   slots; a stale reference to a disabled slot that was re-allocated sees
   the new rope (the port keeps pool indices to reproduce it).
5. **engine.md — `SetRelativePositionOfChildBody`.** Not defined in
   physics.md: the member body's `relativePos := rel + ComplexMul(relRight,
   box transform offset)`.
6. **engine.md — Init scale.** The engine box gets the scale setter twice
   (owner scale, then the clamped value), each a `(One/s)·s` factor.
7. **engine.md — flame animator / parenting.** The child animator drives
   StartEngine/TurnOff and freezes in any time stop (no player-id holder);
   the engine object is parented to the platform (destroyed / deactivated
   with it).
8. **teleport.md — skipped lethal terrain.** Skipped is LethalTerrain
   carrying an `IPlatformAttachable` component, whether attached or not.
   The Player-layer handling detaches the rope at the body's own end and
   ungrounds first (a grounded body moves `normal · 0.05` before the swap
   offset). The ghost shakes (one PRNG pair) also on the tick it ends.
9. **drill.md — hitbox mirroring.** A hitbox mirrors the combo's active
   state only while it is itself active (inactive objects do not update).
10. **meteor.md — smear extents.** The box is initialised with the prefab
    extents and the extents are read back (`One/ie`) before the new ones
    are written; the offset ratio uses that read-back y, not literal 4.
11. **roll.md — hop recursion.** `MoveAlongCurrentPlatFormSwapIfBlocked`
    passes the remaining movement (already × dt) into the recursion, which
    multiplies by dt again; documented with the probe origin/direction.
    player.md §5.5 ("recurses with the remaining movement") should carry
    the same note (not a G3 file; not edited).

## Port decisions (no spec change)

- **Ability bodies** (`abilities/body.rs`): `PlayerPhysics`, `PlayerBody`,
  `PlayerCollision` (state in `Framework::collision`) and the bounds check
  of ability objects are the bopl algorithms with the ability prefab's
  values. Ability components run inside the single `Ability` part in their
  prefab hierarchy order (an object's numbers are contiguous, so this is
  the original order); the collider is its own part (registration order,
  ignored-collider hierarchy numbers).
- **Instant casts** run in `AbilityImpl::update` (the framework's existing
  convention for instant abilities).
- **Deferred caster-body writes** of the grappling hook cast (the caster's
  payload is taken out while it casts): applied right after the cast,
  before anything reads the body.
- **Rope pool** lives in `Physics::ropes` (DetPhysics owns it); the global
  rope counter survives level loads (`Physics::clear`), like the
  original's static.

## Open items (cross-group)

- Grappling hook on a **Rock** (`BounceBall.AttachRope`) and the anchor's
  **black-hole** reel-in (`BlackHole.GetMass`, swallow) need the Rock and
  Black-hole modules; not wired (the hook attaches to them as a plain
  non-platform anchor).
- **Rocket engine smoke ignition** (`FlammableSmoke.Ignite`) needs the
  Smoke module; not wired.
- **Shockwave / Explosion**: Meteor uses its own implementation of the
  shared Shockwave (`meteor::Shockwave`, mine.md + gust.md semantics) and
  a lethal smear object; the canonical modules (mine.rs / gust.rs) should
  absorb or reuse it. Black holes receive no shockwave push yet.
- Teleport's "has a sprite renderer" and the platform-attachable test use
  the spawned object's prefab (`Visual::Prefab`) components; objects of
  other modules must set their `visual` for this to work.

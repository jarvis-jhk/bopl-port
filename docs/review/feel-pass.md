# Review log: feel pass against the original (M9)

Our simulation and presentation compared with the original Bopl Battle,
frame by frame, from scripted replays the original's replay viewer plays.
Tools: `scripts/dev/golden/` (how to run: `docs/reference-capture.md`,
"Golden comparisons"). Captures of the original stay in `scratch/golden/`;
nothing from them is committed. The expected numbers in
`crates/content/tests/golden_expected.txt` come from our sim only.

## Method

1. **Scenario → replay.** A scenario (`scripts/dev/golden/scenarios/*.txt`)
   names the level, seed, up to 4 players (colour, team, abilities) and
   per-player input steps; `golden gen` writes the `.rep` (local-game header
   conventions: delay 0, demo mask 255, keyboard flags 0, colours chosen to
   be easy to track: violet 1, red 4, yellow 0, orange 7).
2. **Our side.** `golden sim` runs the replay through our sim with the
   viewer's input rule (frame *i* = tick *i*; during a time stop only
   protected players get new input) and writes per tick every player's
   position, bopl transform, up vector, scale and the modelled camera, plus
   the events.
3. **The original.** `capture.sh` starts the original with the `.rep` on the
   private Xvfb (`:66`, Steam mode: the viewer needs the Steam API) and
   records 1920×1080 at 60 fps with ffmpeg.
4. **Tracking.** Per frame, the camera x is measured by matching the static
   platform pixels of our renderer's level screenshots (camera x 0 and 10:
   pixels that move with the camera are platforms, pixels that stay are
   backdrop) at ±24 px around the previous frame's shift, with a sub-pixel
   fit (18 px per unit: camera size 30, y fixed at 8). Each player's colour
   (`_ShadowColor` of its material, read from the import) is segmented
   (closing fills the eyes, opening removes slime trails and icon strokes,
   close pieces are grouped); groups containing eye whites rank first; for
   up to six groups: centroid, bounding box and the "caps" (centre of the
   pixels within 14 px of each box edge).
5. **Comparison.** The bopl's head is the box edge along the sim's up vector
   (the slime splat lies on the surface side) and the cap next to it; a
   fitted offset (≈ 0.9 units × scale) maps it to the sim position. Frames
   are aligned to ticks by dynamic time warping: the viewer runs 0..4 ticks
   per rendered frame and the original renders about 25 fps here, falling
   behind real time below 15 fps (the report shows the playback rate; a
   capture slower than 45 ticks/s is flagged SLOW). The alignment pays 1 per
   tick of deviation from real time and ignores residuals under 0.6 units,
   so pose-dependent blob biases cannot warp it. Reported per player: mean
   residual and the share of frames above 1 and 2 units after the spawn
   (from tick 172), the first tick where the median of 30 sightings exceeds
   0.6 units, and for deaths the last tick the original still shows the
   bopl at the position the sim had before dying.
6. **Diagnosis.** `bopl-port --headless --replay FILE.rep` plays the same
   replay in our game; `scripts/dev/golden/sheet.py` puts our frames next to
   the original's aligned frames.

Precision: about ±0.2 units for a bopl in the open, ±0.5 on walls and in
slime-covered spots, a few frames of ±1 during landing squash and ability
poses. Ability objects (guns, boulders, balls) are tracked only where they
keep the player colour; ability effects are judged from the players'
reactions (pushes, deaths, scale) and side-by-side frames.

## Tick correspondence

- **Frame *i* of a replay is the input of the original's tick *i*** (its
  updater counter reads *i* during that tick) **= our step that takes
  `simulation_ticks` from *i* to *i + 1***. sim-trace and golden print the
  counter after the step, so "our tick *n*" consumed frame *n − 1*; a
  `TimestampWhenKilled` of *i* is the same number in both games. Why: the
  Replay scene re-initialises the updater and loads the level in the same
  update; the first playback update runs in the loaded level and consumes
  frame 0 on the level's tick 0 (net-original.md §8, now verified).
- **Measured** with an input-independent clock: `s02-orbit-timing` (Level12,
  every platform orbits; the bopl only jumps, at irregular intervals, so it
  rides different platforms depending on the exact jump tick). Our sim with
  the inputs shifted by −2..+2 ticks against the capture (vertical residual
  only, the camera cannot be measured there):

  | input shift | mean residual | > 1 unit | > 2 units |
  |---|---|---|---|
  | −2 | 0.77 | 28.4 % | 14.1 % |
  | −1 | 1.14 | 38.8 % | 22.6 % |
  | **0** | **0.18** | **3.6 %** | **0.0 %** |
  | +1 | 0.79 | 26.0 % | 15.1 % |
  | +2 | 0.90 | 31.2 % | 16.5 % |

- **Fly-in** (input independent, uses the per-platform random delays): the
  original's frames 300..470 of `1.rep` matched against our renders of
  every second tick pick our ticks 1..167 monotonically with frame − tick
  299..302 and mean pixel differences of 0.6..1.6 (the next-best tick 2..15):
  platform positions, the overshoot curve and the RNG-drawn delays are the
  same tick for tick.
- **Static periods**: when nothing moves for a few seconds, neither the blob
  alignment nor whole-frame matching can tell ticks apart, and a slow
  playback in that period shifts the alignment by up to ~20 ticks (seen in
  `a29-duplicator`); a scenario with a player walking back and forth as a
  clock (`a29b-duplicator-clock`) removed the apparent difference. Timing
  checks need motion in view.
- **Real-time rate**: the viewer runs the ticks the elapsed time asks for,
  at most 4 per rendered frame. The original renders about 25 fps under
  Proton on our Xvfb (2–3 ticks per frame) and less when the machine is
  busy; below 15 fps it falls behind (one early capture, `m02-rotated`,
  played at 24 ticks/s while the machine was loaded: the alignment still
  matched it, mean residual 0.39). Captures now wait for a quiet machine.

### The real end of `1.rep`

The file has 6656 frames, the original's round end was only known to be
≤ 6655. The capture follows our trajectory to the end (mean residual 0.28
units over 6412 sightings, 0.2 % above 2 units, playback 60 ticks/s): the
bopl leaves the wall at tick ~6600, falls into the water and the original
shows it last at its pre-death position on our tick 6635 (sim death event
on our tick 6637). So: **killed on the original's tick 6636 (frame 6636,
`TimestampWhenKilled` 6636, water), round decided on tick 6637 (frame 6637),
6638 recorded frames + 18 padding = 6656** — exactly what our sim does
(death tick 6636, round won event on our tick 6638, winner screen at our
tick 6760). The STATUS note "death 6636, round end 6637" was right.

## Results per scenario

Residuals after the spawn (tick ≥ 172): mean distance in units between the
tracked head and our position (with the fitted offset), share of
sightings above 1 and 2 units. "Visual" = side-by-side frames
(`sheet.py`) at the ticks of interest. Before = after for the sim (no sim
code changed); the presentation fixes are listed below.

### Mechanics

| Scenario | What | Players: mean / >1 / >2 | Deaths sim vs original | Verdict |
|---|---|---|---|---|
| `1.rep` (real match, solo) | Level1, Bow/Gust/Dash cast, walking on walls, water death | 0.28 / 0.4 % / 0.2 % | 6637 vs 6635 (last sighting) | same |
| `s01-jumps` | jumps after the spawn fall, walking, corner walk to a wall | 0.24 / 2.9 % / 0.2 % | – | same (wall pose seen identical) |
| `s02-orbit-timing` | orbiting platforms (Level12), irregular jumps | 0.18 (vertical) | – | same; tick correspondence above |
| `m01-walk-corners` | rounded corners, underside, wall jumps, water | 0.29 / 3.1 % / 1.1 % | 881 vs 879 | same |
| `m02-rotated` | rotated platforms (Level2), slopes, 2 players | 0.25 / 1.2 % / 0.1 %; 0.42 / 13 % / 0 % | – | same (P2's > 1 frames: blob merged with the trail on a corner) |
| `m03-rotated-long` | long rotated platforms (Level21) | 0.33 / 0 / 0; 1.09 / 64 % / 0.8 % | – | same (P2 rests on a trail-covered corner: tracker offset 2 units; frames identical) |
| `m04-space-robots` | space gravity, free robot platforms (Level_new1) | 0.17 / 0.3 % / 0; 0.18 / 2.2 % / 0 (vertical) | 438 (P2) | same |
| `m05-space-asteroids` | round asteroids (Level35), low-gravity jumps | 0.24 / 0.5 % / 0; 0.27 / 3.0 % / 0 | 480 (P2, off-screen) | same |
| `m06-ice` | ice platforms (Snow/Level15): sliding, stopping, turning | 0.23 / 0.8 % / 0; 0.21 / 0 / 0 | P1 772 | same (capture partly slow: playback dipped to 9 ticks/s, alignment held) |

### Abilities (Level1; P1 left upper, P2 right upper, P3/P4 on the main platform)

Players: mean residual / % above 1 / % above 2 units. Deaths are our
event ticks (the original's last sighting near the position before death
is noted where it is reliable; paint splats in the player's colour make it
unreliable for kills). Every row's frames around the cast and the effect
were also looked at side by side.

| Scenario | Cast | Players | Deaths | Verdict |
|---|---|---|---|---|
| `a02-dash` | dash right, back, up-right | P1 0.22/2/0; P2 0.17/0/0 | – | same |
| `a03-grenade` | throw at P2, second throw | P1 0.27/4/0; P2 0.17/1/0 | P1 725 (own grenade) | same (explosion and death on the same tick, platform tilt the same) |
| `a04-bow` | draw 40, shoot right; second shot | P1 0.25/6/0; P2 0.16/0/0 | – | same (> 1 = bow pose) |
| `a05-engine` | plant engine, platform pushed | P1 0.18/0/0; P2 0.18/0/0 | – | same (re-captured: the first capture ended early) |
| `a06-blink-gun` | blink shots at P2 and down-right | P1 0.21/2/0; P2 0.24/6/0 | – | same |
| `a07-gust` | gust above P3, again after walking | P1 0.27/2/1; P2 0.17; P3 0.34/6/4; P4 0.19 | P3 479 (original 484) | same |
| `a08-growth-ray` | grow P2, then P4 | P1 0.20/2/0; P2 0.70/26/0 | – | same (P2's offset scales with its size; frames identical) |
| `a09-rock` | boulder rolls off onto the main platform | P1 0.30/7/0; P3 0.28/3/3; others ≤ 0.20 | – | same |
| `a10-missile` | missile right, steered down | all ≤ 0.23 (P3 1 % > 2) | – | same |
| `a11-spike` | spike through the platform onto P3 | all ≤ 0.23 | P3 371 | same (spike and death on the same tick; the original's yellow splat stays visible) |
| `a12-time-stop` | 9 s channel, walk in the stop; P2 frozen | P1 0.68/24/22 (channel aura over the bopl); P2 0.26/2/0 | – | same channel, cast and frozen P2; see "Time stop" below |
| `a12b-time-stop-clock` | as a12, the caster walking back and forth through the end of the stop, P2 walking after it | P1 0.62/20/10 (aura; inverted colours during the stop); P2 0.34/6/1 | – | same: P2 starts moving on the same tick (±1) after the 690-tick stop; the end fade matches (see "Presentation fixes") |
| `a13-smoke` | smoke grenade, P2 walks through | P1 0.16; P2 0.20 | – | same |
| `a14-platform` | rotate, stretch, drop a platform, walk on it | P1 0.75/5/4; P2 0.17 | – | same (P1 as a platform cursor is not tracked as a bopl) |
| `a15-revival` | marker, walk down to the water | P1 0.20/2/1; P2 0.17 | – | same (marker planted on the same tick; P1 sticks to the wall instead of drowning, so the revive itself is not exercised) |
| `a16-roll` | P3 charges 70 ticks, rolls into P4 | P3 0.73/13/10 (ball); P4 0.33; P1/P2 ≤ 0.20 | P4 459, P3 463 | same: whole-frame matching puts the launch on our tick 451 in both (the blob alignment had drifted 6 ticks); both die on the same ticks |
| `a17-shrink-ray` | shrink P2, then shot at P4 | P3 0.58/15/15 (yellow ray particles); others ≤ 0.28 | – | same |
| `a18-black-hole` | black hole thrown between the players | P1 0.23; P2–P4 ≈ 1.0 (static offset next to the hole) | P1 497 (original 495) | same (hole position, eaten platform, P1 eaten on the same tick) |
| `a19-invisibility` | vanish, walk, reappear | P1 1.11/20/18 (invisible: nothing to track); P2 0.19 | – | same (fade out and in on the same ticks) |
| `a20-meteor` | jump, slam down near P3; again | all ≤ 0.25 | – | same |
| `a21-throw` | rip a boulder, throw it at the top platform | P1 0.45/11/6 (carry pose); P2 0.16 | – | same (platform shrink, boulder flight and bounces) |
| `a22-push` | P3 pushes the main platform | all ≤ 0.28 | P3 359 (squished) | same |
| `a23-tesla-coil` | two coils, P4 walks into the beam | all ≤ 0.22 | – | same |
| `a24-mine` | P3 places a mine, P4 walks over | P3 1.40/76/1, P4 0.65 (both on the trail-covered corner, merged); P1/P2 0.16 | – | same |
| `a25-teleport` | marker, walk, swap back | P1 0.34/3/2; P2 0.19 | – | same |
| `a26-drill` | drill down through P1's platform | P1 0.23; P3 0.39/7/6; others ≤ 0.18 | P3 477, P1 523 | same |
| `a27-grappling-hook` | hook the top platform, swing, release | P1 0.22; P2 0.18 | – | same |
| `a28-beam` | beam at P2, then down-right | P3 0.38/8/8 (beam glare); others ≤ 0.23 | – | same |
| `a29-duplicator` | copy P2, second shot down-right | P1 0.21/3/0; P2 0.19/1/0 | – | same; the second cast first looked 15–20 ticks late in the original, an alignment drift while nothing moved (300 static ticks) |
| `a29b-duplicator-clock` | as a29, plus P3 walking back and forth the whole time as a clock | P1 0.18/1/0; P2 0.16/0/0; P3 0.50/10/6 | – | same: gun at 750, fire 775, platform clone 790–810 on the same ticks; cooldown fill identical every frame |
| `a30-magnet-gun` | grab P2 and pull | P1 0.27/5/2; P2 0.16 | – | same |


### Presentation and flow (item 5)

| Aspect | Original | Ours before | Ours after |
|---|---|---|---|
| Fly-in | platform poses per tick | identical (see above) | – |
| Round end → winner screen | winner screen activation 2 s after the round is decided (our tick 6760 in `1.rep`); the winner's face slides in ~34 ticks later | same tick; face slide within ±3 ticks | – |
| Winner / loser circle | slides up after `ShowUpDelay − x/DelayDifferenceDenominator` = 2 s − x/1000 (rightmost first), along `moveUpAnim` (Hermite 2t − t² over 1 s): `1.rep` circle first visible at our tick ~6780 | slid up at once (0.08 s per slot, linear 0.5 s): visible from the round end (tick 6638) | 2 s − x/1000 delay and the authored curve: visible at 6780, side by side identical |
| Camera follow | rule = ours (same code path: min/max of alive players incl. 0, `1.5·min(dt, 0.0667)` smoothing, clamp, 1/60 snap); measured camera vs our 60 fps model: median 0.24 units — the difference is the 1/60 snap stalling at a frame-rate dependent distance (the original rendered ~25 fps) | same rule | – |
| Ability cooldown indicator | radial fill = timer / cooldown; on ready a size pulse, no colour flash | fill identical tick for tick; pulse late and long, then pale for 20+ ticks; brownish fill; circle 37 px instead of 34 | fixed (see "Presentation fixes") |
| Time stop | stop and inversion end fade | – | measured with `a12b`: same end tick, same fade (see "Presentation fixes") |

Other presentation differences seen side by side in the first pass (death
splats, spike sprite, black hole glow and swallow particles, meteor dust,
teleport swap, "WINNER!!" spacing, indicators during invisibility): see
"Presentation fixes" below for what each turned out to be.

## Fixes

- **Spec** `docs/spec/net-original.md` §7–§8: replay playback verified
  (needs the Steam API), the tick correspondence, the viewer's rate and its
  time-stop input rule, the recorded length (decided tick + 1 frames).
- **Replay runner** (`bopl_content::replay::Replay::viewer_inputs`, used by
  sim-trace, golden and the new test): while time is stopped only players
  protected from it get new frames (the viewer's rule); before, a frozen
  player's input changed under the time stop in our replays.
- **Winner / loser circle timing** (`crates/game/src/match_ui.rs`, the one
  game-side fix): delay `2 − x/1000` s and the authored `moveUpAnim` curve
  instead of 0.08 s per slot over 0.5 s (match-flow.md §11.2 already had it
  right).
- **`bopl-port --replay FILE.rep`** (`crates/game/src/main.rs`,
  `game_app.rs`): plays one of the original's replays (for side-by-side
  frames); no other game change.

No simulation divergence was found: every scenario's trajectories, deaths
and ability effects match the original within the tracking precision, and
the frames compared side by side show the same positions on the same ticks.

## Presentation fixes (M9)

Each difference of the first pass, checked against the original's capture at
the same replay ticks (`scripts/dev/golden/sheet.py`; images in
`scratch/m9/pres/`, scratch only), its cause and the fix. The simulation is
untouched: `golden_expected.txt` only gained the new `a12b` line, all other
scenario hashes are byte-identical, `scripts/check-determinism.sh` agrees
Linux/Windows. Three presentation-only additions to the sim crate's view
code (not hashed): `SlotView::castable` filled in, the invisibility alpha
without the online-only 0.2 floor (the game applies it to local online
players), the black hole's visual size in its look.

| Item | Original (measured) | Cause in ours | Fix | Images |
|---|---|---|---|---|
| Cooldown fill colour | the uncharged sector's pixels × `_CDColor` (rgb and alpha 0.49): circle (33,153,238) over sky → (56,131,128), icon and rim likewise — dark teal | orange `_CDColor` alpha-blended on top (98,133,121) | one-quad `AbilityIndicator` shading (circle, rim, icon, sector multiply), ui.md 5 | `01-indicator-fill-before/after.png` (orig, ours, orig, ours at ticks 560/600); after: (66,140,129) vs (67,142,128) |
| Indicator size | circle 34 px at 1080p = radius `_CircleExtents.x` 1.0 unit × 0.9 | radius 1.15 (37 px) | radius 1.0, rim 0.15 × 0.57 colour | same |
| Reset pulse | `ResetSizeAnimation` + 0.2 in real time (0.25 s); `ResetColor` is **white**, so no flash; the original renders ~12 fps here, so its capture shows the pulse at 2 samples | smoothstep between keys instead of the stored tangents; a white overlay flash over 1 s ("pale for 20+ ticks"); reset on `timer > cooldown` | Hermite with the tangents, no flash, `>=`, reset also on a pickup swap; reset on our tick 681 = the original's tick 680 (same tick as its `ResetAnimation`) | `02-indicator-pulse-before/after.png` |
| Indicators while invisible | Invisibility's own slot is **uncastable** while invisible → alpha capped at `uncastableAbilityIndicatorAlpha` 0.4 (sky blue 205 = 254·(1 − 0.49·0.4)); fully gone with the bopl | the view's `castable` was a stub (`true`); the invisibility alpha had the online-local 0.2 floor for everyone | `castable` from `isAbilityCastable`; floor only for online local players | `03-invisibility-before/after.png` (ticks 292/300/310, zoomed) |
| Black hole size | black disc 33 px at tick 560 (`transform.localScale = s`, s = 100 + mass) | drawn at the prefab's 100 forever (look had no size) | look stretch `s / 100`; radial colour profile now equal to the pixel row (`scratch/pres/bhprof.py`) | `04-black-hole-before/after.png` (ticks 450/480/488/560) |
| Black hole swallow particles | a 1000-particle burst in the platform's shape streaming into the hole | the eaten shape was looked up when drawing, after the platform had left the shape list → default prefab (a few specks and one blob) | shape taken when the cue arrives (the tick before) | same |
| Meteor dust | `MeteorDust` instantiated with `FromToRotation(down, up)` (180°) and `alignToDirection` → frame 0 (a hollow chevron) shows as a V | align-to-direction ignored → upright mound | billboards turn with the emitter's rotation of their emission direction; spike dust (upright emitter) unchanged | `05-meteor-before/after.png` (the original's frames lag ~4 ticks here) |
| "WINNER!!" | bold: +`boldSpacing` 7 × size / 100 per character; no material of its own → the stroked font's material (outline 0.322, black) | no TMP character spacing; outline only from the text's own material | `Text::spacing`, font-asset material fallback (all bold / stroked texts; menus checked against `scratch/ref/01-main-menu.png`, `03-charsel-joined.png`) | `06-winner-before-after-original.png` |
| Roll trail ending | the yellow band wiggles and breaks up (noise strength 2, frequency 0.3, damping) | noise too weak (damped strength not scaled by 1 / frequency) | Unity's damping and position amount, amplitude matched; smoke (`a13`) and growth-ray trail (`a08`) re-checked | `07-roll-trail-before/after/original.png` |
| Time-stop end fade | frame contrast falls from 46 to a flat grey (≈ 8, inverted and normal mixed half and half) and back; ends on tick 1596 (`a12b`: P2 starts moving there) | – | none needed: ours passes the flat point at tick 1536 and ends at 1596, the original's flat point is ~4 ticks off (its frame jitter) | `11-time-stop-end-original.png`, `11-time-stop-end-sheet.png` |
| Death splats | rays, sizes (`bgEffect2` at 30 ppu × (0.5..1)), one-frame lifetime and blending as ours | same geometry (Fix rays identical); the sizes and rotations come from `Random.value` | none: over 7 presenter seeds our yellow coverage is 22–29 k px vs the original's 22 k (main platform); the original sits at the low end of our random spread | `08-splats-ours-vs-original.png` |
| Spike sprite | – | the shaft's texels sample identical (`b17c5b` vs `b17c5b`/`b37b5c`); "lighter" was an earlier thrust frame of the low-fps original | none | `09-spike-ours-vs-original.png` |
| Teleport swap | burst rays, white silhouette (`teleportingObjectEffect`) and orbiting ring, flash particles; the same systems in ours | the capture's alignment drifted ~6 ticks in the idle 20 ticks before the cast, so "flash vs ring" compared different moments; aligned with +6 the sequences match | ring dashes and colours: see "Presentation fixes (prerelease)" | `10-teleport-shift6.png` |

Also from docs/platform.md's gaps: the mouse cursor in the player's colour
(`CursorUpdater`, `12-cursor.png`), wheel-row hover (`13-wheel-hover.png`,
`scripts/dev/m9-wheel-hover.drive`), build-tension rumble (Roll charging,
Meteor, time stop's last 2 s; not testable headless beyond the requests).
Milestone image: `scratch/m9/final.png` (original vs ours at ticks 488 of
`a18`, 446 of `a20`, 1500 of `a12b`).

## Presentation fixes (prerelease)

The minor items left after M9, each checked against the original (captures
of this pass in `scratch/final/ref/`, offline mode on `:66`, except the
teleport replay capture from the M9 pass; images in `scratch/final/`,
scratch only). Simulation untouched: `golden_scenarios` unchanged,
`scripts/check-determinism.sh` 5/5 identical Linux vs Windows; the one
sim-crate change is presentation (the teleport marker's `Look::player`,
not hashed).

| Item | Original (measured) | Ours before | Fix | Images |
|---|---|---|---|---|
| Main menu Exit position | settled: Exit centred under the language button (`scratch/ref/01-main-menu.png`); every list slides in along its own `AnimateInOptions` curve, `optionAnimateDelay` apart, and the list being left falls away along `AnimateOutOptions`, bottom item first (`MainMenu`) | settled pixel-identical, but a linear 3-key slide (the same for every list) left Exit and the language button visibly off-centre in early screenshots (Exit 280 px left at 1 s), and the left list vanished at once | the authored curves and delays per list from the scene, plus the fall-away of the list being left (ui.md 3) | `menu-before-01-menu.png` (old, 1 s), `m3-cmp.png` (settled, original left), `menu-anim-after.png` (slide-in 0.33–1 s, main → settings) |
| Teleport ring dashes | the ring sprites (`ChargeShotAuraSheet`, horizontal glowing dashes) face the camera along the ring: ≈ 2 units long at full height; ring cyan (start colour), only the centre glow (`coreParticleRef`) in lerp(white, owner colour, 0.8) | velocity-aligned quads assumed edge-on: dashes 6 % high; the owner colour on every system of the marker, and the marker had no owner in the view (colour 0: yellow) | Unity's velocity frame (forward = velocity, side axis in the screen plane, 3D rotation Z-X-Y, orthographic projection); colour only on the core system; the marker's look names its caster | `scratch/m9/pres/10-teleport-shift6.png` (before), `teleport-after.png` (ticks 450–530 of `a25`, ours left), `tp450-after.png` (zoom) |
| Winner screen exit on "next level" | the new level loads at once and the banner plays `TriangleOut` over its fly-in: +188 px jump right on the first frame, gone left after ≈ 0.4 s; "ability select" then "next level" slide down (reverse `nextLevel` at speed 2, 0.1 / 0.2 s after the press); the circles slide down, rightmost first (`Disable()`: `moveUpAnim` backwards from `1 − x/1000`) | banner, prompts and circles vanished with the level change | `MatchUi` keeps the winner canvas over the new level until the exit is done; circle slide-down per player (ui.md 8) | `winner-exit-ours-vs-original.png` (ours left at the matching times after the press; original `r12-winner-next.mp4`) |
| Key bindings | per character-select box for the session (`PlayerInit.keybindOverride` → `Player.CustomKeyBinding`), nothing saved | one binding per device type for everybody | per device = per box, saved by device key (serial, else GUID + order, else type), per-type and defaults as fallback (ui.md 6) | `scratch/final/remap/` (`m9-remap.drive`: pad 0 rebound, pad 1 keeps A = jump; log: pad 1's A gives jump + accept, pad 0's A only menu accept) |
| Tutorial HUD | no indicator until a slot is unlocked; camera fixed at x 0 while the bopl walks (`PlayerAverageCamera` disabled); dummies orange | empty cooldown circles with icons for locked slots; the camera followed the bopl (≈ 30 px off at the start); dummy death effect yellow | `locked_slots` (no indicator), `camera_follow` off, player-less effects in the dummy material's colour | `tutorial-camera-hud-ours-vs-original.png` (start and after walking right, ours left), `scratch/final/tut/sheet.png` (orange death effect and splats) |

Not measurable here: the original drew its menus at ~1 fps on this
machine (software rendering, ~10 cores busy), so the menu slide timing
comes from the scene data and the decompiled `MainMenu`, not a capture.
The dummies' death colour was not captured in the original (it needs a
played-through tutorial at ~9 fps); it follows the dummy's material.

## What still differs / open

- Minor presentation: the black hole glow's shading detail (size now
  equal); death splat randomness (by nature); the "ability select" exit
  goes to character select without the original's fade.
- Not covered by these captures: 4-player mid-air collisions, Throw on
  moving platforms, Duplicator clones interacting with ropes, sudden death
  (120 s), more than 4 players (no ground truth exists).
- Tracking limits: bopls merged with thick slime trails on corners and
  ability objects in other colours give pose-dependent offsets of 0.5–2
  units; such frames were checked side by side.

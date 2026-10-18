# Platform layer and renderer

`bopl-render` (crates/render) is the wgpu renderer; `bopl-port`
(crates/game) is the binary: SDL2 window, input and audio, the fixed-step
loop, start-up data flow (install discovery, import) and the level
presentation with the original's look. Both started as copies of
rounds-port's crates (ours) and were adapted to Bopl Battle.

## Running

```sh
source env.sh
cargo build --release -p bopl-port
cargo test -p bopl-render -p bopl-port     # GPU pixel tests need a GPU or lavapipe (else they skip)

target/release/bopl-port                    # window: the game (imports on first start)
target/release/bopl-port --viewer [--level 27]   # window: the M2 level viewer
target/release/bopl-port --headless --sandbox [--level 6] [--script "230:;12:R;3:UJ;40:"] \
    --shots 256,300 --screenshot scratch/m3/s.png       # one scripted bopl, <stem>-<tick>.png
target/release/bopl-port --headless --bot-match --players 12 [--teams 3] [--seed 3] \
    [--level 6] --shots 400,900 --screenshot scratch/m4/b.png [--ticks 3000]
target/release/bopl-port --headless --drive scripts/dev/m4-flow.drive --screenshot scratch/m4/flow.png
target/release/bopl-port --headless --drive scripts/dev/m6-12.drive --screenshot scratch/m6/d12.png
target/release/bopl-port --headless --bot-match --split 1,11 --winner-shots --rounds 3 \
    --screenshot scratch/m7/1v11.png                    # <stem>-winner-<n>.png
target/release/bopl-port --warpzone-host                # online: host a room (also: main menu → online)
target/release/bopl-port --warpzone-join K7QM3X         # join by code, room id or invite link
target/release/bopl-port --net-harness --locals 3x4 --levels 2   # headless online test (docs/net.md)
target/release/bopl-port --headless --level 6 --screenshot scratch/l6.png \
    [--size 1920x1080] [--time 2] [--camera-x -7.1] [--dump-targets scratch/rt] \
    [--overlay] [--music] [--play-sound arrowShoot] [--wav scratch/a.wav]
target/release/bopl-port --headless --level-sheet scratch/levels-gpu.png [--size 480x270] [--cols 12] [--levels 6,27,39]
target/release/bopl-port --list-devices    # GPUs, controllers, SDL audio drivers and devices
target/release/bopl-port --import [--game DIR] [--force]
target/release/bopl-port --data cache/import ...   # use an import dir as is
```

- `--level`: build index 6..129, bag index `b0`.. (`b0` = Level1), or a
  scene name / path suffix (`Level14`, `Snow/Level14`, `espace 3`).
- Game modes (`game_app.rs`): the window starts at the main menu; headless
  runs need `--sandbox`, `--bot-match` or `--drive` (without them a
  headless run is the level viewer, as in M2). Game modes render every
  frame headless (slime trails accumulate on the GPU) and save
  `--shots` after those ticks; `--ticks` stops the run (default: last
  shot + 1; driven runs stop at `quit` or after one hour of game time).
- `--sandbox`: player 1 (yellow, Bow/Gust/Dash) follows `--script`
  (`N:KEYS;...`, keys `L R U D J 1 2 3 S E`, the `sim-trace` syntax).
- `--bot-match`: `bots.rs` bots (seeded, deterministic, from the sim view)
  fight, open the wheel when they lose (odd players) and press jump on the
  winner screen; FFA or `--teams T`.
- `--split A,B,..` (bot match): team sizes in player order (`1,11`,
  `3,3,3,3`; sets the player count); `--teams T` puts player `i` into team
  `i mod T`. `--winner-shots` saves every winner screen 150 ticks after it
  appeared (prompts in) as `<stem>-winner-<n>.png`; `--rounds R` quits after
  the R-th.
- `--drive FILE`: scripted keyboard/controller input through the whole
  game with condition waits (`wait menu|tutorial|charsel|match|round|winner|
  wheel any|level|machines N|busy`) and named shots (`driver.rs`); a `bot
  DEVICE` line lets a bot play that device's player while the device still
  drives menus and the wheel. `key NAME`, `button PAD NAME` and `type TEXT`
  press physical keys / pad buttons (raw presses for key rebinding and text
  entry, and the players' buttons through the current bindings).
  `--realtime` paces a headless run to the wall clock (online from the
  menu). M9 scripts: `scripts/dev/m9-remap.drive` (rebinding a keyboard and
  a controller, then playing with the new keys), `m9-tutorial.drive` (the
  whole tutorial), `m9-online.drive` / `m9-online-reject.drive`
  (`scripts/net/m9-online-ui.sh`: room list, name, lobby, refused join),
  `m9-online-dots.drive` (`GUI_DRIVE=... scripts/net/gui-online.sh`).
  `scripts/dev/m4-flow.drive` walks main menu → character select (km + 2
  pads, colour, team, grid) → 3 rounds → wheel → winner screens → back to
  character select → main menu (~2 minutes, perf-mode).
- Headless runs advance virtual time at `--fps` (60) until `--time` (2 s:
  after the 1 s fade-in) and save the last frame. No window, no audio
  device; `--wav` writes the offline mix. `--dump-targets DIR` saves the
  cameras' render textures (`ReflectionRT.png`, ...).
- Viewer window: Left/Right (arrows, A/D, d-pad, stick) switch levels and play
  `navigateMenu`, Jump plays `slimeJump1`, Esc / Back quits, F11 or
  Alt+Enter toggles borderless fullscreen.
- The game window starts with the settings' display options (screen mode,
  resolution, vsync; borderless fullscreen at first); `--windowed`,
  `--size WxH` (a window), `--fullscreen`, `--no-vsync` override them.
  `--camera-zoom` turns the camera-zoom setting on for a run (bot matches). The overlay (on by default in the
  window) shows the level, connected devices and the last press, in the
  original's TMP font.
- Environment: `WGPU_BACKEND=vulkan|gl`, `WGPU_ADAPTER_NAME=llvmpipe`
  (substring, case-insensitive), `RUST_LOG=info|debug`.

## Game data

- User data dir: `$BOPL_PORT_DATA_DIR` (env.sh: `cache/userdata`), else
  `$XDG_DATA_HOME/bopl-port` / `~/.local/share/bopl-port`,
  `%APPDATA%\bopl-port`, `~/Library/Application Support/bopl-port`
  (`bopl_import::paths`). The import lives in `<data dir>/import-v<N>`.
- Install: `--game DIR`, `$BOPL_GAME_DIR`, `$WARPZONE_DATA_BOPL`, then every
  Steam library of every Steam root (`libraryfolders.vdf`), app 1686940
  first. Verified: both `$BOPL_GAME_DIR` and the Steam library lookup.
- `data.rs` runs `bopl_import::import` (returns in ~0.05 s when up to date)
  and loads `GameData`. Without an install an existing import is used.

## Renderer (`bopl-render`)

The presentation builds a `Frame` of `Pass`es, one per camera of the
original, each with items already sorted; `Renderer::render` draws them in
order into their targets: the output (window surface or `OffscreenTarget`)
or named render textures (`Renderer::render_target(name, size, wrap)`),
which later passes sample by `TextureId`.

- **Gamma space.** The original is a gamma-space Unity project
  (`m_ActiveColorSpace` 0): no sRGB conversion anywhere, blending on the
  stored values; output and render textures are `Rgba8Unorm`, the surface
  prefers `Bgra8Unorm`.
- **One primitive**: `Quad` (texture, uv rect, 3D transform, size, pivot,
  vertex colour, `Shading`, `Blend`, `Stencil`). 3D transforms because the
  wave camera is a perspective camera looking at a tilted plane. `Text`
  items expand into quads.
- **Cameras** (`Camera`): Unity semantics (look along +z, view = inverse
  world with z negated, right-handed projection), orthographic or
  perspective, own near/far, optional forced aspect.
- **Blend modes**: premultiplied (`One, OneMinusSrcAlpha`: sprites and most
  custom shaders), additive (`One, One`), inverse alpha
  (`OneMinusSrcAlpha, SrcAlpha`, the gradient shader).
- **Sprite masks** through an 8-bit stencil per pass: mask quads add or
  remove 1 where the mask texture's alpha reaches the cutoff; sprites test
  "visible inside" (`1 <= s`) or "outside" (`1 > s`).
- One uber-shader (`shaders/quad.wgsl`) with a per-instance shading kind;
  pipelines differ only by blend, stencil mode and target format and are
  created on first use. Textures are sampled without mipmaps (most of the
  original's shaders sample LOD 0 explicitly).
- **Text**: raster (fontdue, embedded Noto Sans for dev overlays) and SDF
  fonts: static TextMeshPro atlases (`SdfFontDesc`) and dynamic ones
  (`DynamicSdfDesc`): glyphs rasterised from the TTF at 2x, converted to a
  distance field (exact Euclidean distance transform) with TMP's encoding
  (0.5 at the outline, `1 / (2 (padding + 1))` per texel) and packed into a
  shared 2048² atlas. Fallback chains like TMP's fallback table; outline
  and dilate parameters.
- Built-in textures: `WHITE`, `TRANSPARENT` (unbound optional inputs).

### Shaders (behaviour of the original's materials)

Each was reimplemented from what it computes; parameters come from the
material in `materials.json`, render textures from the camera rig.

| Original | Shading | Behaviour |
|---|---|---|
| Sprites/Default, Custom/FlippableSprite | `Sprite` | texture × vertex colour × material `_Color` |
| Custom/SingleColorAlpha | `AlphaTint` | vertex colour with the texture's alpha only |
| Custom/AdditiveColoredA8 | `AlphaTint` + additive | same, added |
| Custom/CustomColourShader (grass) | `ChannelColour` | red-minus-blue → `_MainColor`, green-minus-blue → `_ShadowColor`, plus blue; then blended towards the renderer colour by its darkness |
| Unlit/GradientColor | `Gradient` | vertical gradient in texture space (`_GradientHeight` shifts it), alpha from `_AlphaTop`/`_AlphaBot`; inverse-alpha blend, so alpha 1 keeps what is behind |
| Unlit/SlimeTrailedMovable (platforms) | `SlimeTrailed` | the sprite, with the platform's slime-trail texture (brightest of the texel and its 4 diagonal neighbours, scaled by the sprite's green) painted over by the trail alpha; flip from the renderer |
| Custom/ReflectiveWaterWithDistortion | `ReflectiveWater` | full-screen quad: below the screen height of `_WorldHorizonHeight` the reflection texture (looked up mirrored, `_ReflH` above the pixel) over `_WaterColor`, above it the background texture; both offset by the wave texture (fading near the right edge and the bottom), mixed with a vertical gradient (`_Alpha`) and wave noise (`_ExtraNoise`) that fades towards the horizon |
| Custom/WaterDistortion | `WaterDistortion` | the wave texture tiled `_SubDivide` times, offset by animated Perlin noise (`_Time`, `_SinTime`, `_CosTime`, frozen by `_Timestop01`), fading out towards the top |
| Unlit/aurora | `Aurora` | two scrolling samples of its texture along the mask's green channel, wobbled by noise; the mask's blue cuts it |
| Custom/BlackHole2 | `BlackHole` | grab pass: the screen behind refracted around the centre (`_Distortion` = −clamp(mass)), a black (white-hole: bright) disc and an orange proximity glow; `_Scale` = the renderer's scale (`WriteScaleToShader`) |
| Custom/Distortion, Custom/GrabInvert | `Distort`, `GrabInvert` | grab pass, offset by the sprite's normal-map-like channels / colour-inverted inside the sprite (time-stop sphere) |
| Custom/SmearSprite | `Smear` | Dash/Meteor smear: the sprite stretched along the motion, main/shadow colours from the player material |
| Custom/DrawOnlyWhite | `OnlyWhite` | vertex colour where the texture's red is bright, alpha (r·a)⁸ |
| Custom/ExpandingCircle | `ExpandingCircle` | noisy ring growing with `_Progress` (particles: their age fraction × `_ParticleSpeed`, smootherstep, + `_Progress`), `_Mask` red scales alpha (water rings, gun hits on water) |
| Custom/WhiteToColA8 | `WhiteToColour` | vertex colour, towards white where the texture alpha is high |
| Custom/WallMaskedA8 | `WallMasked` | sprite alpha cut by `_platformRT` (dirt trail inside platforms) |
| Custom/ColorSingleChannel, Unlit/ColorChannelsSeparetely | `SingleChannel`, `Channels3` | a mask channel / the three channels each in their own colour |
| Custom/AbilityIndicatorWithCircle | `AbilityIndicator` | the HUD indicator: icon over a rimmed circle, the uncharged sector × `_CDColor` (colour and alpha), icon texels clamped to its atlas rect |
| Unlit/ResizablePlatform | `ResizablePlatform` | Platform ability: rounded rectangle in the vertex colour with a black outline (anti-aliased, half extents and radius from the collider) |
| Custom/LightningA8 | `Lightning` | Tesla coil bolt: a texture row band (`_UVYMin`..`_UVYMax`) blurred in `_Directions` × `_Quality` taps, scrolled by `_Scale` |
| Custom/Beam, BeamGlow, BeamWhite, AdditiveSoftBorders | `Beam` | the beam box: core, glow, soft borders (additive), cut by `_platformRT` where it enters ground; four kinds (core, glow, end, after-glow) |
| Custom/CloneEffect | `Clone` | Duplicator spawn-in: the sprite revealed by a noisy front (`_Progress01`) |
| Legacy Particles/Alpha Blended (Premultiply), Additive (Soft) | `ParticlePremultiply`, `ParticleBlended`, soft-additive blend | the legacy particle shaders (`_TintColor` × 2 for Alpha Blended) |

## Level presentation (`crates/game/src/level_view.rs`)

Every grass/snow level has the same rig; `LevelView` reads it from the
level's cameras (culling mask, depth, clear flag and colour, projection,
`FollowMainCamera`, `ParticleCamera`, `reflectiveCamera`) and renders, in
depth order:

| Camera | Layers | Target | Notes |
|---|---|---|---|
| BackgroundCamera | Sky | `_BackgroundRT` | follows the main camera, y offset 3.2 (applied twice, see below) |
| ReflectiveCamera | all but Sky, Water, ... | `_ReflectionRT` | y negated, offset -2.41 (twice) → mirror line ≈ -11.26 |
| WaveCam | WaveParticle | `_WaveParticleRT` | perspective, fov 60, x follows |
| platform+WaterMaskCamera | wall, terrain, waterMask | `_platformRT` | read by beam abilities |
| SlimeTrailCam (per platform) | SlimeTrail | `_SlimeRT<k>` (sprite texture size) | never cleared (cleared on a level's first frame); ui.md 12 |
| Main Camera | the rest | output | size 30, (x, 8, -100) |

Space levels have only the mask and main cameras.

- **Sorting** like Unity: sorting layer, order in layer, then farther first
  along the view axis, then scene order.
- **Sprite masks**: the water's `SpriteMask` (custom range background/0 to
  Player/5, cutoff 0.2) is active in passes whose culling mask contains its
  layer (Sky), from the first sprite of its range to the last. Platforms use
  "visible outside", which hides their submerged part in the main view but
  not in the reflection.
- **Presentation scripts** reproduced per frame: `FollowMainCamera`
  (including the original adding the y offset a second time when z is
  followed), `FullScreenQuad` (the water fills the view), `ParallaxLayer`
  (`camera × (1 − max(speed, 0.05))`), `KeepInCameraCorner` (space
  backdrop at the right edge), `ScaleBasedOnCameraWidth` (wider sky on
  wider screens), `AlphaFade` (the fade-in, Hermite curve).
- World matrices are composed from the local TRS down the hierarchy (a test
  checks that this reproduces the imported world positions in all 124
  levels); never decompose them: Unity sprites often have z scale 0.

### Seam for the simulation (`present.rs`)

Each frame the game fills a `WorldView` from the (interpolated) sim state:

- `platforms: Vec<PlatformPose { platform, position, rotation, scale,
  visible }>` indexed like `Level::platforms` (sim `FixTransform` → f32);
  children (grass) follow;
- `sprites: Vec<SpriteDraw>` for players, pickups, projectiles, effects
  (import sprite id, pose, colour, flip, sorting layer/order, Unity layer
  for camera culling, optional material id, mask interaction);
- `camera_x` (use `CameraFollow`, the original's follow rule, with the
  alive players' x), `shake`, `time`, `time_stopped`.

`LevelView::frame(&view, size, gpu, renderer, assets)` returns the passes;
a `Scene` (`app.rs`) wraps it: `tick` steps the sim with the devices'
`InputState`s, `draw` builds the view. Sounds are played by name through
`sound::Sounds` (`play("slimeJump1")`, songs with intro + loop).

### Comparison with the original

`scratch/ref/06-match-start.png` (the original, Level1, 1920x1080) against
`--level 6 --camera-x -7.1` (the original's camera had followed the
player): mean absolute difference 1.9/255; sky 0.55/255 (pixel-exact apart
from the missing sparkles), water 4.1/255 (wave-distortion phase differs),
platform edges within a pixel. Also checked: snow (aurora, icebergs,
reflections), space (moon backdrop kept at the right edge), 4:3 and 21:9.

### Match presentation (`match_view.rs`)

`MatchPresenter` turns `World::view()` into a `WorldView` each frame and
observes every tick:

- **Bopls**: the animator's sprite (`SpriteView.sprite` → `Content::sprites`)
  on the `PlayerSprite` renderer (sorting layer Player, mask "outside"),
  the player colour's material (`PlayerColors.playerMaterial`, channel
  shader), facing as an x flip of the `flipTransform`, the animator's own
  flip, the sim rotation and scale. Ability objects and spawned objects
  draw their prefab object's subtree (`Visual::Prefab`); pickups show their
  ability icon.
- **Slime trails** (ui.md 12): the grounded bopl's `slimeTrail` child on
  layer 15 with the world-position wobble and landing re-anchoring; death
  splats (15 Fix rays against the sim's rounded rects on a physics clone)
  for one frame; `LevelView` renders the per-platform trail cameras before
  the other cameras and binds the textures to the platform material.
- **Effects** from `Cue::Effect`: the death effect (anticipation → start →
  splats + `slimeSplash` → end, player material, `slimeDeath`) and the
  water splash (`enter`), timed in ticks from the effect prefabs'
  `AnimationData`.
- **Sounds**: every cue played once per `EventId` (a set pruned after 600
  ticks; rollback re-simulation emits the same ids). `PlayerSound` uses
  variant `((id − 1) mod 4) + 1`; `SoundPrefix` uses the random-with-prefix
  quirk, `easeInBlock` the "least progress" rule. Music: menu song in menus,
  `match_started` per level (TheBopl first, next song after intro + loop),
  draw stops it, sudden death pitch 1.1.
- **Camera**: `CameraFollow` on the alive players' x per tick, frozen on the
  winner screen; shake trauma from `Cue::Shake` (decays 0.6/s).
- **HUD**: three indicators per spawned player at the spec offsets, each
  one quad with the `AbilityIndicator` shading (`Custom/AbilityIndicatorWithCircle`,
  ui.md 5): team-fill circle of radius 1 with a darker rim, the icon, the
  uncharged sector multiplied by `_CDColor`; the reset pulse from the
  prefab's `ResetSizeAnimation` (Hermite, real time; no colour flash: the
  reset colour is white); alpha = invisibility alpha capped at 0.4 while
  uncastable (the view's `SlotView::castable`); off-screen arrows (prefab
  `SceneBoundArrow`, player colour). Online local players stay ≥ 0.2
  visible while invisible (`MatchPresenter::online_local`).

### Ability presentation (M5)

Every visible ability object reports what to draw through `View`
(`crates/sim/src/view.rs`, presentation only, not part of `state_hash`):
`SpriteView.look` (`Look`: animator frame, flip, alpha, stretch, hidden,
player material, per-part `PartLook`s for the prefab's child renderers
with visibility, frame, pose, alpha, tint, shadow colour and four shader
values, loop sound, attached particle prefabs `Attach`, objects to hide),
`View.ropes` (point chains of the rope bodies) and `Cue::Fx` (`FxCue`:
"`Instantiate(prefab field)`" of a presentation prefab at a position,
rotation, scale, colour and value, with the emitting object; level
`DetPhysics` fields use `LEVEL_PREFAB`). Each ability module fills its look
in `AbilityImpl::look` / `SpawnedImpl::look`; platforms made by abilities
come through `abilities::platform_look`.

`match_view.rs` draws the prefab subtree of each object with the look
applied (pose and script overrides for DarknessAura/TimeStop/BeamEffect
scripts, `PlayAnimation` children loop on the clock, player material rules
as in the original: `yellowSlime` renderers, `SparkShot` rockets, ability
roots), the ropes as `RopeMesh` strips (0.2 half width, black on grass,
clamped above the water), beams from their boxes (core/glow/after-glow,
end and hit effects with the beam colour sets), and the presentation
prefabs in `fx.rs`: frame animations (`PlayAnimation`,
`SimpleSpriteAnimator`, Mecanim clips), `AlphaFade`, `AnimateWaveRing`,
`GrowthFlare`, `portalIndicatorParticle` (shape radius from its curve),
`beamAfterEffect`, plus their particle systems. Followers (Dash/Roll
trails with the player's complementary hue, beam hits, magnet/engine/
missile smoke) follow their object and stop emitting when it is gone.
Loop sounds (`loopEngine`, `beamLoop`, `drill_loop`,
`drill_underground_loop`, `meteor_vibrate`, `CreepyAmbientElectrical`)
play while any object asks for them.

**Particles** (`particles.rs`, ported from rounds-port's `psys.rs`, ours):
Unity-like modules (main incl. prewarm, simulation speed, 3D start size;
emission with bursts and rate over distance, path-interpolated for moving
world-space emitters; shapes; velocity incl. exact orbital rotation about
all three axes and radial; limit velocity; force; noise; colour / size /
rotation over lifetime; texture sheets in grid and sprite mode; sub-emitters
on death; play/stop of single systems by scripts). Noise: a velocity from
value noise × strength × position amount, × 1 / frequency when damped
(Unity: damped strength follows the field's size), × 2 to match Unity's
field amplitude (Roll's trail breaking up, smoke). Align to direction: a
billboard turns with the emitter's in-plane rotation of its emission
direction (the meteor's dust, instantiated flipped, shows as a V). Renderer modes:
billboard, stretched (length/velocity scale); a 3D x rotation foreshortens
the quad as an orthographic camera would (water rings become thin
ellipses); velocity alignment (`ParticleSystemRenderSpace.Velocity`, with
a 3D start rotation: the teleport rings and portal sparks) builds the
quad's frame from the particle's effective velocity (forward along it, the
side axis in the screen plane), applies the 3D rotation in Unity's Z, X, Y
order and lets the orthographic camera project it: with x = 90° the
sprite's width runs along the motion, face-on (full-height dashes along
the ring, shorter where the motion leaves the screen plane), as measured
in the capture. Quads carry the age fraction for
custom-stream shaders and are sorted with the sprites by sorting layer and
order; their Unity layer picks the cameras (wave particles go only into
`_WaveParticleRT`).

**Level particles**: the scene's own systems (`PS - sparklySky`) start with
the level (prewarmed); `WaterWaves` platforms get two `WaveRingParticle`
emitters (the left one mirrored) where their bounding box crosses
`Constants.WATER_HEIGHT`, moved inwards by `waveOffsetX`, stopped when the
crossings are closer than `DespawnOffset` or the platform is clear of the
water, paused during time stop; not on space levels.

Gallery: `scratch/m5/gallery.sh [filter]` runs one headless sandbox per
ability (`--sandbox --abilities A,B,C --players 2 --script ...`) and writes
`scratch/m5/<ability>-sheet.png` and the contact sheet
`scratch/m5/gallery.png`. Reference captures of the original (Steam mode,
solo round per ability, `scratch/m5/tools/ref*.sh`) are in
`scratch/ref/abilities/<ability>.mp4` with frame strips `*-strip.png`.

### UI canvases (`ui.rs`) and screens

Menus, character select, winner screen, ability-select circles and the pause
prompt load the original's canvases from the import (scenes 0, 2, 4, the
level's `ExitGameMenu`, prefabs `abSelCombined`, `AbilityGridEntry`) and are
driven by name from our screen code (the original's scripts reimplemented
per screen; their animators approximated by slides or evaluated Mecanim
clips: `WinnerTriangle`, `nextLevel`).

- **Layout**: RectTransform (anchors, pivot, anchored position, size delta,
  local rotation/scale) under a `CanvasScaler` root (scale with screen size:
  expand / shrink / match; constant pixel size). Drawn in a camera pass whose
  world units are output pixels, y up. Controllers add offsets, scale and
  alpha per node.
- **Image**: simple images with the sprite's trimmed rect placed inside the
  node rect, preserve aspect, colour; materials: sprite, slime colours
  (`Custom/UI_CustomColor`, channel shader, recoloured per player), additive,
  the menus' moving circle pattern (`Custom/repeater`, drawn as a grid of
  the circle sprite, `_TimeInMenus` shared across menu screens); Steam
  frames hidden. **Mask** → stencil (one level).
- **TMP text**: font asset by reference (per-language font and its stroke
  variant from the localisation table), size, auto-size (binary search),
  word wrap, margins, alignment, bold dilate, outline from the material
  (the text's own, else the resolved font asset's: stroked fonts bring
  their outline), TMP's extra advance `(normalSpacingOffset +
  characterSpacing + boldSpacing if bold) / 100` em per character
  (`Text::spacing`, also in measuring and auto-size).
- **Localisation**: lower-case English keys → the language's string
  (English shows the key); language chosen in the menu, stored in
  `settings.json` in the user data dir.
- **Screens**: `menu.rs` (main list, settings rows: SFX/music volume,
  abilities 1–3, screen shake; language list; credits; online sub-menu:
  host game / host private / join code / room list / name / back; the room
  browser; the error line `disconnect_errorLog` for refused joins and the
  browser's status; Invite friend dimmed and refused), `charsel.rs`
  (match-flow.md 4 for up to 12 boxes: 4 in the original row, more in two
  rows; join rules below; key rebinding below), `match_ui.rs` (winner
  layouts single/draw/2/3 and N, prompts, circles with portraits by cause of
  death or kills, the wheel stack from the sim's flow state, remote players'
  busy dots, pause prompt), `tutorial.rs` (below).
- **TMP alignment**: a legacy `m_textAlignment` that TMP has not yet
  converted (≠ 65535) wins over the split `m_HorizontalAlignment` /
  `m_VerticalAlignment` fields (TMP converts it on load).
- **World-space canvases** (`Canvas::world_root`): the root's local units
  map to output pixels through a matrix (the tutorial's hint canvases).

### Key rebinding (ui.md 6)

The `rebind keys` row of a character-select box (Accept, or a click on the
icon for keyboard + mouse) starts a rebinding of the box's own device
(`remap.rs`: steps, reserved controls, clashes, display names; tested): the
box's rebind cover shows the step prompt, the circle indicator and the
chosen controls; raw presses (keys, mouse buttons, controller buttons
including the triggers) bind the steps; the box ignores its device until
the rebinding ends (and for the tick that ended it). The result is stored
per device (per player) in `Settings::bindings.devices` (`settings.json`)
under the device's key (`input::device_key`: the keyboard schemes by name,
a controller by its serial number, else its GUID and a connection-order
number, else the type; ui.md 6), which the app hands to the input hub
(`AppRequests::bindings`) at start and after every change; a device
without its own binding uses the per-type one (`bindings.map`, older
settings) and then the defaults. `--list-devices` prints each controller's
key. Test: `scripts/dev/m9-remap.drive` (a second controller keeps the
defaults while the first is rebound: `scratch/final/remap/`). Screenshots `scratch/m9/remap-*.png` (compared with the
original's cover, `scratch/m9/ref/r02-one.png`: same layout).

### Tutorial (docs/spec/tutorial.md)

Main menu → Tutorial (the activating device: a controller plays at once):
`tutorial.rs` runs a one-player `World` on the tutorial scene (sim:
`bopl_sim::tutorial`), feeds it the current device (switching between
keyboard + mouse and controllers like the original), draws the arrows,
pickups and dummies from their scene renderers at the sim's poses (the
arrows' UI slime material mapped to the in-game slime material of the same
colours), the current hint set (the movement hint `M`, the device
indicator, the TextMesh letters as text, the world-space canvas with "hold
down … to aim" / "… to throw grenade" / "click … to dash"), and returns to
the main menu when the sim reports the end. The level presentation is the
scene with those objects (and the pre-placed player) hidden. Like the
original: the camera does not follow (`MatchPresenter::camera_follow` off,
the scene's `PlayerAverageCamera` is disabled), locked slots draw no HUD
indicator at all (`locked_slots`), and the dummies' death effect and
splats use their own orange slime material (`orphan_effect_color`).
`scratch/m9/tutorial-*.png`, `scratch/final/tut/`; the original's start
`scratch/m9/ref/t02-start2.png` matches (same objects, letters and
indicator), walking compared in `scratch/final/tutorial-camera-hud-ours-vs-original.png`.

**Joining** (`charsel.rs`): keyboard + mouse joins with Space/Enter or a
click on a free box; the left keyboard half with Tab, the right half with
Keypad Enter (its Start); controllers with A or Start. The keyboard + mouse
player excludes the keyboard halves and vice versa (shared keys). Esc /
Backspace without a keyboard box, or Back on an unbound controller, returns
to the main menu.

### 12 players and teams (M6, M7)

Decisions in `docs/STATUS.md`; proposals in match-flow.md 17.2.

- **Joining** 12 devices: the keyboard halves and any number of
  controllers (or keyboard + mouse and 11 controllers) on boxes in two
  rows of up to 6; a free box is always added up to 12. Teams cycle 0..3
  with up to 4 boxes (the original), 0..11 beyond (and online).
- **Team colours** 4–11 are derived from the player palette's shadow
  colours (fill ×0.85, border ×0.45, saturated = the colour), most distinct
  first: yellow, turquoise, violet, orange, grey, black, white, purple
  (`match_view::extend_team_colors`); they tint boxes, HUD indicators and
  circles like the original 4.
- **Spawns** for more than 4 players: the generated points (sim).
- **Camera**: the original's fixed size 30 following the alive players' x
  (zoom-out for crowded levels is an M9 feel-pass decision); players
  outside get the off-screen arrows.
- **Winner screen** (`match_ui::winner_images`): the original layouts for
  1 winner, draws up to 4 and teams up to 3; more winners are copies of the
  authored images standing along the banner's upper edge, in two staggered
  rows above 6, scaled to fit (`scratch/m7/1v11-winner-*.png`: 11 winners).
  The prompts' delay counts at most 4 players (12 would hide them for 5 s).
- **Circles**: one row, spacing `min(230, 2400 / N)`, circles scaled with
  the spacing (`(spacing / 230)^1.5`, at least 0.7), so 12 fit at 16:9.
  Players who left an online session keep a greyed circle on the winner
  screen.

### Online (M8)

`crates/game/src/online/` (docs/net.md, "The game side"): main menu →
online → host game / host private / join code (typed; Enter joins, Esc
cancels) / room list (`online/browser.rs`) / name, or `--warpzone-host` /
`--warpzone-join`. The lobby is character
select with the local boxes first and the remote players' boxes after them,
the room code as title and a status line; the host starts. The match is the
usual match scene on bopl-net's predicted state with cues through the event
gate; the pause prompt's "leave" leaves the room. Headless online runs are
paced to the wall clock; the driver can `wait machines N`.
`scripts/net/gui-online.sh` hosts with the full game
(`scripts/dev/m8-online.drive`) while two `--net-peer` processes with 4 bots
each join: 12 players on 3 machines, screenshots `scratch/m8/online-*.png`.

### M9 polish

- **Options** (`menu.rs`, `shared.rs` `Settings`, persisted in
  `settings.json`): every row of the original's settings menu works: SFX /
  music volume (live, through the mixer's buses), abilities, screen shake,
  rumble (off / on / high), vsync, resolution (the display's distinct
  modes, largest first), screen (windowed / borderless / exclusive
  fullscreen), hide (streamer mode: remote names in the online lobby become
  "player N"); plus our "camera zoom" row (a copy of the screen-shake row;
  the rows are re-spaced to fit 11). Display rows apply on Save (Back
  restores them) through `AppRequests::display`; the window host
  (`app.rs` `apply_display`) switches modes (exclusive fullscreen uses the
  closest real display mode; borderless first covers the display, needed
  without a window manager), reconfigures vsync and follows the drawable
  size every frame. `master_volume` (0..10) exists in `settings.json` only.
- **Camera zoom** (setting, off by default; docs/STATUS.md decisions):
  with more than 4 players `CameraFollow::zoom` grows the orthographic size
  from 30 up to 45 so that the alive players' x spread (+10 margin) and the
  highest player (+6) fit, smoothed like the follow; the camera's bottom
  edge stays (the water looks the same, more sky shows); the off-screen
  arrows and mouse aim use the zoomed camera; the level rig follows
  (`WorldView::camera_size`, cameras with `mimicCameraSize`).
- **Audio** (`sound.rs`): bus volumes (SFX = sfx/10, music = current music
  volume, master = engine 0.5 × master/10) so settings act on what plays;
  loops are `PlayOrContinueLoop`: wanted every tick or stopped, `fadeLoop`
  entries (`beamLoop`) fade in/out at their volume per second, nothing can
  keep a loop alive after its match; music ducking (`FadeOutMusicTick`,
  0.4/s during the time-stop cast, 0.2/s while time is stopped, recovery
  `0.3 × dt`). Loops from state the view does not carry (read from the
  `World` by the presenter, no sim change): `beamLoopBig` (a firing beam's
  `BeamSystem` role is a child), `beamLoopHP` (the staff direction changed
  this tick), `reelInLoop` (rope attached, its button held, castable).
- **Rumble** (`match_scene.rs`): big for a player whose bopl or clone was
  hit (`Cue::Effect` death/blast/water), small for the killer
  (`Cue::PlayerDied`), big while a beam is entered; 0.5 s at the player
  prefab's motor speeds (small 0/0.4, big 0.4/0.5), ×3 on "high"; build
  tension (0.05/0.05, continuous) while Roll charges, Meteor hovers and dashes,
  and in a time stop's unstable last 2 s until the cast (read from the
  abilities' state, no sim change); requests go through
  `AppRequests::rumble` to SDL.
- **Presentation**: time-stop force arrows (`ForceArrow` on every prefab
  whose root references a `ForceArrowPrefab`: BoplBodies and black holes;
  rectangle stretched to `|v| · 0.2`, triangle at its end, hidden below
  0.5); black-hole eat particles in the eaten object's shape (the sprite's
  opaque texels as emission points, burst `4 · extents · particlesPerArea`
  ≤ 1000, the object's scale, orbiting the hole through the orbital offset,
  which the particle system now honours; resizable platforms as a
  rectangle); the Duplicator's `DuplicatePlatformEffect` (the platform's
  sprite through `CloneEffect` with `_Direction`, `_UVOffset`,
  `_Progress01` and `alphaAnim`, shaking up to 0.3 × progress; resizable
  platforms as the rounded rectangle moving to the clone's place).
- **Mouse**: character-select rows highlight under the pointer (only when it
  moves), the side arrows step, a click on an ability row opens the grid,
  grid icons highlight and pick, a click on ready readies; the ability
  wheel's arrows and handle take clicks (the mouse player's A / D / W for a
  tick); hovering a wheel row selects it (`HoverCircle`: the hovered row
  becomes a target reached with W / S edges); settings rows' arrow buttons
  step down / up. The driver has `point X Y` (`scripts/dev/m9-mouse.drive`,
  `scripts/dev/m9-wheel-hover.drive`).
- **Cursor** (`CursorUpdater`, ui.md 1): at the start of the main menu,
  character select, tutorial and online screens the scene sends
  `AppRequests::cursor` (the cursor texture with `lerp(black, colour,
  green)`, the keyboard + mouse player's `_ShadowColor` or the default
  yellow); the window host makes it an SDL colour cursor (hot spot top-left);
  matches keep it.
- **Robustness**: no SDL audio subsystem or device → silent (`Audio::null`,
  no voices pile up); a removed playback device is reopened (music
  restarted); a controller unplugged mid-match: the next new controller
  that presses a button takes over the player (also kept for character
  select); a suboptimal surface is reconfigured before the next frame
  (configuring while its image was alive crashed wgpu on fullscreen
  toggles); effects cued while nothing is drawn (minimised) are dropped
  after 2 s; the fixed step is exact (integer nanoseconds × 60: 60 ticks
  per second at any refresh rate, tested at 144/75/30/59.94 Hz; presentation
  draws the latest tick, like the original, no interpolation); a health line
  every minute (`RUST_LOG=info`: RSS, loops, presentation collections).
  Tested on the private Xvfb (`scripts/dev/m9-window-test.sh [audio
  driver]`: resize, F11 both ways, pad unplugged and replaced in a match,
  pause from the new pad, settings Save to exclusive fullscreen and back,
  no audio driver): ticks 60.0/s, no crash. 30 min of 12-bot matches
  (108,000 ticks): presentation collections flat (≈50 cue ids, ≤ 300
  particles), RSS 789 MB after 1 min rising to 1.0 GB as new levels' textures
  are cached (lavapipe keeps GPU memory in the process), flat for the last
  15 min.
- Screenshots: `scratch/m9/polish-*.png`.

### Gaps (planned paths)

- **Ability presentation** (M5 leftovers): Roll's trail corners (the
  original emits per movement sub-step); the Duplicator platform effect's
  flash sprite (its alpha curve is 0 in the shipped data) and spawn-in
  particles' placement are as cued; death splats use our own random
  sizes/rotations (the original's are `Random.value`, not reproducible);
  the black hole glow's shading detail. Velocity-aligned particles without
  a 3D rotation (SpawnIn_platform, SpawnOut_duplicate) stay billboards.
- Texture memory: level textures stay cached for the session (≈1 GB RSS
  with lavapipe after most levels were seen).
- Tutorial: the arrows bob once (the authored curve clamps) like the data
  says.
- UI animators: only the winner scene has Animators; `WinnerTriangle`,
  `TriangleOut` and `nextLevel` (both directions) run on the original's
  clips, the winner screen's exit plays over the next level's fly-in
  (ui.md 8). Other menus' motion comes from scripts, reimplemented with
  their timings and curves (the menus' `AnimateInOptions` /
  `AnimateOutOptions`). The death effect's vibration and gibs' hit-stop are
  not reproduced. The "ability select" exit goes straight to character
  select (no fade).
- Mipmaps (only `Sprites/Default` would use them in the original).

## Input

`input.rs`: keyboard + mouse, two keyboard halves, any number of SDL game
controllers (hot-plug through SDL device events), sampled per tick into
`InputState` (move/aim axes, held/pressed/released `Buttons`; presses
shorter than a tick are latched).

| Scheme | Move | Jump | Slot 0 | Slot 1 | Slot 2 | Start | Select |
|---|---|---|---|---|---|---|---|
| Keyboard + mouse | W A S D / arrows; aim = mouse | Space | left mouse | right mouse | middle, side buttons | Enter | Esc, Backspace |
| Keyboard left (ours) | W A S D | Space | F | H | G | Tab | Esc |
| Keyboard right (ours) | arrows | Right Shift, Keypad 0 | `,` / Keypad 1 | `/` / Keypad 3 | `.` / Keypad 2 | Keypad Enter | Delete |
| Controller | left stick, d-pad, right stick | South | West | East | North | Start | Back |

Menu accept/back follow ui.md 1 (Space/Enter/South/Start, Esc/Backspace/
East/Back; stick moves beyond 0.4). The table above is the default
`Bindings` (serde, `settings.json`: per scheme and action, plus per-device
overrides keyed by `device_key`); the key rebinding writes the rebinding
device's entry only. `KeyboardKeys::buttons` (keyboard
schemes) and `controller_buttons` turn held keys / buttons into
`Buttons` (shared by the input hub and the headless driver).
`gamecontrollerdb.txt` next to the executable adds mappings.

## Audio

`audio/`: SDL playback device (stereo f32, 48 kHz) fed by our mixer
(voices with volume, pitch, pan, loop, delayed start; buses), the same
mixer offline (headless, `--wav`), or nothing (`Audio::null`, no device).
Ogg via lewton. `sound.rs` implements the original's rules (audio.md): one
voice per sound name, effect volume = entry volume × SFX / 10 (SFX bus),
music = music / 10 × 0.1875 (music bus, ducked by time stop), engine
volume 0.5 (master), songs as intro + loop scheduled at the intro length,
loops wanted per tick with fades (see M9 polish).

## Platform notes

- SDL2 is linked against the system runtime `libSDL2-2.0.so.0` (no -dev
  package): `crates/game/build.rs` adds `tools/lib` (symlink) to the link
  path; `BOPL_SDL2_LIB_DIR` overrides it (Windows: `tools/windows/sdl2`).
- SDL hints (`platform.rs`): background joystick events, RawInput/HIDAPI,
  no minimise on focus loss (Linux).
- Timing (`timing.rs`): fixed 1/60 s ticks with an exact integer
  accumulator (hitches clamped to 0.25 s, then the sim slows instead of
  spiralling); the presentation draws the latest tick.

## Testing on the private Xvfb

Windowed runs never use the desktop: the private Xvfb `:66`
(`scripts/dev/xvfb.sh :66`), SDL's `disk` (or `dummy`) audio driver, and
simulated controllers (`--virtual-pads FILE`, `vpad.rs`: SDL virtual
joysticks driven by lines appended to FILE: `attach N`, `detach P`,
`tap P a`, `button P x 1`, `axis P lx 1`, `center P`):

```sh
D=$(scripts/dev/xvfb.sh :66)
env -u WAYLAND_DISPLAY DISPLAY=$D SDL_VIDEODRIVER=x11 SDL_AUDIODRIVER=disk \
    SDL_DISKAUDIOFILE=scratch/sdl-audio.raw RUST_LOG=info \
    target/debug/bopl-port --data cache/import --viewer --level 6 --size 1280x720 \
    --virtual-pads scratch/pads.cmd --quit-after 9 --screenshot scratch/window.png
# meanwhile: echo "attach 2" >> scratch/pads.cmd; echo "tap 1 right" >> scratch/pads.cmd; echo "detach 0" >> scratch/pads.cmd
```

Verified (Oct 2026): window and wgpu surface on Xvfb (lavapipe, no DRI3,
`Bgra8Unorm`), two pads hot-plugged and one unplugged (log and overlay),
a pad press switching the level, music + sounds written by the disk audio
driver (10 s, peak 0.14), `--quit-after` screenshot of the window's frame.

The game itself (main menu) runs the same way without `--viewer`; with a
virtual pad, `tap 0 a` presses Play and further taps join and navigate
character select (verified Oct 2026, llvmpipe ≈ 27 fps at 1280x720). Run the
whole command line in one shell after `source env.sh` (a backgrounded
`source` leaves the binary without `BOPL_PORT_DATA_DIR`, and it would import
into the default user data dir outside the project).

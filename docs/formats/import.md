# Import format (version 1, revision 0)

`bopl-import` converts the player's Bopl Battle install into this directory
layout. The port loads it with `bopl_import::GameData::load(dir)`; the serde
types live in `crates/import/src/format.rs` (the source of truth for field
names). Nothing in it is committed: it is the game's data, regenerated from
the player's copy.

```
source env.sh
cargo run --release -p bopl-import -- [--game DIR] [--out DIR | --dev] [--force]
cargo run --release -p bopl-import --bin level-sheet -- [--data DIR] [--out FILE] [--level N] [--shapes] [--tile WxH] [--cols N]
cargo run --release -p bopl-import --example validate_import -- [--data DIR] [--dump DIR]
```

- **Where**: `<data dir>/import-v<N>` (`bopl_import::paths::import_dir()`):
  the data dir is `$BOPL_PORT_DATA_DIR` (dev: `cache/userdata`, set by
  `env.sh`), else `$XDG_DATA_HOME/bopl-port` / `~/.local/share/bopl-port`,
  `%APPDATA%\bopl-port`, `~/Library/Application Support/bopl-port`.
  `--dev` writes `cache/import` (a dev copy the tools default to), `--out`
  anywhere.
- **Game**: `--game DIR`, `$BOPL_GAME_DIR`, `$WARPZONE_DATA_BOPL` (Warpzone
  launcher, data requirement id `bopl`), then every Steam library
  (`libraryfolders.vdf` of every Steam root, app 1686940 first):
  `steamapps/common/Bopl Battle` (`paths::find_install`).
- **Idempotent and atomic**: written to `<out>.tmp`, then renamed over
  `<out>`. If `index.json` has the current format version, at least the
  current revision and the same source fingerprint (names, sizes, mtimes of
  every file in `BoplBattle_Data` and `Resources`), nothing is done unless
  `--force` (`bopl_import::is_up_to_date(game, dir)` is the same check).
  A non-empty directory without `index.json` is never replaced.
- **Fast re-import**: textures, audio and font files carry a `source_hash`
  (FNV-1a of the source bytes); a re-import moves unchanged PNG/Ogg/TTF
  files over from the previous import instead of encoding them again.
  Timings (12-thread laptop, warm page cache): first import 1.8 s, forced
  re-import 0.25–0.5 s, up-to-date check 0.03 s.
- **Versions and revisions**: `FORMAT_VERSION` (`index.json`
  `format_version`, also the `import-v<N>` name) changes only on
  incompatible changes. `FORMAT_REVISION` counts additive changes within a
  version (new optional fields and files; mark them `#[serde(default)]`);
  an older revision is re-imported automatically.

| Revision | Added |
|---|---|
| 0 | first release |

## Conventions

- **Object IDs**: `"<file>:<path id>"`, e.g. `"sharedassets0.assets:2240"`,
  `"level6:36"`. `<file>` is the SerializedFile name in `BoplBattle_Data`
  (`levelN` is build scene N).
- **Raw fields** (`fields`): a component's or asset's serialized fields
  under their Unity names, converted generically:
  - Object references are `{"$ref": "<id>"}`, null references `null`.
  - **Fixed point stays raw**: the game's `Fix` is `{"m_rawValue": <i64>}`
    (Q32.32, value = raw / 2³²), `Vec2` is `{"x": Fix, "y": Fix}`,
    `AnimationCurveFixed` holds a Unity `AnimationCurve` (floats) plus its
    precomputed `keys` (`KeyFrameF`, raw `Fix`). Never round-trip these
    through floats in the sim.
  - Floats use the shortest form that round-trips the `f32`; non-finite
    floats become `null`.
  - `UInt8`/`bool` script flags stay numbers (`0`/`1`), as Unity stores them.
  - Keys are sorted alphabetically.
  - MonoBehaviour fields omit the header (`m_GameObject`, `m_Enabled`,
    `m_Script`, `m_Name`); the enabled flag is `Component.enabled`.
- **Typed values** next to the raw ones use plain `i64` for `Fix`
  (`FixRaw`) and Unity units otherwise.
- **Coordinates**: Unity world units, y up. Sprite pixel rects use a
  bottom-left origin, except `texture_rect` (PNG space, top-left origin).
- **Transforms**: `local` is the serialized local TRS of Unity's float
  `Transform` (quaternion `[x,y,z,w]`); `world` the composed world position,
  rotation, lossy scale, `rotation_z` (degrees, counter-clockwise) and
  `matrix` `[a,b,c,d,tx,ty]` (`x' = a·x + c·y + tx`, `y' = b·x + d·y + ty`,
  exact with skew). The **simulation's** transform is the `FixTransform`
  (`Object.fix`); in the shipped data both agree (checked for level 1).

## Files

| File | Content |
|---|---|
| `index.json` | `Index`: format version/revision, Unity version, source fingerprint, `scenes` (all 130 build scenes: `build_index`, `name`, `path`, `level`, `file`), `sorting_layers` in draw order (`name`, `id`), `layers` (32 names), `tags` (0 Untagged, 1–6 built-in, then custom), `settings` (`fixed_timestep` 1/60, `gravity` (Physics2D, unused by the game's physics), raw `time`, `physics2d`, `player` (product name, version, default resolution)), `player_prefab`, `counts`. Pretty-printed. |
| `textures.json` | id → `{name, png, width, height, format, filter_mode, wrap_mode, source_hash}` (170; the 10 empty 0x0 dynamic font atlases are skipped) |
| `textures/<file>_<pid>.png` | RGBA8, top row first (BC7/DXT/RGBA32/Alpha8/...; Alpha8 becomes white with alpha) |
| `sprites.json` | id → `{name, texture, rect, texture_rect, texture_rect_offset, pivot, pixels_per_unit, border, packed, packing_rotation, physics_shape}` (1001). The game uses no sprite atlases (`packed` is false everywhere); `physics_shape` is in world units relative to the pivot. |
| `audio.json` | id → `{name, file, channels, frequency, length, samples, source_hash}` (121) |
| `audio/<name>.ogg` | Ogg Vorbis remuxed losslessly from FSB5 (no re-encoding); name clashes get `_2`, `_3`, ... |
| `levels/NNN_<name>.json` | `Level` for build indices 6–129 (see below) |
| `scenes/NNN_<name>.json` | `Scene` (`build_index`, `name`, `path`, `objects`) for 0 MainMenu, 1 Replay, 2 CharacterSelect, 3 ChSelect_online, 4 WinnerTriangle, 5 Tutorial |
| `prefabs/<file>_<pid>.json` | `Prefab {id, name, objects}`: every prefab hierarchy referenced from any scene, the player, the abilities or the global assets, transitively (132: all prefab roots in the asset files) |
| `prefab_index.json` | `{roots: {root id: {name, file, objects}}, objects: {any GameObject / component / Transform id inside a prefab: root id}}`: resolves any `$ref` into its prefab file (`GameData::prefab(id)` accepts any of these ids) |
| `player.json` | `Player {prefab, controller, input_updater, ability_pickup}`: the player prefab root (`GameSessionHandler.PlayerPrefab`'s object, named `Player`), its `SlimeController` component, the `InputUpdater` prefab and the ability pickup prefab (`AbilitySpawner.pickupPrefab`, `AbilityPickup_Dynamic`) |
| `abilities.json` | `Abilities` (see below) |
| `globals.json` | `Globals` (see below) |
| `assets.json` | id → `Asset {id, name, script, fields}`: every ScriptableObject in the asset files except fonts and the localisation table: 35 `AnimationData` (sprite animations), `PlayerColors`, `TeamColors`, `BeamColorList`, `Settings`, the 3 `NamedSpriteList`s, Input System actions (`InputActionAsset` with its bindings, `InputActionReference`s, `InputSettings`), TMP settings / sprite asset / style sheet |
| `localization.json` | `Localization` (see below) |
| `materials.json` | id → `Material {name, shader, render_queue, keywords, textures: {prop: {texture, scale, offset}}, floats, colors}` (110) |
| `animations.json` | `Animations {clips, controllers}`: the 11 Mecanim clips and 10 controllers (UI animators); same structure as rounds-port's (`AnimClip` curves as cubic segments, `AnimController` layers/states/transitions). Most in-game animation is the game's own frame animation (`AnimationData` + `SpriteAnimator`/`PlayAnimation`). |
| `fonts.json` | TextMeshPro font assets: `{id, name, atlases, fields, info, source_font}` (16). `info` has the typed metrics, glyphs, character map and kerning. Dynamic assets (`dyn_*`, `Dynamic_en_se_fallbacks`, `LiberationSans SDF - Fallback`; `fields.m_AtlasPopulationMode` 1) have empty atlases and rasterise from `source_font` at run time. |
| `font_files.json` | Unity `Font` id → `FontFile {name, file, family_names, font_size, ascent, descent, line_spacing, fallbacks, source_hash}` (6) |
| `fonts/<name>.ttf` | the embedded TrueType data (CJK font `XiaolaiMonoSC-Regular` is 21.8 MB) |

### Objects (levels, scenes, prefabs)

A level, scene or prefab is a flat list of `Object`s in which parents come
before their children (`parent` and `children` are indices into the list);
scene roots are in file order. Each object has:

- `id`, `name`, `active` (self), `active_in_hierarchy`, `layer`, `tag`, the
  `local` and `world` transforms, `transform` (the id of its `Transform`;
  fields referencing a Transform hold this id), and `rect` (RectTransform
  layout of UI objects).
- `sprite` (from `SpriteRenderer`): `enabled`, `sprite` id, `color` (tint),
  `flip_x`/`flip_y`, `draw_mode` (0 simple, 1 sliced, 2 tiled) with `size`,
  `sorting_layer` (name), `sorting_layer_index` (draw order),
  `sorting_order`, `mask_interaction` (0 none, 1 inside, 2 outside a
  `SpriteMask`), `materials` (names) and `material_ids`. Draw order follows
  Unity: sorting layer index, order in layer, then larger z first.
- `fix` (from `FixTransform`): `component`, `position`, `rotation`
  (radians), `up`, `right`, `offset`, all raw Q32.32.
- `shape` (from `DPhysicsBox` / `DPhysicsCircle` / `DPhysicsRoundedRect`):
  `component`, `enabled`, `kind` `box` (`extents`, `minimum_extents`),
  `circle` (`radius`) or `rounded_rect` (`extents`, `radius`: a box of half
  size `extents` grown by `radius`), `hierarchy_number`, `min_scale`,
  `max_scale`, `manually_call_init`; raw Q32.32. Extents and radii are
  world units at scale 1 (they already include the authored object scale:
  the water hit box has Transform scale 268x14 and extents 134x7).
- `particles`: the typed `ParticleSystem` + renderer (rounds-port's
  converter: modules in Unity units, see `crates/import/src/particles.rs`).
- `components`: every component except the Transform, in order, as `{id,
  type, script, enabled, fields}`. `script` is the namespace-qualified
  class for MonoBehaviours (`FixTransform`, `StickyRoundedRectangle`,
  `TMPro.TextMeshProUGUI`, ...). Raw `fields` are omitted only for
  `SpriteRenderer`, `ParticleSystem` and `ParticleSystemRenderer` (use the
  typed fields); `FixTransform` and the `DPhysics*` shapes keep theirs.
  `Object::component(class)` finds one by script or engine class.

In levels and scenes world transforms are scene world space; in prefabs
they are relative to the root's parent (the root's world = its local).

### Level

`levels/NNN_<name>.json` (`Level`):

- `build_index`, `name` (as in the original project: `Level1`.. for the
  base FFA levels, `TLevel1`..`TLevel13` and others for 2v2, `e 1`..`e 30`,
  `esnow 1`..`esnow 20`, `espace 1`..`espace 10`; names repeat across
  folders, so use `build_index` or `path` as the key), `path`.
- `level_type`: `GameSessionHandler.levelType`, 0 grass, 1 snow, 2 space.
- `team_spawns` (`GameSessionHandler.teamSpawns`, 4 raw `Vec2`) and
  `teammate_spawn_spacing`.
- `session`: all raw `GameSessionHandler` fields (timings before GO and
  sudden death, level animation curves, prefab refs, ...).
- `physics`: all raw `DetPhysics` fields of the scene (gravity, player
  gravity, rope and beam constants, beam width curve).
- `main_camera` (index of the object with `PlayerAverageCamera` and the
  main `Camera`), `ability_spawner` (index of the `AbilitySpawner`),
  `platforms` (indices of objects with `StickyRoundedRectangle`, in object
  order), `shapes` (indices of every object with a `shape`).
- `objects`: every GameObject of the scene, including the presentation
  (background, parallax `ParallaxLayer`s, water, cameras, slime-trail
  cameras, UI of the exit menu).

Platform objects typically carry `SpriteRenderer`, `StickyRoundedRectangle`
(`platformType`: 0 grass, 1 snow, 2 ice, 3 space, 4 robot, 5 slime),
`AnimateVelocity`, `WaterWaves`, `ShakablePlatform`, `FixTransform`,
`BoplBody` (mass, friction, ...) and `DPhysicsRoundedRect`; some also
`VectorFieldPlatform`, `AntiLockPlatform`, `DPhysicsCircle`/`DPhysicsBox`.
The water kill zone is a `DPhysicsBox` (`WaterHitBox`) without a visible
sprite.

### Abilities (`abilities.json`)

`Abilities {full, demo, full_list, demo_list}` from the `NamedSpriteList`s
`SteamManager.abilityIconsFull` / `abilityIconsDemo` (`AbilityIcons`, 31
entries, and `AbilityIcons_demo`, 12). The list order is the game's ability
index. Each `AbilityEntry`: `index`, `name` (`NullAbility`, `Random`,
`Dash`, `Grenade`, `Bow`, ...), `sprite` (icon), `prefab` (the
`associatedGameObject` prefab root under `prefabs/`), `offensive`, `kind`
(`Ability` or `InstantAbility`), `component` and `fields` of that base
component (`Cooldown`/`cooldown`, `OnlyCastableOnGround`, ...) and
`scripts` (the classes of the root's components: `Dash`, `ThrowItem2`,
`ShootQuantum`, `PlayerPhysics`, `DPhysicsBox`, ...). The spawned objects
(arrows, grenades, explosions, effects) are in `prefabs/` via the raw
references.

### Globals (`globals.json`)

`Globals`:

- `player_colors`, `team_colors`, `beam_colors`, `settings`: the
  ScriptableObjects as `Asset {id, name, script, fields}` (player colours
  reference `materials.json` entries: `playerMaterial`, `uiMaterial`).
- `sprite_lists`: every `NamedSpriteList` by asset name (`AbilityIcons`,
  `AbilityIcons_demo`, `boulders`) as `[{name, sprite, object,
  offensive}]`; `sprite_list_ids` their ids.
- `components`: singleton scene scripts by class, first occurrence in
  build order, as `{scene, object, component, fields}`: `AbilityGrid`,
  `AudioManager` (sounds and songs → AudioClip ids), `CharacterSelectHandler`
  (+`_online`), `CharacterStatsList`, `MainMenu`, `SettingsInitializer`,
  `SteamManager`, `TutorialGameHandler`, `WinnerTriangleCanvas`.

### Localization (`localization.json`)

The game's own `LocalizationTable` asset (not Unity Localization):
`languages` in field order (`en`, `jp`, `ko`, `fr`, `it`, `de`, `es`,
`ptbr`, `ru`, `tr`, `se`, `zhcn`, `zhtw`, `pl`), `strings` per language
(index-aligned; index 0 is the language's own name), `fonts` (the
`*FontAsset*` references → `fonts.json` ids). A `LocalizedText` component
translates its `TextMeshProUGUI`/`TextMesh` by looking up the lower-cased
English text in `en` (`Localization::text(lang, en_text)`).

## Validation

- `cargo run --release -p bopl-import --example validate_import` compares
  every component's raw `fields` of every level, scene and prefab with the
  UnityPy dump (`ref/dump`), and the typed `fix`/`shape` values with the
  dump's raw bits. Result (2026-10-17): 21,137 components compared, all
  equal; 2,668 typed values, all equal; 5 components without usable
  reference (UnityPy fails on `KeyBindButton`).
- `crates/import/tests/import_game.rs` imports the game into
  `target/tmp` and checks counts, the up-to-date logic, the player and
  ability prefabs, level 1's platforms and the localisation (skipped
  without `BOPL_GAME_DIR`).
- `level-sheet` renders every level from the import
  (`scratch/levels-sheet.png`): sprites in Unity draw order within the main
  camera's culling mask and view (orthographic size 30, 16:9), the
  reflective-water quad as a sky gradient, distortion/aurora shaders
  skipped, physics shapes outlined (all with `--shapes`), team spawns as
  red dots. Shapes line up with the platform sprites on all levels.

## Notes for the sim and renderer

- Use `fix` / `shape` / raw `Fix` fields as the simulation's initial state,
  bit for bit; Unity's float `Transform` is presentation only. Update order
  in the original follows `hierarchyNumber` (present on every
  `MonoUpdatable` script's fields and on `DetShape`).
- Sprites: quad = `rect` size / `pixels_per_unit`, origin at `pivot`; tint
  `color`; `texture_rect` addresses the PNG. Some sprites use tight meshes,
  but none are packed, so drawing the rect is exact up to transparent
  pixels.
- The look depends on custom shaders (see `materials.json`):
  `Unlit/SlimeTrailedMovable` (platforms with slime trails rendered by the
  per-platform `SlimeTrailCam`s), `Custom/ReflectiveWaterWithDistortion`
  (the full-screen `ReflectiveWater` quad: reflection + sky gradient
  `_Gradient`/`_Gradient2`, `_WaterColor`), `Custom/WaterDistortion`,
  `Unlit/GradientColor`, `Custom/SingleColorAlpha` / `AdditiveColoredA8`
  (Alpha8 sprites in one colour × `_Alpha`), `Custom/CustomColourShader`,
  `Custom/FlippableSprite`, `Unlit/aurora`. Several cameras render layer
  subsets (culling masks in each `Camera`'s raw fields; `FollowMainCamera`,
  `ParticleCamera`, `reflectiveCamera` scripts) and are composited.
- Frame animations: `AnimationData` assets (`assets.json`: named
  animations of `{sprite, frameDuration}` frames, in sim frames) driven by
  `SpriteAnimator` / `PlayAnimation` components.
- Sounds: `globals.components.AudioManager.fields` lists `sounds` (named
  clips with volume/pitch), `songs` and `menuSong` (`intro` + `songLoop`
  clip ids, `introLength`); clip ids resolve in `audio.json`
  (`GameData::audio_path`, `audio_by_name`).

# Status

Living plan. Update when a step finishes.

## Milestones

- [x] M0 Repo, toolchains in `tools/`, decompilation in `ref/decomp/`, backup
      wired (`.rsync-filter`, `.rsync-prescript`), quota-loop armed.
- [x] M1 📣 Asset import: textures, sprites, audio, all 124 levels, abilities
      and player data readable by `unity-assets`; `bopl-import` writes the data
      dir; contact sheet of every level rendered from imported data
      (`scratch/levels-sheet.png`).
- [x] M2 📣 Renderer + platform: SDL2/wgpu app shows a level; headless
      screenshots; controllers enumerated; audio plays (`docs/platform.md`;
      Level1 vs the original's capture: mean abs diff 1.9/255, sky 0.6).
- [x] M3 📣 Engine + one player: `Fix` math bit-exact (LUTs verified against
      the player's DLL), DetPhysics (boxes, circles, rounded rects), update
      order, a bopl moving, jumping and dying in a sandbox level.
      Sim side (`crates/sim`, `crates/content`, `docs/sim-design.md`): all
      Fix/Vec2/RNG/input golden vectors and all physics vectors match, LUT
      hashes match; Level1 sandbox (spawn on tick 167, corner walking, jump,
      water death, round end) and 12-player determinism/rollback tests pass.
      Presentation: `World::view()` drives the level view (bopl animation
      frames, player colours, facing, platform poses, slime trails painted
      into per-platform textures, death/water-splash effects, HUD ability
      indicators, sounds per event id, camera follow and shake);
      `bopl-port --headless --sandbox` → `scratch/m3/m3-jump.png`.
- [x] M4 📣 Local 2–4 players, full match flow: character select (colour,
      abilities), rounds, ability pickups, mid-game ability select, winner
      triangle, menus. Main menu (settings, language, credits), character
      select for up to 12 boxes (keyboard + mouse, keyboard halves,
      controllers; team, colour, 3 abilities by row or grid), fly-in, HUD,
      round end, loser circles + mid-game wheel (in the sim), winner triangle
      with prompts, next level from the bag (sim), pause prompt, back to
      character select / main menu; bots and a headless flow test
      (`scripts/dev/m4-flow.drive`) → `scratch/m4/m4-overview.png`,
      `scratch/m4/flow-*.png`, `scratch/m4/b{2,4,12}-*.png`. Abilities are
      still stubs (M5). Not yet: key rebinding screen, tutorial, online
      button, particles (see `docs/platform.md`, gaps).
- [x] M5 📣 All abilities: the 29 abilities simulated (see the workstream
      row) and drawn like the original: every ability object's look comes
      through `View` (frames, parts, poses, tints, shader values, ropes,
      beams, ability platforms, `Cue::Fx` presentation prefabs), drawn with
      the prefabs' renderers and 16 more reimplemented shaders (black hole
      grab-pass lensing, smear, lightning, beam, resizable platform, clone,
      expanding circle, ...), a particle system for the abilities' and the
      levels' ParticleSystems (sky sparkles, water wave rings), loop sounds.
      Gallery `scratch/m5/gallery.sh` → `scratch/m5/gallery.png`, one sheet
      per ability; reference captures of the original in
      `scratch/ref/abilities/`. The orange box of the bot match was the
      black hole (`BlackHole2`: a `rectSprite` × 100 with a grab-pass
      shader) drawn as a plain tinted sprite. Gaps: `docs/platform.md`.
- [x] M6 📣 12 local players on one screen: join screen, colours, spawns,
      camera, HUD. Character select with 12 devices (both keyboard halves +
      10 controllers, or keyboard + mouse + 11), two rows of boxes, 12
      colours; generated spawns (sim); the original's camera (zoom-out left
      to M9) with off-screen arrows; HUD indicators per player; winner
      screen layouts for any number of winners (`match_ui::winner_images`:
      the original's up to 4 draw / 3 team images, more along the banner in
      two staggered rows); circles shrink to fit 12; prompt delay capped at
      4 players. Tests: `scripts/dev/m6-12.drive` (12 devices join, choose,
      ready, 3 rounds with bots, wheel, back) → `scratch/m6/d12-*.png`,
      `--bot-match --players 12` → `scratch/m6/b12-*.png`.
- [x] M7 📣 Teams (any split of 12) and free-for-all: team row 0..11 (more
      than 4 boxes, and online), team colours 4–11 derived from the player
      palette (yellow, turquoise, violet, orange, grey, black, white,
      purple), team wins / draws / scores / loser circles per eliminated
      team (sim, unchanged rules), winner layouts up to 11 winners;
      `--bot-match --split 1,11` / `--teams T`, `--winner-shots --rounds R`.
      Screenshots `scratch/m7/{2v2v2v2v2v2,6v6,4x3,1v11}-*.png`
      (`scratch/m7/run.sh`).
- [x] M8 📣 Online over Warpzone with rollback; mixed local+online; tested
      against our own local warpzone-server with several instances. Game
      side in `crates/game/src/online/` (docs/net.md, "The game side"):
      `NetWorld` (NetGame for the sim session, 3-byte `PlayerInput`,
      settings blob, per-slot `extra` = abilities + keyboard/mouse), online
      character select = lobby (local boxes + remote boxes, room code,
      host starts; main menu → online → host / host private / join code;
      `--warpzone-host`, `--warpzone-join ROOM|CODE|LINK`, invite links,
      `WARPZONE_JOIN/HOST`), any mix of local and online slots, the wheel
      and winner screen as sim inputs, the leave rule in the sim
      (`flow::set_gone`: inert, removed at round end, back to character
      select below two teams), GGRS desync checks on `World::state_hash`,
      sound dedup under rollback (`EventGate`). Tests: `--net-harness`
      (headless processes with bots) and `scripts/net/game-matrix.sh`
      (12×1, 3×4, 2+4+6, 1×12, relay, spectators, loss/latency, leave/crash,
      desync injection: 10/10 PASS, results in docs/net.md),
      `scripts/net/gui-online.sh`
      (the full game hosting 4 local players + 2 × 4 remote bots, lobby →
      match → winner → lobby) → `scratch/m8/online-*.png`.
- [x] M9 📣 Audio, options, controller remapping, tutorial, polish, original
      feel pass (side by side with the original on the private Xvfb).
      Done so far (workstream "M9 UI" below): key/button rebinding in
      character select (per device type, `settings.json`), the tutorial
      (sim + screen, `docs/spec/tutorial.md`), online room browser, player
      name, game-data check between machines, remote busy dots; bots send
      normalised directions. Workstream "M9 polish": every option of the
      original (volumes live, rumble, vsync, resolution, screen mode,
      streamer mode) plus camera zoom, persisted; window robustness on the
      private Xvfb (resize, fullscreen, hot-plug in a match, no audio
      device, 30 min run, exact 60 Hz); beam/reel-in loops, force arrows,
      eat particles, duplicator platform effect, mouse in character select
      and on the wheel. Feel pass against the original (workstream "Feel
      pass vs the original", `docs/review/feel-pass.md`): scripted replays
      played by the original's viewer on the private Xvfb, tracked and
      compared tick by tick (`scripts/dev/golden/`); no simulation
      divergence (mechanics, space, ice, orbiting/robot platforms, all 29
      abilities, the real `1.rep`); tick correspondence frame i = tick i;
      winner/loser circle timing fixed. Presentation fixes (feel-pass.md
      "Presentation fixes", before/after in `scratch/m9/pres/`, milestone
      image `scratch/m9/final.png`): HUD indicator as the original's shader
      (size, rim, `_CDColor` multiply, Hermite pulse without the flash,
      uncastable dimming), invisibility alpha (no floor offline), black hole
      visual size and swallow particles, meteor dust (align to direction),
      TMP bold spacing and stroked-font outline ("WINNER!!"), Roll trail
      break-up (particle noise), cursor in the player's colour, wheel-row
      hover, build-tension rumble; the time-stop end and its fade measured
      equal (`a12b-time-stop-clock`); splats and the spike sprite checked
      equal. Prerelease pass (feel-pass.md "Presentation fixes
      (prerelease)", `scratch/final/`): menu lists slide on their authored
      `AnimateInOptions`/`AnimateOutOptions` curves (Exit position), teleport
      rings as Unity velocity-aligned quads with the core-only colour, the
      winner screen's exit over the next level (`TriangleOut`, prompts,
      circles sliding down), key bindings per player (device key: serial /
      GUID / type, saved), tutorial HUD (no locked indicators, fixed camera,
      orange dummy deaths). What remains (minor): black hole glow shading
      detail, no fade on "ability select", death splat randomness
      (docs/platform.md gaps).
- [x] M10 📣 Windows build cross-compiled, runs under Proton/Wine:
      `scripts/build-windows.sh` (x86_64-pc-windows-gnu, llvm-mingw + SDL2
      2.32.10 in `tools/windows`, `target/windows`), `scripts/run-windows.sh`
      (Proton's wine in bwrap, no network, Xvfb :66, prefix `tools/wineprefix`).
      Level 6 screenshot from the exe byte-identical to Linux
      (`scratch/m10/{linux,win}-l6.png`); 12-bot match: events identical tick
      for tick, shots 400/2999 identical, 1500 differs in 2 pixels (f32 libm
      sin/cos in presentation). Sim determinism: `scripts/check-determinism.sh`
      (`scripts/xcheck`: sim + the game's bots, `World::state_hash` every 60
      ticks, 5 configs × 6000 ticks, 2–12 players, FFA/teams, grass/snow/space)
      identical Linux vs Windows, also with each side's own import.
- [x] M11 📣 Packaging: `scripts/package.sh [--verify]` →
      `dist/bopl-port-0.1.0-linux-x86_64.tar.gz` (6.5 MB, play.sh, SDL2 not
      bundled unless `--bundle-sdl`), `-windows-x86_64.zip` (9.3 MB, SDL2.dll),
      `SHA256SUMS`; names checked against the descriptor's globs and exec paths;
      licenses (`packaging/gen-licenses.py`, OFL, SDL2), `THIRD_PARTY_NOTICES.md`;
      `scripts/check-no-game-data.sh` (repo + archives); CI (`.github/`:
      no-game-data, fmt, clippy, test, Linux + Windows packages; release on
      tag); README, `docs/players.md`, `docs/building.md`. Both archives tested
      unpacked in `scratch/pkgtest/` (import from `$BOPL_GAME_DIR` into a
      scratch data dir + headless screenshot; Windows under Wine, screenshot
      byte-identical to Linux).

## Workstreams (current)

| Stream | State |
|---|---|
| Reference dump (`scripts/dev/dump_assets.py` → `ref/dump/*.json`, 55 MB, UnityPy + TypeTreeGeneratorAPI) | done |
| Specs: engine (`fixmath`, `engine`, `physics`, `player`) | done (v1); oracle in `scratch/fixoracle` |
| Specs: abilities (`abilities.md`, `abilities/*.md`: 29 abilities + Random/Null) | done (v1); duplicated shared parts (gun body, throw-and-hold, scale ray) to reconcile; open items in the private `docs/spec/review/spec-open-items.md` |
| Specs: flow (`match-flow`, `levels`, `ui`, `net-original`, `audio`) | done (v1) |
| Asset reader + import (`crates/unity`, `crates/import`) | done (M1): all 48,502 objects read exactly, 0 value mismatches vs `ref/dump` except a known UnityPy misread; import of textures (BC7 via `bcdec_rs`), sprites, 121 Ogg clips, TMP fonts + TTFs, 124 levels + 6 scenes as object JSON with raw Q32.32 `FixTransform`/`DPhysics*` values, 132 prefabs (player, 31 abilities, everything referenced), globals, assets, localisation; validated against the dump (21,137 components equal); forced re-import 0.3–0.8 s, cold 1.8 s. Format: `docs/formats/import.md`, reader: `docs/formats/unity-assets.md`. |
| Reference capture (original under Proton on Xvfb :66, `scripts/dev/ref-run.sh`, `docs/reference-capture.md`) | done: menus, solo match, replay playback; offline by default |
| Platform + renderer (`crates/render`, `crates/game`) | done (M2): wgpu renderer with the original's camera rig as passes (background, mirrored reflection, perspective wave camera, platform mask, main), render textures by global name, sprite masks via stencil, the 9 level shaders reimplemented from their behaviour (gamma space), TMP SDF text (static atlases + dynamic glyphs generated from the TTFs); SDL2 window, keyboard/mouse + keyboard halves + any number of hot-plugged controllers, mixer with Ogg, the original's sound/song rules; `--headless`, `--level-sheet`, `--list-devices`, `--import`. Seam for the sim: `present::WorldView`. Gaps: particles (sparkly sky, water waves), slime-trail cameras, UI canvases (see `docs/platform.md`). |
| Simulation core (`crates/sim`) | done (M3 sim side, `docs/sim-design.md`): `Fix` (all 40 op families of `vectors.json` bit-exact, 0 mismatches), sin/tan LUTs generated at start-up in double-double (≈0.2 s, SHA-256 match; `libm` 0.2.16 also matches), Vec2, RNG, curves, introsort; DetPhysics (narrow phase, ray casts, point helpers: 13,500 vectors exact; broad phase, response, composites, queries); updater (batches, hierarchy numbers, brackets, destroy queue); SpriteAnimator; platforms (sticky, spring, shake, anti-lock, vector field, grow-on-start, BoplBody); the bopl in full (surface walking incl. Atan2 quirk, jump, air control, gravity incl. space, land/turn/stop, eating, pushing, kill rules, water/blast-zone death, permanent revive); match handler (fly-in/out, spawning for up to 12 players with generated spawns, round end, draw, sudden death, winner screen); ability framework + registry with 31 stub modules (each ability = one file), pickups and spawners; `View` + rollback-stable events; `PlayerInput` 3-byte Pod. Tests: 11 vector/unit tests + 4 level tests (`crates/content/tests/sim_levels.rs`). Perf (12 players, Level13, release): clone + step 113 µs, clone 5 µs. Faults (Fix div by zero …) end the round as a draw. Gaps: the 29 abilities themselves (M5), ropes/beams/duplicates (ability-owned hooks), revive clones, next-level flow (M4). Replay `scratch/ref/replays/1.rep` (solo, Level1, Bow/Gust/Dash): our death (water) on tick 6661, round end 6662; the file's length (6656 frames incl. 1..32 padding) puts the original's round end at ≤ 6655, so we are ≥ 7 ticks late — expected while Bow/Gust/Dash are stubs (the player cast all three); re-check after M5 and against a screenshot of the replay viewer. |
| Abilities (M5): four groups in parallel worktrees (G1 guns/beam, G2 throws/explosives, G3 movement/ropes, G4 misc incl. time stop), integrated on `abilities-integration` | implemented and integrated (`docs/review/abilities-integration.md`, architecture in `docs/sim-design.md`): all 29 abilities + Random; one ability body, one spawn path, one hook set, one BoplBody, one Explosion/Shockwave, shared PlantObjectOnPlatform/RingBuffer; cross-group interactions wired (ropes on rocks/black holes/platforms, Throw carrying engines and anchors, magnet boulders/items, smoke ignition, scale rays on black holes and spells, Duplicator clones of every spawned type). Tests: per-group suites, `abilities_all.rs` (12 players, all abilities, 7 levels, determinism + rollback), `abilities_interactions.rs`; `scripts/check-determinism.sh --game` passes. Replay `1.rep`: death tick 6636, round end 6637 (≤ 6655; confirmed against a capture of the original, `docs/review/feel-pass.md`). Gaps: see the review log. |
| Ability presentation (M5, `view.rs` looks, `match_view.rs`, `fx.rs`, `particles.rs`, `crates/render`) | done: looks for all 29 abilities, `Cue::Fx` effects, ropes, beams, particles (abilities + level sparkles/water rings), shaders, loop sounds; gallery + reference comparison (teleport, black hole, beam, roll, missile, ... in `scratch/ref/abilities/`); sim state hashes unchanged (looks and events are not hashed). Gaps: `docs/platform.md` (Gaps). |
| Playable match + menus (`crates/game`) | done (M3 presentation + M4): `sim_match.rs` (session = world + level cache; flow requests handled after each step), `match_view.rs` (sim view → `WorldView`: bopls, trails, effects, indicators, off-screen arrows, sounds by event id, camera), `ui.rs` (uGUI canvases from the import: RectTransform layout, Image, TMP text with margins/auto-size/wrap, masks, canvas scaler, localisation and per-language fonts), `menu.rs`, `charsel.rs`, `match_ui.rs` (winner triangle, circles + wheel, pause), `bots.rs`, `driver.rs` (headless scripted input with waits), `game_app.rs`. Sim additions (outside `abilities/`): `flow.rs` (level bag, per-level seeds, circles, mid-game wheel and winner-screen inputs from raw inputs, `FlowRequest`), `World::flow`, `View::flow`/extra player fields. Tests: `crates/content/tests/flow.rs` (winner screen → next level, wheel edits + blocking + back, determinism + rollback), UI layout/driver/bot/lobby unit tests. Gaps: particles, UI animator fidelity (approximated slides); key rebinding and tutorial: see M9 UI. |
| Import → sim adapter (`crates/content`) | done (M3): `bopl_content::load` builds `Arc<Content>` (sprite table, 35 `AnimationData`, 132 generic prefabs with raw `Value` fields, typed player and pickup prefabs, ability list, layer matrix) and `load_level` the `LevelData` (session values and Fix curves, DetPhysics parameters, platforms in DFS collection order, water box, spawners, 4 + 8 generated spawns); `.rep` reader/writer (`replay.rs`); `sim-trace` bin. Import gaps: none blocking. |
| Rollback netcode (`crates/net`) | done (net layer, M8 prerequisite): `bopl-net` = rounds-net/tfp-net (GGRS 0.13 over Warpzone peer channels, host-authoritative lobby, star topology with host forwarding, freeze/resume drops, spectators, desync reports, rejoin recognition), generic over the sim (`NetGame`, any `Pod` input), the settings blob (`MatchSettings::game`) and per-player `extra`; 12 slots, 12 unique colours, up to 12 teams, 60 Hz. Descriptor `packaging/warpzone/bopl-port.toml` passes `check-games`. 13 unit + 12 in-process match tests pass; multi-process matrix (12 × 1, 4 × 3, 2 + 4 + 6, loss/latency, forced relay, spectators, leave/crash, desync) 13/13 PASS. Hook points (character select = lobby, mid-game ability select = sim inputs) and game API in `docs/net.md`. |
| Online game side (`crates/game/src/online/`, M8) | done: adapter (`NetWorld`, settings blob, `extra`), `EventGate`, `OnlineMatch`, `OnlineScreen` (Warpzone login, host/join, lobby in character select, match, back to the lobby), harness `--net-harness`/`--net-peer`, `scripts/net/game-matrix.sh`, `scripts/net/gui-online.sh`. Sim additions (outside `abilities/`): `flow::set_gone`/`is_gone`/`connected_teams`, `Flow::gone`, gone players skipped by `teams_left`, the win and draw rules and `spawn_players` (session.rs). Room browser, name entry, game-data check and busy dots: see M9 UI. Gaps: mid-match join. |
| M9 UI: remap, tutorial, online browser | done (not the feel pass): **rebinding** (`remap.rs`, `charsel.rs` rebind cover, `input.rs` `KeyboardKeys`/`controller_buttons`, `Settings::bindings` → `AppRequests::bindings` → input hub; keyboard halves too; tests in `remap.rs`, `input.rs`, `driver.rs`; `scripts/dev/m9-remap.drive` → `scratch/m9/remap-*.png`, matches the original's cover `scratch/m9/ref/r02-one.png`); **tutorial** (`docs/spec/tutorial.md`; import `GameData::level(5)` built from the scene, `bopl_content::tutorial_level`, sim `tutorial.rs` + `LevelData::tutorial`/`World::tutorial`, game `tutorial.rs`, menu entry; `crates/content/tests/tutorial.rs`; `scripts/dev/m9-tutorial.drive` plays it through → `scratch/m9/tutorial-*.png`, start screen vs the original `scratch/m9/ref/t02-start2.png`); **online** (`online/browser.rs` room list, name entry, `NetConfig::content_id` + `Hello.content` (bopl-net protocol 2, reject reasons, guest leaves on reject), busy dots; `scripts/net/m9-online-ui.sh` → `scratch/m9/online-ui-*.png`, `online-reject-*.png`; `GUI_DRIVE=scripts/dev/m9-online-dots.drive scripts/net/gui-online.sh scratch/m9/dots`); `bots.rs` uses `angle_byte_from_direction` (`scripts/check-determinism.sh` re-run: 5/5 identical, Linux and Windows agree); `ui.rs` honours TMP's legacy `m_textAlignment`, world-space canvases; driver `key`/`button`/`type`, `wait tutorial|busy`, `--realtime`. Bindings are per player since the prerelease pass (`Bindings::devices` by `input::device_key`; ui.md 6); tutorial HUD, camera and dummy deaths as the original. Gaps: none known. |
| M9 polish (`crates/game`, `crates/render`; docs/platform.md "M9 polish") | done (not the feel pass): **options** (`menu.rs`, `shared.rs`): SFX/music volume applied live through mixer buses, abilities, screen shake, rumble (off/on/high, `match_scene.rs`: hit/kill/beam rumble at the prefab's motor speeds), vsync, resolution (display modes), screen (windowed/borderless/exclusive), hide (streamer mode: remote names "player N"), our camera zoom row (off by default: > 4 players, size 30..45 to fit the spread, bottom edge fixed); `master_volume` in `settings.json`; display applied on Save via `AppRequests::display`, start-up from the settings (`--windowed`, `--size`, `--fullscreen`, `--no-vsync`, `--camera-zoom` override). **Audio**: `PlayOrContinueLoop` per tick with `fadeLoop` fades (no loop outlives its match), music ducking during time stop, `beamLoopBig`/`beamLoopHP`/`reelInLoop` from world state (no sim change). **Presentation**: time-stop force arrows, eat particles in the eaten sprite's shape (orbital offset now honoured by the particle system), `DuplicatePlatformEffect` (CloneEffect direction/offset/progress, resizable variant), mouse hover/clicks in character select and on the wheel arrows/handle, settings arrow buttons, driver `point X Y`. **Robustness**: silent without an audio subsystem/device, playback device reopened when removed, controller re-paired after hot-plug mid-match, wgpu crash on suboptimal surfaces (fullscreen toggle) fixed, borderless covers the display without a WM, exact integer fixed step (60 ticks/s at any refresh), pending effects dropped while not drawn, minute health log. Tests: `scripts/dev/m9-window-test.sh [disk|bogusdriver]` on Xvfb :66 (60.0 ticks/s, no crash), `scripts/dev/m9-mouse.drive`, 30 min 12-bot run (collections flat, RSS plateaus at ≈1.0 GB from cached level textures), unit tests (settings compatibility, zoom, fixed step). Screenshots `scratch/m9/polish-*.png`. Gaps: see docs/platform.md (velocity-aligned particles and the winner exit done in the prerelease pass). |
| Feel pass vs the original (`scripts/dev/golden/`, `docs/review/feel-pass.md`) | done: 38 scripted replays (`scripts/dev/golden/scenarios/`: mechanics on Level1/2/21, Level12 orbit field, space robots/asteroids, ice, every ability with 2–4 players, a duplicator run with a walking "clock") + `1.rep`, played by the original (Steam mode, `:66`), captured at 60 fps, tracked (camera from static platforms vs our level renders, bopls by colour) and aligned to ticks (time warping; whole-frame matching against `bopl-port --replay` renders: `refine.py`); bin `golden` (gen/sim/track/compare), `capture.sh`, `run.sh`, `run-all.sh`, `refine-all.sh`, `sheet.py`. Tick correspondence: replay frame i = the original's tick i = our step i → i+1 (input-shift scan on orbiting platforms: only shift 0 matches); `1.rep` killed on tick 6636, decided on 6637 (6638 frames + 18 padding). No sim divergence; fly-in identical tick for tick. Fixes: the replay viewer's time-stop input rule (`Replay::viewer_inputs`), winner/loser circle slide timing (`match_ui.rs`), `bopl-port --replay`; spec net-original.md §7–8. Regression numbers from our sim only: `crates/content/tests/golden_scenarios.rs`. Presentation differences: fixed or measured equal (review log, "Presentation fixes"; scenario `a12b-time-stop-clock` added). |
| 12 players + teams presentation (M6, M7) | done: winner layouts for N winners, circle scaling, team colours 4–11, prompt delay cap, greyed circles of players who left; `scripts/dev/m6-12.drive`, `--split`, `--winner-shots`, `--rounds`. |
| Windows + packaging + CI (`scripts/{fetch-windows-toolchain,windows-env,build-windows,run-windows,check-determinism,check-no-game-data,package*}.sh`, `scripts/xcheck`, `packaging/{linux,windows,licenses,gen-licenses.py}`, `.github/`) | done (M10, M11): see the milestones and `docs/building.md`. `scripts/check-determinism.sh --game` re-run with the integrated abilities: Linux and Windows agree. |

## How to run

All after `source env.sh`; CPU-heavy steps under
the machine's performance-mode helper (`perf-mode run -r "<why>" -- <cmd>`
below) when it has one.

- Import into the user data dir (`$BOPL_PORT_DATA_DIR/import-v1`, dev:
  `cache/userdata/import-v1`): `cargo run --release -p bopl-import`
  (`--dev` → `cache/import`, `--force` re-imports, `--game DIR`).
- Level contact sheet: `cargo run --release -p bopl-import --bin level-sheet`
  → `scratch/levels-sheet.png` (`--level N --tile 1280x720 --shapes` for one
  level with physics shapes).
- The game (menus → character select → matches): `cargo run --release -p
  bopl-port` (imports on first start; `--data cache/import` uses the dev
  import as is). The M2 level viewer: `bopl-port --viewer [--level 27]`.
- Sandbox (one bopl, scripted input): `target/release/bopl-port --headless
  --sandbox [--level 6] [--script "230:;12:R;3:UJ;40:"] --shots 256,300
  --screenshot scratch/m3/s.png` (`<stem>-<tick>.png`).
- Bot match (1..12 bots, next level automatically): `bopl-port --headless
  --bot-match --players 12 [--teams 3] [--level 6] [--seed 3] --shots
  400,900 --screenshot scratch/m4/b.png [--ticks 3000]` (`RUST_LOG=info`
  logs deaths, round ends and level loads).
- Headless flow test (menu → select → 3 rounds → wheel → winner → back):
  `perf-mode run -r flow -- target/release/bopl-port --headless --drive
  scripts/dev/m4-flow.drive --screenshot scratch/m4/flow.png` (~2 min;
  syntax in `crates/game/src/driver.rs`).
- Headless screenshot: `target/release/bopl-port --headless --level 6
  --screenshot scratch/l6.png [--size 1024x768] [--time 2] [--camera-x -7.1]
  [--dump-targets scratch/rt]`; all levels through the GPU path:
  `... --headless --level-sheet scratch/levels-gpu.png [--size 320x180]`.
- Devices: `bopl-port --list-devices`; import only: `bopl-port --import
  [--game DIR] [--force]`; audio offline: `--headless --music --play-sound
  arrowShoot --wav scratch/a.wav`.
- Window test on the private Xvfb (virtual pads, disk audio): see
  `docs/platform.md`, "Testing on the private Xvfb".
- Simulation: `cargo test --release -p bopl-sim -p bopl-content` (golden
  vectors skip without `scratch/fixoracle/vectors.json`, level tests without
  an import); per-tick trace of scripted input or a replay:
  `cargo run --release -p bopl-content --bin sim-trace -- --level 6
  --script "220:;40:R;1:RJ;200:L" --every 5 --events` or `-- --replay
  scratch/ref/replays/1.rep` (CSV; `--out FILE`).
- Reader vs UnityPy dump: `cargo run --release -p unity-assets --example validate`;
  audio: `... --example audio_check`; import vs dump:
  `cargo run --release -p bopl-import --example validate_import`.
- Regenerate embedded type trees after a game update:
  `python -I scripts/dev/gen_typetrees.py`.
- Golden comparison with the original (`docs/reference-capture.md`, "Golden
  comparisons"): `perf-mode run -r golden -- scripts/dev/golden/run.sh
  scripts/dev/golden/scenarios/a04-bow.txt` (capture + track + compare →
  `scratch/golden/<name>/report.txt`), all: `scripts/dev/golden/run-all.sh`;
  side by side: `python -I scripts/dev/golden/sheet.py NAME TICK...`; our game
  playing a replay: `bopl-port --headless --replay FILE.rep --shots T,..
  --screenshot F.png`.
- Online (bopl-net, `docs/net.md`): `cargo test -p bopl-net`; local Warpzone
  server `scripts/net/dev-server.sh` (http 3024 / udp 3504, `BOPL_WZ_PORT`,
  `BOPL_WZ_UDP_PORT`; `--check` validates `packaging/warpzone`), then
  `WARPZONE_SERVER=http://127.0.0.1:3024 cargo run -p bopl-net --example netsim -- run --locals 4x3`;
  full matrix `scripts/net/netsim-matrix.sh [frames] [filters...]` (perf-mode).
- M9 polish: `perf-mode run -r win -- scripts/dev/m9-window-test.sh [disk|bogusdriver]`
  (private Xvfb :66 → `scratch/m9/polish/win*/`), `bopl-port --headless
  --drive scripts/dev/m9-mouse.drive --screenshot scratch/m9/polish/m.png
  --size 1280x720`, zoom: `--bot-match --players 12 --camera-zoom`.
- Presentation checks against the original: `python -I
  scripts/dev/golden/sheet.py NAME TICK... --crop X0,Y0,X1,Y1 --scale S`;
  wheel hover `bopl-port --headless --drive scripts/dev/m9-wheel-hover.drive
  --screenshot scratch/m9/wheel/h.png --size 1280x720`.
- M9 UI: `bopl-port --headless --drive scripts/dev/m9-remap.drive
  --screenshot scratch/m9/remap.png --ticks 2000`, `... m9-tutorial.drive
  --screenshot scratch/m9/tutorial.png --ticks 2600`, online
  `scripts/net/m9-online-ui.sh` (perf-mode; starts the dev server).
- Online with the game: host `bopl-port --warpzone-host` (or main menu →
  online), join `bopl-port --warpzone-join CODE|ROOM|LINK`
  (`WARPZONE_SERVER=http://127.0.0.1:3024` for the dev server); headless
  test `target/release/bopl-port --net-harness --locals 3x4 --levels 2
  [--delay 40 --jitter 10 --loss 3] [--leave M@F] [--crash M@F]
  [--desync M@F] [--spectators N] [--relay] [--private] [--teams T]`;
  matrix `scripts/net/game-matrix.sh [filter]` and the on-screen test
  `scripts/net/gui-online.sh` (both perf-mode, several minutes; they start
  the dev server if needed).
- 12 local devices: `bopl-port --headless --drive scripts/dev/m6-12.drive
  --screenshot scratch/m6/d12.png`; team bot matches with winner screens:
  `bopl-port --headless --bot-match --split 1,11 --winner-shots --rounds 3
  --screenshot scratch/m7/1v11.png` (`--teams T` for i mod T).
- Windows: `scripts/build-windows.sh` (fetches the toolchain), then
  `scripts/run-windows.sh --headless --level 6 --screenshot scratch/w.png`
  (any bopl-port args; `EXE=` another exe); determinism Linux vs Windows:
  `perf-mode run -r xcheck -- scripts/check-determinism.sh [--game]`.
- Release archives: `perf-mode run -r package -- scripts/package.sh --verify`
  (→ `dist/`, smoke tests in `scratch/pkgtest/`); `scripts/check-no-game-data.sh`
  (`--worktree`, `--archives FILE...`). Details: `docs/building.md`.
- Tests: `cargo test --workspace --release` (game-data tests skip without
  `BOPL_GAME_DIR`).

## Decisions (orchestrator defaults, revisit with the owner if needed)

- Owner, 2026-10-17: reference runs may use Steam mode when needed (offline
  default); a brief `/dev/uinput` virtual gamepad is OK for multi-player
  reference matches; the empty `compatdata/1686940` and
  `shadercache/1686940/*` folders Steam created were removed;
  `crates/unity/typetrees/scripts.json` stays committed.

- Teams for > 4 players: up to 12 teams (any split of 12, like rounds-port);
  team colours 4–11 derived from the 12 existing player colours.
- Camera: the original's fixed camera for ≤ 4 players; for more players the
  same, with an optional zoom-out (setting "camera zoom", off by default,
  M9 polish: up to 1.5× when the players spread wider than the screen).
- Vsync defaults to on (the original: off) so that the game does not spin
  uncapped by default; borderless fullscreen like the original.
- Online leave rule (docs/net.md): a player whose machine left is inert for
  the rest of the round, removed from the next level on, and fewer than two
  teams with connected players end the session (back to the lobby) —
  instead of the original's "everybody back to character select at once".
- Online teams: always `Teams(12)`; an unchosen team gets the smallest team
  (so the default is free-for-all).
- Original quirks (level bag returning to Level1, teammate spawn offset by
  raw team id) are reproduced by default; fixes only as options.
- Spawn points: the 4 per level for ≤ 4 players; generated extra points for
  more (match-flow.md §17.2).

## Release

- Published 2026-10-18: github.com/jarvis-jhk/bopl-port (public, fresh
  history), prerelease v0.1.0 built by `release.yml`, Warpzone descriptor
  `games/bopl-port.toml` (jarvis-jhk/warpzone). The specs moved to the private
  repository `jarvis-jhk/bopl-port-spec` (`docs/spec/`, nested checkout).

## Facts established

- Steam app 1686940, build 19133296. Unity **2022.3.60f1**, Mono
  (`Assembly-CSharp.dll` 4.1 MB, 375 types after ILSpy). Steam via
  Facepunch.Steamworks; new Input System; TextMeshPro.
- Scenes (`BuildSettings`): 0 MainMenu, 1 Replay, 2 CharacterSelect,
  3 ChSelect_online, 4 WinnerTriangle, 5 Tutorial, then 124 levels:
  FFA 6–44 and 68–69 (base, Snow, Space), 2v2 45–67, extraLevels 70–129
  (`e 1..30`, `esnow 1..20`, `espace 1..10`).
- Deterministic engine: `BoplFixedMath.Fix` is Q32.32 in an `i64`
  (`One` = 2³², FixedMath.NET style) with 205 887-entry sin and tan lookup
  tables; physics is the game's own `DetPhysics` (`DPhysicsBox`,
  `DPhysicsCircle`, `DPhysicsRoundedRect`, `FixTransform`), driven by
  `Updater`. Original netcode: lockstep input packets over Steam sockets.
- Original player count: 4; teams are a free per-player choice of team 0–3
  (2v2, 1v3, 1v1v2 are just choices). Sessions are endless (score = rounds
  won); see `docs/spec/match-flow.md`.

# Reference capture: running the original game

How to run the original Bopl Battle (Windows Unity build, Steam app 1686940)
headless for reference screenshots, recordings and replays. All of this is
for development only: nothing it produces (screenshots, videos, replays, logs)
may be committed. Output goes to `scratch/ref/`.

## Rules it follows

- The Steam install (`$BOPL_GAME_DIR`) is only read, once, to make a plain copy
  in `tools/bopl-original/` (`cp -a`, no hardlinks), plus a `steam_appid.txt`
  in the copy.
- Proton's prefix (`STEAM_COMPAT_DATA_PATH`) is `tools/bopl-prefix/`. HOME,
  XDG dirs and `/tmp` for Wine are also in `tools/`.
- Everything runs inside bubblewrap: `/` is read-only and only the project is
  writable. Proton's own `dist.lock` is replaced by a private file so Proton
  can't write to its install.
- Display: always the private Xvfb `:66` (`scripts/dev/xvfb.sh`). No window
  manager needed. Nothing appears on the desktop.
- Audio: Wine's pulse/alsa drivers are disabled, so FMOD falls back to silent
  software output.

## Scripts

| Script | Purpose |
|---|---|
| `scripts/dev/ref-run.sh [--bg] [args]` | start the game (copies the install on first use) |
| `scripts/dev/ref-run.sh --bg path/N.rep` | play a replay (the original loads its Replay scene) |
| `scripts/dev/ref-run.sh --stop` / `--status` | stop it (wineserver -k) / check it's running |
| `scripts/dev/ref-run.sh --wine reg query ...` | run a Wine tool inside the prefix |
| `scripts/dev/ref-shot.sh cmd...` | drive and capture: `shot NAME`, `key K...`, `hold K SEC`, `down/up K`, `click X Y [BTN]`, `move X Y`, `sleep S`, `rec NAME SEC` |
| `scripts/dev/xvfb-wm.sh` | optional metacity on `:66` (not needed) |

Environment variables: `REF_STEAM` (0 = offline, the default; 1 = talk to the running Steam client for the full ability set; Steam may create compatdata/shadercache folders and updates playtime — allowed when needed, owner 2026-10-17), `REF_GL` (0, or 1 for
OpenGL), `REF_SIZE` (1920x1080), `REF_DISPLAY` (:66), `REF_UNITY_ARGS`,
`PROTON_DIR` (Steam's "Proton - Experimental"), `PROTON_LOG=1`, `WINEDEBUG`,
`REF_WRAP` (e.g. an `strace` prefix). Logs: `scratch/ref/logs/run-*.log`
(Proton/Wine) and `*.log.unity` (Unity player log, passed with `-logFile`).

Example session (about 20 s from start to main menu):

    scripts/dev/ref-run.sh --bg
    scripts/dev/ref-shot.sh sleep 20 shot 01-main-menu
    scripts/dev/ref-shot.sh click 960 155 sleep 4 shot 02-charsel      # PLAY
    scripts/dev/ref-shot.sh click 417 835 sleep 2 shot 03-joined       # join box 1 with the mouse
    scripts/dev/ref-shot.sh key w w Return sleep 1.5 shot 04-grid      # ability slot 3, open grid
    scripts/dev/ref-shot.sh key Escape s Return sleep 1 key Return     # back, checkmark = ready, start
    scripts/dev/ref-shot.sh rec 08-round 12 &                          # 12 s video (x11grab)
    scripts/dev/ref-shot.sh hold d 0.8 key space click 1400 400 ...    # play
    scripts/dev/ref-run.sh --stop

Coordinates are in screen pixels at 1920x1080. Hold keys for at least about
150 ms (`ref-shot.sh` does this already) and pause before clicking so the UI
registers the hover. Now and then a key press is lost, so check the result
with a screenshot.

## What works

- Proton Experimental's `proton run` without the Steam Linux Runtime
  container, with D3D11 through DXVK on the real GPU. Main menu, character
  select, ability grid, a match, the winner screen and replay playback all
  work at full speed (about 1 CPU core).
- Steam integration: through Proton's lsteamclient the game talks to the
  user's running Steam client (via the `~/.steam/steam.pipe` FIFO and
  `/dev/shm`, the only shared writable path). The full ability set is
  available. The game's DLC check is `SteamClient.AppId == 1686940`.
- Offline mode (`REF_STEAM=0`): Steam is hidden (no `~/.steam` in HOME,
  private `/dev/shm`, `steamclient64.dll` disabled) and the game is started
  with `proton runinprefix`. Proton's `steam.exe` shim would wait forever
  without Steam. The game shows "ERROR: could not connect to Steam", Online is
  greyed out, and it runs in **demo mode**: most abilities are locked in the
  grid.
- Replays: every finished round writes `replays/N.rep`. Passing a `.rep` path
  on the command line plays it back in real time with the same outcome.
  Recording that with `ref-shot.sh rec` gives an input file plus a video of
  the original. This is the golden-test path.

## Files the original writes (inside our prefix)

`tools/bopl-prefix/pfx/drive_c/users/steamuser/AppData/LocalLow/Johan Gronvall/BoplBattle/`

- `replays/N.rep`: numbered from 1, at most 1000 kept. Format on disk: not
  gzip. It starts with the encoded start-request packet, followed by blocks of
  32 frames for 4 player slots each (an `InputPacketQuad`, so at most 4
  players), delta-coded against the previous frame. Details are in
  `docs/spec/net-original.md`. A solo 3-minute round came to 4.4 KB.
- `Unity/<id>/Analytics/{values,config}`: Unity analytics state.
- Settings are Unity PlayerPrefs, stored in the registry:
  `tools/bopl-prefix/pfx/user.reg`, key `[Software\\Johan Gronvall\\BoplBattle]`
  (NumberOfAbilities, MusicVolume, SfxVolume, Rumble, Vsync, resolution,
  FullScreen, Language, key bindings, ...; value names carry Unity's `_h<hash>`
  suffix).
- The Unity player log normally goes to `Player.log` in the same folder. We
  redirect it to `scratch/ref/logs/`.

## Limitations

- **Input devices.** The original allows exactly one keyboard+mouse player
  (one per character select). Every other player needs a gamepad, and keyboard
  "press any key to join" only works while an unassigned gamepad is connected,
  so the keyboard player joins by clicking a box. Two keyboard players are
  therefore impossible. A solo match works: one ready player gets START, and
  the round ends when that player dies. Multi-player reference matches need
  real or virtual gamepads. A virtual uinput gamepad (`/dev/uinput` is writable
  by the user) would be visible system-wide, including to the desktop session
  and Steam Input, so it needs the owner's approval first. Another option is to drive
  multi-player scenes from replays recorded elsewhere, e.g. on a real machine
  with pads.
- **Rendering on Xvfb.** By default, Mesa's Vulkan X11 WSI kills the X
  connection right after DXVK creates its swapchain ("XIO: fatal IO error 2 ...
  after 366 requests", 5–15 s after start). `ref-run.sh` therefore sets
  `MESA_VK_WSI_DEBUG=sw` (software present path, which works).
  `MESA_VK_WSI_DEBUG=noshm` still crashes. Unity's Vulkan renderer is not in
  this build. `REF_GL=1` (Unity OpenGL, running on llvmpipe) works too, but
  uses about 4–5 cores and screenshots lag behind input by seconds.
- **Steam side effects** (Steam mode only). These are writes made by the
  Steam client itself, which the sandbox cannot prevent. When it first saw the
  app running, Steam created an empty `steamapps/compatdata/1686940/` and empty
  `steamapps/shadercache/1686940/{DXVK_state_cache,fozmediav1,nvidiav1}`. It
  updates `appmanifest_1686940.acf` and the library cache (last played,
  playtime) when the game exits. In the main menu the game also creates a
  Steam lobby that friends can join. Offline mode has none of these effects,
  but it is demo-only.
- **Audio** is off. Recordings are silent. A null sink would need a private
  PulseAudio/PipeWire server, which isn't set up.
- **Replay playback** is real time only (no fast-forward found yet). Player
  colours in playback can differ from the live match (purple vs yellow in our
  test).
- Saved settings persist in the prefix (for example `FullScreen=1`). Delete
  `tools/bopl-prefix` for a clean first run.

## Golden comparisons (scripts/dev/golden/)

Our simulation against the original, tick by tick, from scripted replays
(method and results: `docs/review/feel-pass.md`). Everything the original
produces (captures, frames) stays in `scratch/golden/`.

| Piece | What |
|---|---|
| `scripts/dev/golden/scenarios/*.txt` | scripted scenarios (level, seed, up to 4 players with colour/team/abilities, per-player `TICKS:KEYS` steps; format in `crates/content/src/scenario.rs`) |
| `golden gen SC.txt OUT.rep` | compiles a scenario to a `.rep` (bin `golden` in `bopl-content`) |
| `golden sim REP OUT.csv [--shift N] [--platforms F]` | our sim with the replay viewer's input rule: per tick and player position, bopl transform, up vector, scale, modelled camera; `OUT.csv.events` (spawns, deaths, round end, winner screen) |
| `scripts/dev/golden/capture.sh REP OUT.mkv [SECONDS]` | the original plays the replay (Steam mode, `:66`), ffmpeg records 1920×1080 at 60 fps; waits for a quiet machine first (`GOLDEN_MAX_LOAD`) |
| `golden track MKV OUT.csv --ref R0.png --ref2 R10.png --colors C,..` | per frame: camera x (static platforms matched against our renderer's level screenshots at camera x 0 and 10; the pair selects pixels that move with the camera), and per player colour up to 6 blobs (eyes first; centroid, box, head caps) |
| `golden compare SIM.csv TRACK.csv [--y-only] [--out F]` | aligns frames to ticks (dynamic time warping: the viewer runs 0..4 ticks per rendered frame), fits the blob-to-position offset, reports residuals, first divergence, playback rate, death sightings |
| `scripts/dev/golden/run.sh SC.txt` / `run-all.sh` | the whole chain per scenario → `scratch/golden/<name>/report.txt`, `summary.txt` |
| `bopl-port --headless --replay REP --shots T,.. --screenshot F.png` | our game playing the same replay, for side-by-side frames |
| `scripts/dev/golden/sheet.py NAME TICK...` | our frames next to the original's aligned frames (scratch only) |
| `scripts/dev/golden/refine.py NAME` / `refine-all.sh` | re-aligns frames to ticks by matching whole frames against our renders of every tick (`map.csv`, then `golden compare --map`); for scenarios where the blob alignment drifted |

Notes:

- The replay viewer only works with the Steam API (`REF_STEAM=1`).
- Frame *i* of a replay is the input of the original's tick *i* = our tick
  *i + 1* (net-original.md §8).
- Player colours in a replay are the header's colour bytes (the original
  writes the player id); scenarios use violet (1), red (4), yellow (0) and
  orange (7), which stand out from grass levels.
- The original renders about 25 fps here; when the machine is busy it drops
  below 15 fps and the playback falls behind real time (the report then
  says SLOW). The alignment copes, but heavy jobs during a capture make the
  results noisy: capture first, track afterwards (`run-all.sh` does).
- Levels where every platform moves (Level12's orbit field, free robot
  platforms) have no static geometry for the camera match: compare with
  `--y-only` (`run.sh` does it when the scenario's comment says so).
- Timing needs motion in view: while nothing moves, a slow stretch of
  playback can shift the alignment by many ticks; give such scenarios a
  "clock" (a player walking back and forth, see `a29b-duplicator-clock`).
- Captures slower than 45 ticks/s somewhere are flagged SLOW in the report;
  they still compare, but re-capture on a quiet machine for timing claims.
- Expected numbers stored in git come from our sim only (no footage): see
  `crates/content/tests/golden_scenarios.rs`.

# Roadmap

What is done lives in [`docs/STATUS.md`](docs/STATUS.md) (milestones M0–M11:
the simulation, all 124 levels and 29 abilities, menus, tutorial, 12 local
players, teams, online over Warpzone with rollback, Windows, packaging).
This file is what comes next, most important first. Ideas and bug reports:
GitHub issues.

## Current priorities

1. **CI hardening.** A Wine job: the Windows build's smoke test (headless
   screenshot) and the Linux-vs-Windows state-hash check
   (`scripts/check-determinism.sh`) on every push, if it fits on the runner.
   Releases keep coming from `release.yml`; prereleases until the game is
   called ready.
2. **Warpzone end to end.** The launcher flow against the live server
   (warpzone.virtualtabletop.io): install, finding the game data, import,
   host/join, codes and invite links, teams. Fix what breaks.
3. **Fault reports.** Crashes and failed imports reported (opt-in, no
   personal data) through a public channel that turns into GitHub issues.
4. **Steam Deck.** Fullscreen and pad-friendly defaults under `SteamDeck=1`.
5. **Real hardware check.** Real controllers on a desktop, real Windows.
6. **Remaining presentation gaps** (docs/platform.md, "Gaps"): the black
   hole glow's shading detail, a fade on leaving "ability select", death
   splat sizes/rotations.
7. **Mid-match join** online (today a guest joins at the lobby).

## macOS support

Next after the current priorities (Jan, 2026-10-19). No implementation yet.

- **Build:** a macOS build for Apple Silicon and Intel (a universal binary
  via `lipo`, or two archives), built and smoke-tested (headless screenshot,
  `--version`, a short sim run) on a GitHub `macos-*` runner in `ci.yml`,
  and attached to every release by `release.yml` (`.app` in a `.zip` or
  `.dmg`, SDL2 bundled).
- **Player docs:** macOS install/start notes in `docs/players.md`. Unsigned
  is fine: document the Gatekeeper workaround (right-click → Open, or
  `xattr -dr com.apple.quarantine bopl-port.app`).
- **Known risks:**
  - *SDL2:* bundle the dylib inside the `.app` (or link statically) with the
    right rpath; Homebrew SDL2 on the runner is newer than what players have,
    and the run-time lookups of newer SDL functions (`platform.rs`, `vpad.rs`)
    must work against the bundled copy.
  - *Graphics:* wgpu's Metal backend (no OpenGL path on macOS); the window
    needs a Metal layer from SDL (`SDL_WINDOW_METAL`), Retina scaling
    (drawable size vs window size), and the headless/offscreen capture must
    work on the runner without a display.
  - *Controllers:* SDL's macOS game-controller backend (GCController / IOKit
    HID); Xbox/PlayStation/Switch pads, hot-plug, 12 players; virtual pads
    used by tests.
  - *Game data and paths:* the user data dir is
    `~/Library/Application Support/bopl-port` and Steam is looked up under
    `~/Library/Application Support/Steam` (already in `crates/import`, never
    tested). Bopl Battle's Steam release may be Windows-only, so a Mac player
    may have to fetch the Windows depot (SteamCMD with a forced platform) or
    copy the install over; a Mac Unity build would also lay its data out
    differently (`*.app/Contents/Resources/Data`). Check both.
  - *Warpzone:* the Warpzone launcher/client on macOS (does it exist, how it
    starts the game and passes the session), our descriptor in
    `packaging/warpzone/` needs a macOS entry, and cross-platform rollback
    needs the Mac build to match the Linux/Windows state hashes
    (`scripts/check-determinism.sh`, aarch64 included).
  - *Notarization:* not now; revisit if Gatekeeper friction is too high.

## Later

- Tier 3 on Warpzone: a ranked queue (1v1, 2v2) with placements checked by
  the simulation's state hash.
- The original's ability unlock progression as an option (everything is
  unlocked from the start by default).

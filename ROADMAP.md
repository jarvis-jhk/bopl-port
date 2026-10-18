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

## Later

- Tier 3 on Warpzone: a ranked queue (1v1, 2v2) with placements checked by
  the simulation's state hash.
- macOS build.
- The original's ability unlock progression as an option (everything is
  unlocked from the start by default).

# Lane notes (JARVIS)

Running log of the bopl-port lane: decisions, problems, what waits on whom.
The plan is `ROADMAP.md`, the state `docs/STATUS.md`.

## 2026-10-18 — handoff

- Taken over at 08bc708. CI and Release green on GitHub, v0.1.0 prerelease
  published, no open issues.
- First worker build: `cargo build --release` + `cargo test --workspace
  --release` in the worker (lavapipe, `/data/bopl` read-only, the carried-over
  `cache/userdata` import): 240 passed, 0 failed.
- `ROADMAP.md` added from STATUS.md and the handoff brief.
- Fault reporting: the repo is public, so the app must not carry the private
  report URL; plan is the public-channel → GitHub Action → issue route
  (roadmap item 3).
- No Wine/Proton in the worker: Windows smoke and Linux-vs-Windows
  determinism belong in CI (roadmap item 1).
- CI Wine job: `scripts/check-determinism.sh` needs an import, which CI
  cannot have (no game data). In CI, Wine can only cover the data-free parts
  (exe smoke start, Fix vectors / LUT hash, sim runs on a synthetic level if
  xcheck gets one). Decide that in the next session.
- 2026-10-19: ROADMAP gained a "macOS support" milestone after the current
  priorities (request from the adhoc lane on Jan's behalf; roadmap only, no
  implementation). The old "macOS build" line under Later went into it.

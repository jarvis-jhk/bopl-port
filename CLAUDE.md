# bopl-port — rules for agents

A clean-room Rust reimplementation of Bopl Battle (Johan Grönvall, Unity
2022.3.60f1, Mono) for Linux and Windows (macOS later): up to 12 players, local
on one screen with controllers, online over Warpzone, or both mixed. Read
`docs/ARCHITECTURE.md` and `docs/STATUS.md` first; `docs/STATUS.md` is the
single source of truth for what is done and what is next.

## Hard rules

- **No writes outside this directory** (`~/Projects/boplbattle`). Every tool
  and cache lives in `./tools` (`source env.sh` sets RUSTUP_HOME, CARGO_HOME,
  DOTNET, pip, XDG cache, TMPDIR, WINEPREFIX, WARPZONE_HOME, the port's data
  dir). No system packages, no `~/.cargo`, no `~/.local`, no global config, no
  `/tmp` (use `scratch/`). Other projects (`~/Projects/rounds`,
  `~/Projects/towerfall`, `~/Projects/warpzone`) may be *read* and code we own
  may be copied from them, never written to. The only exceptions are the
  machine's own helper tools the orchestrator runs.
  Never run `bopl-port` (or anything that imports) without `source env.sh`
  in the same shell: without `BOPL_PORT_DATA_DIR` it imports into
  `~/.local/share/bopl-port`. Beware `&` backgrounding the `source`.
- **The game install is read-only** (`$BOPL_GAME_DIR`). Never write into it.
  Running the original is allowed (owner, 2026-10-17) only headless on a private
  Xvfb (`scripts/dev/xvfb.sh`, display :66), from a copy under `scratch/` or
  `tools/`, with a Wine/Proton prefix inside `tools/`; never on the desktop.
  Steam mode (`REF_STEAM=1`, talks to the running Steam client: full ability
  set, playtime, Steam may create compatdata/shadercache folders) is allowed
  when needed (owner, 2026-10-17); offline stays the default. A virtual gamepad
  via `/dev/uinput` for multi-player reference matches is allowed briefly
  (visible system-wide): create it only for the run, remove it right after.
- **Nothing from the game goes into git**: no assets, no decompiled code, no
  extracted data or lookup tables, no screenshots containing game art in
  committed files. The repo will be public open source; the game's data is
  read from the player's own copy at run time.
- **Clean room.** `ref/decomp/` (ILSpy output) is reference only, git-ignored.
  Behaviour is described in our own words in `docs/spec/*.md` (formulas,
  constants, fixed-point bit patterns, update order and state machines are
  fine; code is not). Rust code is written from the specs; never transliterate
  C# from `ref/decomp`. If a spec is ambiguous, consult the decompilation, then
  fix the spec.
- **The specs are private.** `docs/spec/` is its own git checkout of the
  private repository `jarvis-jhk/bopl-port-spec` (with the spec review under
  `docs/spec/review/`) and is ignored here; commit spec changes there. Never
  publish, quote or paste them (this repo, issues, PRs, commit messages,
  release notes); code comments may cite a section (`docs/spec/physics.md
  §4.2`), not quote it.
- **Agents do not run git** unless the orchestrator says so; the orchestrator
  commits.
- CPU-heavy work (> ~30 s: release builds, test suites, imports, long headless
  runs): wrap it in the machine's performance-mode helper when it has one.
- Headless only: no windows on the user's desktop. Use `--headless` for
  screenshots or the private Xvfb; put output in `scratch/` (git-ignored).

## Product decisions (owner)

- Name: **bopl-port** (crates `bopl-*`, binary `bopl-port`).
- All abilities unlocked from the start (the original's unlock progression may
  come later as an option).
- Subagents: Claude only.

## Layout

| Path | What |
|---|---|
| `crates/unity` | `unity-assets`: Unity 2022.3 SerializedFile reader (copied from rounds-port, ours) |
| `crates/import` | `bopl-import`: Steam install → our own data format in the user data dir |
| `crates/sim` | `bopl-sim`: deterministic Q32.32 fixed-point simulation (no I/O, no rendering) |
| `crates/render` | `bopl-render`: wgpu renderer, offscreen/headless capable |
| `crates/net` | `bopl-net`: GGRS rollback over Warpzone peer channels |
| `crates/game` | `bopl-port` binary: SDL2 platform layer, menus, app flow |
| `docs/spec/` | clean-room behaviour specs written from the reference (private checkout, see above; `review/` = spec review) |
| `scripts/` | bootstrap, dev scripts |
| `packaging/warpzone/` | our Warpzone game descriptor |
| `ext/` | re-clonable third-party checkouts (warpzone, rea), git-ignored, read only |
| `ref/`, `cache/`, `scratch/`, `tools/`, `target/` | git-ignored |

## Conventions

- `source env.sh` before cargo/python/dotnet. Python tools: `python -I`.
- Rust 2024 edition, `cargo fmt`, `cargo clippy` clean, unit tests next to the
  code. The sim reproduces the original's fixed-point arithmetic bit for bit
  (`Fix` = i64 Q32.32); no floats in sim logic, no `HashMap` iteration in
  logic, no wall-clock time, no threads, all randomness from a seeded PRNG in
  the state.
- Keep `docs/STATUS.md` current when finishing a step.

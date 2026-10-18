# Building and developing

Everything a developer needs lives in `./tools` (git-ignored): the Rust
toolchain, .NET and ILSpy (only for the reference decompilation), a Python
venv, the Windows cross toolchain. Nothing is installed system-wide and
nothing outside the project directory is written. Players do not need any of
this: see [players.md](players.md).

## Setup

Prerequisites on the host: `curl`, `python3`, a C compiler (`cc`), the SDL2
runtime library (`libSDL2-2.0.so.0`; no `-dev` package needed locally) and a
Vulkan or OpenGL driver. For the Windows run script also `bwrap`
(bubblewrap) and Steam's "Proton - Experimental"; for packaging `zip`.

```sh
scripts/bootstrap-tools.sh     # rustup + stable, Windows target, dotnet, ilspycmd, venv, tools/lib/libSDL2.so
source env.sh                  # every shell: PATH, RUSTUP_HOME, CARGO_HOME, ... all under ./tools
cargo build --release          # or: cargo build  (dev profile: dependencies at opt-level 3)
```

`env.sh` also exports `BOPL_GAME_DIR` (default: the Steam install,
`~/.steam/steam/steamapps/common/Bopl Battle`) and `BOPL_PORT_DATA_DIR`
(`cache/userdata`, so development never touches `~/.local/share`). **Always
`source env.sh` in the same shell before running `bopl-port`**: without
`BOPL_PORT_DATA_DIR` the game imports into the real user data dir.

On a machine with `libsdl2-dev` installed (CI), `tools/lib` is not needed:
`crates/game/build.rs` only adds it to the link path when it exists
(`BOPL_SDL2_LIB_DIR` overrides it).

## The game data (import)

The repository holds no game data. The game and the tests read the player's
own Bopl Battle install (read-only) through a converted copy:

```sh
target/release/bopl-port --import                     # find Bopl Battle, convert into the user data dir
target/release/bopl-port --import --game "/path/to/Bopl Battle" [--force]
cargo run --release -p bopl-import -- --dev           # dev copy: cache/import (tools, tests)
```

The install is looked up in this order: `--game DIR`, `$BOPL_GAME_DIR`,
`$WARPZONE_DATA_BOPL` (set by the Warpzone launcher), then every Steam
library (Linux: Steam roots and `libraryfolders.vdf`; Windows: the registry
and Program Files). A warm re-import returns in ~0.05 s, a full one takes
~2 s; a changed format (`FORMAT_VERSION` in `crates/import`) re-imports on
the next start. Format: [formats/import.md](formats/import.md).

## Running

Headless only on the development machine (no windows on the desktop; use
the private Xvfb for windowed tests, [platform.md](platform.md)):

```sh
target/release/bopl-port --headless --level 6 --screenshot scratch/l6.png
target/release/bopl-port --headless --bot-match --players 12 --shots 400,900 --screenshot scratch/b.png
target/release/bopl-port --headless --drive scripts/dev/m4-flow.drive --screenshot scratch/flow.png
target/release/bopl-port --list-devices
```

The windowed game is `target/release/bopl-port` with no arguments. All
options: `bopl-port --help` and [platform.md](platform.md).
`RUST_LOG=info|debug` turns logging up; `WGPU_BACKEND=vulkan|gl` and
`WGPU_ADAPTER_NAME=llvmpipe` select the GPU (llvmpipe = software Vulkan,
useful for CI and for comparing output across machines).

## Tests, formatting, lints

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --release        # the simulation tests want --release
scripts/check-no-game-data.sh           # nothing from the game in the tracked files
```

Tests that need game data (level and flow tests in `crates/content`, import
validation, golden vectors from `scratch/fixoracle`) look for an import or
`BOPL_GAME_DIR` and **skip when it is absent**, so a plain checkout (and CI)
passes without Bopl Battle. The renderer's pixel tests need any Vulkan/GL
adapter (lavapipe is enough). Run the full set after an import.

`scripts/check-no-game-data.sh` rejects images, audio, Unity files, .NET
assemblies, C# and decompiled code, replays and any file over 256 KB that is
not allowlisted (with its reason) in the script; `--archives FILE...` checks
release archives, `--worktree` the tree instead of `git ls-files`.

## Online: dev server and netsim

```sh
scripts/net/dev-server.sh                  # local warpzone-server (ext/warpzone), http://127.0.0.1:3024, udp 3504
scripts/net/dev-server.sh --check          # validate packaging/warpzone/bopl-port.toml (check-games)
export WARPZONE_SERVER=http://127.0.0.1:3024
cargo run -p bopl-net --example netsim -- run --locals 4x3
scripts/net/netsim-matrix.sh [frames] [filters...]   # the multi-process matrix (minutes, all cores: perf-mode)
```

`BOPL_WZ_PORT` / `BOPL_WZ_UDP_PORT` move the server (the defaults avoid the
other ports projects). `ext/warpzone` is a re-clonable checkout of Warpzone
(server, launcher, docs). Details, link simulation and results:
[net.md](net.md).

## Windows (cross-compiled from Linux)

```sh
scripts/fetch-windows-toolchain.sh   # llvm-mingw + SDL2 2.32.10 mingw release into tools/windows (SHA-256 pinned)
scripts/build-windows.sh [--debug] [--xcheck]   # -> target/dist/windows/{bopl-port.exe, SDL2.dll}
scripts/run-windows.sh --version
scripts/run-windows.sh --headless --level 6 --screenshot scratch/win.png
```

* Target `x86_64-pc-windows-gnu`, linked with llvm-mingw's clang/lld;
  `fetch-windows-toolchain.sh` also creates `tools/windows/gcc-shim`
  (`libgcc`/`libgcc_eh` mapped onto compiler-rt/libunwind, which Rust's
  `-gnu` target asks for). `source scripts/windows-env.sh` sets the linker,
  `CC`/`AR` and that link path for any `cargo build --target
  x86_64-pc-windows-gnu`. Windows builds use `target/windows/`
  (`BOPL_WIN_TARGET_DIR`), so they never block or invalidate `target/`.
* Release builds are GUI-subsystem programs (no console window); logs go to
  stderr when it is redirected.
* `run-windows.sh` runs an exe (`EXE=...`, default the staged
  `bopl-port.exe`) with Proton's `wine` directly (no Steam, no proton
  script) inside bubblewrap: `/` read-only, only the project writable, a
  private `/tmp` and `/dev/shm`, **no network** (`BOPL_WINE_NET=1` keeps it,
  for online tests against the dev server), the prefix in `tools/wineprefix`,
  HOME and XDG dirs in `tools/`. Graphics: X11 on the private Xvfb `:66`
  (`BOPL_WINE_DISPLAY`), Vulkan through winevulkan to the host's Mesa, DXVK's
  `dxgi.dll` in the prefix. `BOPL_PORT_DATA_DIR` and `BOPL_GAME_DIR` are
  handed over as `Z:\...` paths; a caller's own `BOPL_PORT_DATA_DIR` wins
  over `env.sh`'s.

### Cross-platform determinism

Online play between Linux and Windows needs the simulation to be
bit-identical on both. The sim only uses `Fix` (i64) arithmetic; the
sine/tangent tables are generated from IEEE-754 basic operations only.
`scripts/check-determinism.sh [--game]` checks it:

* `scripts/xcheck` (`bopl-xcheck`, a small tool outside the workspace: the
  sim plus the game's own bots from `crates/game/src/bots.rs`) plays bot
  matches (12 FFA, 12 in 3 teams, 4, 8 in 2 teams, 2; five levels incl. snow
  and space; 6000 ticks with level changes) natively and as an exe under
  Wine, printing the lookup-table hash and `World::state_hash` (the hash
  online desync detection uses) every 60 ticks: the outputs must be
  identical. `BOPL_XCHECK_WIN_DATA` gives the Windows side the import
  `bopl-port.exe` made itself, as on two real machines.
* `--game` also runs `bopl-port --headless --bot-match --players 12` and the
  same with the exe: the event logs (deaths, round ends, level loads with
  their ticks) must match; screenshots are compared and reported.

Result (Oct 2026): all hashes identical, also with each side using its own
import; bot-match events identical; level screenshots byte-identical
(llvmpipe on both); bot-match screenshots identical but for 2 pixels (max
8/255) at one tick, from presentation-only `f32` `sin`/`cos` (platform
libm). The only difference between a Linux and a Windows import is the
presentation-only `world.rotation_z` (an `f32` `atan2`) in level JSON.

## Release archives

```sh
scripts/package.sh [--verify] [--bundle-sdl] [--no-build] [--linux-only | --windows-only]
scripts/package-linux.sh [--bundle-sdl] [--no-build] [--verify]   # dist/bopl-port-<version>-linux-x86_64.tar.gz
scripts/package-windows.sh [--no-build] [--verify]                # dist/bopl-port-<version>-windows-x86_64.zip
scripts/package-checksums.sh                                      # dist/SHA256SUMS (all of the above run it)
```

`<version>` is `[workspace.package] version` in `Cargo.toml` (also what
`--version` prints and the game reports to Warpzone). `dist/` is
git-ignored. Each archive has one top-level folder `bopl-port-<version>/`
with the binary, a README for players (`packaging/linux/README.txt`,
`packaging/windows/README.txt`), `LICENSE-*`, `THIRD_PARTY_NOTICES.md` and
`licenses/` (OFL for the dev font, SDL2's zlib license where SDL2 is
shipped, and `RUST_DEPENDENCIES.txt`: the license texts of every linked
crate for that target, generated by `packaging/gen-licenses.py`, which fails
on copyleft without an alternative). Archives are reproducible for a given
build and `SOURCE_DATE_EPOCH` (CI uses the commit time). Release builds go to
`target/package` (Linux) and `target/windows`; `--locked` is used when `CI`
is set.

`package.sh` builds both, checks the names against the Warpzone
descriptor's asset globs and exec paths, and runs
`check-no-game-data.sh --archives` over them.

* **Linux**: the stripped release binary (~17 MB; the Windows exe ~22 MB) and `play.sh` (puts a
  bundled `lib/` first on `LD_LIBRARY_PATH`, explains a missing SDL2, then
  execs the binary with all arguments). SDL2 is **not** bundled by default
  (every desktop distribution and SteamOS ship it, and the binary links the
  soname); `--bundle-sdl` copies the build host's library into `lib/`. The
  script refuses a binary that needs a newer glibc than 2.35 (Ubuntu 22.04,
  which CI builds on; `BOPL_MAX_GLIBC` overrides). One tarball serves
  desktop Linux and the Steam Deck.
* **Windows**: `bopl-port.exe` (stripped with llvm-strip) and `SDL2.dll`
  (official SDL2 2.32.10 mingw build).
* `--verify` unpacks into `scratch/pkgtest/{linux,windows}` and runs the
  packaged program with `BOPL_PORT_DATA_DIR` inside that folder:
  `--version`, `--list-devices`, and with the game installed
  `--import --game "$BOPL_GAME_DIR"` plus a headless level screenshot (the
  Windows one under `run-windows.sh`; it is compared with the Linux one, and
  with `WGPU_ADAPTER_NAME=llvmpipe` they are byte-identical).

## Continuous integration and releases

`.github/workflows/ci.yml` (push to `main`, pull requests), all on
`ubuntu-22.04`:

| Job | What |
|---|---|
| no-game-data | `scripts/check-no-game-data.sh` over the tracked files |
| fmt | `cargo fmt --check` (workspace and `scripts/xcheck`) |
| clippy | `clippy --workspace --all-targets -D warnings`, and `scripts/xcheck` |
| test | `cargo test --workspace --locked` (no game data: those tests skip; lavapipe for the pixel tests) |
| package (linux) | `scripts/package-linux.sh --verify` (smoke test without game data), artifact |
| package (windows) | toolchain (cached, pinned), `scripts/package-windows.sh`, PE and DLL-import check, artifact |

Rust is installed into `tools/` by `.github/actions/setup-rust`, so `env.sh`
and the scripts behave as on a developer machine; `libsdl2-dev` comes from
apt. The cross-platform determinism check needs game data and Wine, so it
runs on a developer machine, not in CI.

`.github/workflows/release.yml` runs on a tag `v<version>` (must equal the
version in `Cargo.toml`): checks for game data, builds both archives, writes
`SHA256SUMS` over them, checks the archives again and creates the GitHub
release (a prerelease for `v0.*` and tags with a `-suffix`).

To cut a release: bump `[workspace.package] version`, run any `cargo build`
(updates `Cargo.lock`), commit, tag `v<version>`, push the tag. If the
import format version changed, update the two `present` paths in
`packaging/warpzone/bopl-port.toml`.

## Warpzone descriptor

`packaging/warpzone/bopl-port.toml` tells Warpzone's server, website and
launcher how to install and start the game:

* `[install]` names the release assets (`bopl-port-*-linux-x86_64.tar.gz`
  for Linux and the Deck, `bopl-port-*-windows-x86_64.zip`) and
  `SHA256SUMS`; the launcher verifies downloads and unpacks them per
  version.
* `[launch]` runs `bopl-port-*/play.sh` (Linux, Deck) or
  `bopl-port-*/bopl-port.exe`; `--warpzone-join <room>` and
  `--warpzone-host` are handled by the game.
* `[[data]]` finds Bopl Battle (`BoplBattle_Data/globalgamemanagers`,
  `BoplBattle.exe`), passes it as `WARPZONE_DATA_BOPL` on every start, and
  runs `bopl-port --import --game {path}` once until `import-v1/index.json`
  exists in the data dir.

Validate after every change: `scripts/net/dev-server.sh --check`.

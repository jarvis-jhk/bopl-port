# bopl-port

A clean-room reimplementation of [Bopl Battle](https://store.steampowered.com/app/1686940/Bopl_Battle/)
in Rust, for Linux and Windows (macOS later): up to 12 bopls, local on one
screen with controllers, online through [Warpzone](https://github.com/jarvis-jhk/warpzone),
or both mixed.

**Status: feature-complete pre-release.** The asset import, the
renderer with the original's look, the bit-exact fixed-point simulation, all
29 abilities, the menus and options, character select for up to 12 players,
teams of any split, the full match flow (rounds, ability pickups, the losers'
ability wheel, winner screen, next level), online play over Warpzone with
rollback netcode, the tutorial and key/button remapping exist and are tested
headless, and a tick-by-tick feel pass against the original is done.
v0.1.0 is a prerelease. Details and the milestone list:
[docs/STATUS.md](docs/STATUS.md).

You need your own copy of Bopl Battle. bopl-port contains no game data: it
reads the art, sounds, music and levels from your installation at first start
(read-only, converted into its own data folder). Not affiliated with Johan
Grönvall or the Bopl Battle developers.

Clean room: the behaviour was first described in our own words from the
original (specs kept private, they describe its internals), and the Rust code
was written from those descriptions.

## Features

- The original's rules and feel: Bopl Battle's own deterministic Q32.32
  fixed-point arithmetic and physics reproduced bit for bit, all 124 levels,
  the original look (reflections, water, aurora, slime trails), menus and
  sounds from your install. All abilities unlocked from the start.
- Up to **12 players**: any mix of local players (keyboard and mouse, two
  keyboard halves, any number of controllers) and online players.
  Free-for-all or teams of any split.
- **Online over Warpzone**: public and private rooms, join
  codes, invite links, NAT traversal with relay fallback and **rollback
  netcode** (GGRS) on the deterministic simulation, so remote players feel as
  responsive as the one next to you. The simulation is checked to be
  bit-identical between the Linux and the Windows build.
- Linux (x86_64, Steam Deck) and Windows (x86_64) builds; the Windows build is
  cross-compiled from Linux and also runs under Wine/Proton.
- Headless mode (offscreen rendering, no window, no sound card) for
  screenshots, bot matches and automated tests.

## Install and play

See [docs/players.md](docs/players.md): install (Warpzone launcher, or the
tarball / zip from the [releases page](https://github.com/jarvis-jhk/bopl-port/releases)),
first start and import, controls, local and online multiplayer,
troubleshooting.

| System | Download | Start |
|---|---|---|
| Linux x86_64 (glibc 2.35+), Steam Deck | `bopl-port-<version>-linux-x86_64.tar.gz` | `./play.sh` |
| Windows 10/11 x86_64 | `bopl-port-<version>-windows-x86_64.zip` | `bopl-port.exe` |

## Screenshots

*None in the repository on purpose: every screenshot of bopl-port shows the
original's art (loaded from your install), and the repository never contains
images made from it.*

## Building and contributing

```sh
scripts/bootstrap-tools.sh   # toolchains into ./tools (nothing outside the project)
source env.sh
cargo build --release
target/release/bopl-port --import          # convert your Bopl Battle install
target/release/bopl-port                   # play
```

[docs/building.md](docs/building.md) covers the dev setup, the import, tests,
the Windows cross-build, the online dev server and netsim, packaging and CI.

| Doc | What |
|---|---|
| [docs/players.md](docs/players.md) | Installing and playing |
| [docs/building.md](docs/building.md) | Building, testing, packaging, releases |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crates, data flow, simulation design |
| [docs/platform.md](docs/platform.md) | Platform layer, renderer, headless modes |
| [docs/net.md](docs/net.md) | Online play, rollback, test results |
| [docs/STATUS.md](docs/STATUS.md) | Milestones and what is next |
| [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) | Third-party licenses |

The project is clean-room: behaviour is described in our own words in
`docs/spec/`, and the Rust code is written from those specs. No game assets,
no decompiled code and no images of the original's art are in the repository
or the release archives (`scripts/check-no-game-data.sh` checks both in CI).

## License

Code: MIT or Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)). Bopl Battle and its assets belong to their
authors and are not part of this project. Third-party components:
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

# Architecture

## Goals

1. Play Bopl Battle natively on Linux and Windows (macOS later) from the
   player's own Steam copy, with the original's look, feel, abilities and
   levels.
2. Up to 12 players: any mix of local players on one screen (controllers and
   keyboard) and online players. Free-for-all and teams.
3. Online play over [Warpzone](https://github.com/jarvis-jhk/warpzone) only
   (rooms, codes, NAT traversal, relay). No Steam networking.
4. Public open source: the repo holds no game data and no decompiled code.

## Data flow

```
Steam install (read-only)
  └─ unity-assets ── parses SerializedFiles (2022.3, format 22)
       └─ bopl-import ── writes <data dir>/import-v<N>/:
            textures/*.png, sprites.json, audio/*.ogg, levels/*.json,
            abilities.json, prefabs/*.json, fonts/…, fixlut.bin
                 └─ loaded at start by bopl-port (sim gets plain Rust data)
```

- The import runs at first start (or `bopl-port --import <path>`), finds the
  install through Steam's library folders, and is versioned, so a format change
  re-imports.
- The fixed-point sine/tangent lookup tables are taken from (or verified
  against) the player's own `Assembly-CSharp.dll`, never shipped.

## Simulation (`bopl-sim`)

- The original is already deterministic: all game logic runs on `Fix`
  (Q32.32 in an `i64`) and the game's own physics (`DetPhysics`), stepped by
  `Updater` at a fixed rate. We reproduce that arithmetic bit for bit, which
  gives the original's feel and makes rollback safe on every platform without
  float concerns.
- Pure function of `(state, inputs for every player) → state`. `State: Clone`;
  cloning is how rollback saves frames. No I/O, seeded PRNG in the state, no
  wall-clock time, no `HashMap` iteration in logic.
- Emits events (ability used, hit, death, sound cues) that the presentation
  layer turns into particles and sounds; events from rolled-back frames are
  deduplicated.
- Game flow (rounds, scores, ability selection, level changes) lives in the
  sim too, so it is identical on every peer, and selections are inputs.

## Players and input

- A player slot (up to 12) is bound to a device on some machine: keyboard
  and mouse, keyboard halves, or a controller (SDL2 GameController).
- Each frame, each machine produces inputs for its local slots only. Locally
  that goes straight into the sim; online, through GGRS.

## Networking (`bopl-net`)

- Same design as rounds-port: GGRS P2P over Warpzone peer channels
  (`warpzone-core`), host-authoritative lobby, star topology with host
  forwarding, freeze/resume for drop-outs, checksum desync detection.
  Descriptor in `packaging/warpzone/bopl-port.toml`.

## Presentation (`bopl-render`, `bopl-port`)

- SDL2 for window, input and audio; wgpu for rendering (sprites, shapes,
  particles, TextMeshPro SDF text from the import); the same renderer runs
  offscreen (`--headless`) for screenshots and tests.
- Camera: like the original for ≤ 4 players; more players get larger spawn
  layouts and a camera framing everyone.

## Builds

- Linux x86_64: `cargo build --release`.
- Windows x86_64: cross-compiled (`x86_64-pc-windows-gnu`), tested under
  Proton/Wine (`tools/wineprefix`).
- macOS: later.

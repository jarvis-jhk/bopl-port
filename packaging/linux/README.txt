bopl-port - a clean-room reimplementation of Bopl Battle (Linux x86_64)
=======================================================================

You need your own copy of Bopl Battle (Steam: https://store.steampowered.com/app/1686940/Bopl_Battle/).
This package contains no game data; the first start converts the art, sounds,
music and levels of your installation into bopl-port's own data folder
(~/.local/share/bopl-port, about 170 MB). The Steam install is only read,
never modified. Not affiliated with Johan Gronvall or the Bopl Battle
developers.

Status: early development (see the project page below for what works).

Start
-----
    ./play.sh

The first start looks for Bopl Battle in your Steam libraries. If it is
somewhere else (the folder that contains BoplBattle.exe and BoplBattle_Data):

    ./play.sh --import --game "/path/to/Bopl Battle"
    ./play.sh

Requirements
------------
- 64-bit Linux with glibc 2.35 or newer (Ubuntu 22.04+, Debian 12+, Fedora 36+,
  SteamOS 3 / Steam Deck).
- SDL2 (libSDL2-2.0.so.0). Steam Deck and nearly every desktop already have
  it. A package built with --bundle-sdl carries its own copy in lib/.
- A Vulkan driver (or OpenGL as fallback): Mesa (AMD, Intel), NVIDIA
  proprietary. WGPU_BACKEND=gl ./play.sh forces OpenGL.

Steam Deck
----------
Desktop Mode: unpack the archive anywhere in your home folder, right-click
play.sh -> "Add to Steam" (or Steam -> Games -> Add a Non-Steam Game ->
Browse, choose play.sh; set the file filter to "All Files"). In Steam Input
choose the "Gamepad" template.

Controls
--------
                    Move              Jump           Abilities 1 / 2 / 3          Start         Back
Controller          left stick, pad   A (South)      X / B / Y (West/East/North)  Start         Back
Keyboard + mouse    W A S D, arrows   Space          left / right / middle mouse  Enter         Esc
Keyboard, left      W A S D           Space          F / H / G                    Tab           Esc
Keyboard, right     arrow keys        Right Shift    , / slash / .  (or KP 1/3/2) Keypad Enter  Delete

Up to 12 players on one screen: every controller and both keyboard halves can
join in character select (A or Start on a pad, Tab / Keypad Enter for the
halves, Space or Enter for keyboard + mouse). Fullscreen: F11 or Alt+Enter.

Online play
-----------
Online play through Warpzone (https://github.com/jarvis-jhk/warpzone) is being
built: rooms, codes and invite links, any mix of local and online players up
to 12. If you installed bopl-port through the Warpzone launcher, it starts the
game straight into a room from an invite link.

Troubleshooting
---------------
- "needs the SDL2 library": install your distribution's SDL2 package.
- Black window / "no suitable GPU": update your graphics driver; try
  WGPU_BACKEND=gl ./play.sh. ./play.sh --list-devices shows what the game sees.
- No controller: ./play.sh --list-devices lists controllers; a pad without an
  SDL mapping can be added with a gamecontrollerdb.txt next to the binary.
- Import looks wrong: delete ~/.local/share/bopl-port/import-v* and start again.
- Log output: RUST_LOG=info ./play.sh

Licenses
--------
bopl-port is MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE). Third-party
components: THIRD_PARTY_NOTICES.md and the licenses/ folder.
Source and documentation: https://github.com/jarvis-jhk/bopl-port

bopl-port - a clean-room reimplementation of Bopl Battle (Windows x86_64)
=========================================================================

You need your own copy of Bopl Battle (Steam: https://store.steampowered.com/app/1686940/Bopl_Battle/).
This package contains no game data; the first start converts the art, sounds,
music and levels of your installation into bopl-port's own data folder
(%APPDATA%\bopl-port, about 170 MB). The Steam install is only read, never
modified. Not affiliated with Johan Gronvall or the Bopl Battle developers.

Status: early development (see the project page below for what works).

Start
-----
Unzip the folder anywhere (not into Program Files) and run bopl-port.exe.
SDL2.dll must stay next to it.

The first start looks for Bopl Battle through Steam (registry and library
folders). If it is somewhere else, open a command prompt in this folder:

    bopl-port.exe --import --game "D:\Games\Bopl Battle"
    bopl-port.exe

(the folder that contains BoplBattle.exe and BoplBattle_Data).

Requirements
------------
- Windows 10 or 11, 64-bit.
- A graphics driver with Vulkan 1.1 or DirectX 12. Setting WGPU_BACKEND=dx12
  or WGPU_BACKEND=vulkan in the environment forces one.
- Windows SmartScreen may warn about an unsigned download: "More info" ->
  "Run anyway". The checksum is in SHA256SUMS next to the download.

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
More than four Xbox pads work through raw input; any SDL-supported pad works.

Online play
-----------
Online play through Warpzone (https://github.com/jarvis-jhk/warpzone) is being
built: rooms, codes and invite links, any mix of local and online players up
to 12. Allow bopl-port.exe in the Windows firewall prompt for direct
connections; otherwise the Warpzone relay is used.

Troubleshooting
---------------
- Window does not open / "no suitable GPU": update the graphics driver; try
  WGPU_BACKEND=dx12 or vulkan. "bopl-port.exe --list-devices > devices.txt"
  writes what the game sees.
- Log output (the game has no console window): in a command prompt,
  "set RUST_LOG=info" and then "bopl-port.exe 2> bopl-port.log".
- Import looks wrong: delete %APPDATA%\bopl-port\import-v* and start again.

Licenses
--------
bopl-port is MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE). Third-party
components, including SDL2 (zlib license): THIRD_PARTY_NOTICES.md and the
licenses\ folder.
Source and documentation: https://github.com/jarvis-jhk/bopl-port

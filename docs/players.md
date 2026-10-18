# Playing bopl-port

> **Status: early development.** This page describes what the game does
> today; the remaining work (the feel pass against the original, some
> presentation details) is tracked in [STATUS.md](STATUS.md).

bopl-port is a clean-room reimplementation of Bopl Battle for Linux and
Windows (macOS later). It contains none of the original's art, sound or code:
you need your own copy of [Bopl Battle on Steam](https://store.steampowered.com/app/1686940/Bopl_Battle/),
and bopl-port converts what it needs from that install the first time it
starts. The Steam install is only read, never modified. Not affiliated with
Johan Grönvall or the Bopl Battle developers.

## Install

Pick one way.

**With Warpzone (easiest, keeps itself up to date).** Install the
[Warpzone launcher](https://github.com/jarvis-jhk/warpzone), open the
"Bopl Battle (bopl-port)" page and press Install. The launcher finds your
Bopl Battle install itself (Steam libraries, Proton and Wine prefixes), has
the game convert it once, and verifies the download against its checksum. It
also starts the game straight into a room from invite links and "Join"
buttons.

**By hand.** Download from the
[releases page](https://github.com/jarvis-jhk/bopl-port/releases):

| System | File | Start |
|---|---|---|
| Linux (x86_64, glibc 2.35+) and Steam Deck | `bopl-port-<version>-linux-x86_64.tar.gz` | unpack, run `./play.sh` |
| Windows 10/11 (x86_64) | `bopl-port-<version>-windows-x86_64.zip` | unzip, run `bopl-port.exe` |

Check the download against `SHA256SUMS` (`sha256sum -c --ignore-missing SHA256SUMS`
on Linux, `Get-FileHash` in PowerShell). Linux needs SDL2
(`libSDL2-2.0.so.0`, already present on the Steam Deck and nearly every
desktop distribution; otherwise install `libsdl2-2.0-0` / `SDL2` / `sdl2`
with your package manager) and a Vulkan or OpenGL driver. Windows needs a
Vulkan or DirectX 12 capable graphics driver; keep `SDL2.dll` next to the
exe, and unzip somewhere other than Program Files. SmartScreen may warn
about the unsigned download ("More info", "Run anyway").

**Steam Deck.** In Desktop Mode unpack the tarball somewhere in your home
folder, right-click `play.sh` and choose "Add to Steam" (or Steam, Add a
Non-Steam Game, Browse, show All Files, pick `play.sh`). Controllers work in
Game Mode with the "Gamepad" Steam Input template.

## First start and import

1. Start the game. If it finds Bopl Battle in a Steam library it converts the
   data (a few seconds) and opens the main menu. The converted data goes to
   `~/.local/share/bopl-port` (Linux) or `%APPDATA%\bopl-port` (Windows),
   about 170 MB, in a folder `import-v<N>`. Delete that folder to start over;
   nothing else is touched.
2. If Bopl Battle is not found (a library Steam does not list, a copy from
   somewhere else), point the game at the folder that contains
   `BoplBattle.exe` and `BoplBattle_Data`:

   ```sh
   ./play.sh --import --game "/path/to/Bopl Battle"           # Linux
   bopl-port.exe --import --game "D:\Games\Bopl Battle"       # Windows (command prompt)
   ```

   or set `BOPL_GAME_DIR` to that folder. Then start the game normally. If
   the install is removed later, the converted data keeps working.
3. After a bopl-port update that changes the data format, the next start
   converts again by itself.

Settings (volumes, number of abilities, screen shake, rumble, display,
streamer mode, camera zoom, language, your key and button bindings, your
online name) are kept in `settings.json` in the same folder.
`BOPL_PORT_DATA_DIR` moves the whole folder elsewhere.

## Menus

Main menu: **Play** (character select), **Online** (see below),
**Tutorial**, **Settings** (below), the language selector (every language of
your Bopl Battle copy), **Credits**, **Exit**. F11 or Alt+Enter toggles
between a window and borderless fullscreen.

**Settings** (left/right or the arrow buttons change a row, **Save** keeps
everything and applies the display rows; Esc leaves without changing the
display):

| Row | Values | |
|---|---|---|
| SFX vol, music vol | 0 to 10 | apply at once (default 10 and 7, like the original) |
| Abilities | 1, 2, 3 | ability slots per player |
| Screen shake | on / off | |
| Rumble | off / on / high | controller rumble when you are hit, when you get a kill and while a beam charges; high = 3× stronger |
| Vsync | on / off | on by default (the original defaults to off) |
| Resolution | your display's modes | the window size, or the mode of exclusive fullscreen |
| Screen | windowed / borderless / fullscreen | borderless fullscreen by default |
| Hide | nothing / names / names and avatars | streamer mode: other players' names show as "player 1", "player 2", ... online |
| Camera zoom | on / off | ours, off by default: with more than 4 players the camera zooms out (up to 1.5×) when the players spread wider than the screen |

`settings.json` also has `master_volume` (0 to 10, default 10; the original
has no master slider). Command-line options override the display settings
for one run: `--windowed`, `--size WxH` (a window of that size),
`--fullscreen`, `--no-vsync`.

**Tutorial.** The original's tutorial: walk to the floating arrows (the
hints show the keys), jump for the Bow, shoot the target dummy, catch the
Grenade and throw it at the second dummy (an arrow works too), catch Dash
and dash, and you are back in the main menu. You cannot die for good (the
water puts you back at the start). The hints follow your device: press a
button on a controller and they switch to controller buttons (that
controller then plays), press a key and they switch back. Esc (or Back on
the controller) leaves at any time. If you start it with a controller, it
plays with that controller.

## Controls

The default controls (every player can remap their own, see "Changing keys
and buttons" below):

| | Controller | Keyboard + mouse | Keyboard, left half | Keyboard, right half |
|---|---|---|---|---|
| Move / aim | left stick, d-pad (right stick aims) | W A S D or arrows; mouse aims | W A S D | arrow keys |
| Jump | A (South) | Space | Space | Right Shift, Keypad 0 |
| Ability 1 | X (West) | left mouse button | F | `,` or Keypad 1 |
| Ability 2 | B (East) | right mouse button | H | `/` or Keypad 3 |
| Ability 3 | Y (North) | middle or side mouse button | G | `.` or Keypad 2 |
| Join / Start | A or Start | Space, Enter or a click | Tab | Keypad Enter |
| Back | Back | Esc, Backspace | Esc | Delete |

* Menus take the arrow keys and W A S D, Space/Enter to accept, Esc or
  Backspace to go back, and any pad's d-pad or stick, A and B.
* The keyboard + mouse player and the two keyboard halves exclude each
  other (they share keys): either one player on keyboard and mouse, or two
  players on one keyboard.
* Any controller SDL knows works, and any number of them; on Windows more
  than four Xbox pads work through raw input. A pad SDL does not know can be
  added with a `gamecontrollerdb.txt` next to the program (community
  database: github.com/mdqinc/SDL_GameControllerDB).
* Start (controller) or Esc (keyboard) opens the pause prompt in a match.
* The mouse works in the menus and in character select (hover a row, click
  the side arrows, click an ability to open the grid and an icon to pick
  it, click ready), on the winner screen's prompts and on your ability
  wheel's arrows.
* If a controller is unplugged during a match, plug one in (the same or
  another) and press a button: it takes over that player.

### Changing keys and buttons

In character select, go up to the round keyboard or controller icon at the
top of your box (or click it with the mouse) and press Accept: the box asks
for one control after the other, like the original:

* keyboard (+ mouse) and the keyboard halves: jump, the three abilities in
  their on-screen order (left = ability 1, top = ability 3, right =
  ability 2), then up, left, down, right;
* controllers: jump and the three abilities (the sticks and the d-pad
  always move).

Press the key, mouse button or controller button you want for each. A
control you already picked in this round is refused (the entry wiggles);
Esc (keyboard) or Back (controller) cancels and keeps the old controls.
Start, Back, Guide and the d-pad of a controller and a keyboard half's own
Start/Back keys stay reserved; a keyboard half cannot take a key of the
other half. The keyboard + mouse player keeps the arrow keys for moving
unless one of them was given another job.

The new controls apply at once to **that box's device only**: every player
has their own set, so two people with the same kind of pad can play with
different buttons. They are saved in `settings.json` and come back with the
device: the keyboard + mouse player and each keyboard half by name, a
controller by its serial number (most USB and Bluetooth pads report one),
otherwise by its model and the order it was plugged in among identical
pads. `bopl-port --list-devices` shows each controller's key. A device
without its own set uses the defaults (or the per-type set an older
version saved). This works the same in local and online matches. Menus keep
their fixed keys (arrows, Space/Enter, Esc/Backspace; A/Start and B/Back on
controllers). To go back to the defaults, delete the `bindings` entry from
`settings.json` (or just the device's entry under `bindings.devices`).

## Local multiplayer (up to 12 on one screen)

Play opens character select. Every device joins with its Join button (A or
Start on a controller, Tab and Keypad Enter for the keyboard halves, Space,
Enter or a click on a free box for keyboard + mouse); up to 12 boxes, four in
the original's row and more in two rows. Each player picks a colour (12, all
different), a team, and their abilities (one to three, as set in Settings;
all abilities are unlocked), then readies up. The match starts when everybody
is ready.

A match is endless rounds on levels drawn from the original's level bag: the
last bopl (or team) standing wins the round. Losers get a ring around their
portrait on the winner screen and can open the ability wheel to swap one
ability before the next level; press Jump to continue, or go back to
character select from the winner screen.

## Online (Warpzone)

Online play uses [Warpzone](https://github.com/jarvis-jhk/warpzone): accounts
are anonymous device keys with a username, rooms and join codes come from the
Warpzone server, connections are encrypted and direct where possible (NAT
traversal), through Warpzone's relay otherwise. The game runs a rollback
simulation (the original's deterministic fixed-point physics, identical on
Linux and Windows), so remote players feel as responsive as local ones as
long as the connection is decent (under ~150 ms round trip is comfortable). A
match has at most 12 players in any mix: twelve machines with one player
each, three couches of four, ...

**Main menu → Online:**

* **Host game**: a public room, listed in everybody's room list. The room's
  id is shown at the top of the lobby.
* **Host private**: an unlisted room with a join code (shown at the top of
  the lobby, e.g. `K7QM3X`); give the code to your friends.
* **Join code**: type the code (or a public room's id) and press Enter.
* **Room list**: the public bopl-port rooms with their player count,
  refreshed every few seconds; pick one to join (left/right turns the page
  when there are many). Rooms of another bopl-port version are marked and
  cannot be joined.
* **Name**: the name the others see (letters, digits, spaces; Enter keeps
  it, Esc cancels). Empty = your Warpzone account name. With several
  players on your machine they show as "name 1", "name 2", ...

The lobby is character select: everybody seats their local players
(keyboard, keyboard halves, controllers, as offline) and readies up; remote
players appear as boxes after yours. The host starts once everybody is
ready and at least two teams play. Leaving the lobby (Esc / Back without a
box) leaves the room. On the winner screen a remote player who has the
ability wheel open shows three bouncing dots on their circle.

From the command line (or the Warpzone launcher):

```sh
./play.sh --warpzone-host                 # host a public room at once
./play.sh --warpzone-join K7QM3X          # join by code
./play.sh --warpzone-join v7thncejf4      # join by room id
./play.sh --warpzone-join 'warpzone://join/v7thncejf4#secret'   # invite link
```

(`WARPZONE_HOST=1` and `WARPZONE_JOIN=...` do the same.) The game talks to
the public Warpzone server by itself; `WARPZONE_SERVER=<url>` points it at
another one (for example your own, see Warpzone's self-hosting guide).

**Every machine needs the same bopl-port version and the same Bopl Battle
build** (the simulation reads the levels and abilities from your converted
copy). The host checks both when you join and refuses otherwise, with the
reason on the main menu ("different bopl battle data than the host": update
Bopl Battle in Steam on both machines and start bopl-port again so it
converts the new data).

* **Someone drops out**: the match pauses briefly, then continues without
  them; their bopl stands still for the rest of the round and is gone from
  the next level on. If only one team is left, everybody goes back to the
  lobby.
* **The host leaves**: the room closes.

## Troubleshooting

* **Nothing happens / the window stays black.** Run it from a terminal
  (`./play.sh`) to see messages; `RUST_LOG=info` adds more. On Windows the
  game has no console window: in a command prompt, `set RUST_LOG=info` and
  `bopl-port.exe 2> bopl-port.log`. `bopl-port --list-devices` shows the
  graphics adapters, controllers and audio devices the game sees. Update your
  graphics driver; force a backend with `WGPU_BACKEND=vulkan`, `gl` (Linux)
  or `dx12` (Windows).
* **"needs the SDL2 library" (Linux).** Install `libsdl2-2.0-0` (Debian,
  Ubuntu, Mint), `SDL2` (Fedora), `sdl2` (Arch).
* **Bopl Battle is not found.** See First start; the folder must contain
  `BoplBattle_Data/globalgamemanagers`, and Steam must have finished
  installing the game.
* **The import was interrupted or looks wrong.** Delete the `import-v*`
  folder in the data folder and start again (`--import --force` does the
  same).
* **No sound.** Check the volumes in Settings; on Linux the game uses SDL's
  audio (PulseAudio, PipeWire, ALSA); `--list-devices` lists the playback
  devices. Without any audio device the game runs silently; if the device
  goes away while playing (headset unplugged) it switches to the default
  one.
* **A controller is not recognised.** `--list-devices` lists joysticks
  without a controller mapping; add a line for it to `gamecontrollerdb.txt`
  next to the program.
* **Cannot connect online.** Behind strict NATs the relay is used
  automatically, which adds latency; allow bopl-port in the Windows firewall
  prompt. Everybody must run the same bopl-port version and Bopl Battle
  build; the main menu shows why a join was refused.
* **My controls are wrong.** Rebind them in character select, or delete the
  `bindings` entry in `settings.json` for the defaults.

## Report problems

Open an issue at <https://github.com/jarvis-jhk/bopl-port/issues> with the
version (`bopl-port --version` or the archive name), your system and graphics
driver, and the output of `bopl-port --list-devices`. Please do not attach
game files, and only screenshots that are needed to show the problem.

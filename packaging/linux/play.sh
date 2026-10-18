#!/bin/sh
# Starts bopl-port. Add this file to Steam as a non-Steam game (Steam Deck,
# Big Picture) or run it from a terminal / the Warpzone launcher. Every
# argument is passed on to the game.
#
# If a lib/ directory sits next to this script (a package built with
# --bundle-sdl), its libraries are tried before the system's.
here=$(dirname "$(readlink -f "$0")")
if [ -d "$here/lib" ]; then
    LD_LIBRARY_PATH="$here/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export LD_LIBRARY_PATH
fi

# A missing SDL2 would otherwise only show up as a dynamic-linker line.
if command -v ldd >/dev/null 2>&1 && ldd "$here/bopl-port" 2>/dev/null | grep -q 'libSDL2.* not found'; then
    cat >&2 <<'MSG'
bopl-port needs the SDL2 library (libSDL2-2.0.so.0), which this system lacks.
Install it with your package manager, for example:
    Debian / Ubuntu / Mint:  sudo apt install libsdl2-2.0-0
    Fedora:                  sudo dnf install SDL2
    Arch / SteamOS:          sudo pacman -S sdl2   (SteamOS ships it already)
MSG
    exit 1
fi

exec "$here/bopl-port" "$@"

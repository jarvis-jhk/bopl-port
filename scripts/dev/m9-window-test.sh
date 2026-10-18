#!/bin/bash
# Windowed robustness on the private Xvfb :66: resize, fullscreen toggle,
# controller hot-plug during a match, (optionally) no audio device.
# Usage: scripts/dev/m9-window-test.sh [audio-driver=disk] [out=scratch/m9/polish/win]
set -u
R="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$R"
source env.sh
AUD=${1:-disk}
OUT=${2:-scratch/m9/polish/win}
mkdir -p "$OUT"
D=$(scripts/dev/xvfb.sh :66)
DISPLAY=$D xdotool mousemove 1919 1079
PADS=$OUT/pads.cmd
: > "$PADS"
LOG=$OUT/game.log
shot() { DISPLAY=$D import -window root "$OUT/$1.png" 2>/dev/null; echo "shot $1"; }
pad() { echo "$*" >> "$PADS"; }
env -u WAYLAND_DISPLAY DISPLAY=$D SDL_VIDEODRIVER=x11 SDL_AUDIODRIVER=$AUD \
    SDL_DISKAUDIOFILE=$OUT/sdl-audio.raw RUST_LOG=info \
    target/release/bopl-port --windowed --size 1280x720 --virtual-pads "$PADS" \
    --quit-after 110 --screenshot "$OUT/final.png" > "$LOG" 2>&1 &
G=$!
sleep 8
WIN=$(DISPLAY=$D xdotool search --name "bopl-port" | head -1)
echo "window $WIN"
pad attach 2
sleep 2
shot 01-menu
pad tap 0 a; sleep 2
pad tap 0 a; sleep 0.5; pad tap 1 a; sleep 1.5
shot 02-joined
pad tap 0 start; sleep 0.5; pad tap 1 start; sleep 2.5
pad tap 0 start; sleep 5
shot 03-match
# resize the window (no WM: the X server applies it)
DISPLAY=$D xdotool windowsize "$WIN" 960 600; sleep 2
shot 04-resized-960x600
DISPLAY=$D xdotool windowsize "$WIN" 1600 900; sleep 2
shot 05-resized-1600x900
# fullscreen toggle (F11) and back
DISPLAY=$D xdotool windowfocus --sync "$WIN" 2>/dev/null
DISPLAY=$D xdotool key --clearmodifiers F11; sleep 3
shot 06-fullscreen
DISPLAY=$D xdotool key --clearmodifiers F11; sleep 3
shot 07-windowed-again
# hot-plug: unplug pad 0 mid-match, plug a new one, press a button
pad axis 1 lx 1; sleep 1; pad center 1
pad detach 0; sleep 3
shot 08-pad0-unplugged
pad attach 1; sleep 1
pad tap 2 a; sleep 1
pad axis 2 lx -1; sleep 1.5; pad center 2
shot 09-new-pad-plays
sleep 3
pad tap 2 start; sleep 2
shot 10-pause-from-new-pad
pad tap 2 b; sleep 1
wait $G
echo "exit $?"
grep -E "display|controller|continues|audio|fps|ticks/s|window:|WARN|ERROR|health" "$LOG" | tail -40

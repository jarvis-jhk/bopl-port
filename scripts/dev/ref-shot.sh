#!/bin/bash
# Drive the original game running on the private Xvfb (scripts/dev/ref-run.sh)
# and take screenshots. Commands run left to right:
#   shot NAME          screenshot of the whole screen -> scratch/ref/NAME.png
#   key KEY...         tap X keysyms (Return, space, Escape, Up, w, a, ...)
#   down KEY / up KEY  hold / release a key
#   hold KEY SECONDS   hold a key for a while
#   click X Y [BTN]    move the mouse to X,Y (screen pixels) and click (1=left, 3=right)
#   move X Y           move the mouse
#   sleep SECONDS      wait
#   focus              focus the game window (done automatically first)
#   rec NAME SECONDS   record the screen with ffmpeg -> scratch/ref/NAME.mp4 (blocking)
# e.g. scripts/dev/ref-shot.sh click 960 155 sleep 3 shot 02-charsel
# Env: REF_DISPLAY (:66), REF_OUT (scratch/ref), REF_FPS (30, for rec),
#      REF_WAIT (60: seconds to wait for the game window)
set -euo pipefail
R="$(cd "$(dirname "$0")/../.." && pwd)"
export DISPLAY=${REF_DISPLAY:-:66}
OUT=${REF_OUT:-"$R/scratch/ref"}
mkdir -p "$OUT"
unset WAYLAND_DISPLAY

win() { xdotool search --name '^BoplBattle$' 2>/dev/null | head -1; }
focus() {
  local w i
  for i in $(seq "${REF_WAIT:-60}"); do w=$(win); [ -n "$w" ] && break; sleep 1; done
  [ -n "$w" ] || { echo "game window not found on $DISPLAY" >&2; exit 1; }
  xdotool windowactivate --sync "$w" 2>/dev/null || xdotool windowfocus --sync "$w"
}
# Unity polls input once per frame: keep keys down for a few frames.
tap() { xdotool keydown "$1"; sleep 0.15; xdotool keyup "$1"; sleep 0.15; }

focus
while [ $# -gt 0 ]; do
  c=$1; shift
  case "$c" in
    shot)  import -window root "$OUT/$1.png"; echo "$OUT/$1.png"; shift ;;
    key)   while [ $# -gt 0 ] && ! [[ "$1" =~ ^(shot|key|down|up|hold|click|move|sleep|focus|rec)$ ]]; do
             tap "$1"; shift; done ;;
    down)  xdotool keydown "$1"; shift ;;
    up)    xdotool keyup "$1"; shift ;;
    hold)  xdotool keydown "$1"; sleep "$2"; xdotool keyup "$1"; shift 2 ;;
    click) b=1; if [[ "${3:-}" =~ ^[1-9]$ ]]; then b=$3; fi
           xdotool mousemove "$1" "$2"; sleep 0.3; xdotool mousemove_relative 1 0; sleep 0.3
           xdotool mousedown $b; sleep 0.2; xdotool mouseup $b; sleep 0.1
           shift 2; [ $b = 1 ] && [[ "${1:-}" != 1 ]] || shift ;;
    move)  xdotool mousemove "$1" "$2"; shift 2 ;;
    sleep) sleep "$1"; shift ;;
    focus) focus ;;
    rec)   ffmpeg -loglevel error -y -f x11grab -framerate "${REF_FPS:-30}" -video_size \
             "$(xdpyinfo | awk '/dimensions:/{print $2}')" -i "$DISPLAY" -t "$2" \
             -c:v libx264 -preset veryfast -pix_fmt yuv420p "$OUT/$1.mp4"
           echo "$OUT/$1.mp4"; shift 2 ;;
    *) echo "unknown command: $c" >&2; exit 2 ;;
  esac
done

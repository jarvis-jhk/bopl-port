#!/bin/bash
# Play a .rep in the ORIGINAL (headless, private Xvfb :66, Steam mode: the
# replay viewer needs the Steam API) and record the screen with ffmpeg.
#
#   scripts/dev/golden/capture.sh FILE.rep OUT.mkv [SECONDS]
#
# SECONDS defaults to frames/60 + 40 (start-up + fly-in margin). Recording
# starts as soon as the game window exists. Output stays in scratch/.
# Env: GOLDEN_FPS (60), REF_STEAM (1), REF_SIZE (1920x1080), GOLDEN_MAX_LOAD (7:
# wait for the 1-minute load average to drop below it before starting).
set -euo pipefail
R="$(cd "$(dirname "$0")/../../.." && pwd)"
source "$R/env.sh"
REP=$1; OUT=$2
[ -f "$REP" ] || { echo "no replay $REP" >&2; exit 1; }
if [ -n "${3:-}" ]; then SECS=$3; else
  # frames = 32 per block; the block count is not in the header, so ask sim-trace's reader.
  N=$("$R/target/release/golden" frames "$REP")
  SECS=$(( N / 60 + 40 ))
fi
export REF_STEAM=${REF_STEAM:-1}
export DISPLAY=${REF_DISPLAY:-:66}
SIZE=${REF_SIZE:-1920x1080}
"$R/scripts/dev/ref-run.sh" --stop >/dev/null 2>&1 || true
# The replay viewer falls behind real time when the original renders below
# 15 fps (CPU contention); wait (up to 15 min) for a calmer machine.
for _ in $(seq 180); do
  awk -v m="${GOLDEN_MAX_LOAD:-7}" '{exit !($1 < m)}' /proc/loadavg && break; sleep 5
done
"$R/scripts/dev/ref-run.sh" --bg "$REP"
for _ in $(seq 120); do
  xdotool search --name '^BoplBattle$' >/dev/null 2>&1 && break; sleep 0.25
done
mkdir -p "$(dirname "$OUT")"
nice -n 5 ffmpeg -loglevel error -y -f x11grab -framerate "${GOLDEN_FPS:-60}" -video_size "$SIZE" \
  -i "$DISPLAY" -t "$SECS" -c:v libx264 -preset ultrafast -crf 12 -pix_fmt yuv444p "$OUT" || true
"$R/scripts/dev/ref-run.sh" --stop >/dev/null 2>&1 || true
echo "$OUT"

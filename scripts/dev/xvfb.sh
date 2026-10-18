#!/bin/bash
# Start a private headless X server (Xvfb unpacked into tools/ by apt-get download + dpkg -x).
# Usage: scripts/dev/xvfb.sh [display=:66] [WxHxD=1920x1080x24]; prints the DISPLAY.
set -euo pipefail
R="$(cd "$(dirname "$0")/../.." && pwd)"
D=${1:-:66}; S=${2:-1920x1080x24}
X="$R/tools/xvfb-root/usr/bin/Xvfb"
if [ ! -x "$X" ]; then
  mkdir -p "$R/tools/debs" && (cd "$R/tools/debs" && apt-get download xvfb >/dev/null)
  dpkg -x "$R"/tools/debs/xvfb_*.deb "$R/tools/xvfb-root"
fi
if ! DISPLAY=$D xdpyinfo >/dev/null 2>&1; then
  nohup "$X" "$D" -screen 0 "$S" -nolisten tcp >"$R/scratch/xvfb${D#:}.log" 2>&1 &
  for _ in $(seq 50); do DISPLAY=$D xdpyinfo >/dev/null 2>&1 && break; sleep 0.1; done
fi
echo "$D"

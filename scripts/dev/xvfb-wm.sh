#!/bin/bash
# Run metacity on the private Xvfb (default :66) so fullscreen/maximize behave
# like on a desktop. Everything it might write stays in tools/wm-home; no
# session D-Bus, in-memory GSettings. Usage: scripts/dev/xvfb-wm.sh [display]
set -euo pipefail
R="$(cd "$(dirname "$0")/../.." && pwd)"
D=${1:-:66}
H="$R/tools/wm-home"; mkdir -p "$H"/{config,cache,data,run}; chmod 700 "$H/run"
if ! pgrep -f "metacity.*--display=$D" >/dev/null; then
  env -i PATH=/usr/bin:/bin HOME="$H" XDG_CONFIG_HOME="$H/config" XDG_CACHE_HOME="$H/cache" \
    XDG_DATA_HOME="$H/data" XDG_RUNTIME_DIR="$H/run" GSETTINGS_BACKEND=memory \
    DBUS_SESSION_BUS_ADDRESS=disabled: NO_AT_BRIDGE=1 DISPLAY="$D" \
    nohup metacity --display="$D" --replace >"$R/scratch/metacity${D#:}.log" 2>&1 &
  sleep 1
fi
pgrep -fa "metacity.*--display=$D" | head -1

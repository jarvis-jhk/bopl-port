#!/bin/bash
# Run the ORIGINAL Bopl Battle (Windows Unity build) under Steam's Proton,
# headless on the private Xvfb, for reference screenshots and replays.
#
#   scripts/dev/ref-run.sh [--bg] [game args...]   start (foreground, or --bg)
#   scripts/dev/ref-run.sh --stop                  kill the game + wineserver
#   scripts/dev/ref-run.sh --status                is it running? window id
#   scripts/dev/ref-run.sh --wine <cmd...>         run e.g. `reg query` in the prefix
#   e.g. scripts/dev/ref-run.sh --bg scratch/ref/replays/1.rep   (plays a replay)
#
# Safety (CLAUDE.md): nothing outside the project is written.
#   - The game runs from a plain copy in tools/bopl-original (cp, no hardlinks;
#     the Steam install is only read once, by that copy).
#   - Proton's prefix (STEAM_COMPAT_DATA_PATH) is tools/bopl-prefix.
#   - Everything runs inside bubblewrap with / mounted read-only and only the
#     project bound read-write; /tmp is private (tools/bopl-tmp) and Proton's
#     dist.lock is replaced by a private file, so Proton cannot touch its own
#     install.  /dev/shm stays shared: the Steam API talks to the running Steam
#     client through shared memory there (+ the ~/.steam/steam.pipe FIFO).
#   - HOME is tools/bopl-home, whose .steam is a symlink to the real (read-only
#     here) ~/.steam so lsteamclient finds sdk64/steamclient.so.
#     Side effect outside our control: the Steam client itself creates an
#     empty steamapps/compatdata/1686940/ when it sees the app running, counts
#     playtime, and the game opens a (friends-joinable) Steam lobby in the menu.
#     REF_STEAM=0 hides ~/.steam: no Steam contact at all, but then the game
#     runs in demo mode (DLC check = SteamClient.AppId) with fewer abilities.
#   - DISPLAY is always the private Xvfb (:66); audio drivers are disabled.
# Rendering: D3D11 via DXVK on the real GPU; presenting to Xvfb needs Mesa's
# software WSI path (MESA_VK_WSI_DEBUG=sw), otherwise the X connection dies
# right after swapchain creation ("XIO: fatal IO error 2"). REF_GL=1 uses
# Unity's OpenGL renderer instead (llvmpipe, ~4-5 cores, slow).
# Env: REF_DISPLAY (:66), REF_SIZE (1920x1080), REF_STEAM (0), REF_GL (0),
#      REF_UNITY_ARGS (extra player args), PROTON_DIR (Proton - Experimental),
#      WINEDEBUG (-all), PROTON_LOG (unset), REF_WRAP (e.g. strace ...),
#      REF_LOG (scratch/ref/logs/run-<time>.log)
set -euo pipefail
R="$(cd "$(dirname "$0")/../.." && pwd)"
source "$R/env.sh"

APPID=1686940
STEAM_ROOT="$(readlink -f "$HOME/.steam/steam")"
PROTON_DIR=${PROTON_DIR:-"$STEAM_ROOT/steamapps/common/Proton - Experimental"}
GAME="$R/tools/bopl-original"
COMPAT="$R/tools/bopl-prefix"
T="$R/tools"
D=${REF_DISPLAY:-:66}
SIZE=${REF_SIZE:-1920x1080}
W=${SIZE%x*}; H=${SIZE#*x}
LOGDIR="$R/scratch/ref/logs"
STEAM=${REF_STEAM:-0}
if [ "$STEAM" = 1 ]; then BHOME="$T/bopl-home"; else BHOME="$T/bopl-home-offline"; fi
mkdir -p "$LOGDIR" "$COMPAT" "$T/bopl-tmp" "$BHOME" "$T/bopl-xdg/config" \
  "$T/bopl-xdg/data" "$T/bopl-xdg/cache" "$T/bopl-xdg/run"
chmod 700 "$T/bopl-xdg/run"
if [ "$STEAM" = 1 ]; then
  [ -e "$BHOME/.steam" ] || ln -s "$HOME/.steam" "$BHOME/.steam"
  SHM=(--dev-bind /dev/shm /dev/shm)   # Steam IPC lives here
else
  SHM=(--tmpfs /dev/shm)
fi
# Offline: steam_api64 must fail to load Proton's steamclient64.dll, otherwise
# lsteamclient hits an assert (modal dialog, black window).
NOSTEAM_DLL=""; [ "$STEAM" = 1 ] || NOSTEAM_DLL=";steamclient64=d;steamclient=d"
: >>"$T/bopl-proton-dist.lock"

[ -f "$PROTON_DIR/proton" ] || { echo "Proton not found: $PROTON_DIR" >&2; exit 1; }

# One-time plain copy of the install (never hardlinks into the Steam folder).
if [ ! -f "$GAME/.copied" ]; then
  [ -f "$BOPL_GAME_DIR/BoplBattle.exe" ] || { echo "no game at $BOPL_GAME_DIR" >&2; exit 1; }
  echo "copying $BOPL_GAME_DIR -> $GAME" >&2
  mkdir -p "$GAME" && cp -a --no-preserve=links "$BOPL_GAME_DIR/." "$GAME/" && date >"$GAME/.copied"
fi
# Facepunch.Steamworks sets SteamAppId itself; the file is a harmless backup.
[ -f "$GAME/steam_appid.txt" ] || echo $APPID >"$GAME/steam_appid.txt"

sandbox() {
  bwrap \
    --ro-bind / / \
    --dev-bind /dev /dev \
    "${SHM[@]}" \
    --proc /proc \
    --bind "$R" "$R" \
    --bind "$T/bopl-tmp" /tmp \
    --ro-bind /tmp/.X11-unix /tmp/.X11-unix \
    --bind "$T/bopl-proton-dist.lock" "$PROTON_DIR/dist.lock" \
    --unsetenv WAYLAND_DISPLAY --unsetenv DBUS_SESSION_BUS_ADDRESS \
    --setenv DISPLAY "$D" \
    --setenv HOME "$BHOME" \
    --setenv WINEPREFIX "$COMPAT/pfx" \
    --setenv XDG_CONFIG_HOME "$T/bopl-xdg/config" \
    --setenv XDG_DATA_HOME "$T/bopl-xdg/data" \
    --setenv XDG_CACHE_HOME "$T/bopl-xdg/cache" \
    --setenv XDG_RUNTIME_DIR "$T/bopl-xdg/run" \
    --setenv MESA_SHADER_CACHE_DIR "$T/bopl-xdg/cache/mesa" \
    --setenv STEAM_COMPAT_CLIENT_INSTALL_PATH "$STEAM_ROOT" \
    --setenv STEAM_COMPAT_DATA_PATH "$COMPAT" \
    --setenv STEAM_COMPAT_INSTALL_PATH "$GAME" \
    --setenv STEAM_COMPAT_APP_ID $APPID \
    --setenv SteamAppId $APPID \
    --setenv SteamGameId $APPID \
    --setenv PROTON_LOG_DIR "$LOGDIR" \
    ${PROTON_LOG:+--setenv PROTON_LOG "$PROTON_LOG"} \
    --setenv WINEDEBUG "${WINEDEBUG:--all}" \
    --setenv WINEDLLOVERRIDES "winemenubuilder.exe=d;winepulse.drv=d;winealsa.drv=d$NOSTEAM_DLL" \
    --setenv PULSE_SERVER none \
    --setenv PROTON_USE_XALIA 0 \
    --setenv STEAM_COMPAT_SHADER_PATH "$T/bopl-xdg/cache/shadercache" \
    --setenv DXVK_LOG_PATH "$LOGDIR" \
    --setenv DXVK_STATE_CACHE_PATH "$T/bopl-xdg/cache" \
    --setenv MESA_VK_WSI_DEBUG "${MESA_VK_WSI_DEBUG:-sw}" \
    --chdir "$GAME" \
    "$@"
}

game_pids() { pgrep -f 'Z:\\.*BoplBattle\.exe|proton run .*/BoplBattle\.exe' || true; }

case "${1:-}" in
  --stop)
    sandbox "$PROTON_DIR/files/bin/wineserver" -k 2>/dev/null || true
    sleep 1; p=$(game_pids); [ -n "$p" ] && kill $p 2>/dev/null || true
    exit 0 ;;
  --status)
    p=$(game_pids); [ -n "$p" ] || { echo "not running"; exit 1; }
    echo "pids: $p"
    DISPLAY=$D xdotool search --name '^BoplBattle$' 2>/dev/null | head -1 || true
    exit 0 ;;
  --wine)
    shift
    sandbox "$PROTON_DIR/files/bin/wine" "$@"
    exit $? ;;
esac

BG=0
if [ "${1:-}" = --bg ]; then BG=1; shift; fi
[ -z "$(game_pids)" ] || { echo "already running (scripts/dev/ref-run.sh --stop)" >&2; exit 1; }

"$R/scripts/dev/xvfb.sh" "$D" "${SIZE}x24" >/dev/null

# Unix paths to replays (*.rep) are passed to the game as Z:\ paths; the game
# then opens its Replay scene instead of the menu.
args=()
for a in "$@"; do
  if [[ "$a" == *.rep && -f "$a" ]]; then a="$(realpath "$a")"; a="Z:${a//\//\\}"; fi
  args+=("$a")
done
set -- "${args[@]+"${args[@]}"}"

LOG=${REF_LOG:-"$LOGDIR/run-$(date +%Y%m%d-%H%M%S).log"}
LOG="$(realpath -m "$LOG")"
ULOG="Z:${LOG//\//\\}.unity"
[ "${REF_GL:-0}" = 1 ] && REF_UNITY_ARGS="-force-glcore ${REF_UNITY_ARGS:-}"
# Proton's steam.exe shim (verb "run") blocks forever without a Steam client;
# offline we start the game directly in the prefix ("runinprefix").
VERB=run; [ "$STEAM" = 1 ] || VERB=runinprefix
cmd=(${REF_WRAP:-} python3 "$PROTON_DIR/proton" $VERB "$GAME/BoplBattle.exe"
     -screen-fullscreen 0 -screen-width "$W" -screen-height "$H" ${REF_UNITY_ARGS:-}
     -logFile "$ULOG" "$@")
echo "log: $LOG (Unity: $LOG.unity)" >&2
if [ $BG = 1 ]; then
  REF_LOG="$LOG" setsid nohup "$0" "$@" >/dev/null 2>&1 </dev/null &
  echo "started in background; display $D" >&2
else
  sandbox "${cmd[@]}" >"$LOG" 2>&1 </dev/null
fi

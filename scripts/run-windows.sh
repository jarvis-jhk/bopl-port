#!/bin/bash
# Runs the cross-compiled bopl-port.exe (or any exe: EXE=...) under Proton's
# Wine, sandboxed so that nothing outside the project directory can be
# written and nothing is contacted:
#   - bubblewrap mounts / read-only (the game install included), the project
#     read-write, gives Wine a private /tmp (tools/wine-tmp; wineserver puts
#     its socket there) and a private in-memory /dev/shm;
#   - its own network namespace (no network at all, so neither Steam nor any
#     server can be reached; BOPL_WINE_NET=1 keeps the network for online
#     tests against scripts/net/dev-server.sh);
#   - HOME/XDG dirs and the Wine prefix (tools/wineprefix, env.sh) live in
#     tools/; Proton's wine binary is run directly (no Steam, no proton
#     script, nothing written into the Proton install).
#
# Display: Wine's Vulkan (winevulkan) needs a user driver, and with Wine's
# "null" graphics driver Vulkan instance creation hangs, so Wine uses the
# X11 driver on the project's private Xvfb (scripts/dev/xvfb.sh, :66), never
# the user's desktop. Rendering then goes through wgpu's Vulkan backend ->
# winevulkan -> the host's Mesa (llvmpipe on Xvfb). Audio: SDL's dummy
# driver unless SDL_AUDIODRIVER says otherwise.
#
# Usage: scripts/run-windows.sh [bopl-port args...]
#   e.g. scripts/run-windows.sh --headless --level 6 --screenshot scratch/win.png
# Env: PROTON_DIR (default: Steam's "Proton - Experimental"), EXE (default:
# target/dist/windows/bopl-port.exe), WINEDEBUG (default -all),
# BOPL_WINE_DISPLAY (default :66). BOPL_PORT_DATA_DIR and BOPL_GAME_DIR
# (env.sh) are passed on as Windows paths (Z:\...).
set -euo pipefail
# env.sh sets the dev data dir; a caller's own BOPL_PORT_DATA_DIR (package
# smoke tests use one in scratch/) wins.
_caller_data_dir=${BOPL_PORT_DATA_DIR:-}
source "$(dirname "$0")/../env.sh"
[ -n "$_caller_data_dir" ] && export BOPL_PORT_DATA_DIR="$_caller_data_dir"
cd "$BOPL_ROOT"

PROTON_DIR=${PROTON_DIR:-"$HOME/.steam/steam/steamapps/common/Proton - Experimental"}
WINE="$PROTON_DIR/files/bin/wine"
EXE=${EXE:-target/dist/windows/bopl-port.exe}
[ -x "$WINE" ] || { echo "Proton wine not found at $WINE" >&2; exit 1; }
[ -f "$EXE" ] || { echo "$EXE missing: run scripts/build-windows.sh" >&2; exit 1; }

DISPLAY_NUM=$(scripts/dev/xvfb.sh "${BOPL_WINE_DISPLAY:-:66}")

T="$BOPL_ROOT/tools"
mkdir -p "$T/wine-home" "$T/wine-tmp" "$T/wine-xdg/config" "$T/wine-xdg/data" "$WINEPREFIX"

# Windows paths (Wine maps / to Z:).
winpath() { printf 'Z:%s' "${1//\//\\}"; }
game_env=(--unsetenv BOPL_GAME_DIR --unsetenv WARPZONE_DATA_BOPL)
[ -d "${BOPL_GAME_DIR:-}" ] && game_env+=(--setenv BOPL_GAME_DIR "$(winpath "$BOPL_GAME_DIR")")
[ -n "${WARPZONE_DATA_BOPL:-}" ] && game_env+=(--setenv WARPZONE_DATA_BOPL "$(winpath "$WARPZONE_DATA_BOPL")")
[ -n "${BOPL_PORT_DATA_DIR:-}" ] && game_env+=(--setenv BOPL_PORT_DATA_DIR "$(winpath "$(readlink -m "$BOPL_PORT_DATA_DIR")")")
net_args=(--unshare-net)
[ "${BOPL_WINE_NET:-0}" = 1 ] && net_args=()

sandbox() {
    bwrap \
        --ro-bind / / \
        --dev-bind /dev /dev \
        --tmpfs /dev/shm \
        --proc /proc \
        "${net_args[@]}" \
        --bind "$BOPL_ROOT" "$BOPL_ROOT" \
        --bind "$T/wine-tmp" /tmp \
        --ro-bind "/tmp/.X11-unix/X${DISPLAY_NUM#:}" "/tmp/.X11-unix/X${DISPLAY_NUM#:}" \
        --setenv DISPLAY "$DISPLAY_NUM" \
        --unsetenv WAYLAND_DISPLAY \
        --setenv HOME "$T/wine-home" \
        --setenv XDG_CONFIG_HOME "$T/wine-xdg/config" \
        --setenv XDG_DATA_HOME "$T/wine-xdg/data" \
        --setenv XDG_CACHE_HOME "$XDG_CACHE_HOME" \
        --setenv WINEPREFIX "$WINEPREFIX" \
        --setenv WINEDLLPATH "$PROTON_DIR/files/lib/vkd3d:$PROTON_DIR/files/lib/wine" \
        --setenv LD_LIBRARY_PATH "$PROTON_DIR/files/lib/x86_64-linux-gnu:$PROTON_DIR/files/lib/i386-linux-gnu" \
        --setenv WINEDLLOVERRIDES "winemenubuilder.exe=d;mscoree=d;mshtml=d;dxgi=n" \
        --setenv WINEDEBUG "${WINEDEBUG:--all}" \
        --setenv SDL_AUDIODRIVER "${SDL_AUDIODRIVER:-dummy}" \
        "${game_env[@]}" \
        --chdir "$BOPL_ROOT" \
        "$WINE" "$@"
}

# First run creates the prefix.
if [ ! -f "$WINEPREFIX/system.reg" ]; then
    sandbox wineboot -u
fi

# wgpu's exe imports dxgi.dll. Wine's own dxgi needs wined3d + vkd3d, which
# Proton only wires up through its launcher; like Proton does for games, use
# DXVK's dxgi.dll (Vulkan-based) inside the prefix.
SYS32="$WINEPREFIX/drive_c/windows/system32"
DXVK_DXGI="$PROTON_DIR/files/lib/wine/dxvk/x86_64-windows/dxgi.dll"
cmp -s "$DXVK_DXGI" "$SYS32/dxgi.dll" || cp -f "$DXVK_DXGI" "$SYS32/dxgi.dll"

sandbox "$EXE" "$@"

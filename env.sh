# Source this: keeps every toolchain and cache inside the project directory.
R="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export BOPL_ROOT="$R"
export RUSTUP_HOME="$R/tools/rustup" CARGO_HOME="$R/tools/cargo"
export DOTNET_ROOT="$R/tools/dotnet" DOTNET_CLI_HOME="$R/tools/dotnet-home"
export NUGET_PACKAGES="$R/tools/nuget" DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 DOTNET_ROLL_FORWARD=Major
export npm_config_cache="$R/tools/npm-cache" PIP_CACHE_DIR="$R/tools/pip-cache"
export XDG_CACHE_HOME="$R/tools/xdg-cache" WINEPREFIX="$R/tools/wineprefix"
export TMPDIR="$R/scratch/tmp"
mkdir -p "$TMPDIR"
export PATH="$R/tools/cargo/bin:$R/tools/dotnet:$R/tools/dotnet-home/.dotnet/tools:$R/tools/venv/bin:$PATH"
# Location of the original game (read-only; never written to).
# A read-only /data/bopl (a sandboxed build worker) wins over the Steam default.
if [ -z "${BOPL_GAME_DIR:-}" ] && [ -d /data/bopl ]; then BOPL_GAME_DIR=/data/bopl; fi
export BOPL_GAME_DIR="${BOPL_GAME_DIR:-$HOME/.steam/steam/steamapps/common/Bopl Battle}"
# Dev: the port's user data dir (import, settings, Warpzone identity) stays in the project.
export BOPL_PORT_DATA_DIR="$R/cache/userdata"
export WARPZONE_HOME="$R/cache/warpzone"

#!/bin/bash
# Builds dist/bopl-port-<version>-windows-x86_64.zip (and dist/SHA256SUMS)
# with the llvm-mingw cross toolchain (scripts/fetch-windows-toolchain.sh,
# scripts/build-windows.sh).
#
# Layout (exec = "bopl-port-*/bopl-port.exe" in the Warpzone descriptor):
#   bopl-port-<version>/{bopl-port.exe, SDL2.dll, README.txt, LICENSE-*,
#                        THIRD_PARTY_NOTICES.md, licenses/ (incl. SDL2 zlib)}
#
# Usage: scripts/package-windows.sh [--no-build] [--verify]
#   --no-build  reuse target/dist/windows/ from the last scripts/build-windows.sh
#   --verify    unpack the zip into scratch/pkgtest/windows and run it headless
#               under scripts/run-windows.sh (Proton's Wine, sandboxed, no
#               network): --version, --list-devices and, when the game is
#               installed, `--import --game` into a data dir inside scratch/
#               plus a headless level screenshot (compared with the Linux
#               package's, if that one was verified first)
set -euo pipefail
source "$(dirname "$0")/../env.sh"
cd "$BOPL_ROOT"
source scripts/package-common.sh

BUILD=1 VERIFY=0
for a in "$@"; do
    case $a in
        --no-build) BUILD=0 ;;
        --verify) VERIFY=1 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done

VERSION=$(pkg_version)
[ -n "$VERSION" ] || { echo "cannot read the version from Cargo.toml" >&2; exit 1; }
NAME="bopl-port-$VERSION"
OUT="$DIST/$NAME-windows-x86_64.zip"
STAGE="$DIST/.stage-windows"
EPOCH=$(pkg_epoch)

if [ $BUILD = 1 ]; then
    scripts/build-windows.sh >/dev/null   # fetches the toolchain if missing; release profile
fi
SRC=target/dist/windows
[ -f "$SRC/bopl-port.exe" ] && [ -f "$SRC/SDL2.dll" ] || { echo "$SRC incomplete: run scripts/build-windows.sh" >&2; exit 1; }
STRIP=tools/windows/llvm-mingw/bin/llvm-strip

pkg_clean_stage "$STAGE"
PKG="$STAGE/$NAME"
mkdir -p "$PKG"
install -m 755 "$SRC/bopl-port.exe" "$PKG/bopl-port.exe"
"$STRIP" --strip-all "$PKG/bopl-port.exe"
install -m 644 "$SRC/SDL2.dll" "$PKG/SDL2.dll"
# Notepad-friendly line endings for the readme.
sed 's/$/\r/' packaging/windows/README.txt > "$PKG/README.txt"
pkg_stage_docs "$PKG" x86_64-pc-windows-gnu
install -m 644 packaging/licenses/SDL2-LICENSE.txt "$PKG/licenses/SDL2-LICENSE.txt"

mkdir -p "$DIST"
chmod -R go-w "$STAGE"
find "$STAGE" -exec touch -h -d "@$EPOCH" {} +
rm -f "$OUT" "$OUT.part"
(cd "$STAGE" && find "$NAME" | LC_ALL=C sort | zip -X -9 -q "$OUT.part" -@)
mv "$OUT.part" "$OUT"
rm -rf "$STAGE"
scripts/check-no-game-data.sh --archives "$OUT"

if [ $VERIFY = 1 ]; then
    V="$BOPL_ROOT/scratch/pkgtest/windows"
    rm -rf "$V"; mkdir -p "$V"
    unzip -q "$OUT" -d "$V"
    P="$V/$NAME"
    test -f "$P/bopl-port.exe" && test -f "$P/SDL2.dll"
    export EXE="$P/bopl-port.exe"
    # run-windows.sh hands this to the exe as Z:\...\userdata.
    export BOPL_PORT_DATA_DIR="$V/userdata"
    scripts/run-windows.sh --version 2>/dev/null
    scripts/run-windows.sh --list-devices 2>/dev/null | tr -d '\r' | sed -n 1,12p
    if pkg_have_game; then
        g="Z:${BOPL_GAME_DIR//\//\\}"
        scripts/run-windows.sh --import --game "$g" 2>&1 | grep -v '^wineserver' || true
        test -f "$BOPL_PORT_DATA_DIR/import-v1/index.json"
        WGPU_ADAPTER_NAME=${WGPU_ADAPTER_NAME:-llvmpipe} scripts/run-windows.sh \
            --headless --level 6 --size 1280x720 --screenshot "$V/level6.png" 2>/dev/null | tr -d '\r' | grep screenshot || true
        test -s "$V/level6.png" && echo "packaged Windows build: import + headless level screenshot ok ($V/level6.png)"
        L="$BOPL_ROOT/scratch/pkgtest/linux/level6.png"
        if [ -f "$L" ]; then
            if cmp -s "$L" "$V/level6.png"; then
                echo "level screenshot byte-identical to the packaged Linux build's"
            else
                echo "note: level screenshot differs from the packaged Linux build's (different GPU adapter? set WGPU_ADAPTER_NAME=llvmpipe for both)"
            fi
        fi
    else
        echo "Bopl Battle not installed: skipped the import and screenshot test"
    fi
fi

scripts/package-checksums.sh "$DIST" >/dev/null
echo "$OUT ($(human "$OUT"))"
